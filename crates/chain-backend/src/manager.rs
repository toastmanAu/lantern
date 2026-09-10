//! Backend profiles and which one is active.
//!
//! Spec §18 keeps two axes apart: network is which chain, backend is how it
//! is reached. Switching backend within a network leaves accounts untouched.

use std::path::{Path, PathBuf};

use lantern_sdk_schema::{BackendKind, BackendProfile, Network};
use serde::{Deserialize, Serialize};

use crate::backend::ChainBackend;
use crate::backends::{FullNode, RemoteLight};
use crate::error::BackendError;

const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackendsFile {
    version: u32,
    active_profile_id: Option<String>,
    profiles: Vec<BackendProfile>,
}

/// What `activate` resolved to build, before it tears down the outgoing
/// backend. Kept as an owned plan rather than a borrow of the profile so the
/// teardown below does not need to hold `profile` alive across the `await`.
enum ActivationTarget {
    RemoteLight { endpoint: String },
    Full { kind: BackendKind, endpoint: String },
}

/// Owns the backend profiles and whichever one is live.
pub struct BackendManager {
    path: PathBuf,
    profiles: Vec<BackendProfile>,
    active_id: Option<String>,
    active: Option<Box<dyn ChainBackend>>,
}

impl std::fmt::Debug for BackendManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendManager")
            .field("profiles", &self.profiles.len())
            .field("active_id", &self.active_id)
            .finish_non_exhaustive()
    }
}

impl BackendManager {
    /// Open the profile list, or seed spec §6's first-run defaults.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Io`] if the file exists but cannot be read,
    /// and [`BackendError::Corrupt`] if it cannot be parsed or its version
    /// is not the one this build understands.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BackendError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                profiles: Self::defaults(),
                active_id: Some("default-mainnet".to_string()),
                active: None,
            });
        }
        let bytes = std::fs::read(&path)?;
        let file: BackendsFile =
            serde_json::from_slice(&bytes).map_err(|_| BackendError::Corrupt)?;
        if file.version != FILE_VERSION {
            return Err(BackendError::Corrupt);
        }
        Ok(Self {
            path,
            profiles: file.profiles,
            active_id: file.active_profile_id,
            active: None,
        })
    }

    /// Mainnet and testnet, both on the embedded light client.
    fn defaults() -> Vec<BackendProfile> {
        vec![
            BackendProfile {
                id: "default-mainnet".to_string(),
                label: "Mainnet (bundled light client)".to_string(),
                network: Network::Mainnet,
                kind: BackendKind::EmbeddedLight,
                endpoint: None,
            },
            BackendProfile {
                id: "default-testnet".to_string(),
                label: "Testnet (bundled light client)".to_string(),
                network: Network::Testnet,
                kind: BackendKind::EmbeddedLight,
                endpoint: None,
            },
        ]
    }

    pub fn profiles(&self) -> &[BackendProfile] {
        &self.profiles
    }

    /// # Errors
    ///
    /// Returns [`BackendError::DuplicateProfile`] if a profile with the same
    /// id is already registered.
    pub fn add_profile(&mut self, profile: BackendProfile) -> Result<(), BackendError> {
        if self.profiles.iter().any(|p| p.id == profile.id) {
            return Err(BackendError::DuplicateProfile);
        }
        self.profiles.push(profile);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`BackendError::ProfileNotFound`] if no profile has this id,
    /// or whatever error stopping the active backend produces if the
    /// removed profile is the one currently running. On a `stop()` failure
    /// the profile list is left untouched, so a failed graceful shutdown
    /// never silently edits the list out from under a still-running backend.
    pub async fn remove_profile(&mut self, id: &str) -> Result<(), BackendError> {
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or(BackendError::ProfileNotFound)?;
        if self.active_id.as_deref() == Some(id) {
            if let Some(active) = self.active.take() {
                active.stop().await?;
            }
            self.active_id = None;
        }
        self.profiles.remove(index);
        Ok(())
    }

    /// The network of the active profile, defaulting to mainnet.
    pub fn current_network(&self) -> Network {
        self.active_id
            .as_ref()
            .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
            .map_or(Network::Mainnet, |p| p.network)
    }

    pub fn current_backend(&self) -> Option<&dyn ChainBackend> {
        self.active.as_deref()
    }

    /// Stop whatever is running and bring up the named profile.
    ///
    /// `EmbeddedLight` is not constructed here: it needs a binary path and a
    /// data directory, which plan 1f resolves. Activating one without those
    /// is `Unsupported` rather than a silent no-op.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::ProfileNotFound`] if `profile_id` is not
    /// registered, [`BackendError::Unsupported`] for a remote profile
    /// without an endpoint or for the embedded kind, and whatever error the
    /// chosen backend's constructor or `start` produces otherwise.
    pub async fn activate(&mut self, profile_id: &str) -> Result<(), BackendError> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .ok_or(BackendError::ProfileNotFound)?
            .clone();

        // Resolve everything that can fail without I/O first: an embedded
        // profile or a missing endpoint is a caller mistake, not a transport
        // failure, and must not cost the user a working connection just to
        // discover it. `activate_backend` already gets this right for the
        // embedded path; this mirrors it here.
        let target = match profile.kind {
            BackendKind::RemoteLight => ActivationTarget::RemoteLight {
                endpoint: profile.endpoint.clone().ok_or(BackendError::Unsupported(
                    "remote backend without an endpoint",
                ))?,
            },
            BackendKind::LocalFull | BackendKind::RemoteFull => ActivationTarget::Full {
                kind: profile.kind,
                endpoint: profile.endpoint.clone().ok_or(BackendError::Unsupported(
                    "remote backend without an endpoint",
                ))?,
            },
            BackendKind::EmbeddedLight => {
                return Err(BackendError::Unsupported(
                    "embedded light client needs a binary path; construct it directly",
                ));
            }
        };

        // Only now tear down. If construction or `start()` below still
        // fails, the old backend is already stopped — accepted, because that
        // is an I/O failure rather than a caller mistake, and keeping the
        // old backend alive while connecting the new one risks two
        // supervised `ckb-light-client` children running at once, which is
        // worse. Do not "fix" this into a double-spawn.
        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        self.active_id = None;

        let backend: Box<dyn ChainBackend> = match target {
            ActivationTarget::RemoteLight { endpoint } => {
                Box::new(RemoteLight::new(profile.network, endpoint)?)
            }
            ActivationTarget::Full { kind, endpoint } => {
                Box::new(FullNode::connect(profile.network, kind, endpoint).await?)
            }
        };
        backend.start().await?;
        self.active = Some(backend);
        self.active_id = Some(profile.id);
        Ok(())
    }

    /// Adopt an already-constructed backend, for the embedded kind.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::ProfileNotFound`] if `profile_id` is not
    /// registered, or whatever error stopping the previous backend or
    /// starting the new one produces.
    pub async fn activate_backend(
        &mut self,
        profile_id: &str,
        backend: Box<dyn ChainBackend>,
    ) -> Result<(), BackendError> {
        if !self.profiles.iter().any(|p| p.id == profile_id) {
            return Err(BackendError::ProfileNotFound);
        }
        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        backend.start().await?;
        self.active = Some(backend);
        self.active_id = Some(profile_id.to_string());
        Ok(())
    }

    /// # Errors
    ///
    /// Returns whatever error stopping the active backend produces.
    pub async fn shutdown(&mut self) -> Result<(), BackendError> {
        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        Ok(())
    }

    /// Write `backends.json` via a temporary file and a rename.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Corrupt`] if the profiles cannot be
    /// serialised, and [`BackendError::Io`] if the write or rename fails.
    pub fn save(&self) -> Result<(), BackendError> {
        let file = BackendsFile {
            version: FILE_VERSION,
            active_profile_id: self.active_id.clone(),
            profiles: self.profiles.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|_| BackendError::Corrupt)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{BackendKind, BackendProfile, Network};
    use serde_json::json;
    use tempfile::tempdir;

    use super::BackendManager;
    use crate::error::BackendError;
    use crate::testing::FakeNode;

    fn profile(id: &str, network: Network, kind: BackendKind) -> BackendProfile {
        BackendProfile {
            id: id.to_string(),
            label: id.to_string(),
            network,
            kind,
            endpoint: Some("http://127.0.0.1:8114/".to_string()),
        }
    }

    #[test]
    fn a_missing_file_yields_the_first_run_defaults() {
        let dir = tempdir().expect("tempdir");
        let manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        assert_eq!(manager.profiles().len(), 2, "one per network");
        assert_eq!(manager.current_network(), Network::Mainnet);
        assert!(
            manager
                .profiles()
                .iter()
                .all(|p| p.kind == BackendKind::EmbeddedLight),
            "spec §6: the embedded light client is the default"
        );
    }

    #[test]
    fn profiles_round_trip_through_disk() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("backends.json");
        {
            let mut manager = BackendManager::open(&path).expect("opens");
            manager
                .add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull))
                .expect("adds");
            manager.save().expect("saves");
        }
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.contains("\"remote_full\""), "{text}");

        let manager = BackendManager::open(&path).expect("reopens");
        assert!(manager.profiles().iter().any(|p| p.id == "pi"));
    }

    #[tokio::test]
    async fn duplicate_and_missing_profile_ids_are_rejected() {
        let dir = tempdir().expect("tempdir");
        let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        manager
            .add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull))
            .expect("adds");
        assert!(matches!(
            manager.add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull)),
            Err(BackendError::DuplicateProfile)
        ));
        assert!(matches!(
            manager.remove_profile("nope").await,
            Err(BackendError::ProfileNotFound)
        ));
    }

    #[test]
    fn a_corrupt_or_future_file_is_refused_not_replaced() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("backends.json");

        let corrupt = b"{ not json".to_vec();
        std::fs::write(&path, &corrupt).expect("writes");
        assert!(matches!(
            BackendManager::open(&path),
            Err(BackendError::Corrupt)
        ));
        assert_eq!(
            std::fs::read(&path).expect("reads"),
            corrupt,
            "a failed open must not touch the file on disk"
        );

        let future_version = br#"{"version": 2, "activeProfileId": "x", "profiles": []}"#.to_vec();
        std::fs::write(&path, &future_version).expect("writes");
        assert!(matches!(
            BackendManager::open(&path),
            Err(BackendError::Corrupt)
        ));
        assert_eq!(
            std::fs::read(&path).expect("reads"),
            future_version,
            "a failed open must not touch the file on disk"
        );
    }

    #[tokio::test]
    async fn activating_an_unreachable_profile_fails_and_leaves_nothing_connected() {
        let dir = tempdir().expect("tempdir");
        let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        // Point at a dead port: activation of a remote backend must fail
        // loudly rather than leaving a half-switched manager.
        manager
            .add_profile(BackendProfile {
                endpoint: Some("http://127.0.0.1:1/".to_string()),
                ..profile("dead", Network::Testnet, BackendKind::RemoteFull)
            })
            .expect("adds");
        assert!(manager.activate("dead").await.is_err());
        assert!(
            manager.current_backend().is_none(),
            "a failed activation leaves nothing active"
        );
    }

    #[tokio::test]
    async fn activating_a_profile_switches_the_active_network() {
        let dir = tempdir().expect("tempdir");
        let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        assert_eq!(
            manager.current_network(),
            Network::Mainnet,
            "default-mainnet is active before any activation"
        );

        let node = FakeNode::builder()
            .respond(
                "local_node_info",
                json!({
                    "version": "0.5.5",
                    "node_id": "QmTestNode",
                    "active": true,
                    "addresses": [],
                    "protocols": [],
                    "connections": "0x0"
                }),
            )
            .start()
            .await;
        manager
            .add_profile(BackendProfile {
                endpoint: Some(node.url()),
                ..profile("remote-testnet", Network::Testnet, BackendKind::RemoteLight)
            })
            .expect("adds");

        manager
            .activate("remote-testnet")
            .await
            .expect("activation succeeds against a reachable fake node");

        assert!(
            manager.current_backend().is_some(),
            "a successful activation leaves a backend connected"
        );
        assert_eq!(
            manager.current_network(),
            Network::Testnet,
            "activation switches the active network to the new profile's"
        );
    }
}

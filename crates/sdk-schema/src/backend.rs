//! Chain-backend types that cross the IPC boundary.
//!
//! A backend answers "how do we reach the chain", which spec §18 keeps
//! deliberately separate from "which chain" (`Network`). Accounts are scoped
//! to a network; backends are swappable within one.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::types::Network;

/// How a backend reaches the chain.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
#[specta(rename_all = "snake_case")]
pub enum BackendKind {
    /// A `ckb-light-client` subprocess Lantern owns.
    EmbeddedLight,
    /// Someone else's light client, reached over RPC.
    RemoteLight,
    /// A full node on this machine.
    LocalFull,
    /// A full node elsewhere.
    RemoteFull,
}

impl BackendKind {
    /// Light backends sync only the scripts they are told to watch.
    pub const fn is_light(self) -> bool {
        matches!(self, Self::EmbeddedLight | Self::RemoteLight)
    }

    /// Only the embedded kind has a process whose lifetime Lantern owns.
    pub const fn is_supervised(self) -> bool {
        matches!(self, Self::EmbeddedLight)
    }
}

/// What the active backend can actually do. Data, not assumptions: the UI
/// disables features it lacks rather than failing at call time (spec §6).
///
/// Four independent capability flags, not mutually exclusive states, so a
/// state machine or enum split does not fit; the wire shape here is fixed
/// by spec §6 and consumed as booleans by the frontend.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct BackendCapabilities {
    /// Light clients sync nothing until scripts are registered.
    pub needs_script_registration: bool,
    /// Full nodes can serve arbitrary blocks; light clients cannot.
    pub can_fetch_arbitrary_blocks: bool,
    pub can_estimate_cycles: bool,
    /// A full node with the indexer switched off cannot answer `get_cells`.
    pub indexer_available: bool,
}

/// Where a backend is in its lifecycle.
///
/// Block heights are `u64` and export as TypeScript `number`. That is safe:
/// CKB heights are ~2.2e7 against a 9.007e15 safe-integer limit. **Monetary
/// values are not safe this way** — 33.6 billion CKB in shannons is 3.36e18 —
/// so any capacity added later must use a string newtype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "state")]
#[specta(rename_all = "camelCase")]
pub enum BackendStatus {
    /// Starting up, or waiting for the first RPC response.
    Connecting,
    /// Reachable, still catching up. For light backends `current` is filter
    /// sync, which lags header sync and is what the user cares about.
    Syncing {
        current: u64,
        target: u64,
    },
    Synced {
        tip: u64,
    },
    Error {
        message: String,
    },
    Stopped,
}

impl BackendStatus {
    /// Whether queries can be trusted to return complete results.
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Synced { .. } | Self::Syncing { .. })
    }
}

/// Spec §18's `(network × backend × name)` tuple.
///
/// Named "backend profile" deliberately: plan 1c already uses "profile" for
/// the directory holding `vault.bin`, and one word with two meanings will
/// eventually be read the wrong way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct BackendProfile {
    pub id: String,
    pub label: String,
    pub network: Network,
    pub kind: BackendKind,
    /// `None` for `EmbeddedLight`, whose port is allocated at spawn time.
    pub endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{BackendKind, BackendStatus};

    #[test]
    fn kind_predicates_split_light_from_full_and_supervised_from_not() {
        assert!(BackendKind::EmbeddedLight.is_light());
        assert!(BackendKind::RemoteLight.is_light());
        assert!(!BackendKind::LocalFull.is_light());
        assert!(!BackendKind::RemoteFull.is_light());
        assert!(BackendKind::EmbeddedLight.is_supervised());
        assert!(!BackendKind::RemoteLight.is_supervised());
    }

    #[test]
    fn kind_serialises_as_snake_case() {
        let json = serde_json::to_string(&BackendKind::EmbeddedLight).expect("serialises");
        assert_eq!(json, "\"embedded_light\"");
        let json = serde_json::to_string(&BackendKind::RemoteFull).expect("serialises");
        assert_eq!(json, "\"remote_full\"");
    }

    #[test]
    fn status_is_a_tagged_union_and_only_reachable_states_are_usable() {
        let json = serde_json::to_string(&BackendStatus::Syncing {
            current: 10,
            target: 20,
        })
        .expect("serialises");
        assert!(json.contains("\"state\":\"syncing\""), "{json}");
        assert!(json.contains("\"current\":10"), "{json}");

        assert!(BackendStatus::Synced { tip: 1 }.is_usable());
        assert!(
            BackendStatus::Syncing {
                current: 1,
                target: 2
            }
            .is_usable()
        );
        assert!(!BackendStatus::Connecting.is_usable());
        assert!(!BackendStatus::Stopped.is_usable());
        assert!(
            !BackendStatus::Error {
                message: "x".into()
            }
            .is_usable()
        );
    }
}

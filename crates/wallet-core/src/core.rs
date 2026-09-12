//! The orchestrator.
//!
//! Owns the unlocked vault, the account registry, the lock modules, and
//! the active network. `SigningCoordinator` is the one signing entry
//! point (spec §7); it dispatches through `LockModule` and never names a
//! scheme. Synchronous and per-digest in plan 1c; plan 1e's Task 8 makes
//! it async and request-shaped, matching `LockModule::sign`. Task 14
//! ("`wallet-core` sends") replaces `SigningCoordinator::sign` with
//! `WalletCore::send`, which builds the request itself.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lantern_account_registry::{
    AccountRegistry, StoredAccount, WalletOrigin, account_id, to_record,
};
use lantern_chain_backend::{
    BackendError, BackendManager, ChainBackend, H256, JsonBytes, Script, ScriptHashType,
    WatchedScript,
};
use lantern_sdk_schema::{
    AccountRecord, Derivation, LockType, Network, SignedWitness, SigningRequest,
};
use lantern_vault::{ExposeSecret, Vault};

use crate::error::CoreError;
use crate::keyring::Keyring;
use crate::locks::LockRegistry;
use crate::mnemonic::{MnemonicFormat, Phrase, WordCount};

/// Where a profile keeps its two files. Resolving the OS data directory
/// is the Tauri shell's job (plan 1f).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    pub vault: PathBuf,
    pub accounts: PathBuf,
}

impl ProfilePaths {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            vault: dir.join("vault.bin"),
            accounts: dir.join("accounts.json"),
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An unlocked profile.
pub struct WalletCore {
    vault: Vault,
    accounts: AccountRegistry,
    locks: LockRegistry,
    network: Network,
    paths: ProfilePaths,
    backend: Option<BackendManager>,
}

impl std::fmt::Debug for WalletCore {
    /// Opaque by design: the vault and account registry carry key-adjacent
    /// state that must never round-trip through `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletCore").finish_non_exhaustive()
    }
}

impl WalletCore {
    /// Create a new profile. Refuses to overwrite an existing vault or
    /// account file. Returns the phrase exactly once for the
    /// show-and-confirm flow.
    pub fn create(
        paths: ProfilePaths,
        password: &[u8],
        network: Network,
        words: WordCount,
    ) -> Result<(Self, Phrase), CoreError> {
        if paths.vault.exists() || paths.accounts.exists() {
            return Err(CoreError::AlreadyInitialised);
        }
        let mut vault = Vault::create(&paths.vault, password)?;
        let phrase = Keyring::create(&mut vault, words)?;
        let mut accounts = AccountRegistry::open(&paths.accounts)?;
        accounts.set_origin(WalletOrigin::Created);
        accounts.save()?;
        Ok((
            Self {
                vault,
                accounts,
                locks: LockRegistry::with_first_party(),
                network,
                paths,
                backend: None,
            },
            phrase,
        ))
    }

    /// Create a new profile from an existing phrase (standard or
    /// combined). Refuses to overwrite an existing vault or account file.
    pub fn import(
        paths: ProfilePaths,
        password: &[u8],
        network: Network,
        phrase: &str,
    ) -> Result<Self, CoreError> {
        if paths.vault.exists() || paths.accounts.exists() {
            return Err(CoreError::AlreadyInitialised);
        }
        // Validate before creating the vault so a typo never leaves an orphan file.
        crate::mnemonic::parse_phrase(phrase)?;
        let mut vault = Vault::create(&paths.vault, password)?;
        Keyring::import(&mut vault, phrase)?;
        let mut accounts = AccountRegistry::open(&paths.accounts)?;
        accounts.set_origin(WalletOrigin::Imported);
        accounts.save()?;
        Ok(Self {
            vault,
            accounts,
            locks: LockRegistry::with_first_party(),
            network,
            paths,
            backend: None,
        })
    }

    /// Open an existing profile.
    ///
    /// Re-derives every derived account's lock args and refuses a registry
    /// that no longer matches the seed. `accounts.json` is plaintext, so this
    /// is the cheapest integrity guarantee that does not require
    /// authenticating the file.
    pub fn unlock(
        paths: ProfilePaths,
        password: &[u8],
        network: Network,
    ) -> Result<Self, CoreError> {
        let vault = Vault::unlock(&paths.vault, password)?;
        if !Keyring::is_initialised(&vault) {
            return Err(CoreError::SeedMissing);
        }
        let accounts = AccountRegistry::open(&paths.accounts)?;
        let locks = LockRegistry::with_first_party();
        Self::verify_registry(&vault, &accounts, &locks)?;
        Ok(Self {
            vault,
            accounts,
            locks,
            network,
            paths,
            backend: None,
        })
    }

    /// Re-derive each account and compare against what is stored.
    ///
    /// An account with no `Derivation` is a mismatch, not a skip: nothing in
    /// the wallet creates one today, and skipping it would let an attacker
    /// null the field to smuggle a swapped `lock_args` past verification.
    fn verify_registry(
        vault: &Vault,
        accounts: &AccountRegistry,
        locks: &LockRegistry,
    ) -> Result<(), CoreError> {
        for account in accounts.list() {
            // An account with no derivation cannot be re-derived, and nothing
            // in the wallet creates one. Treating it as unverifiable rather
            // than as trusted closes an otherwise trivial bypass: an attacker
            // could null the derivation and swap the lock args in one edit.
            let Some(derivation) = account.derivation else {
                return Err(CoreError::RegistryMismatch {
                    account_id: account.id.clone(),
                });
            };
            let module = locks.get(account.lock_type)?;
            let seed = Keyring::seed_for(vault, module.seed_kind())?;
            let expected = module.derive_lock_args(seed.expose_secret(), &derivation)?;
            drop(seed);
            if expected != account.lock_args {
                return Err(CoreError::RegistryMismatch {
                    account_id: account.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Replace the lock modules (tests, future extension host).
    #[must_use]
    pub fn with_locks(mut self, locks: LockRegistry) -> Self {
        self.locks = locks;
        self
    }

    pub const fn network(&self) -> Network {
        self.network
    }

    pub fn mnemonic_format(&self) -> Result<MnemonicFormat, CoreError> {
        Keyring::format(&self.vault)
    }

    /// The start height to record for an account created right now.
    ///
    /// Decided here, never deferred. An account's address is usable the
    /// instant `create_account` returns, and the window between that and the
    /// first successful sync is unbounded — no backend need ever be attached.
    /// Resolving the height to "the tip whenever sync first runs" would
    /// therefore skip filters for any block that funded the address in
    /// between, and the wrong height is persisted, so it never self-heals.
    ///
    /// `0` is the honest answer whenever the current tip is not known: it
    /// costs a full filter scan, which is the very cost `watch_from_block`
    /// exists to avoid, but over-scanning is slow while under-scanning shows
    /// a zero balance with no error anywhere. A stale tip from a lagging
    /// light client is safe for the same reason — earlier only means more
    /// scanning.
    async fn start_height_for_new_account(&self) -> u64 {
        match self.accounts.origin() {
            // An import may have arbitrary history: scan everything.
            WalletOrigin::Imported => 0,
            WalletOrigin::Created => {
                let Some(backend) = self
                    .backend
                    .as_ref()
                    .and_then(BackendManager::current_backend)
                else {
                    return 0;
                };
                match backend.tip_header().await {
                    Ok(header) => u64::from(header.inner.number),
                    Err(e) => {
                        tracing::warn!(
                            "could not read the chain tip for a new account; \
                             scanning from genesis instead: {e}"
                        );
                        0
                    }
                }
            }
        }
    }

    /// Derive the next receiving account under the secp256k1 module.
    ///
    /// Async because the start height is settled here, against the attached
    /// backend, rather than deferred to the first sync — see
    /// `start_height_for_new_account`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::UnsupportedLock`] if the secp256k1 module is not
    /// registered, and any vault, lock-module or registry error raised while
    /// deriving or persisting the account.
    pub async fn create_account(&mut self, label: &str) -> Result<AccountRecord, CoreError> {
        // Settle the height before touching the vault, so no secret is held
        // across an await.
        let watch_from_block = self.start_height_for_new_account().await;
        let module = self.locks.get(LockType::Secp256k1Blake160)?;
        let derivation = Derivation {
            change: 0,
            index: self.accounts.next_index(module.lock_type(), 0),
        };
        let seed = Keyring::seed_for(&self.vault, module.seed_kind())?;
        let lock_args = module.derive_lock_args(seed.expose_secret(), &derivation)?;
        drop(seed);
        let stored = StoredAccount {
            id: account_id(module.lock_type(), &lock_args),
            label: label.to_string(),
            lock_type: module.lock_type(),
            extension_id: module.extension_id().to_string(),
            lock_args,
            derivation: Some(derivation),
            created_at: now_unix(),
            watch_from_block: Some(watch_from_block),
        };
        let record = to_record(
            &stored,
            self.network,
            &module.script_template(),
            module.capabilities(),
        )?;
        self.accounts.add(stored)?;
        self.accounts.save()?;
        Ok(record)
    }

    /// Public projections of every account for the active network.
    pub fn accounts(&self) -> Result<Vec<AccountRecord>, CoreError> {
        self.accounts
            .list()
            .iter()
            .map(|account| {
                let module = self.locks.get(account.lock_type)?;
                to_record(
                    account,
                    self.network,
                    &module.script_template(),
                    module.capabilities(),
                )
                .map_err(CoreError::from)
            })
            .collect()
    }

    /// Re-prove the password against the vault file, then render the
    /// phrase. Takes `&mut self` because it must re-lock this vault's
    /// pages after `fresh` (a second, temporary `Vault`) drops: page
    /// locks are per page, not reference-counted, so `fresh`'s guards can
    /// unlock pages this vault's own secrets share.
    pub fn reveal_mnemonic(&mut self, password: &[u8]) -> Result<Phrase, CoreError> {
        let fresh = Vault::unlock(&self.paths.vault, password)?;
        let phrase = Keyring::phrase(&fresh)?;
        fresh.lock();
        // `fresh`'s page-lock guards just dropped and may have unlocked
        // pages this vault's master key or blobs share; restore them.
        self.vault.relock();
        Ok(phrase)
    }

    /// Zeroize the unlocked state.
    pub fn lock(self) {
        self.vault.lock();
    }

    pub const fn signer(&self) -> SigningCoordinator<'_> {
        SigningCoordinator { core: self }
    }

    /// Adopt a backend manager. The manager owns the active backend; the
    /// wallet only asks it questions.
    ///
    /// Refuses a manager whose active profile is on another network. Nothing
    /// further down would notice: secp256k1 lock args are chain-independent,
    /// so a testnet wallet pointed at mainnet renders `ckt1…` addresses over
    /// mainnet cells and would build real mainnet spends behind a
    /// testnet-labelled UI. This is the only place the two claims meet.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::BackendNetworkMismatch`] if the manager's active
    /// profile names a different network from this wallet's.
    pub fn attach_backend(&mut self, manager: BackendManager) -> Result<(), CoreError> {
        let backend = manager.current_network();
        if backend != self.network {
            return Err(CoreError::BackendNetworkMismatch {
                wallet: self.network,
                backend,
            });
        }
        self.backend = Some(manager);
        Ok(())
    }

    pub fn backend(&self) -> Option<&dyn ChainBackend> {
        self.backend
            .as_ref()
            .and_then(BackendManager::current_backend)
    }

    /// The start height recorded for an account, if it has one.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::AccountNotFound`] if no account has this id.
    pub fn watch_from_block(&self, account_id: &str) -> Result<Option<u64>, CoreError> {
        self.accounts
            .get(account_id)
            .map(|a| a.watch_from_block)
            .ok_or(CoreError::AccountNotFound)
    }

    /// Tell the backend which scripts to watch, from each account's own
    /// recorded start height.
    ///
    /// A no-op on full backends, which index everything; on light backends it
    /// is the difference between syncing and sitting idle forever.
    ///
    /// Heights are never resolved here: `create_account` settles them (see
    /// `start_height_for_new_account`), so this call is idempotent and reads
    /// only. A `None` height can still reach here from a
    /// hand-edited `accounts.json`; it is treated as `0`, never as the tip.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Backend`] if no backend is attached and active,
    /// if the backend request fails, or if an account's lock module reports
    /// a hash type this wallet does not understand.
    pub async fn sync_watched_scripts(&self) -> Result<(), CoreError> {
        let Some(backend) = self
            .backend
            .as_ref()
            .and_then(BackendManager::current_backend)
        else {
            return Err(CoreError::Backend(BackendError::NotReady));
        };

        let mut watched = Vec::new();
        for account in self.accounts.list() {
            let module = self.locks.get(account.lock_type)?;
            let template = module.script_template();
            let hash_type = match template.hash_type {
                0 => ScriptHashType::Data,
                1 => ScriptHashType::Type,
                2 => ScriptHashType::Data1,
                4 => ScriptHashType::Data2,
                _ => {
                    return Err(CoreError::Backend(BackendError::Unsupported(
                        "unknown script hash type",
                    )));
                }
            };
            let script = Script {
                code_hash: H256(template.code_hash),
                hash_type,
                args: JsonBytes::from_vec(account.lock_args.clone()),
            };
            watched.push(WatchedScript::lock(
                script,
                account.watch_from_block.unwrap_or(0),
            ));
        }

        backend.watch_scripts(&watched).await?;
        Ok(())
    }
}

/// The single signing entry point.
pub struct SigningCoordinator<'a> {
    core: &'a WalletCore,
}

impl SigningCoordinator<'_> {
    /// Sign every group in `req` under `account_id`'s lock module.
    ///
    /// TEMPORARY surface for plan 1e's in-progress signing path. `req` is
    /// still supplied whole by the caller here; Task 14 ("`wallet-core`
    /// sends") replaces this with `WalletCore::send`, which builds `req`
    /// itself from a real `TransferPlan` rather than taking one ready-made.
    /// Kept so the account-lookup and lock-resolution plumbing this crate
    /// already owns stays exercised against `LockModule::sign`'s new,
    /// request-shaped contract rather than the retired per-digest one.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::AccountNotFound`] if `account_id` is not
    /// registered, [`CoreError::NoSigningMaterial`] if the account has no
    /// derivation on file, and whatever the resolved [`LockModule::sign`]
    /// reports.
    pub async fn sign(
        &self,
        account_id: &str,
        req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, CoreError> {
        let account = self
            .core
            .accounts
            .get(account_id)
            .ok_or(CoreError::AccountNotFound)?;
        let module = self.core.locks.get(account.lock_type)?;
        account.derivation.ok_or(CoreError::NoSigningMaterial)?;
        let seed = Keyring::seed_for(&self.core.vault, module.seed_kind())?;
        module
            .sign(seed.expose_secret(), req)
            .await
            .map_err(CoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use lantern_sdk_schema::{
        AccountCapabilities, Derivation, LockError, LockModule, LockType, Network, ScriptTemplate,
        SeedKind, SignedWitness, SigningGroup, SigningRequest, WitnessSize,
    };
    use tempfile::tempdir;

    use super::{ProfilePaths, WalletCore};
    use crate::error::CoreError;
    use crate::locks::LockRegistry;
    use crate::mnemonic::WordCount;

    struct FakeLock {
        kind: SeedKind,
        seen: Arc<Mutex<Vec<usize>>>,
    }

    #[async_trait]
    impl LockModule for FakeLock {
        fn lock_type(&self) -> LockType {
            LockType::Secp256k1Blake160
        }
        fn extension_id(&self) -> &'static str {
            "test.fake"
        }
        fn capabilities(&self) -> AccountCapabilities {
            AccountCapabilities {
                can_sign: true,
                hardware: false,
            }
        }
        fn script_template(&self) -> ScriptTemplate {
            ScriptTemplate {
                code_hash: [0x11; 32],
                hash_type: 1,
            }
        }
        fn seed_kind(&self) -> SeedKind {
            self.kind
        }
        fn witness_size(&self) -> WitnessSize {
            WitnessSize::Fixed(1)
        }
        fn derive_lock_args(&self, seed: &[u8], d: &Derivation) -> Result<Vec<u8>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            let tag = u8::try_from(d.index).unwrap_or(u8::MAX);
            Ok(vec![tag; 20])
        }
        async fn sign(
            &self,
            seed: &[u8],
            req: &SigningRequest,
        ) -> Result<Vec<SignedWitness>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            Ok(req
                .groups
                .iter()
                .filter_map(SigningGroup::witness_index)
                .map(|index| SignedWitness {
                    index,
                    witness: vec![0xAB],
                })
                .collect())
        }
    }

    fn fake_registry(kind: SeedKind) -> (LockRegistry, Arc<Mutex<Vec<usize>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut registry = LockRegistry::new();
        registry.register(Box::new(FakeLock {
            kind,
            seen: Arc::clone(&seen),
        }));
        (registry, seen)
    }

    #[tokio::test]
    async fn coordinator_hands_raw_entropy_to_a_pq_shaped_module() {
        let dir = tempdir().expect("tempdir");
        let (locks, seen) = fake_registry(SeedKind::RawEntropy);
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words24,
        )
        .expect("creates");
        let mut core = core.with_locks(locks);
        let record = core.create_account("pq").await.expect("creates account");
        assert_eq!(record.extension_id, "test.fake");
        let req = SigningRequest {
            tx: ckb_types::packed::Transaction::default(),
            inputs: Vec::new(),
            groups: vec![SigningGroup {
                lock_hash: [0u8; 32],
                input_indices: vec![0],
                derivation: Derivation {
                    change: 0,
                    index: 0,
                },
            }],
        };
        core.signer().sign(&record.id, &req).await.expect("signs");
        assert_eq!(*seen.lock().expect("mutex"), vec![32, 32]);
    }

    #[tokio::test]
    async fn coordinator_hands_the_bip39_seed_to_a_bip32_module() {
        let dir = tempdir().expect("tempdir");
        let (locks, seen) = fake_registry(SeedKind::Bip39Seed);
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words12,
        )
        .expect("creates");
        let mut core = core.with_locks(locks);
        core.create_account("hd").await.expect("creates account");
        assert_eq!(*seen.lock().expect("mutex"), vec![64]);
    }

    #[tokio::test]
    async fn empty_lock_registry_is_unsupported_lock() {
        let dir = tempdir().expect("tempdir");
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            WordCount::Words12,
        )
        .expect("creates");
        let mut core = core.with_locks(LockRegistry::new());
        assert!(matches!(
            core.create_account("x").await,
            Err(CoreError::UnsupportedLock(LockType::Secp256k1Blake160))
        ));
    }
}

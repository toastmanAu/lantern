//! The orchestrator.
//!
//! Owns the unlocked vault, the account registry, the lock modules, and
//! the active network. `SigningCoordinator` is the one signing entry
//! point (spec §7); it dispatches through `LockModule` and never names a
//! scheme. Synchronous in plan 1c; plan 1e turns it into the async
//! `sign(tx)` when device signers and submission exist.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lantern_account_registry::{AccountRegistry, StoredAccount, account_id, to_record};
use lantern_sdk_schema::{AccountRecord, Derivation, LockType, Network};
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
        let accounts = AccountRegistry::open(&paths.accounts)?;
        Ok((
            Self {
                vault,
                accounts,
                locks: LockRegistry::with_first_party(),
                network,
                paths,
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
        let accounts = AccountRegistry::open(&paths.accounts)?;
        Ok(Self {
            vault,
            accounts,
            locks: LockRegistry::with_first_party(),
            network,
            paths,
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

    /// Derive the next receiving account under the secp256k1 module.
    pub fn create_account(&mut self, label: &str) -> Result<AccountRecord, CoreError> {
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
            watch_from_block: None,
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
}

/// The single signing entry point.
pub struct SigningCoordinator<'a> {
    core: &'a WalletCore,
}

impl SigningCoordinator<'_> {
    /// Sign a 32-byte digest for `account_id`. Returns the witness lock bytes.
    pub fn sign_digest(&self, account_id: &str, digest: &[u8; 32]) -> Result<Vec<u8>, CoreError> {
        let account = self
            .core
            .accounts
            .get(account_id)
            .ok_or(CoreError::AccountNotFound)?;
        let module = self.core.locks.get(account.lock_type)?;
        let derivation = account.derivation.ok_or(CoreError::NoSigningMaterial)?;
        let seed = Keyring::seed_for(&self.core.vault, module.seed_kind())?;
        module
            .sign_digest(seed.expose_secret(), &derivation, digest)
            .map_err(CoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use lantern_sdk_schema::{
        AccountCapabilities, Derivation, LockError, LockModule, LockType, Network, ScriptTemplate,
        SeedKind,
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
        fn witness_lock_len(&self) -> usize {
            1
        }
        fn derive_lock_args(&self, seed: &[u8], d: &Derivation) -> Result<Vec<u8>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            let tag = u8::try_from(d.index).unwrap_or(u8::MAX);
            Ok(vec![tag; 20])
        }
        fn sign_digest(
            &self,
            seed: &[u8],
            _: &Derivation,
            digest: &[u8; 32],
        ) -> Result<Vec<u8>, LockError> {
            self.seen.lock().expect("mutex").push(seed.len());
            Ok(digest.to_vec())
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

    #[test]
    fn coordinator_hands_raw_entropy_to_a_pq_shaped_module() {
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
        let record = core.create_account("pq").expect("creates account");
        assert_eq!(record.extension_id, "test.fake");
        core.signer()
            .sign_digest(&record.id, &[0u8; 32])
            .expect("signs");
        assert_eq!(*seen.lock().expect("mutex"), vec![32, 32]);
    }

    #[test]
    fn coordinator_hands_the_bip39_seed_to_a_bip32_module() {
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
        core.create_account("hd").expect("creates account");
        assert_eq!(*seen.lock().expect("mutex"), vec![64]);
    }

    #[test]
    fn empty_lock_registry_is_unsupported_lock() {
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
            core.create_account("x"),
            Err(CoreError::UnsupportedLock(LockType::Secp256k1Blake160))
        ));
    }
}

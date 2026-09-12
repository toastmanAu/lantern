//! The orchestrator.
//!
//! Owns the unlocked vault, the account registry, the lock modules, and the
//! active network, and is where every side effect lives: fetching candidate
//! cells, dispatching to a `LockModule`, and broadcasting. `tx-builder` stays
//! pure by receiving what this module resolves.
//!
//! [`WalletCore::send`] is the one signing entry point (spec §7). It builds
//! the `SigningRequest` itself from a real `TransferPlan`, so nothing outside
//! this crate can hand the vault a transaction of its own choosing, and it
//! exposes the seed exactly once per send no matter how many script groups
//! the plan contains.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lantern_account_registry::{
    AccountRegistry, StoredAccount, WalletOrigin, account_id, to_record,
};
use lantern_chain_backend::{BackendError, BackendManager, ChainBackend, H256, WatchedScript};
use lantern_sdk_schema::{AccountRecord, Derivation, LockType, Network, SigningRequest};
use lantern_tx_builder::TransferRequest;
use lantern_vault::{ExposeSecret, Vault};

use crate::error::CoreError;
use crate::keyring::Keyring;
use crate::locks::LockRegistry;
use crate::mnemonic::{MnemonicFormat, Phrase, WordCount};
use crate::send::{apply_witnesses, collect_candidates, decode_address, lock_script_for};

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

    /// Build, sign and broadcast a transfer from one account.
    ///
    /// The whole side-effecting path in one place: page the account's cells
    /// out of the backend, hand them to the pure builder, dispatch the plan to
    /// the account's lock module, splice the witnesses it returns back into
    /// the transaction, and broadcast.
    ///
    /// The seed is read once and handed over once, covering every script group
    /// in the plan, so the number of exposures does not grow with the number
    /// of inputs.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::AccountNotFound`] if `account_id` is not
    /// registered, [`CoreError::NoSigningMaterial`] if it carries no
    /// derivation, [`CoreError::Registry`] if `recipient` is not a valid
    /// address on this wallet's network, [`CoreError::Backend`] if the chain
    /// is unreachable or no backend is attached, [`CoreError::Build`] if the
    /// transfer cannot be constructed, [`CoreError::Lock`] if the module
    /// refuses to sign, and [`CoreError::WitnessOutOfRange`] if the module
    /// returns a witness outside the groups it was asked to sign.
    pub async fn send(
        &mut self,
        account_id: &str,
        recipient: &str,
        amount: u64,
    ) -> Result<H256, CoreError> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or(CoreError::AccountNotFound)?
            .clone();
        let module = self.locks.get(account.lock_type)?;
        // The builder fills `SigningGroup.derivation` from this, and the
        // module derives its key from that. An account with none cannot sign
        // at all, so refuse before a transaction exists.
        let derivation = account.derivation.ok_or(CoreError::NoSigningMaterial)?;
        let backend = self
            .backend
            .as_ref()
            .and_then(BackendManager::current_backend)
            .ok_or(CoreError::Backend(BackendError::NotReady))?;

        // Decoded before anything touches the network: a mistyped or
        // wrong-chain address is the most likely failure here, and paying for
        // a full cell scan to discover it is wasted latency.
        let recipient = decode_address(recipient, self.network)?;
        let change_lock = lock_script_for(&account, module)?;

        // `current_backend()` hands back whatever is active, not whatever is
        // ready. A backend that fails `is_usable()` — a light client still
        // fetching filters, a supervised client that has not finished coming
        // up, one reporting an error — still answers `get_cells`, just from
        // an incomplete index. Everything downstream then behaves exactly as
        // if the wallet were empty or short of funds, which is a lie told
        // confidently; worse, a lagging index can serve a cell that is
        // already spent, and that transaction builds, signs and is refused
        // only by the pool. Spec §8 names this gate.
        let status = backend.status().await;
        if !status.is_usable() {
            return Err(CoreError::BackendNotUsable { status });
        }

        let candidates = collect_candidates(backend, &change_lock).await?;
        let request = TransferRequest {
            candidates,
            recipient,
            amount,
            change_lock,
            // Read from the resolved module, never assumed. `tx-builder`
            // cannot depend on the signer crate, so this is the only place the
            // placeholder the builder writes and the placeholder the module
            // demands can be made the same object. The secp256k1 module
            // refuses to sign a group whose first slot is not byte-for-byte
            // its own placeholder, so a mismatch fails loudly here rather than
            // as a -52 on chain.
            witness_size: module.witness_size(),
            derivation,
            cell_deps: module.cell_deps(self.network),
            fee_rate: lantern_tx_builder::DEFAULT_FEE_RATE,
        };
        let plan = lantern_tx_builder::build_transfer(&request)?;

        let signing = SigningRequest {
            tx: plan.tx.clone(),
            inputs: plan.inputs,
            groups: plan.groups,
        };
        let seed = Keyring::seed_for(&self.vault, module.seed_kind())?;
        let witnesses = module.sign(seed.expose_secret(), &signing).await?;
        drop(seed);

        let signed = apply_witnesses(plan.tx, &signing, witnesses)?;
        backend
            .send_transaction(&signed.into())
            .await
            .map_err(Into::into)
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
            watched.push(WatchedScript::lock(
                lock_script_for(account, module)?,
                account.watch_from_block.unwrap_or(0),
            ));
        }

        backend.watch_scripts(&watched).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use lantern_chain_backend::{BackendManager, testing::FakeNode};
    use lantern_sdk_schema::{
        AccountCapabilities, AccountRecord, CellDep, Derivation, LockError, LockModule, LockType,
        Network, ScriptTemplate, SeedKind, SignedWitness, SigningGroup, SigningRequest,
        WitnessSize,
    };
    use tempfile::tempdir;

    use super::{ProfilePaths, WalletCore};
    use crate::error::CoreError;
    use crate::locks::LockRegistry;
    use crate::mnemonic::WordCount;

    const SHANNONS_PER_CKB: u64 = 100_000_000;
    const BROADCAST_HASH: &str =
        "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1";
    /// The fake node's chain tip, and the height its filter sync has reached.
    const TIP: &str = "0x1554ef4";

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
        fn cell_deps(&self, _: Network) -> Vec<CellDep> {
            Vec::new()
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

    fn node_info() -> serde_json::Value {
        serde_json::json!({
            "version": "0.5.5", "node_id": "QmTestNode", "active": true,
            "addresses": [], "protocols": [], "connections": "0x0"
        })
    }

    /// A light client's tip, and its filter progress for `lock_args` at that
    /// same height — i.e. fully synced for this wallet's one script.
    ///
    /// `send` gates on [`lantern_sdk_schema::BackendStatus::is_usable`], and a
    /// light backend derives that by comparing `get_tip_header` against the
    /// `get_scripts` row for a script it has registered. A fixture missing
    /// either answer reports `Error` or `Connecting` and is refused before a
    /// cell is ever fetched.
    fn tip_header() -> serde_json::Value {
        serde_json::json!({
            "compact_target": "0x1a08a97e", "dao": format!("0x{}", "00".repeat(32)),
            "epoch": "0x1", "extra_hash": format!("0x{}", "00".repeat(32)),
            "hash": format!("0x{}", "01".repeat(32)),
            "nonce": "0x0", "number": TIP,
            "parent_hash": format!("0x{}", "00".repeat(32)),
            "proposals_hash": format!("0x{}", "00".repeat(32)),
            "timestamp": "0x1", "transactions_root": format!("0x{}", "00".repeat(32)),
            "version": "0x0"
        })
    }

    fn scripts_at_tip(lock_args: &str) -> serde_json::Value {
        serde_json::json!([{
            "script": {
                "code_hash": format!("0x{}", "11".repeat(32)),
                "hash_type": "type",
                "args": lock_args
            },
            "script_type": "lock",
            "block_number": TIP
        }])
    }

    /// A `get_cells` reply holding one spendable cell under `lock_args`,
    /// exhausting in a single page.
    fn one_cell(lock_args: &str, capacity: u64) -> serde_json::Value {
        serde_json::json!({
            "objects": [{
                "block_number": "0x1554e00",
                "out_point": { "index": "0x0", "tx_hash": format!("0x{}", "d1".repeat(32)) },
                "output": {
                    "capacity": format!("{capacity:#x}"),
                    "lock": {
                        "code_hash": format!("0x{}", "11".repeat(32)),
                        "hash_type": "type",
                        "args": lock_args
                    },
                    "type": null
                },
                "output_data": "0x",
                "tx_index": "0x1"
            }],
            "last_cursor": "0x"
        })
    }

    async fn manager_for(dir: &std::path::Path, url: String) -> BackendManager {
        let mut manager = BackendManager::open(dir.join("backends.json")).expect("manager");
        manager
            .add_profile(lantern_sdk_schema::BackendProfile {
                id: "fake".into(),
                label: "fake".into(),
                network: Network::Testnet,
                kind: lantern_sdk_schema::BackendKind::RemoteLight,
                endpoint: Some(url),
            })
            .expect("adds");
        manager.activate("fake").await.expect("activates");
        manager
    }

    /// Drive a whole `send` against a `FakeLock`, and report the seed lengths
    /// the module was handed, in order.
    ///
    /// Through `send` rather than through a hand-built `SigningRequest`: the
    /// property under test is that the *real* path asks `Keyring` for exactly
    /// the material `seed_kind()` names, which a test that called the module
    /// itself could not observe.
    async fn seed_lengths_across_a_send(
        kind: SeedKind,
        words: WordCount,
    ) -> (AccountRecord, Vec<usize>) {
        let dir = tempdir().expect("tempdir");
        let (locks, seen) = fake_registry(kind);
        let (core, _phrase) = WalletCore::create(
            ProfilePaths::in_dir(dir.path()),
            b"pw",
            Network::Testnet,
            words,
        )
        .expect("creates");
        let mut core = core.with_locks(locks);
        let record = core.create_account("one").await.expect("creates account");
        let args = record.public_metadata["lockArgs"]
            .as_str()
            .expect("lockArgs")
            .to_string();

        let node = FakeNode::builder()
            .respond("local_node_info", node_info())
            .respond("get_tip_header", tip_header())
            .respond("get_scripts", scripts_at_tip(&args))
            .respond("set_scripts", serde_json::json!(null))
            .respond("get_cells", one_cell(&args, 1000 * SHANNONS_PER_CKB))
            .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
            .start()
            .await;
        core.attach_backend(manager_for(dir.path(), node.url()).await)
            .expect("same network");
        // Registration first, as a real launch does it: a light client that
        // has been asked to watch nothing reports `Connecting`, and `send`
        // refuses an unusable backend rather than scanning an index that is
        // guaranteed to be empty.
        core.sync_watched_scripts().await.expect("registers");
        // A self-transfer: `FakeLock`'s own template round-trips through the
        // address the registry rendered for it, so no second fixture is needed.
        core.send(&record.id, &record.address, 100 * SHANNONS_PER_CKB)
            .await
            .expect("sends");

        let lengths = seen.lock().expect("mutex").clone();
        (record, lengths)
    }

    #[tokio::test]
    async fn sending_hands_raw_entropy_to_a_pq_shaped_module() {
        let (record, lengths) =
            seed_lengths_across_a_send(SeedKind::RawEntropy, WordCount::Words24).await;
        assert_eq!(record.extension_id, "test.fake");
        assert_eq!(
            lengths,
            vec![32, 32],
            "24 words is 32 bytes of entropy, handed once to derive the \
             account and once to sign"
        );
    }

    #[tokio::test]
    async fn sending_hands_the_bip39_seed_to_a_bip32_module() {
        let (_, lengths) =
            seed_lengths_across_a_send(SeedKind::Bip39Seed, WordCount::Words12).await;
        assert_eq!(
            lengths,
            vec![64, 64],
            "a BIP39 seed is 64 bytes whatever the word count, and the same \
             12-word wallet would yield 16 bytes of raw entropy — so this \
             cannot pass if `send` ignored `seed_kind()`"
        );
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

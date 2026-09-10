//! On-disk account list: `accounts.json` = `{ "version": 2, "origin": ..., "accounts": [...] }`.
//! Public data only. Writes are tmp-then-rename.

use std::fs;
use std::path::{Path, PathBuf};

use lantern_sdk_schema::{Derivation, LockType};
use serde::{Deserialize, Serialize};

use crate::error::RegistryError;

const FILE_VERSION: u32 = 2;

/// How the wallet behind this registry was opened.
///
/// It decides the start height of a newly derived account: a wallet created
/// moments ago has no history, an imported one may have years of it. Plan 1c's
/// `WalletCore` does not remember this across a lock/unlock cycle, so it is
/// persisted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletOrigin {
    Created,
    Imported,
}

impl Default for WalletOrigin {
    /// A file that does not say is assumed to be an import, which scans more
    /// than necessary rather than missing history.
    fn default() -> Self {
        Self::Imported
    }
}

/// What `accounts.json` holds per account. Public data only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    #[serde(with = "hex::serde")]
    pub lock_args: Vec<u8>,
    pub derivation: Option<Derivation>,
    pub created_at: u64,
    /// Height a light backend should begin filtering from.
    ///
    /// Always settled when the account is created, never later: the tip at
    /// creation time if a backend was attached and reachable, otherwise `0`.
    /// Imports record `0`, since an imported seed may have arbitrary history.
    ///
    /// `None` therefore only reaches this field from a v1 file mid-migration
    /// or a hand-edited v2 one, and every reader must treat it as `0`.
    /// **Never resolve it to a tip read later than creation** — the account's
    /// address is handed out the moment it is created, so a height read at any
    /// later moment can sit above the block that funded it, and the account
    /// then reads as empty forever with nothing reporting an error.
    #[serde(default)]
    pub watch_from_block: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct AccountsFile {
    version: u32,
    #[serde(default)]
    origin: WalletOrigin,
    accounts: Vec<StoredAccount>,
}

/// Deterministic account id: `<lock slug>-<hex lock args>`.
pub fn account_id(lock_type: LockType, lock_args: &[u8]) -> String {
    format!("{}-{}", lock_type.slug(), hex::encode(lock_args))
}

/// In-memory account list bound to its file.
#[derive(Debug)]
pub struct AccountRegistry {
    path: PathBuf,
    origin: WalletOrigin,
    accounts: Vec<StoredAccount>,
}

impl AccountRegistry {
    /// A missing file is an empty registry. A present but unreadable file
    /// is `Corrupt`; it is never silently replaced.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                origin: WalletOrigin::default(),
                accounts: Vec::new(),
            });
        }
        let bytes = fs::read(&path)?;
        let mut file: AccountsFile =
            serde_json::from_slice(&bytes).map_err(|_| RegistryError::Corrupt)?;
        match file.version {
            1 => {
                // A v1 file cannot say how the wallet was opened, and its
                // accounts carry no start height. Assume an import and scan
                // from genesis: slow, never wrong.
                file.origin = WalletOrigin::Imported;
                for account in &mut file.accounts {
                    account.watch_from_block = Some(0);
                }
            }
            FILE_VERSION => {}
            _ => return Err(RegistryError::Corrupt),
        }
        Ok(Self {
            path,
            origin: file.origin,
            accounts: file.accounts,
        })
    }

    pub const fn origin(&self) -> WalletOrigin {
        self.origin
    }

    /// Record how the wallet was opened. Called once, at creation or import.
    pub const fn set_origin(&mut self, origin: WalletOrigin) {
        self.origin = origin;
    }

    /// Move an account's filter start height, for an explicit, user-initiated
    /// rescan ("scan this account again from block N").
    ///
    /// **Not for resolving a deferred height.** Start heights are settled in
    /// `create_account` and are never left pending; see the note on
    /// [`StoredAccount::watch_from_block`] for why a later-read tip loses
    /// funds. Lowering a height also costs a real rescan on a light backend,
    /// so this is a deliberate user action, not a repair path.
    ///
    /// # Errors
    ///
    /// [`RegistryError::NotFound`] if no account has this id.
    pub fn set_watch_from_block(&mut self, id: &str, block: u64) -> Result<(), RegistryError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        account.watch_from_block = Some(block);
        Ok(())
    }

    pub fn add(&mut self, account: StoredAccount) -> Result<(), RegistryError> {
        if self.get(&account.id).is_some() {
            return Err(RegistryError::DuplicateId);
        }
        self.accounts.push(account);
        Ok(())
    }

    pub fn list(&self) -> &[StoredAccount] {
        &self.accounts
    }

    pub fn get(&self, id: &str) -> Option<&StoredAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn set_label(&mut self, id: &str, label: &str) -> Result<(), RegistryError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        account.label = label.to_string();
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<StoredAccount, RegistryError> {
        let position = self
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        Ok(self.accounts.remove(position))
    }

    /// One past the highest derived index on `(lock_type, change)`, or 0.
    /// Indices are scoped per lock type as well as per branch: two
    /// different lock modules deriving from the same seed each start
    /// their own account at index 0.
    pub fn next_index(&self, lock_type: LockType, change: u32) -> u32 {
        self.accounts
            .iter()
            .filter(|a| a.lock_type == lock_type)
            .filter_map(|a| a.derivation)
            .filter(|d| d.change == change)
            .map(|d| d.index)
            .max()
            .map_or(0, |max| max + 1)
    }

    /// Write `accounts.json` via `accounts.json.tmp` + rename.
    pub fn save(&self) -> Result<(), RegistryError> {
        let file = AccountsFile {
            version: FILE_VERSION,
            origin: self.origin,
            accounts: self.accounts.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|_| RegistryError::Corrupt)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Derivation, LockType};
    use tempfile::tempdir;

    use super::{AccountRegistry, StoredAccount, WalletOrigin, account_id};
    use crate::error::RegistryError;

    fn acct(index: u32, args: u8) -> StoredAccount {
        let lock_args = vec![args; 20];
        StoredAccount {
            id: account_id(LockType::Secp256k1Blake160, &lock_args),
            label: format!("Account {index}"),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            lock_args,
            derivation: Some(Derivation { change: 0, index }),
            created_at: 1_700_000_000,
            watch_from_block: None,
        }
    }

    #[test]
    fn id_is_slug_dash_hex() {
        assert_eq!(
            account_id(LockType::Secp256k1Blake160, &[0xab, 0xcd]),
            "secp256k1_blake160-abcd"
        );
    }

    #[test]
    fn missing_file_opens_empty() {
        let dir = tempdir().expect("tempdir");
        let reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        assert!(reg.list().is_empty());
        assert_eq!(reg.next_index(LockType::Secp256k1Blake160, 0), 0);
    }

    #[test]
    fn round_trips_through_disk_with_hex_args_and_camel_case() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        {
            let mut reg = AccountRegistry::open(&path).expect("opens");
            reg.add(acct(0, 0x11)).expect("adds");
            reg.add(acct(1, 0x22)).expect("adds");
            reg.save().expect("saves");
        }
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 2"), "{text}");
        assert!(
            text.contains("\"lockArgs\": \"1111111111111111111111111111111111111111\""),
            "{text}"
        );
        assert!(
            text.contains("\"lockType\": \"secp256k1_blake160\""),
            "{text}"
        );
        assert!(
            !path.with_extension("json.tmp").exists(),
            "tmp file left behind"
        );

        let reg = AccountRegistry::open(&path).expect("reopens");
        assert_eq!(reg.list(), &[acct(0, 0x11), acct(1, 0x22)]);
        assert_eq!(reg.get(&acct(1, 0x22).id), Some(&acct(1, 0x22)));
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        assert!(matches!(
            reg.add(acct(5, 0x11)),
            Err(RegistryError::DuplicateId)
        ));
    }

    #[test]
    fn corrupt_or_wrong_version_is_an_error_not_an_empty_registry() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, b"{ not json").expect("writes");
        assert!(matches!(
            AccountRegistry::open(&path),
            Err(RegistryError::Corrupt)
        ));
        std::fs::write(&path, br#"{ "version": 99, "accounts": [] }"#).expect("writes");
        assert!(matches!(
            AccountRegistry::open(&path),
            Err(RegistryError::Corrupt)
        ));
    }

    #[test]
    fn label_update_and_remove() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        let id = acct(0, 0x11).id;
        reg.set_label(&id, "Savings").expect("relabels");
        assert_eq!(reg.get(&id).map(|a| a.label.as_str()), Some("Savings"));
        assert!(matches!(
            reg.set_label("nope", "x"),
            Err(RegistryError::NotFound)
        ));
        let removed = reg.remove(&id).expect("removes");
        assert_eq!(removed.label, "Savings");
        assert!(reg.list().is_empty());
        assert!(matches!(reg.remove(&id), Err(RegistryError::NotFound)));
    }

    #[test]
    fn next_index_skips_gaps_and_is_per_branch() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        reg.add(acct(1, 0x22)).expect("adds");
        reg.add(acct(5, 0x33)).expect("adds");
        assert_eq!(reg.next_index(LockType::Secp256k1Blake160, 0), 6);
        assert_eq!(reg.next_index(LockType::Secp256k1Blake160, 1), 0);
    }

    #[test]
    fn a_v1_file_migrates_to_v2_conservatively() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        // A real plan 1c file: version 1, no origin, no watchFromBlock.
        let v1 = r#"{
          "version": 1,
          "accounts": [{
            "id": "secp256k1_blake160-1111111111111111111111111111111111111111",
            "label": "Main",
            "lockType": "secp256k1_blake160",
            "extensionId": "core.secp256k1",
            "lockArgs": "1111111111111111111111111111111111111111",
            "derivation": { "change": 0, "index": 0 },
            "createdAt": 1700000000
          }]
        }"#;
        std::fs::write(&path, v1).expect("writes");

        let reg = AccountRegistry::open(&path).expect("opens a v1 file");
        assert_eq!(
            reg.origin(),
            WalletOrigin::Imported,
            "a v1 file cannot say; assume the worst"
        );
        assert_eq!(
            reg.list()[0].watch_from_block,
            Some(0),
            "scan everything rather than risk missing history"
        );

        reg.save().expect("saves");
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 2"), "{text}");
        assert!(text.contains("\"origin\": \"imported\""), "{text}");
        assert!(text.contains("\"watchFromBlock\": 0"), "{text}");

        // Round trip through a fresh reopen and compare the WHOLE struct in
        // one assertion, so a regression that drops any field (not just the
        // two the migration touches) fails this test.
        let reg2 = AccountRegistry::open(&path).expect("reopens the migrated file");
        assert_eq!(
            reg2.list()[0],
            StoredAccount {
                id: "secp256k1_blake160-1111111111111111111111111111111111111111".into(),
                label: "Main".into(),
                lock_type: LockType::Secp256k1Blake160,
                extension_id: "core.secp256k1".into(),
                lock_args: vec![0x11; 20],
                derivation: Some(Derivation {
                    change: 0,
                    index: 0
                }),
                created_at: 1_700_000_000,
                watch_from_block: Some(0),
            }
        );
        assert_eq!(
            reg2.origin(),
            WalletOrigin::Imported,
            "origin must survive the reload, not merely the migration"
        );
    }

    #[test]
    fn a_v1_file_migrates_every_account() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        let v1 = r#"{
          "version": 1,
          "accounts": [
            {
              "id": "secp256k1_blake160-1111111111111111111111111111111111111111",
              "label": "First",
              "lockType": "secp256k1_blake160",
              "extensionId": "core.secp256k1",
              "lockArgs": "1111111111111111111111111111111111111111",
              "derivation": { "change": 0, "index": 0 },
              "createdAt": 1700000000
            },
            {
              "id": "secp256k1_blake160-2222222222222222222222222222222222222222",
              "label": "Second",
              "lockType": "secp256k1_blake160",
              "extensionId": "core.secp256k1",
              "lockArgs": "2222222222222222222222222222222222222222",
              "derivation": { "change": 0, "index": 1 },
              "createdAt": 1700000001
            },
            {
              "id": "secp256k1_blake160-3333333333333333333333333333333333333333",
              "label": "Third",
              "lockType": "secp256k1_blake160",
              "extensionId": "core.secp256k1",
              "lockArgs": "3333333333333333333333333333333333333333",
              "derivation": null,
              "createdAt": 1700000002
            }
          ]
        }"#;
        std::fs::write(&path, v1).expect("writes");

        let reg = AccountRegistry::open(&path).expect("opens a v1 file");
        // Assert the count first so a truncating bug cannot make the
        // per-account loop below vacuously pass.
        assert_eq!(reg.list().len(), 3);
        for account in reg.list() {
            assert_eq!(
                account.watch_from_block,
                Some(0),
                "account {} did not migrate",
                account.id
            );
        }
        assert_eq!(
            reg.list().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec![
                "secp256k1_blake160-1111111111111111111111111111111111111111",
                "secp256k1_blake160-2222222222222222222222222222222222222222",
                "secp256k1_blake160-3333333333333333333333333333333333333333",
            ],
            "account order must be preserved; the registry indexes by position"
        );
        assert_eq!(reg.list()[2].derivation, None);
    }

    #[test]
    fn a_v1_file_with_no_accounts_migrates_cleanly() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, r#"{ "version": 1, "accounts": [] }"#).expect("writes");

        let reg = AccountRegistry::open(&path).expect("opens a v1 file");
        assert!(reg.list().is_empty());
        assert_eq!(reg.origin(), WalletOrigin::Imported);

        reg.save().expect("saves");
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 2"), "{text}");
    }

    #[test]
    fn imported_is_the_default_origin_and_the_migration_target() {
        assert_eq!(
            WalletOrigin::default(),
            WalletOrigin::Imported,
            "the v1 migration's explicit Imported and serde's default must agree; \
             if you change Default, the migration arm is what keeps v1 files honest"
        );
    }

    #[test]
    fn a_v2_file_round_trips_with_origin_and_heights() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        {
            let mut reg = AccountRegistry::open(&path).expect("opens empty");
            reg.set_origin(WalletOrigin::Created);
            let mut account = acct(0, 0x11);
            account.watch_from_block = Some(22_000_000);
            reg.add(account).expect("adds");
            reg.save().expect("saves");
        }
        let reg = AccountRegistry::open(&path).expect("reopens");
        assert_eq!(reg.origin(), WalletOrigin::Created);
        assert_eq!(reg.list()[0].watch_from_block, Some(22_000_000));
    }

    #[test]
    fn a_future_version_is_still_corrupt() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(
            &path,
            r#"{ "version": 3, "origin": "created", "accounts": [] }"#,
        )
        .expect("writes");
        assert!(matches!(
            AccountRegistry::open(&path),
            Err(RegistryError::Corrupt)
        ));
    }

    #[test]
    fn a_start_height_can_be_resolved_later() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        let id = acct(0, 0x11).id;
        assert_eq!(reg.get(&id).expect("present").watch_from_block, None);
        reg.set_watch_from_block(&id, 22_000_000).expect("resolves");
        assert_eq!(
            reg.get(&id).expect("present").watch_from_block,
            Some(22_000_000)
        );
        assert!(matches!(
            reg.set_watch_from_block("nope", 1),
            Err(RegistryError::NotFound)
        ));
    }
}

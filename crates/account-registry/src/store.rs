//! On-disk account list: `accounts.json` = `{ "version": 1, "accounts": [...] }`.
//! Public data only. Writes are tmp-then-rename.

use std::fs;
use std::path::{Path, PathBuf};

use lantern_sdk_schema::{Derivation, LockType};
use serde::{Deserialize, Serialize};

use crate::error::RegistryError;

const FILE_VERSION: u32 = 1;

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
}

#[derive(Serialize, Deserialize)]
struct AccountsFile {
    version: u32,
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
                accounts: Vec::new(),
            });
        }
        let bytes = fs::read(&path)?;
        let file: AccountsFile =
            serde_json::from_slice(&bytes).map_err(|_| RegistryError::Corrupt)?;
        if file.version != FILE_VERSION {
            return Err(RegistryError::Corrupt);
        }
        Ok(Self {
            path,
            accounts: file.accounts,
        })
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

    /// One past the highest derived index on `change`, or 0.
    pub fn next_index(&self, change: u32) -> u32 {
        self.accounts
            .iter()
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

    use super::{AccountRegistry, StoredAccount, account_id};
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
        assert_eq!(reg.next_index(0), 0);
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
        assert!(text.contains("\"version\": 1"), "{text}");
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
        std::fs::write(&path, br#"{ "version": 2, "accounts": [] }"#).expect("writes");
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
        assert_eq!(reg.next_index(0), 6);
        assert_eq!(reg.next_index(1), 0);
    }
}

//! The `Vault` struct — the public face of the crate.
//!
//! Lifecycle:
//!
//! 1. `Vault::create(path, password)` — generates random salt + nonce,
//!    derives the master key, writes an empty encrypted store to disk, and
//!    returns an unlocked `Vault` ready for `put`.
//! 2. `Vault::unlock(path, password)` — reads the file, re-derives the
//!    master key, decrypts the store, returns an unlocked `Vault`.
//! 3. `vault.put(name, bytes)` / `vault.get(name)` — read/write blobs.
//! 4. `vault.save()` — re-encrypts with a FRESH nonce and rewrites the file.
//!    The salt is unchanged so the password doesn't re-derive.
//! 5. `vault.lock()` — consumes the vault, zeroizes the master key + store.
//!
//! **Memory hygiene guarantees:**
//! - Master key is always inside a `SecretBox<[u8; 32]>`.
//! - Inner store is always inside a `SecretBox<InnerStore>`.
//! - The `password` parameter is borrowed; the caller is responsible
//!   for zeroizing their password buffer after the call returns.
//! - `mlock` is NOT performed in this plan (1b). See the module doc for
//!   why, and plan 1c for the follow-up.

use std::fs;
use std::path::{Path, PathBuf};

use rand::{rngs::OsRng, RngCore};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox};
use zeroize::{Zeroize, Zeroizing};

use crate::aead::{open, seal};
use crate::error::VaultError;
use crate::format::{Header, HEADER_LEN, NONCE_LEN, SALT_LEN};
use crate::kdf::derive_master_key;
use crate::secret::InnerStore;

#[derive(Debug)]
pub struct Vault {
    path: PathBuf,
    header: Header,
    master_key: SecretBox<[u8; 32]>,
    store: SecretBox<InnerStore>,
}

impl Vault {
    pub fn create(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, VaultError> {
        let path = path.as_ref().to_path_buf();
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);
        let header = Header::new_v1(salt, nonce);

        let mut key_bytes = derive_master_key(
            password,
            &header.salt,
            header.m_cost_kib,
            header.t_cost,
            header.p_cost,
        )?;
        let master_key = SecretBox::new(Box::new(key_bytes));
        key_bytes.zeroize();

        let store = SecretBox::new(Box::new(InnerStore::new()));
        let v = Self { path, header, master_key, store };
        v.save()?;
        Ok(v)
    }

    pub fn unlock(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, VaultError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path)?;
        if bytes.len() < HEADER_LEN {
            return Err(VaultError::TruncatedHeader {
                need: HEADER_LEN,
                got: bytes.len(),
            });
        }
        let header = Header::decode(&bytes[..HEADER_LEN])?;
        let ciphertext = &bytes[HEADER_LEN..];

        let mut key_bytes = derive_master_key(
            password,
            &header.salt,
            header.m_cost_kib,
            header.t_cost,
            header.p_cost,
        )?;

        let plaintext: Zeroizing<Vec<u8>> = match open(&key_bytes, &header.nonce, ciphertext) {
            Ok(p) => Zeroizing::new(p),
            Err(_) => {
                // AEAD failure is either wrong password or tampering. From
                // the caller's perspective the former is vastly more common,
                // so map to WrongPassword — but a true tampering attack
                // can't be distinguished without additional state.
                return Err(VaultError::WrongPassword);
            }
        };

        let store: InnerStore = ciborium::from_reader(plaintext.as_slice())
            .map_err(|_| VaultError::SerdeFailed)?;
        // plaintext drops here (and on all error paths above), zeroizing its buffer

        let master_key = SecretBox::new(Box::new(key_bytes));
        key_bytes.zeroize();

        Ok(Self {
            path,
            header,
            master_key,
            store: SecretBox::new(Box::new(store)),
        })
    }

    /// Store a blob under a name. **Blob names must not contain secret
    /// material** — they are stored in the encrypted payload but are NOT
    /// individually zeroized on drop (only their `Vec<u8>` values are).
    /// Use stable identifiers like `"mnemonic"` or `"ckb-key-0"`, NOT
    /// anything that reveals user-specific structure.
    pub fn put(&mut self, name: &str, value: &[u8]) {
        self.store
            .expose_secret_mut()
            .blobs
            .insert(name.to_string(), value.to_vec());
    }

    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.store.expose_secret().blobs.get(name).cloned()
    }

    pub fn save(&self) -> Result<(), VaultError> {
        let mut plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
        ciborium::into_writer(self.store.expose_secret(), &mut *plaintext)
            .map_err(|_| VaultError::SerdeFailed)?;

        // Fresh nonce on every save — this is the XChaCha guarantee.
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let mut header = self.header;
        header.nonce = nonce;

        let ciphertext = seal(self.master_key.expose_secret(), &nonce, &plaintext)?;
        // plaintext drops here (or on any earlier ? path) — automatic zeroize

        let mut out: Vec<u8> = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&ciphertext);

        // Atomic-ish write: write to `<path>.tmp` then rename.
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, &out)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Consume the vault, zeroizing held secrets.
    pub fn lock(self) {
        // Drop impls on SecretBox + MasterKey handle zeroization.
        drop(self);
    }

    /// Expose the master key to a closure without cloning it. Used by the
    /// subkey module in Task 9.
    pub(crate) fn with_master_key<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8; 32]) -> R,
    {
        f(self.master_key.expose_secret())
    }

    /// Derive an extension-scoped 32-byte subkey via HKDF-SHA256 over the
    /// vault master key. See `subkey.rs` for the info-string format.
    pub fn extension_subkey(
        &self,
        extension_id: &str,
    ) -> Result<SecretBox<[u8; 32]>, VaultError> {
        self.with_master_key(|mk| crate::subkey::derive_extension_subkey(mk, extension_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Tests use the same 64 MiB Argon2 params as production so they
    // round-trip through the real header. Slow (~400ms each) but correct.

    #[test]
    fn create_then_unlock_empty_vault() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        {
            let _v = Vault::create(&path, b"correct horse battery staple").unwrap();
        }
        let v = Vault::unlock(&path, b"correct horse battery staple").unwrap();
        assert_eq!(v.get("anything"), None);
    }

    #[test]
    fn put_then_get_survives_save_and_unlock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        {
            let mut v = Vault::create(&path, b"pw").unwrap();
            v.put("mnemonic", b"abandon abandon abandon");
            v.save().unwrap();
        }
        let v = Vault::unlock(&path, b"pw").unwrap();
        assert_eq!(v.get("mnemonic").as_deref(), Some(&b"abandon abandon abandon"[..]));
    }

    #[test]
    fn wrong_password_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let _ = Vault::create(&path, b"right").unwrap();
        let err = Vault::unlock(&path, b"wrong").unwrap_err();
        assert!(matches!(err, VaultError::WrongPassword));
    }

    #[test]
    fn save_rotates_nonce() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        let bytes_1 = fs::read(&path).unwrap();
        v.put("x", b"1");
        v.save().unwrap();
        let bytes_2 = fs::read(&path).unwrap();
        // Header nonce is bytes 34..58. Must differ.
        assert_ne!(&bytes_1[34..58], &bytes_2[34..58]);
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        fs::write(&path, b"not a lantern vault at all").unwrap();
        let err = Vault::unlock(&path, b"pw").unwrap_err();
        // Could be TruncatedHeader or BadMagic depending on file length.
        assert!(matches!(
            err,
            VaultError::BadMagic | VaultError::TruncatedHeader { .. }
        ));
    }
}

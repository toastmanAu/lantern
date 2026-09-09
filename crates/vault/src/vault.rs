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
//! - With the `mlock` feature the master key and every blob are page-locked
//!   while unlocked (best effort; see `mlock.rs`). Guards are released after
//!   the secrets are zeroized because `locks` is the last field.

use std::fs;
use std::path::{Path, PathBuf};

use rand::{RngCore, rngs::OsRng};
use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretSlice};
use zeroize::{Zeroize, Zeroizing};

use crate::aead::{open, seal};
use crate::error::VaultError;
use crate::format::{HEADER_LEN, Header, NONCE_LEN, SALT_LEN};
use crate::kdf::derive_master_key;
use crate::mlock::PageLocks;
use crate::secret::InnerStore;

#[derive(Debug)]
pub struct Vault {
    path: PathBuf,
    header: Header,
    master_key: SecretBox<[u8; 32]>,
    store: SecretBox<InnerStore>,
    locks: PageLocks,
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
        // `[u8; 32]` is Copy, so `Box::new(key_bytes)` above took a COPY — the
        // stack slot `key_bytes` still holds the key bytes. Zeroize that copy
        // now so the only live key lives inside the SecretBox.
        key_bytes.zeroize();

        let store = SecretBox::new(Box::new(InnerStore::new()));
        let mut v = Self {
            path,
            header,
            master_key,
            store,
            locks: PageLocks::new(),
        };
        v.relock();
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

        let Ok(plaintext_bytes) = open(&key_bytes, &header.nonce, ciphertext) else {
            // AEAD failure is either wrong password or tampering. From the caller's
            // perspective the former is vastly more common, so map to WrongPassword —
            // but a true tampering attack can't be distinguished without additional state.
            return Err(VaultError::WrongPassword);
        };
        let plaintext: zeroize::Zeroizing<Vec<u8>> = zeroize::Zeroizing::new(plaintext_bytes);

        let store: InnerStore =
            ciborium::from_reader(plaintext.as_slice()).map_err(|_| VaultError::SerdeFailed)?;
        // plaintext drops here (and on all error paths above), zeroizing its buffer

        let master_key = SecretBox::new(Box::new(key_bytes));
        // `[u8; 32]` is Copy, so `Box::new(key_bytes)` above took a COPY — the
        // stack slot `key_bytes` still holds the key bytes. Zeroize that copy
        // now so the only live key lives inside the SecretBox.
        key_bytes.zeroize();

        let mut v = Self {
            path,
            header,
            master_key,
            store: SecretBox::new(Box::new(store)),
            locks: PageLocks::new(),
        };
        v.relock();
        Ok(v)
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
        self.relock();
    }

    /// Copy of a blob inside a zeroizing, `Debug`-opaque wrapper.
    pub fn get(&self, name: &str) -> Option<SecretSlice<u8>> {
        self.store
            .expose_secret()
            .blobs
            .get(name)
            .map(|v| SecretSlice::from(v.clone()))
    }

    /// Re-lock this vault's pages. Called after any change to the store
    /// because `Vec` reallocation moves the bytes.
    ///
    /// Call after any other `Vault` in the process has been dropped: page
    /// locks are per page, not reference-counted, so another vault's
    /// guards can unlock pages this vault shares.
    pub fn relock(&mut self) {
        self.locks.clear();
        self.locks.lock(&self.master_key.expose_secret()[..]);
        for blob in self.store.expose_secret().blobs.values() {
            self.locks.lock(blob);
        }
    }

    pub fn save(&self) -> Result<(), VaultError> {
        let mut plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
        ciborium::into_writer(self.store.expose_secret(), &mut *plaintext)
            .map_err(|_| VaultError::SerdeFailed)?;

        // Fresh nonce on every save — this is the XChaCha guarantee.
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let mut header = self.header;
        // Note: self.header.nonce is intentionally NOT updated — the on-disk
        // nonce rotates each save, but in-memory we keep the original header
        // (it's re-read from disk on next unlock anyway).
        header.nonce = nonce;

        let ciphertext = seal(self.master_key.expose_secret(), &nonce, &plaintext)?;
        // plaintext drops here (or on any earlier ? path) — automatic zeroize

        let mut out: Vec<u8> = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&ciphertext);

        // Atomic-ish write: write to `<path>.tmp` then rename.
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, &out)?;
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
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
    pub fn extension_subkey(&self, extension_id: &str) -> Result<SecretBox<[u8; 32]>, VaultError> {
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
        assert!(v.get("anything").is_none());
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
        assert_eq!(
            v.get("mnemonic").map(|s| s.expose_secret().to_vec()),
            Some(b"abandon abandon abandon".to_vec())
        );
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

    #[test]
    fn get_returns_secret_slice_with_opaque_debug() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("k", b"top secret");
        let got = v.get("k").expect("present");
        assert_eq!(got.expose_secret(), b"top secret");
        let dbg = format!("{got:?}");
        assert!(!dbg.contains("top secret"), "debug leaked the blob: {dbg}");
        assert!(v.get("missing").is_none());
    }

    #[test]
    fn page_locks_cover_master_key_and_every_blob_or_warn() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.bin");
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("a", b"1");
        v.put("b", b"22");
        let expected = if cfg!(feature = "mlock") { 3 } else { 0 };
        assert!(
            v.locks.locked_regions() == expected || v.locks.warned(),
            "regions={} warned={}",
            v.locks.locked_regions(),
            v.locks.warned()
        );
        v.lock();
    }
}

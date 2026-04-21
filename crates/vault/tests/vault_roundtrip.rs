//! End-to-end integration test for the vault crate.
//!
//! Exercises the public API exactly as a consumer crate
//! (`lantern-wallet-core`) will. If any test here fails, the downstream
//! consumer will break the same way — so these tests are the canonical
//! contract of the vault module.

use lantern_vault::{Vault, VaultError};
use secrecy::ExposeSecret;
use std::fs;
use tempfile::tempdir;

#[test]
fn full_lifecycle() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test-vault.bin");

    // Create with a password and store two blobs.
    {
        let mut v = Vault::create(&path, b"hunter2").unwrap();
        v.put("mnemonic", b"abandon abandon abandon art");
        v.put("ckb-key-0", &[0xABu8; 32]);
        v.save().unwrap();
    }

    // Reopen with the same password.
    let v = Vault::unlock(&path, b"hunter2").unwrap();
    assert_eq!(
        v.get("mnemonic").as_deref(),
        Some(&b"abandon abandon abandon art"[..])
    );
    assert_eq!(v.get("ckb-key-0").as_deref(), Some(&[0xABu8; 32][..]));
    assert_eq!(v.get("not-there"), None);
}

#[test]
fn wrong_password_rejected_without_partial_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    {
        let mut v = Vault::create(&path, b"right").unwrap();
        v.put("secret", b"value");
        v.save().unwrap();
    }
    let err = Vault::unlock(&path, b"wrong").unwrap_err();
    assert!(matches!(err, VaultError::WrongPassword));
}

#[test]
fn tampered_file_fails_unlock() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    {
        let mut v = Vault::create(&path, b"pw").unwrap();
        v.put("secret", b"value");
        v.save().unwrap();
    }
    // Flip one bit in the last byte. The AEAD ciphertext+tag spans bytes
    // 58..end, so the last byte is always inside the Poly1305-authenticated
    // region — tampering must be detected on unlock.
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&path, bytes).unwrap();

    let err = Vault::unlock(&path, b"pw").unwrap_err();
    // Tampering is indistinguishable from wrong password at this layer.
    assert!(matches!(err, VaultError::WrongPassword));
}

#[test]
fn extension_subkey_is_stable_across_sessions() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let subkey_session_1 = {
        let v = Vault::create(&path, b"pw").unwrap();
        let sk = v.extension_subkey("fiberquest").unwrap();
        *sk.expose_secret()
    };
    let subkey_session_2 = {
        let v = Vault::unlock(&path, b"pw").unwrap();
        let sk = v.extension_subkey("fiberquest").unwrap();
        *sk.expose_secret()
    };
    assert_eq!(subkey_session_1, subkey_session_2);
}

#[test]
fn save_twice_rotates_nonce_but_preserves_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("vault.bin");
    let mut v = Vault::create(&path, b"pw").unwrap();
    v.put("x", b"y");
    v.save().unwrap();
    let after_1 = fs::read(&path).unwrap();
    v.save().unwrap();
    let after_2 = fs::read(&path).unwrap();
    assert_ne!(&after_1[34..58], &after_2[34..58], "nonce must rotate");
    assert_eq!(v.get("x").as_deref(), Some(&b"y"[..]));
}

//! AEAD wrapper around `XChaCha20Poly1305`.
//!
//! Thin boundary: takes key + nonce + plaintext/ciphertext slices, returns
//! the other. No state, no retained secrets. Callers own nonce freshness —
//! this module just does not care where the nonce came from.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};

use crate::error::VaultError;

pub fn seal(key: &[u8; 32], nonce: &[u8; 24], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .encrypt(XNonce::from_slice(nonce), plaintext)
        .map_err(|_| VaultError::AuthFailed)
}

pub fn open(key: &[u8; 32], nonce: &[u8; 24], ciphertext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| VaultError::AuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let pt = b"hello lantern vault";
        let ct = seal(&key, &nonce, pt).unwrap();
        assert_ne!(&ct[..pt.len()], pt); // actually encrypted
        assert_eq!(ct.len(), pt.len() + 16); // + tag
        let roundtrip = open(&key, &nonce, &ct).unwrap();
        assert_eq!(roundtrip, pt);
    }

    #[test]
    fn wrong_key_fails() {
        let key = [0x11u8; 32];
        let bad = [0x12u8; 32];
        let nonce = [0x22u8; 24];
        let ct = seal(&key, &nonce, b"x").unwrap();
        assert!(matches!(open(&bad, &nonce, &ct), Err(VaultError::AuthFailed)));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let mut ct = seal(&key, &nonce, b"hello").unwrap();
        ct[0] ^= 1;
        assert!(matches!(open(&key, &nonce, &ct), Err(VaultError::AuthFailed)));
    }

    #[test]
    fn wrong_nonce_fails() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 24];
        let bad_nonce = [0x23u8; 24];
        let ct = seal(&key, &nonce, b"hello").unwrap();
        assert!(matches!(
            open(&key, &bad_nonce, &ct),
            Err(VaultError::AuthFailed)
        ));
    }
}

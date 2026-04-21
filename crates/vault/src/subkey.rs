//! HKDF-SHA256 subkey derivation for extension-scoped vault keys.
//!
//! Each extension gets a deterministic 32-byte subkey derived from the
//! vault master key using HKDF with an extension-specific info string.
//! Extensions can use their subkey for their own AEAD (the vault itself
//! never touches extension plaintext beyond the blob APIs).
//!
//! HKDF info format: `"lantern.vault.v1.ext." || extension_id`.
//! Using a namespaced info string ensures collisions with future subkey
//! purposes (e.g. `"lantern.vault.v1.backup."`) are impossible.

use hkdf::Hkdf;
use secrecy::SecretBox;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::error::VaultError;

const INFO_PREFIX: &[u8] = b"lantern.vault.v1.ext.";

/// Derive a 32-byte subkey for an extension. Returns a `SecretBox` so the
/// caller cannot accidentally log or serialise it.
pub fn derive_extension_subkey(
    master_key: &[u8; 32],
    extension_id: &str,
) -> Result<SecretBox<[u8; 32]>, VaultError> {
    if extension_id.is_empty() {
        return Err(VaultError::InvalidExtensionId);
    }
    let hk = Hkdf::<Sha256>::new(None, master_key);
    let mut info: Vec<u8> = Vec::with_capacity(INFO_PREFIX.len() + extension_id.len());
    info.extend_from_slice(INFO_PREFIX);
    info.extend_from_slice(extension_id.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(&info, &mut out)
        .map_err(|_| VaultError::HkdfFailed)?;
    let boxed = SecretBox::new(Box::new(out));
    out.zeroize();
    Ok(boxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn same_inputs_same_subkey() {
        let mk = [0x77u8; 32];
        let k1 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        let k2 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        assert_eq!(k1.expose_secret(), k2.expose_secret());
    }

    #[test]
    fn different_extensions_diverge() {
        let mk = [0x77u8; 32];
        let k1 = derive_extension_subkey(&mk, "ckb.core").unwrap();
        let k2 = derive_extension_subkey(&mk, "ckb.fiber").unwrap();
        assert_ne!(k1.expose_secret(), k2.expose_secret());
    }

    #[test]
    fn different_master_keys_diverge() {
        let k1 = derive_extension_subkey(&[0x01; 32], "ext").unwrap();
        let k2 = derive_extension_subkey(&[0x02; 32], "ext").unwrap();
        assert_ne!(k1.expose_secret(), k2.expose_secret());
    }

    #[test]
    fn empty_extension_id_rejected() {
        let mk = [0x77u8; 32];
        let err = derive_extension_subkey(&mk, "").unwrap_err();
        assert!(matches!(err, VaultError::InvalidExtensionId));
    }
}

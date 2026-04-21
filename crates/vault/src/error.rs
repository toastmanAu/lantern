//! Error type for the vault crate.
//!
//! Display impls must NEVER surface secret bytes — inner error values from
//! `argon2` or `chacha20poly1305` are mapped to opaque variants.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("vault file is not a Lantern vault (magic header mismatch)")]
    BadMagic,

    #[error("unsupported vault format version: got {0}, supported: 1")]
    UnsupportedVersion(u8),

    #[error("unsupported KDF algorithm id: {0}")]
    UnsupportedKdf(u8),

    #[error("vault header is truncated (need {need} bytes, got {got})")]
    TruncatedHeader { need: usize, got: usize },

    #[error("key derivation failed")]
    KdfFailed,

    #[error("wrong password")]
    WrongPassword,

    /// Internal use only — map this to [`VaultError::WrongPassword`] before
    /// surfacing to callers. A true tampering attack and a wrong password
    /// cannot be distinguished at the AEAD layer, and exposing `AuthFailed`
    /// to users creates a confusing UX.
    #[error("vault payload failed authentication (tampered or corrupt)")]
    AuthFailed,

    #[error("vault inner-store serialisation failed")]
    SerdeFailed,

    #[error("HKDF expansion failed")]
    HkdfFailed,

    #[error("extension_id must be non-empty")]
    InvalidExtensionId,
}

#[cfg(test)]
mod tests {
    use super::VaultError;

    #[test]
    fn error_display_is_redacted() {
        // Secret-bearing errors must never print raw secret bytes.
        let e = VaultError::WrongPassword;
        let s = format!("{e}");
        assert_eq!(s, "wrong password");
    }

    #[test]
    fn io_error_converts() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: VaultError = io_err.into();
        assert!(matches!(e, VaultError::Io(_)));
    }
}

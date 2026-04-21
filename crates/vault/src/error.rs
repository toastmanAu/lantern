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

    #[error("vault payload failed authentication (tampered or corrupt)")]
    AuthFailed,

    #[error("vault inner-store serialisation failed")]
    SerdeFailed,

    #[error("HKDF expansion failed")]
    HkdfFailed,
}

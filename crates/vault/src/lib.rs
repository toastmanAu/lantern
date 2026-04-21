#![forbid(unsafe_code)]

//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material. See plan 1b for scope.
//! Memory locking (`mlock`) is intentionally NOT implemented here — it lands
//! in plan 1c behind a platform feature flag. All other memory-hygiene
//! guarantees (zeroize, `SecretBox`, no Debug leakage) are in force.

pub(crate) mod aead;
pub mod error;
pub(crate) mod format;
pub(crate) mod kdf;
#[allow(dead_code)]
pub(crate) mod secret;
pub(crate) mod subkey;
pub mod vault;

pub use error::VaultError;
pub use vault::Vault;

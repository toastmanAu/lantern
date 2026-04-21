#![forbid(unsafe_code)]

//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material. See plan 1b for scope.
//! Memory locking (`mlock`) is intentionally NOT implemented here — it lands
//! in plan 1c behind a platform feature flag. All other memory-hygiene
//! guarantees (zeroize, SecretBox, no Debug leakage) are in force.

pub mod aead;
pub mod error;
pub mod format;
pub mod kdf;
pub mod secret;
pub mod vault;

pub use error::VaultError;
pub use vault::Vault;

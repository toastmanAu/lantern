#![forbid(unsafe_code)]

//! Lantern vault.
//!
//! Encrypted at-rest storage for secret material. See plan 1b for the file
//! format and plan 1c for memory locking.
//!
//! Memory hygiene in force: zeroize on drop, `SecretBox`/`SecretSlice`
//! wrapping, no `Debug` leakage, and (with the default `mlock` feature)
//! best-effort page locking of the master key and every blob while the
//! vault is unlocked. Transient buffers are zeroized but not locked.

pub(crate) mod aead;
pub mod error;
pub(crate) mod format;
pub(crate) mod kdf;
pub(crate) mod mlock;
#[allow(dead_code)]
pub(crate) mod secret;
pub(crate) mod subkey;
pub mod vault;

pub use error::VaultError;
pub use secrecy::{ExposeSecret, SecretBox, SecretSlice};
pub use vault::Vault;

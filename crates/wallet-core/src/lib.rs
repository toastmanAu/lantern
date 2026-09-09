#![forbid(unsafe_code)]

//! Lantern wallet core.
//!
//! Orchestrates the vault, the account registry, and the lock modules.
//! Owns the seed lifecycle (`Keyring`), the module map (`LockRegistry`),
//! and the single signing entry point (`SigningCoordinator`).

pub mod error;
pub mod mnemonic;

pub use error::CoreError;
pub use mnemonic::{MnemonicFormat, Phrase, WordCount};

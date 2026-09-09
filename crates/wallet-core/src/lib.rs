#![forbid(unsafe_code)]

//! Lantern wallet core.
//!
//! Orchestrates the vault, the account registry, and the lock modules.
//! Owns the seed lifecycle (`Keyring`), the module map (`LockRegistry`),
//! and the single signing entry point (`SigningCoordinator`).

pub mod core;
pub mod error;
pub mod keyring;
pub mod locks;
pub mod mnemonic;

pub use core::{ProfilePaths, SigningCoordinator, WalletCore};
pub use error::CoreError;
pub use keyring::Keyring;
pub use locks::LockRegistry;
pub use mnemonic::{MnemonicFormat, Phrase, WordCount};

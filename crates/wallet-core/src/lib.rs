#![forbid(unsafe_code)]

//! Lantern wallet core.
//!
//! Orchestrates the vault, the account registry, the lock modules and the
//! chain backend. Owns the seed lifecycle (`Keyring`), the module map
//! (`LockRegistry`), and the single signing entry point,
//! `WalletCore::send`, which builds what it signs rather than accepting a
//! transaction from a caller.

pub mod core;
pub mod error;
pub mod keyring;
pub mod locks;
pub mod mnemonic;
// Private: `send`'s helpers are implementation detail of
// `WalletCore::send`, and nothing outside this crate calls them.
mod send;

pub use core::{ProfilePaths, WalletCore};
pub use error::CoreError;
pub use keyring::Keyring;
pub use lantern_chain_backend::{BackendManager, ChainBackend};
pub use locks::LockRegistry;
pub use mnemonic::{MnemonicFormat, Phrase, WordCount};

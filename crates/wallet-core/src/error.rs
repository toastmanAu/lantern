//! Error type.
//!
//! Wraps the lower crates' errors and adds orchestration failures.
//! `Display` never carries phrase words or key bytes; `InvalidMnemonic`
//! deliberately drops the `bip39` detail, which can echo the offending word.

use lantern_account_registry::RegistryError;
use lantern_sdk_schema::{LockError, LockType};
use lantern_vault::VaultError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Vault(#[from] VaultError),

    #[error(transparent)]
    Registry(#[from] RegistryError),

    #[error(transparent)]
    Lock(#[from] LockError),

    #[error("wallet is already initialised")]
    AlreadyInitialised,

    #[error("no seed material in the vault")]
    SeedMissing,

    #[error("invalid mnemonic: expected 12, 15, 18, 21, 24, 36, 54 or 72 valid words")]
    InvalidMnemonic,

    #[error("account not found")]
    AccountNotFound,

    #[error("no lock module registered for {0:?}")]
    UnsupportedLock(LockType),

    #[error("account carries no signing material")]
    NoSigningMaterial,

    #[error("stored account {account_id} does not match the wallet seed")]
    RegistryMismatch { account_id: String },

    #[error(transparent)]
    Backend(#[from] lantern_chain_backend::BackendError),
}

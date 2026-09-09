//! Error types for the schema crate and the `LockModule` contract.
//!
//! `Display` text must never carry key material. `LockError::Signing`
//! carries a scheme-specific message that implementors must keep free of
//! secrets.

use thiserror::Error;

/// Failure while rendering TypeScript bindings.
#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("TypeScript export failed: {0}")]
    Export(String),
}

/// Failure inside a `LockModule` implementation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LockError {
    #[error("seed material has the wrong shape for this lock module")]
    InvalidSeed,

    #[error("derivation is out of range for this lock module")]
    InvalidDerivation,

    #[error("signing failed: {0}")]
    Signing(String),
}

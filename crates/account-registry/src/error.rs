//! Error type. `Display` never carries file contents.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("accounts file is corrupt or has an unsupported version")]
    Corrupt,

    #[error("an account with this id already exists")]
    DuplicateId,

    #[error("account not found")]
    NotFound,

    #[error("address encoding failed")]
    Address,
}

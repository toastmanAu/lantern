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

    /// Why a string is not a usable address. Addresses are public data, so
    /// naming the reason leaks nothing and is the difference between "try
    /// again" and knowing a mainnet address was pasted into a testnet wallet.
    #[error("invalid address: {0}")]
    InvalidAddress(&'static str),
}

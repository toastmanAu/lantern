//! Error type. `Display` never carries key bytes.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("invalid secp256k1 secret key")]
    InvalidKey,

    #[error("invalid or unrecoverable signature")]
    InvalidSignature,

    #[error("seed must be 16 to 64 bytes")]
    InvalidSeedLength,

    #[error("derivation index must be below 2^31")]
    DerivationOverflow,
}

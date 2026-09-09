#![forbid(unsafe_code)]

//! Lantern `secp256k1_blake160` signer.
//!
//! Stateless curve math for the canonical CKB lock: BIP32 derivation on the
//! Neuron-compatible path, recoverable ECDSA over a 32-byte digest, the
//! RFC 0019 `sighash_all` digest, and the `LockModule` implementation that
//! `wallet-core` dispatches to. No storage, no I/O.

mod error;
mod hash;
mod key;

pub use error::SignerError;
pub use hash::blake160;
pub use key::{PublicKey, SigningKey, public_key};

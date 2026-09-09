#![forbid(unsafe_code)]

//! Lantern `secp256k1_blake160` signer.
//!
//! Stateless curve math for the canonical CKB lock: BIP32 derivation on the
//! Neuron-compatible path, recoverable ECDSA over a 32-byte digest, the
//! RFC 0019 `sighash_all` digest, and the `LockModule` implementation that
//! `wallet-core` dispatches to. No storage, no I/O.

mod error;
mod hash;
mod hd;
mod key;
mod lock;
mod sighash;
mod sign;

pub use error::SignerError;
pub use hash::blake160;
pub use hd::{Branch, CKB_COIN_TYPE, derive_ckb_key};
pub use key::{PublicKey, SigningKey, public_key};
pub use lock::{HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock};
pub use sighash::sighash_all;
pub use sign::{recover, sign_recoverable};

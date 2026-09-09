#![forbid(unsafe_code)]

//! Lantern account registry.
//!
//! Plaintext, public-only storage of `StoredAccount` records beside the
//! vault, plus the projection to the IPC `AccountRecord`. Never holds a
//! secret and never names a concrete lock scheme: callers pass the
//! `ScriptTemplate` and capabilities they obtained from the account's
//! `LockModule`.

mod address;
mod error;
mod record;
mod store;

pub use address::encode_full;
pub use error::RegistryError;
pub use record::to_record;
pub use store::{AccountRegistry, StoredAccount, account_id};

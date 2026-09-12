//! Error type.
//!
//! Wraps the lower crates' errors and adds orchestration failures.
//! `Display` never carries phrase words or key bytes; `InvalidMnemonic`
//! deliberately drops the `bip39` detail, which can echo the offending word.

use lantern_account_registry::RegistryError;
use lantern_sdk_schema::{BackendStatus, LockError, LockType, Network};
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

    /// The attached backend claims a different chain from the one this
    /// wallet renders addresses for. Nothing downstream would error —
    /// secp256k1 lock args are chain-independent — so this is caught here or
    /// not at all.
    #[error("wallet is on {wallet:?} but the backend is on {backend:?}")]
    BackendNetworkMismatch { wallet: Network, backend: Network },

    /// The active backend is attached and answering, but its index cannot be
    /// trusted to be complete yet.
    ///
    /// [`BackendStatus::is_usable`] is the predicate, documented as "queries
    /// can be trusted to return complete results". Spending through a backend
    /// that fails it is not a degraded read, it is a wrong one: a light
    /// client still fetching filters serves a partial cell set, so a funded
    /// wallet reports no spendable cells or insufficient funds, and a lagging
    /// index can serve a cell that has already been spent — which builds and
    /// signs cleanly and is refused by the pool.
    #[error(
        "the backend reports {status:?} and its cell scan may be incomplete; \
         wait until it reports syncing or synced before sending"
    )]
    BackendNotUsable { status: BackendStatus },

    #[error(transparent)]
    Backend(#[from] lantern_chain_backend::BackendError),

    #[error(transparent)]
    Build(#[from] lantern_tx_builder::BuildError),

    /// A lock module returned a witness for a slot outside the script groups
    /// it was handed. With third-party extension signers the alternative to
    /// refusing is one module silently overwriting another lock's witness.
    #[error("lock module returned a witness for index {index}, which is not in its script groups")]
    WitnessOutOfRange { index: usize },
}

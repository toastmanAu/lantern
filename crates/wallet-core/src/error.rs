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

    /// The active backend is not in a state where a cell scan means anything:
    /// it is not connected, not yet started, reporting an error, or — for a
    /// light client — has been asked to watch no scripts at all.
    ///
    /// [`BackendStatus::is_usable`] is the predicate. It is `Synced |
    /// Syncing`, so this variant is produced for `Connecting`, `Error` and
    /// `Stopped` and for nothing else. The case that earns it is a light
    /// client with an empty registration list: it indexes nothing for this
    /// wallet, so `get_cells` returns nothing however funded the wallet is,
    /// and every caller downstream would report no spendable cells. Plan 1d's
    /// supervised-restart bug, where a rebuilt client loses its
    /// registrations, arrives the same way.
    ///
    /// **What this does not promise.** A *registered* light client that is
    /// still fetching filters reports `Syncing`, which is usable, so it does
    /// not raise this error even though its candidate set really is partial —
    /// an incomplete index is not caught here. `is_usable` admitting
    /// `Syncing` is plan 1d's design, not this variant's; whether the send
    /// path should demand `Synced` outright is a plan 1f decision.
    #[error(
        "the backend reports {status:?}, so a cell scan would return nothing \
         regardless of this wallet's balance; wait until it is connected and \
         reporting sync progress, and check its scripts are registered"
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

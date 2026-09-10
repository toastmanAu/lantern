#![forbid(unsafe_code)]

//! Lantern chain backend.
//!
//! One `dyn`-safe trait over four ways of reaching a CKB chain: a light
//! client Lantern supervises, someone else's light client, a local full node,
//! and a remote full node. Capabilities are data rather than assumptions, so
//! the UI can disable what a backend cannot do instead of failing at call
//! time.

pub mod cursor;
pub mod error;
pub mod indexer;
pub mod query;

pub use cursor::{CellPage, Cursor};
pub use error::BackendError;
pub use indexer::{IndexerCell, Order, Pagination, ScriptStatus, ScriptType, SearchKey, Tip};
pub use query::{CellQuery, WatchedScript};

// Re-exported so downstream crates need not depend on `ckb-jsonrpc-types`
// directly; these are the only chain types this crate's API exposes.
pub use ckb_jsonrpc_types::{
    CellOutput, HeaderView, JsonBytes, OutPoint, Script, Transaction, TransactionWithStatusResponse,
};
pub use ckb_types::H256;

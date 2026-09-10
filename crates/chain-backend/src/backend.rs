//! The one interface the rest of the wallet sees.
//!
//! Spec §6: nothing above this line should care whether the chain source is a
//! supervised light client, someone else's light client, or a full node.

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::query::{CellQuery, WatchedScript};

/// A source of chain data.
///
/// Object-safe by way of `async_trait`, because `BackendManager` holds one
/// behind a `Box<dyn ChainBackend>` and swaps it at runtime.
#[async_trait]
pub trait ChainBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn network(&self) -> Network;

    /// What this backend can do, as data. Callers disable features they
    /// cannot use rather than discovering it through a failed call.
    fn capabilities(&self) -> BackendCapabilities;

    async fn status(&self) -> BackendStatus;
    async fn tip_header(&self) -> Result<HeaderView, BackendError>;
    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError>;
    async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError>;
    async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError>;

    /// Register scripts to watch. A no-op returning `Ok` on full backends,
    /// which index everything already.
    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError>;

    async fn start(&self) -> Result<(), BackendError>;
    async fn stop(&self) -> Result<(), BackendError>;
}

#[cfg(test)]
mod tests {
    use super::ChainBackend;
    use crate::backends::RemoteLight;
    use lantern_sdk_schema::Network;

    #[test]
    fn the_trait_is_object_safe() {
        // If `ChainBackend` were not `dyn`-safe, this would fail to compile,
        // not merely fail to run — `BackendManager` depends on that.
        let backend = RemoteLight::new(Network::Testnet, "http://127.0.0.1:1/").expect("backend");
        let _boxed: Box<dyn ChainBackend> = Box::new(backend);
    }
}

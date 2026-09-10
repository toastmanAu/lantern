//! JSON-RPC client for a CKB full node.
//!
//! A full node indexes every script, so there is nothing to register; what it
//! may lack is the indexer itself, which `get_indexer_tip` reports honestly.

use std::time::Duration;

use ckb_jsonrpc_types::{HeaderView, LocalNode, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use serde_json::json;

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::indexer::{IndexerCell, Pagination, Tip};
use crate::query::CellQuery;
use crate::rpc::RpcClient;

/// A client for one CKB full-node endpoint.
#[derive(Debug)]
pub struct FullRpc {
    rpc: RpcClient,
}

impl FullRpc {
    /// Build a client bound to `url`, timing out any single call after
    /// `timeout`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Transport`] if the underlying HTTP client
    /// cannot be built.
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, BackendError> {
        Ok(Self {
            rpc: RpcClient::new(url, timeout)?,
        })
    }

    pub fn url(&self) -> &str {
        self.rpc.url()
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn local_node_info(&self) -> Result<LocalNode, BackendError> {
        self.rpc.call("local_node_info", json!([])).await
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.call("get_tip_header", json!([])).await
    }

    /// The indexer's tip, or `None` when the node runs without an indexer.
    ///
    /// This doubles as the capability probe: a node answering `null` cannot
    /// serve `get_cells`, and saying so up front beats a confusing RPC error
    /// at the first balance query.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn indexer_tip(&self) -> Result<Option<Tip>, BackendError> {
        self.rpc.call("get_indexer_tip", json!([])).await
    }

    /// One page of a cell scan. The returned page enforces the cursor rules.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        let params = json!([
            query.search_key(),
            query.order,
            query.limit_param(),
            after.map(Cursor::as_str),
        ]);
        let page: Pagination<IndexerCell> = self.rpc.call("get_cells", params).await?;
        Ok(CellPage::new(page.objects, &page.last_cursor))
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc.call("get_transaction", json!([hash])).await
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        // "passthrough" matches ckb-sdk's default outputs-validator choice.
        self.rpc
            .call("send_transaction", json!([tx, "passthrough"]))
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::FullRpc;
    use crate::testing::FakeNode;

    #[tokio::test]
    async fn indexer_tip_is_some_when_the_indexer_is_enabled() {
        let node = FakeNode::builder()
            .respond(
                "get_indexer_tip",
                json!({
                    "block_hash": "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1",
                    "block_number": "0x1554ef4"
                }),
            )
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let tip = rpc.indexer_tip().await.expect("call").expect("indexer on");
        // 0x1554ef4 == 22_367_988 (matches indexer.rs's identical fixture).
        assert_eq!(u64::from(tip.block_number), 22_367_988);
    }

    #[tokio::test]
    async fn a_null_indexer_tip_means_the_indexer_is_off() {
        // Verified against a real node: this is how the capability is probed.
        let node = FakeNode::builder()
            .respond("get_indexer_tip", json!(null))
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert!(rpc.indexer_tip().await.expect("call").is_none());
    }

    #[tokio::test]
    async fn get_cells_obeys_the_same_cursor_rules_as_the_light_client() {
        let node = FakeNode::builder()
            .respond("get_cells", json!({"objects": [], "last_cursor": "0x"}))
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let script = serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script");
        let page = rpc
            .get_cells(&crate::query::CellQuery::lock(script), None)
            .await
            .expect("page");
        assert!(page.is_exhausted());
        assert!(page.next().is_none());
    }

    #[tokio::test]
    async fn send_transaction_asks_for_the_passthrough_validator() {
        // Without this parameter a node falls back to `well_known_scripts_only`
        // and rejects outputs using non-standard locks — which is every
        // post-quantum and passkey lock this wallet plans to support.
        let node = FakeNode::builder()
            .respond(
                "send_transaction",
                json!("0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"),
            )
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let tx: ckb_jsonrpc_types::Transaction = serde_json::from_value(json!({
            "version": "0x0",
            "cell_deps": [],
            "header_deps": [],
            "inputs": [],
            "outputs": [],
            "outputs_data": [],
            "witnesses": []
        }))
        .expect("a minimal transaction");

        rpc.send_transaction(&tx).await.expect("sends");

        let (method, params) = node.calls().into_iter().next().expect("one call");
        assert_eq!(method, "send_transaction");
        assert_eq!(
            params[1], "passthrough",
            "dropping the validator makes the node reject non-standard locks: {params}"
        );
    }
}

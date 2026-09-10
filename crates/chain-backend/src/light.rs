//! JSON-RPC client for a `ckb-light-client`.
//!
//! A light client syncs only the scripts it has been told to watch, so
//! `set_scripts` is not optional here the way it is absent on a full node.

use std::time::Duration;

use ckb_jsonrpc_types::{HeaderView, LocalNode, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use serde_json::{Value, json};

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::indexer::{IndexerCell, Pagination, ScriptStatus, SetScriptsCommand};
use crate::query::{CellQuery, WatchedScript};
use crate::rpc::RpcClient;

/// A client for one `ckb-light-client` endpoint.
#[derive(Debug)]
pub struct LightRpc {
    rpc: RpcClient,
}

impl LightRpc {
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
        self.rpc.call("send_transaction", json!([tx])).await
    }

    /// Register scripts to watch.
    ///
    /// Always `partial`. The `all` command replaces the server's entire
    /// script list, so two wallets pointed at one shared light client would
    /// erase each other's registrations.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn set_scripts_partial(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        let statuses: Vec<ScriptStatus> = scripts
            .iter()
            .map(WatchedScript::to_script_status)
            .collect();
        let _: Value = self
            .rpc
            .call("set_scripts", json!([statuses, SetScriptsCommand::Partial]))
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn get_scripts(&self) -> Result<Vec<ScriptStatus>, BackendError> {
        self.rpc.call("get_scripts", json!([])).await
    }

    /// How far filter sync has actually got: the slowest watched script.
    ///
    /// `None` when nothing is watched, which means there is nothing to sync
    /// rather than that sync is complete.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn filter_progress(&self) -> Result<Option<u64>, BackendError> {
        let scripts = self.get_scripts().await?;
        Ok(scripts.iter().map(|s| u64::from(s.block_number)).min())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::LightRpc;
    use crate::indexer::ScriptType;
    use crate::query::{CellQuery, WatchedScript};
    use crate::testing::FakeNode;

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    fn one_cell_page(cursor: &str) -> serde_json::Value {
        json!({
            "objects": [{
                "block_number": "0x1554e00",
                "out_point": {"index": "0x0", "tx_hash": "0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"},
                "output": {
                    "capacity": "0x1718c7e00",
                    "lock": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"},
                    "type": null
                },
                "output_data": "0x",
                "tx_index": "0x1"
            }],
            "last_cursor": cursor
        })
    }

    #[tokio::test]
    async fn paging_stops_at_the_real_terminal_cursor() {
        // Exactly what testnet does: a page, then an empty page with "0x".
        let node = FakeNode::builder()
            .respond_sequence(
                "get_cells",
                vec![
                    one_cell_page("0x40aabb"),
                    json!({"objects": [], "last_cursor": "0x"}),
                ],
            )
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let query = CellQuery::lock(script());

        let first = rpc.get_cells(&query, None).await.expect("page 1");
        assert_eq!(first.cells().len(), 1);
        let cursor = first.next().cloned().expect("page 1 continues");

        let second = rpc.get_cells(&query, Some(&cursor)).await.expect("page 2");
        assert!(second.cells().is_empty());
        assert!(second.next().is_none(), "must not resume from the sentinel");
        assert!(second.is_exhausted());
        assert_eq!(node.call_count("get_cells"), 2, "and the pager stopped");
    }

    #[tokio::test]
    async fn set_scripts_is_always_partial_never_all() {
        let node = FakeNode::builder()
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script(), 22_000_000)])
            .await
            .expect("registers");

        let (_, params) = node.calls().into_iter().next().expect("one call");
        assert_eq!(
            params[1], "partial",
            "`all` would wipe every script on a shared server: {params}"
        );
        assert_eq!(params[0][0]["block_number"], "0x14fb180");
        assert_eq!(params[0][0]["script_type"], "lock");
    }

    #[tokio::test]
    async fn filter_sync_progress_is_the_minimum_watched_height() {
        let node = FakeNode::builder()
            .respond(
                "get_scripts",
                json!([
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"}, "script_type": "lock", "block_number": "0x64"},
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x22"}, "script_type": "lock", "block_number": "0x20"}
                ]),
            )
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert_eq!(
            rpc.filter_progress().await.expect("progress"),
            Some(0x20),
            "the slowest script is the honest answer"
        );
    }

    #[tokio::test]
    async fn no_watched_scripts_means_no_filter_progress() {
        let node = FakeNode::builder()
            .respond("get_scripts", json!([]))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert!(rpc.filter_progress().await.expect("progress").is_none());
    }

    #[tokio::test]
    async fn script_type_round_trips_through_the_wire() {
        assert_eq!(
            serde_json::to_value(ScriptType::Lock).expect("ser"),
            json!("lock")
        );
    }
}

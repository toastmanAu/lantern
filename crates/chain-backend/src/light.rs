//! JSON-RPC client for a `ckb-light-client`.
//!
//! A light client syncs only the scripts it has been told to watch, so
//! `set_scripts` is not optional here the way it is absent on a full node.

use std::sync::Mutex;
use std::time::Duration;

use ckb_jsonrpc_types::{
    BlockNumber, HeaderView, LocalNode, Transaction, TransactionWithStatusResponse,
};
use ckb_types::H256;
use serde_json::{Value, json};

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::indexer::{IndexerCell, Pagination, ScriptStatus, ScriptType, SetScriptsCommand};
use crate::query::{CellQuery, WatchedScript};
use crate::rpc::RpcClient;

/// Identity of one registered script: what `get_scripts` keys on.
type ScriptKey = (ckb_jsonrpc_types::Script, ScriptType);

/// A client for one `ckb-light-client` endpoint.
#[derive(Debug)]
pub struct LightRpc {
    rpc: RpcClient,
    /// Scripts this client has registered, so [`Self::filter_progress`]
    /// answers for our own scripts rather than for whatever else a shared
    /// light client happens to be indexing.
    registered: Mutex<Vec<ScriptKey>>,
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
            registered: Mutex::new(Vec::new()),
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

    /// Register scripts to watch, never behind what the server has already
    /// filtered.
    ///
    /// Always `partial`. The `all` command replaces the server's entire
    /// script list, so two wallets pointed at one shared light client would
    /// erase each other's registrations.
    ///
    /// `partial` **overwrites** each script's stored height with whatever is
    /// sent — there is no `max()` on the server side; sending a lower number
    /// is precisely how a rescan is forced, and it also rewinds the global
    /// `min_filtered_block_number` and clears the matched-block set. Since
    /// this call runs at least once per launch from an account's *original*
    /// start height, sending that height unguarded would restart filter sync
    /// from there every time — from genesis, forever, for an imported
    /// wallet. So every script is raised to the progress `get_scripts`
    /// reports before it is sent.
    ///
    /// The guard lives here rather than in a backend so every caller of this
    /// client inherits it. A deliberate rescan therefore needs an explicit
    /// new method; it must never be reachable by accident from here.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure — including a failure to read the current progress, which is
    /// refused rather than assumed to be zero.
    pub async fn set_scripts_partial(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        let known = self.get_scripts().await?;
        let statuses: Vec<ScriptStatus> = scripts
            .iter()
            .map(|watched| {
                let mut status = watched.to_script_status();
                let progress = known
                    .iter()
                    .find(|s| s.script_type == watched.script_type && s.script == watched.script)
                    .map_or(0, |s| u64::from(s.block_number));
                status.block_number = BlockNumber::from(watched.from_block.max(progress));
                status
            })
            .collect();
        let _: Value = self
            .rpc
            .call("set_scripts", json!([statuses, SetScriptsCommand::Partial]))
            .await?;
        let keys: Vec<ScriptKey> = scripts
            .iter()
            .map(|w| (w.script.clone(), w.script_type))
            .collect();
        *self
            .registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = keys;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn get_scripts(&self) -> Result<Vec<ScriptStatus>, BackendError> {
        self.rpc.call("get_scripts", json!([])).await
    }

    /// How far filter sync has actually got for *our* scripts: the slowest
    /// one this client registered.
    ///
    /// `None` when this client has registered nothing, which means there is
    /// nothing of ours to sync rather than that sync is complete.
    ///
    /// Restricted to our own registrations because a light client can be
    /// shared: another wallet's script sitting at height 0 would otherwise
    /// pin this wallet at `Syncing { current: 0 }` forever.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] on any transport, decode, or node-side
    /// failure.
    pub async fn filter_progress(&self) -> Result<Option<u64>, BackendError> {
        let ours: Vec<ScriptKey> = self
            .registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if ours.is_empty() {
            return Ok(None);
        }
        let scripts = self.get_scripts().await?;
        Ok(scripts
            .iter()
            .filter(|s| {
                ours.iter()
                    .any(|(script, kind)| *kind == s.script_type && script == &s.script)
            })
            .map(|s| u64::from(s.block_number))
            .min())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::LightRpc;
    use crate::error::BackendError;
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

    /// A `get_scripts` reply naming `script()` at `height`.
    fn progress_at(height: &str) -> serde_json::Value {
        json!([{
            "script": {
                "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                "hash_type": "type",
                "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
            },
            "script_type": "lock",
            "block_number": height
        }])
    }

    #[tokio::test]
    async fn set_scripts_is_always_partial_never_all() {
        let node = FakeNode::builder()
            .respond("get_scripts", json!([]))
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script(), 22_000_000)])
            .await
            .expect("registers");

        let (_, params) = node
            .calls()
            .into_iter()
            .find(|(method, _)| method == "set_scripts")
            .expect("a set_scripts call");
        assert_eq!(
            params[1], "partial",
            "`all` would wipe every script on a shared server: {params}"
        );
        assert_eq!(params[0][0]["block_number"], "0x14fb180");
        assert_eq!(params[0][0]["script_type"], "lock");
    }

    #[tokio::test]
    async fn registration_never_sends_a_height_behind_the_servers_own_progress() {
        // `partial` overwrites the stored height, so a lower number is a
        // rescan request. The wallet re-sends an account's original start
        // height at least once per launch; unguarded, that is a rewind.
        let node = FakeNode::builder()
            .respond("get_scripts", progress_at("0x1554ef4"))
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script(), 0)])
            .await
            .expect("registers");

        let (_, params) = node
            .calls()
            .into_iter()
            .find(|(method, _)| method == "set_scripts")
            .expect("a set_scripts call");
        assert_eq!(
            params[0][0]["block_number"], "0x1554ef4",
            "a stored 0 must be raised to the server's progress, not sent"
        );
    }

    #[tokio::test]
    async fn registration_keeps_a_stored_height_ahead_of_the_servers_progress() {
        // The other direction: a freshly created account starts above what a
        // shared server has filtered, and must not be dragged backwards.
        let node = FakeNode::builder()
            .respond("get_scripts", progress_at("0x20"))
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script(), 0x0155_4ef4)])
            .await
            .expect("registers");

        let (_, params) = node
            .calls()
            .into_iter()
            .find(|(method, _)| method == "set_scripts")
            .expect("a set_scripts call");
        assert_eq!(params[0][0]["block_number"], "0x1554ef4");
    }

    #[tokio::test]
    async fn registration_refuses_rather_than_guessing_when_progress_cannot_be_read() {
        // Assuming "no progress" on a failed read is exactly the rewind this
        // guard exists to prevent.
        let node = FakeNode::builder()
            .fail("get_scripts", -32000, "unavailable")
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let err = rpc
            .set_scripts_partial(&[WatchedScript::lock(script(), 0)])
            .await
            .expect_err("must refuse");
        assert!(
            matches!(err, BackendError::Rpc { code: -32000, .. }),
            "{err:?}"
        );
        assert_eq!(
            node.call_count("set_scripts"),
            0,
            "and nothing was registered"
        );
    }

    /// A script with `args`, otherwise identical to [`script`].
    fn script_with_args(args: &str) -> ckb_jsonrpc_types::Script {
        serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": args
        }))
        .expect("script")
    }

    #[tokio::test]
    async fn filter_sync_progress_is_the_minimum_of_our_own_watched_heights() {
        let node = FakeNode::builder()
            .respond(
                "get_scripts",
                json!([
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"}, "script_type": "lock", "block_number": "0x64"},
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x22"}, "script_type": "lock", "block_number": "0x20"}
                ]),
            )
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[
            WatchedScript::lock(script_with_args("0x11"), 0),
            WatchedScript::lock(script_with_args("0x22"), 0),
        ])
        .await
        .expect("registers");
        assert_eq!(
            rpc.filter_progress().await.expect("progress"),
            Some(0x20),
            "the slowest script is the honest answer"
        );
    }

    #[tokio::test]
    async fn another_wallets_script_does_not_pin_our_progress_at_zero() {
        // A shared light client indexes everyone's scripts. Taking the min
        // over all of them would report someone else's fresh registration
        // as our sync progress, forever.
        let node = FakeNode::builder()
            .respond(
                "get_scripts",
                json!([
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"}, "script_type": "lock", "block_number": "0x64"},
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0xdead"}, "script_type": "lock", "block_number": "0x0"}
                ]),
            )
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script_with_args("0x11"), 0)])
            .await
            .expect("registers");
        assert_eq!(
            rpc.filter_progress().await.expect("progress"),
            Some(0x64),
            "the stranger's script at 0 is not ours to wait for"
        );
    }

    #[tokio::test]
    async fn nothing_registered_by_this_client_means_no_filter_progress() {
        let node = FakeNode::builder()
            .respond("get_scripts", json!([]))
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert!(rpc.filter_progress().await.expect("progress").is_none());
        assert_eq!(
            node.call_count("get_scripts"),
            0,
            "with nothing of ours registered there is nothing to ask about"
        );
    }

    #[tokio::test]
    async fn script_type_round_trips_through_the_wire() {
        assert_eq!(
            serde_json::to_value(ScriptType::Lock).expect("ser"),
            json!("lock")
        );
    }
}

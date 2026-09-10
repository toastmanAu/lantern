//! A light client someone else runs.
//!
//! Same RPC as the embedded kind, none of the process ownership — and one
//! extra hazard: the server's script list is shared, so registration must
//! never replace it wholesale.

use std::time::Duration;

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};

use crate::backend::ChainBackend;
use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::light::LightRpc;
use crate::query::{CellQuery, WatchedScript};

/// Default per-request timeout. Generous, because a light client under sync
/// load can be slow to answer, and a spurious timeout looks like an outage.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A light client reached over the network.
#[derive(Debug)]
pub struct RemoteLight {
    network: Network,
    rpc: LightRpc,
}

impl RemoteLight {
    /// # Errors
    ///
    /// Returns [`BackendError::Transport`] if the underlying HTTP client
    /// cannot be built.
    pub fn new(network: Network, url: impl Into<String>) -> Result<Self, BackendError> {
        Ok(Self {
            network,
            rpc: LightRpc::new(url, DEFAULT_TIMEOUT)?,
        })
    }

    /// Shared by both light backends: compare filter progress to the tip.
    pub(crate) async fn light_status(rpc: &LightRpc) -> BackendStatus {
        let tip = match rpc.tip_header().await {
            Ok(header) => u64::from(header.inner.number),
            Err(e) => {
                return BackendStatus::Error {
                    message: e.to_string(),
                };
            }
        };
        match rpc.filter_progress().await {
            // Nothing watched: there is nothing to sync, so the backend is as
            // ready as it can be rather than stuck reporting 0 of tip.
            Ok(None) => BackendStatus::Synced { tip },
            Ok(Some(current)) if current >= tip => BackendStatus::Synced { tip },
            Ok(Some(current)) => BackendStatus::Syncing {
                current,
                target: tip,
            },
            Err(e) => BackendStatus::Error {
                message: e.to_string(),
            },
        }
    }

    pub(crate) const fn light_capabilities() -> BackendCapabilities {
        BackendCapabilities {
            needs_script_registration: true,
            can_fetch_arbitrary_blocks: false,
            can_estimate_cycles: true,
            indexer_available: true,
        }
    }
}

#[async_trait]
impl ChainBackend for RemoteLight {
    fn kind(&self) -> BackendKind {
        BackendKind::RemoteLight
    }

    fn network(&self) -> Network {
        self.network
    }

    fn capabilities(&self) -> BackendCapabilities {
        Self::light_capabilities()
    }

    async fn status(&self) -> BackendStatus {
        Self::light_status(&self.rpc).await
    }

    async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.tip_header().await
    }

    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        self.rpc.get_cells(query, after).await
    }

    async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc.get_transaction(hash).await
    }

    async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        self.rpc.send_transaction(tx).await
    }

    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        self.rpc.set_scripts_partial(scripts).await
    }

    async fn start(&self) -> Result<(), BackendError> {
        // Nothing to start: someone else owns this process. Confirm it answers.
        self.rpc.local_node_info().await.map(|_| ())
    }

    async fn stop(&self) -> Result<(), BackendError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RemoteLight;
    use crate::backend::ChainBackend;
    use crate::query::WatchedScript;
    use crate::testing::FakeNode;
    use lantern_sdk_schema::{BackendKind, BackendStatus, Network};

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type", "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    fn header(number: &str) -> serde_json::Value {
        json!({
            "compact_target": "0x1a08a97e", "dao": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "epoch": "0x1", "extra_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "nonce": "0x0", "number": number,
            "parent_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "proposals_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "timestamp": "0x1", "transactions_root": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "version": "0x0"
        })
    }

    #[tokio::test]
    async fn a_light_backend_declares_that_it_needs_registration() {
        let node = FakeNode::builder().start().await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.kind(), BackendKind::RemoteLight);
        assert_eq!(backend.network(), Network::Testnet);
        let caps = backend.capabilities();
        assert!(
            caps.needs_script_registration,
            "a light client syncs nothing otherwise"
        );
        assert!(!caps.can_fetch_arbitrary_blocks);
        assert!(
            caps.indexer_available,
            "light clients always answer get_cells"
        );
    }

    #[tokio::test]
    async fn status_is_syncing_until_the_slowest_script_reaches_the_tip() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([{
                "script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"},
                "script_type": "lock", "block_number": "0x20"
            }]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(
            backend.status().await,
            BackendStatus::Syncing {
                current: 0x20,
                target: 0x64
            }
        );
    }

    #[tokio::test]
    async fn status_is_synced_once_filters_catch_up() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([{
                "script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"},
                "script_type": "lock", "block_number": "0x64"
            }]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.status().await, BackendStatus::Synced { tip: 0x64 });
    }

    #[tokio::test]
    async fn with_nothing_watched_the_backend_is_synced_not_stuck_at_zero() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.status().await, BackendStatus::Synced { tip: 0x64 });
    }

    #[tokio::test]
    async fn an_unreachable_node_reports_error_rather_than_panicking() {
        // Port 1 is reserved and refuses connections.
        let backend = RemoteLight::new(Network::Testnet, "http://127.0.0.1:1/").expect("backend");
        assert!(matches!(
            backend.status().await,
            BackendStatus::Error { .. }
        ));
    }

    #[tokio::test]
    async fn watch_scripts_registers_partially() {
        let node = FakeNode::builder()
            .respond("set_scripts", json!(null))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        backend
            .watch_scripts(&[WatchedScript::lock(script(), 100)])
            .await
            .expect("registers");
        let (_, params) = node.calls().into_iter().next().expect("a call");
        assert_eq!(params[1], "partial");
    }
}

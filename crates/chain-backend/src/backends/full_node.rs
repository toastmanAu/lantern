//! A CKB full node, local or remote.
//!
//! Indexes every script, so registration is a no-op; may have the indexer
//! switched off, which is probed once at connect rather than discovered
//! later through a failing query.

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};

use crate::backend::ChainBackend;
use crate::backends::remote_light::DEFAULT_TIMEOUT;
use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::full::FullRpc;
use crate::query::{CellQuery, WatchedScript};

/// What `get_blockchain_info` calls each chain.
const fn chain_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "ckb",
        Network::Testnet => "ckb_testnet",
    }
}

/// A CKB full node, probed once at connect.
#[derive(Debug)]
pub struct FullNode {
    network: Network,
    kind: BackendKind,
    rpc: FullRpc,
    capabilities: BackendCapabilities,
}

impl FullNode {
    /// Connect and probe. Connecting succeeds even without an indexer: the
    /// backend is usable for `send_transaction` and reports the gap honestly
    /// so the UI can disable what depends on it (spec §6).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] if the underlying HTTP client cannot be built
    /// or the initial probe fails.
    pub async fn connect(
        network: Network,
        kind: BackendKind,
        url: impl Into<String>,
    ) -> Result<Self, BackendError> {
        let rpc = FullRpc::new(url, DEFAULT_TIMEOUT)?;
        let indexer_available = rpc.indexer_tip().await?.is_some();
        Ok(Self {
            network,
            kind,
            rpc,
            capabilities: BackendCapabilities {
                needs_script_registration: false,
                can_fetch_arbitrary_blocks: true,
                can_estimate_cycles: true,
                indexer_available,
            },
        })
    }

    /// Spec §6 first-run step 2: is there a usable node on this machine?
    /// Returns its capabilities when one answers, `None` otherwise. Offers,
    /// never switches.
    pub async fn probe_local(url: &str) -> Option<BackendCapabilities> {
        let node = Self::connect(Network::Mainnet, BackendKind::LocalFull, url)
            .await
            .ok()?;
        Some(node.capabilities)
    }
}

#[async_trait]
impl ChainBackend for FullNode {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn network(&self) -> Network {
        self.network
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    async fn status(&self) -> BackendStatus {
        let tip = match self.rpc.tip_header().await {
            Ok(header) => u64::from(header.inner.number),
            Err(e) => {
                return BackendStatus::Error {
                    message: e.to_string(),
                };
            }
        };
        if !self.capabilities.indexer_available {
            // Chain data is current even though cells cannot be queried.
            return BackendStatus::Synced { tip };
        }
        match self.rpc.indexer_tip().await {
            Ok(Some(indexer)) => {
                let current = u64::from(indexer.block_number);
                if current >= tip {
                    BackendStatus::Synced { tip }
                } else {
                    BackendStatus::Syncing {
                        current,
                        target: tip,
                    }
                }
            }
            Ok(None) => BackendStatus::Synced { tip },
            Err(e) => BackendStatus::Error {
                message: e.to_string(),
            },
        }
    }

    async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.tip_header().await
    }

    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        if !self.capabilities.indexer_available {
            return Err(BackendError::Unsupported("get_cells"));
        }
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

    /// A full node indexes every script; there is nothing to register.
    async fn watch_scripts(&self, _scripts: &[WatchedScript]) -> Result<(), BackendError> {
        Ok(())
    }

    /// Confirm the endpoint answers *and* that it is on the chain this
    /// profile claims.
    ///
    /// Nothing else in the stack compares the two: lock args are
    /// chain-independent, so a testnet profile pointed at a mainnet node
    /// produces no error, just `ckt1…` addresses over mainnet cells and a
    /// tx builder that would spend them for real.
    async fn start(&self) -> Result<(), BackendError> {
        self.rpc.local_node_info().await?;
        let expected = chain_name(self.network);
        let actual = self.rpc.chain_name().await?;
        if actual == expected {
            Ok(())
        } else {
            Err(BackendError::NetworkMismatch { expected, actual })
        }
    }

    async fn stop(&self) -> Result<(), BackendError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::FullNode;
    use crate::backend::ChainBackend;
    use crate::query::WatchedScript;
    use crate::testing::FakeNode;
    use lantern_sdk_schema::{BackendKind, BackendStatus, Network};

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

    fn tip(number: &str) -> serde_json::Value {
        json!({"block_hash": "0x0000000000000000000000000000000000000000000000000000000000000001", "block_number": number})
    }

    #[tokio::test]
    async fn a_node_with_an_indexer_is_fully_capable() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", tip("0x64"))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::LocalFull, node.url())
            .await
            .expect("connects");
        let caps = backend.capabilities();
        assert!(caps.indexer_available);
        assert!(caps.can_fetch_arbitrary_blocks);
        assert!(
            !caps.needs_script_registration,
            "a full node indexes everything"
        );
    }

    #[tokio::test]
    async fn a_node_without_an_indexer_says_so_instead_of_failing_later() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", json!(null))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::RemoteFull, node.url())
            .await
            .expect("connects anyway");
        assert!(!backend.capabilities().indexer_available);
    }

    #[tokio::test]
    async fn get_cells_is_refused_up_front_when_the_indexer_is_off() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", json!(null))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::RemoteFull, node.url())
            .await
            .expect("connects");
        let script = serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type", "args": "0x11"
        }))
        .expect("script");
        let err = backend
            .get_cells(&crate::query::CellQuery::lock(script), None)
            .await
            .expect_err("must refuse");
        assert!(
            matches!(err, crate::BackendError::Unsupported("get_cells")),
            "{err:?}"
        );
        assert_eq!(node.call_count("get_cells"), 0, "and it never hit the wire");
    }

    #[tokio::test]
    async fn status_compares_the_indexer_tip_with_the_chain_tip() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", tip("0x20"))
            .respond("get_tip_header", header("0x64"))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::LocalFull, node.url())
            .await
            .expect("connects");
        assert_eq!(
            backend.status().await,
            BackendStatus::Syncing {
                current: 0x20,
                target: 0x64
            }
        );
    }

    #[tokio::test]
    async fn watching_scripts_is_a_no_op_that_touches_no_wire() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", tip("0x64"))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::LocalFull, node.url())
            .await
            .expect("connects");
        let script = serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type", "args": "0x11"
        }))
        .expect("script");
        backend
            .watch_scripts(&[WatchedScript::lock(script, 0)])
            .await
            .expect("no-op succeeds");
        assert_eq!(node.call_count("set_scripts"), 0);
    }

    #[tokio::test]
    async fn probing_a_dead_local_port_reports_absence_not_an_error() {
        assert!(FullNode::probe_local("http://127.0.0.1:1/").await.is_none());
    }

    fn node_info() -> serde_json::Value {
        json!({
            "version": "0.200.0", "node_id": "QmTestNode", "active": true,
            "addresses": [], "protocols": [], "connections": "0x0"
        })
    }

    #[tokio::test]
    async fn starting_against_the_wrong_chain_is_refused() {
        // The default profile list makes `default-mainnet` active on a fresh
        // install, so a user who unlocks a testnet wallet is one attach away
        // from querying mainnet cells behind ckt1… addresses.
        let node = FakeNode::builder()
            .respond("get_indexer_tip", tip("0x64"))
            .respond("local_node_info", node_info())
            .respond("get_blockchain_info", json!({"chain": "ckb"}))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::RemoteFull, node.url())
            .await
            .expect("connects");
        let err = backend.start().await.expect_err("must refuse");
        assert!(
            matches!(
                &err,
                crate::BackendError::NetworkMismatch { expected, actual }
                    if *expected == "ckb_testnet" && actual == "ckb"
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn starting_against_the_right_chain_succeeds() {
        let node = FakeNode::builder()
            .respond("get_indexer_tip", tip("0x64"))
            .respond("local_node_info", node_info())
            .respond("get_blockchain_info", json!({"chain": "ckb_testnet"}))
            .start()
            .await;
        let backend = FullNode::connect(Network::Testnet, BackendKind::RemoteFull, node.url())
            .await
            .expect("connects");
        backend.start().await.expect("same chain");
        assert_eq!(
            node.call_count("get_blockchain_info"),
            1,
            "the chain must actually be probed, not assumed"
        );
    }
}

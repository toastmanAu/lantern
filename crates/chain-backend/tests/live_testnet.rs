//! Real-network checks, skipped unless `LANTERN_LIVE_TESTNET=1`.
//!
//! The hermetic suite proves the code does what the fixtures say. This proves
//! the fixtures still describe reality — the gap that has bitten this project
//! before, when an invented empty-page cursor hid a permanent paging failure.

use std::time::Duration;

use lantern_chain_backend::query::CellQuery;
use lantern_chain_backend::{ChainBackend, FullRpc};
use lantern_sdk_schema::{BackendKind, Network};

const RPC: &str = "https://testnet.ckb.dev/";

fn enabled() -> bool {
    std::env::var("LANTERN_LIVE_TESTNET").as_deref() == Ok("1")
}

fn funded_lock() -> ckb_jsonrpc_types::Script {
    serde_json::from_value(serde_json::json!({
        "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
        "hash_type": "type",
        "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
    }))
    .expect("script")
}

#[tokio::test]
async fn live_paging_terminates_against_a_real_node() {
    if !enabled() {
        eprintln!("skipped: set LANTERN_LIVE_TESTNET=1 to run");
        return;
    }
    let rpc = FullRpc::new(RPC, Duration::from_secs(30)).expect("client");
    let query = CellQuery::lock(funded_lock()).with_limit(1);

    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = rpc.get_cells(&query, cursor.as_ref()).await.expect("page");
        pages += 1;
        assert!(pages < 500, "a real scan should not run away");
        if page.is_exhausted() {
            break;
        }
        cursor = page.next().cloned();
    }
    assert!(pages >= 1, "the scan ran");
    eprintln!("live scan terminated after {pages} pages");
}

#[tokio::test]
async fn live_indexer_tip_reports_a_plausible_height() {
    if !enabled() {
        eprintln!("skipped: set LANTERN_LIVE_TESTNET=1 to run");
        return;
    }
    let rpc = FullRpc::new(RPC, Duration::from_secs(30)).expect("client");
    let tip = rpc.indexer_tip().await.expect("call").expect("indexer on");
    assert!(
        u64::from(tip.block_number) > 20_000_000,
        "testnet was past 22M in September 2026"
    );
}

#[tokio::test]
async fn live_full_backend_reports_status() {
    if !enabled() {
        eprintln!("skipped: set LANTERN_LIVE_TESTNET=1 to run");
        return;
    }
    let backend = lantern_chain_backend::backends::FullNode::connect(
        Network::Testnet,
        BackendKind::RemoteFull,
        RPC,
    )
    .await
    .expect("connects");
    assert!(backend.capabilities().indexer_available);
    assert!(
        backend.status().await.is_usable(),
        "a public node should be usable"
    );
}

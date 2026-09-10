//! Real-network checks, `#[ignore]`d and additionally gated on
//! `LANTERN_LIVE_TESTNET=1`.
//!
//! Run them with:
//!
//! ```text
//! LANTERN_LIVE_TESTNET=1 cargo test -p lantern-chain-backend \
//!     --features testing --test live_testnet -- --ignored
//! ```
//!
//! Two gates, deliberately. The env check alone left a skipped run printing
//! `test live_… ok`, indistinguishable from a real live pass — and CI never
//! sets the variable, so those lines read "ok" in every CI run forever.
//! `#[ignore]` makes cargo print `ignored` instead, and means a future CI
//! edit would need *both* the variable and `-- --ignored` before it started
//! hitting a public node.
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
#[ignore = "live: set LANTERN_LIVE_TESTNET=1 and run with --ignored"]
async fn live_paging_terminates_against_a_real_node() {
    if !enabled() {
        eprintln!("skipped: LANTERN_LIVE_TESTNET is not 1");
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
    assert!(
        pages >= 2,
        "expected at least two pages at limit(1): either cursor threading \
         broke (page one reported itself exhausted, which a `pages >= 1` \
         assertion could not tell from a working round trip) or the funded \
         test address has drained below two cells — got {pages}"
    );
    eprintln!("live scan terminated after {pages} pages");
}

#[tokio::test]
#[ignore = "live: set LANTERN_LIVE_TESTNET=1 and run with --ignored"]
async fn live_indexer_tip_reports_a_plausible_height() {
    if !enabled() {
        eprintln!("skipped: LANTERN_LIVE_TESTNET is not 1");
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
#[ignore = "live: set LANTERN_LIVE_TESTNET=1 and run with --ignored"]
async fn live_full_backend_reports_status() {
    if !enabled() {
        eprintln!("skipped: LANTERN_LIVE_TESTNET is not 1");
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

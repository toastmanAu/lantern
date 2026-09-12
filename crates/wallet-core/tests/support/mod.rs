//! Fixtures shared by this crate's integration tests.
//!
//! Split out of `wallet_e2e.rs` purely for size. Everything here builds or
//! reads wire-shaped JSON — the replies a node would send, and the
//! transaction the wallet put on the wire — so it is fixture plumbing rather
//! than assertion.
//!
//! Not every test binary uses every helper, so the module carries
//! `#![allow(dead_code)]`: Cargo compiles this file separately into each
//! `tests/*.rs` that declares it, and the unused half would warn in the
//! others.

#![allow(dead_code)]

use lantern_chain_backend::testing::FakeNode;
use lantern_sdk_schema::{AccountRecord, Network};
use lantern_signer_secp256k1::{blake160, recover, sighash_all};

pub const SHANNONS_PER_CKB: u64 = 100_000_000;

/// The system `secp256k1_blake160_sighash_all` code hash, as the indexer
/// renders it in a `get_cells` reply.
pub const SECP_CODE_HASH: &str =
    "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8";

/// What the fake node answers `send_transaction` with.
pub const BROADCAST_HASH: &str =
    "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1";

/// RFC 0021's own published testnet vector — the identical string
/// `account-registry`'s encoder is pinned against. Used as the recipient so
/// the send path's address decoding is exercised against a third-party value
/// rather than against a round trip through our own encoder.
pub const RECIPIENT: &str = "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqgutnjd";
pub const RECIPIENT_ARGS: &str = "0xb39bbc0b3673c7d36450bc14cfcdad2d559c6c64";

pub fn lock_args_of(record: &AccountRecord) -> Vec<u8> {
    let hex_args = record.public_metadata["lockArgs"]
        .as_str()
        .expect("lockArgs is a string");
    hex::decode(hex_args.trim_start_matches("0x")).expect("hex")
}

/// A `local_node_info` reply shaped like a real light client's, so
/// `RemoteLight::start()`'s reachability probe succeeds.
pub fn local_node_info_json() -> serde_json::Value {
    serde_json::json!({
        "version": "0.5.5",
        "node_id": "QmTestNode",
        "active": true,
        "addresses": [],
        "protocols": [],
        "connections": "0x0"
    })
}

pub fn header_json(number: &str) -> serde_json::Value {
    serde_json::json!({
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

/// A `BackendManager` with one active `RemoteLight` profile on `network`,
/// pointed at `url`.
///
/// `RemoteLight`, not `RemoteFull`: `FullNode::watch_scripts` is an
/// unconditional no-op (a full node indexes everything), so a test built on
/// it could not tell a real registration from a deleted one. A light
/// client's `watch_scripts` really issues `set_scripts` over the wire.
pub async fn light_manager(
    dir: &std::path::Path,
    network: Network,
    url: String,
) -> lantern_chain_backend::BackendManager {
    let mut manager =
        lantern_chain_backend::BackendManager::open(dir.join("backends.json")).expect("manager");
    manager
        .add_profile(lantern_sdk_schema::BackendProfile {
            id: "fake".into(),
            label: "fake".into(),
            network,
            kind: lantern_sdk_schema::BackendKind::RemoteLight,
            endpoint: Some(url),
        })
        .expect("adds");
    manager.activate("fake").await.expect("activates");
    manager
}

pub fn hex_args(args: &[u8]) -> String {
    format!("0x{}", hex::encode(args))
}

/// One `get_cells` row, shaped exactly as a live testnet reply.
pub fn cell_json(
    lock_args: &str,
    capacity: u64,
    tx_hash_fill: u8,
    index: u32,
) -> serde_json::Value {
    serde_json::json!({
        "block_number": "0x1554e00",
        "out_point": {
            "index": format!("{index:#x}"),
            "tx_hash": format!("0x{}", hex::encode([tx_hash_fill; 32])),
        },
        "output": {
            "capacity": format!("{capacity:#x}"),
            "lock": { "code_hash": SECP_CODE_HASH, "hash_type": "type", "args": lock_args },
            "type": null
        },
        "output_data": "0x",
        "tx_index": "0x1"
    })
}

/// A page of rows plus the continuation token the node would return.
pub fn cells_page(cells: &[serde_json::Value], last_cursor: &str) -> serde_json::Value {
    serde_json::json!({ "objects": cells, "last_cursor": last_cursor })
}

/// The chain tip every fixture here reports, and the filter height a synced
/// light client has reached for the wallet's own script.
pub const TIP: &str = "0x1554ef4";

/// A `get_scripts` reply placing `lock_args` at `height`.
///
/// `send` gates on `BackendStatus::is_usable()`, and a light backend derives
/// that by comparing `get_tip_header` with the `get_scripts` row for a script
/// *this client registered*. Without this reply the backend reports
/// `Connecting` — nothing of ours is watched, so its index is empty by
/// construction — and a send is refused before a cell is fetched.
pub fn scripts_at(lock_args: &str, height: &str) -> serde_json::Value {
    serde_json::json!([{
        "script": { "code_hash": SECP_CODE_HASH, "hash_type": "type", "args": lock_args },
        "script_type": "lock",
        "block_number": height
    }])
}

/// The three replies a light backend needs before it will report `Synced`
/// for `lock_args`: the tip, the filter progress, and an answer to the
/// registration itself.
pub fn synced_light_node(lock_args: &str) -> lantern_chain_backend::testing::FakeNodeBuilder {
    FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json(TIP))
        .respond("get_scripts", scripts_at(lock_args, TIP))
        .respond("set_scripts", serde_json::json!(null))
}

/// A node holding exactly one spendable cell under `lock_args`, whose scan
/// exhausts in a single page (`last_cursor` is the real `"0x"` sentinel).
pub async fn single_cell_node(lock_args: &str, capacity: u64) -> FakeNode {
    synced_light_node(lock_args)
        .respond(
            "get_cells",
            cells_page(&[cell_json(lock_args, capacity, 0xa1, 0)], "0x"),
        )
        .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
        .start()
        .await
}

/// Attach `manager` and register the wallet's scripts, as a real launch does.
///
/// Registration is what makes a light backend `is_usable()`: until it has
/// been asked to watch something it reports `Connecting`, and `send` refuses
/// that rather than scanning an index guaranteed to hold nothing.
pub async fn attach_and_register(
    core: &mut lantern_wallet_core::WalletCore,
    manager: lantern_chain_backend::BackendManager,
) {
    core.attach_backend(manager).expect("same network");
    core.sync_watched_scripts().await.expect("registers");
}

/// The transaction that actually reached the wire, as the node saw it.
pub fn broadcast_tx(node: &FakeNode) -> serde_json::Value {
    let (_, params) = node
        .calls()
        .into_iter()
        .find(|(method, _)| method == "send_transaction")
        .expect("broadcast happened");
    params[0].clone()
}

/// Recover the signer's `blake160` from a BROADCAST transaction, re-deriving
/// the RFC 0019 digest the way the on-chain lock does.
///
/// Nothing here reads any value the wallet computed: the transaction hash
/// comes from the broadcast raw part, the digest from the broadcast witness
/// slots with the group's lock field zeroed in place, and the public key from
/// the signature those bytes carry. If the wallet hashed anything other than
/// what it sent — the `-52` class this plan exists to rule out — the recovered
/// key is a different, unrelated point and the comparison fails.
pub fn recover_blake160_from_broadcast(tx_json: &serde_json::Value) -> [u8; 20] {
    let json_tx: lantern_chain_backend::Transaction =
        serde_json::from_value(tx_json.clone()).expect("a serialised transaction");
    let packed = ckb_types::packed::Transaction::from(json_tx);

    let mut tx_hash = [0u8; 32];
    tx_hash.copy_from_slice(&packed.calc_tx_hash().raw_data());

    let slots: Vec<Vec<u8>> = packed
        .witnesses()
        .into_iter()
        .map(|w| w.raw_data().to_vec())
        .collect();
    let first = slots.first().expect("a witness slot for the group").clone();
    assert_eq!(
        first.len(),
        85,
        "a WitnessArgs whose only field is a 65-byte lock"
    );
    let mut signature = [0u8; 65];
    signature.copy_from_slice(&first[20..85]);
    let mut zeroed = first;
    zeroed[20..85].fill(0);

    // One account means one script group covering every input, so the
    // remaining slots are the group's own and follow it in the stream.
    let others: Vec<&[u8]> = slots[1..].iter().map(Vec::as_slice).collect();
    let digest = sighash_all(&tx_hash, &zeroed, &others);
    blake160(&recover(&signature, &digest).expect("the broadcast signature recovers"))
}

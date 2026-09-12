//! Chain-backend attachment and light-client script registration.
//!
//! Split out of `wallet_e2e.rs` for size. These are plan 1d's concerns —
//! which scripts get registered, at which height, and what happens when the
//! backend is on the wrong chain or absent — and none of them builds or signs
//! a transaction, so they share only the node fixtures.

mod support;

use lantern_chain_backend::testing::FakeNode;
use lantern_sdk_schema::Network;
use lantern_wallet_core::{CoreError, ProfilePaths, WalletCore, WordCount};
use support::{header_json, light_manager, local_node_info_json, lock_args_of};
use tempfile::tempdir;

const TANK: &str =
    "tank planet champion pottery together intact quick police asset flower sudden question";

#[tokio::test]
async fn a_new_account_records_a_start_height_now_rather_than_deferring_it() {
    // An imported wallet may have arbitrary history: genesis, always.
    let imported_dir = tempdir().expect("tempdir");
    let mut imported = WalletCore::import(
        ProfilePaths::in_dir(imported_dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = imported.create_account("Imported").await.expect("account");
    assert_eq!(
        imported.watch_from_block(&account.id).expect("known"),
        Some(0),
        "an imported wallet may have arbitrary history"
    );

    // A created wallet with no backend cannot know the tip. Genesis is the
    // only safe answer: the address is usable the moment this returns, and
    // the wait for a backend is unbounded.
    let created_dir = tempdir().expect("tempdir");
    let (mut created, _phrase) = WalletCore::create(
        ProfilePaths::in_dir(created_dir.path()),
        b"pw",
        Network::Testnet,
        WordCount::Words12,
    )
    .expect("creates");
    let account = created.create_account("Fresh").await.expect("account");
    assert_eq!(
        created.watch_from_block(&account.id).expect("known"),
        Some(0),
        "with no backend the tip is unknown, so scan everything rather than \
         resolve to a tip read at some later, unbounded moment"
    );
}

#[tokio::test]
async fn a_created_account_records_the_tip_read_at_creation_time() {
    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        .start()
        .await;

    let dir = tempdir().expect("tempdir");
    let (mut core, _phrase) = WalletCore::create(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        WordCount::Words12,
    )
    .expect("creates");
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    let account = core.create_account("Fresh").await.expect("account");
    assert_eq!(
        core.watch_from_block(&account.id).expect("known"),
        Some(0x0155_4ef4),
        "with a reachable backend the height is the tip at creation, not later"
    );
    assert!(
        node.call_count("get_tip_header") >= 1,
        "the tip must actually be read at creation time"
    );

    // And it persists, so a later unlock does not rescan.
    core.lock();
    let core = WalletCore::unlock(ProfilePaths::in_dir(dir.path()), b"pw", Network::Testnet)
        .expect("reopens");
    assert_eq!(
        core.watch_from_block(&account.id).expect("known"),
        Some(0x0155_4ef4)
    );
}

/// Asserts a `set_scripts` call registered exactly two distinct scripts,
/// each at `expected_height_hex`, via `partial` (never `all`), each a `lock`
/// with `ScriptHashType::Type`. Split out of the test body so the test
/// itself stays under clippy's line-count lint.
fn assert_registered_two_distinct_scripts(params: &[serde_json::Value], expected_height_hex: &str) {
    assert_eq!(
        params[1], "partial",
        "`all` would wipe every script on a shared light client"
    );
    let scripts = params[0].as_array().expect("scripts is an array");
    assert_eq!(
        scripts.len(),
        2,
        "both accounts must be registered, not just the first"
    );
    let mut all_args = Vec::new();
    for entry in scripts {
        assert_eq!(
            entry["block_number"], expected_height_hex,
            "each script must be registered at the height its account recorded"
        );
        assert_eq!(entry["script_type"], "lock");
        assert_eq!(
            entry["script"]["hash_type"], "type",
            "the secp256k1 module uses ScriptHashType::Type"
        );
        all_args.push(
            entry["script"]["args"]
                .as_str()
                .expect("args is a string")
                .to_string(),
        );
    }
    assert_ne!(
        all_args[0], all_args[1],
        "the two accounts must not have been registered as the same script"
    );
}

#[tokio::test]
async fn syncing_registers_every_script_at_its_recorded_height() {
    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        // The server knows nothing yet, so the recorded heights stand.
        .respond("get_scripts", serde_json::json!([]))
        .respond("set_scripts", serde_json::json!(null))
        .start()
        .await;

    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, _phrase) =
        WalletCore::create(paths, b"pw", Network::Testnet, WordCount::Words12).expect("creates");
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");
    let account = core.create_account("Fresh").await.expect("account");
    let second = core.create_account("Second").await.expect("second account");
    assert_ne!(
        account.id, second.id,
        "the loop must be proven over more than one account"
    );

    core.sync_watched_scripts().await.expect("syncs");

    let (_, params) = node
        .calls()
        .into_iter()
        .find(|(method, _)| method == "set_scripts")
        .expect("watch_scripts must actually call set_scripts");
    assert_registered_two_distinct_scripts(
        params.as_array().expect("params is an array"),
        "0x1554ef4",
    );
}

#[tokio::test]
async fn syncing_twice_never_rewinds_the_servers_filter_progress() {
    // An imported wallet: every account starts at genesis, so a second sync
    // that resent the stored height would rewind the light client to block 0
    // and re-download every filter — on every launch, forever.
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Imported").await.expect("account");
    assert_eq!(core.watch_from_block(&account.id).expect("known"), Some(0));

    let args = format!("0x{}", hex::encode(lock_args_of(&account)));
    let progressed = serde_json::json!([{
        "script": {
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": args
        },
        "script_type": "lock",
        "block_number": "0x1554ef4"
    }]);
    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        // First sync: the server knows nothing. Second: it is at 0x1554ef4.
        .respond_sequence("get_scripts", vec![serde_json::json!([]), progressed])
        .respond("set_scripts", serde_json::json!(null))
        .start()
        .await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    core.sync_watched_scripts().await.expect("first sync");
    core.sync_watched_scripts().await.expect("second sync");

    let sent: Vec<serde_json::Value> = node
        .calls()
        .into_iter()
        .filter(|(method, _)| method == "set_scripts")
        .map(|(_, params)| params[0][0]["block_number"].clone())
        .collect();
    assert_eq!(sent.len(), 2, "both syncs must have registered");
    assert_eq!(
        sent[0], "0x0",
        "nothing known yet, so the stored height stands"
    );
    assert_eq!(
        sent[1], "0x1554ef4",
        "the second sync must send the server's own progress, not the stored \
         height — `partial` overwrites the stored height unconditionally, so \
         resending 0x0 rewinds filter sync to genesis and clears the matched \
         blocks"
    );
}

#[tokio::test]
async fn attaching_a_backend_on_another_network_is_refused() {
    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .start()
        .await;

    let dir = tempdir().expect("tempdir");
    let (mut core, _phrase) = WalletCore::create(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        WordCount::Words12,
    )
    .expect("creates");

    // secp256k1 lock args are chain-independent, so nothing downstream would
    // error: the wallet would render ckt1… addresses over mainnet cells.
    let manager = light_manager(dir.path(), Network::Mainnet, node.url()).await;
    let err = core
        .attach_backend(manager)
        .expect_err("a mainnet backend must not serve a testnet wallet");
    assert!(
        matches!(
            err,
            CoreError::BackendNetworkMismatch {
                wallet: Network::Testnet,
                backend: Network::Mainnet
            }
        ),
        "{err:?}"
    );
    assert!(
        core.backend().is_none(),
        "a rejected manager must not be adopted"
    );
}

#[tokio::test]
async fn syncing_without_a_backend_reports_not_ready() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (core, _phrase) =
        WalletCore::create(paths, b"pw", Network::Testnet, WordCount::Words12).expect("creates");
    let result = core.sync_watched_scripts().await;
    assert!(matches!(
        result,
        Err(CoreError::Backend(
            lantern_chain_backend::BackendError::NotReady
        ))
    ));
}

//! End-to-end proof across all four crates. Every assertion that involves
//! key material cross-checks against `signer-secp256k1`, whose own tests
//! pin the lumos and BIP32 vectors.

use lantern_sdk_schema::{AccountRecord, Network};
use lantern_signer_secp256k1::{SigningKey, blake160, public_key, recover};
use lantern_vault::{Vault, VaultError};
use lantern_wallet_core::{CoreError, MnemonicFormat, ProfilePaths, WalletCore, WordCount};
use tempfile::tempdir;

const TANK: &str =
    "tank planet champion pottery together intact quick police asset flower sudden question";
const P1: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const P2: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const P3: &str = "letter advice cage absurd amount doctor acoustic avoid letter advice cage above";

fn lock_args_of(record: &AccountRecord) -> Vec<u8> {
    let hex_args = record.public_metadata["lockArgs"]
        .as_str()
        .expect("lockArgs is a string");
    hex::decode(hex_args.trim_start_matches("0x")).expect("hex")
}

#[tokio::test]
async fn create_three_accounts_lock_unlock_sign_and_recover() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, phrase) =
        WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words24)
            .expect("creates");
    assert_eq!(phrase.word_count(), 24);

    let a0 = core.create_account("One").await.expect("account 0");
    let a1 = core.create_account("Two").await.expect("account 1");
    let a2 = core.create_account("Three").await.expect("account 2");
    assert_eq!(a0.public_metadata["derivation"]["index"], 0);
    assert_eq!(a1.public_metadata["derivation"]["index"], 1);
    assert_eq!(a2.public_metadata["derivation"]["index"], 2);
    assert!(a0.address.starts_with("ckt1"), "{}", a0.address);
    assert_ne!(a0.id, a1.id);
    assert_ne!(lock_args_of(&a0), lock_args_of(&a1));
    core.lock();

    let mut core = WalletCore::unlock(paths, b"pw", Network::Testnet).expect("unlocks");
    let accounts = core.accounts().expect("lists");
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[1], a1);

    let a3 = core.create_account("Four").await.expect("account 3");
    assert_eq!(a3.public_metadata["derivation"]["index"], 3);

    let digest = [0x5au8; 32];
    let signature = core.signer().sign_digest(&a1.id, &digest).expect("signs");
    assert_eq!(signature.len(), 65);
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&signature);
    let recovered = recover(&arr, &digest).expect("recovers");
    assert_eq!(blake160(&recovered).to_vec(), lock_args_of(&a1));

    assert!(matches!(
        core.signer().sign_digest("nope", &digest),
        Err(CoreError::AccountNotFound)
    ));
}

#[tokio::test]
async fn imported_phrase_yields_the_lumos_key_and_reveals_with_the_password() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let mut core = WalletCore::import(paths, b"pw", Network::Testnet, TANK).expect("imports");
    assert_eq!(
        core.mnemonic_format().expect("format"),
        MnemonicFormat::Single
    );

    let account = core.create_account("Imported").await.expect("account");
    let key_bytes = hex::decode("848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14")
        .expect("hex");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&key_bytes);
    let expected = blake160(&public_key(&SigningKey::from_bytes(arr).expect("valid")));
    assert_eq!(lock_args_of(&account), expected.to_vec());
    assert_eq!(
        account.lock_type,
        lantern_sdk_schema::LockType::Secp256k1Blake160
    );
    assert_eq!(account.extension_id, "core.secp256k1");
    assert!(account.capabilities.can_sign);

    let again = core.reveal_mnemonic(b"pw").expect("reveals");
    assert_eq!(again.expose(), TANK);
    assert!(matches!(
        core.reveal_mnemonic(b"wrong"),
        Err(CoreError::Vault(VaultError::WrongPassword))
    ));
}

#[tokio::test]
async fn quantum_purse_combined_phrase_imports_and_signs() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let combined = format!("{P1} {P2} {P3}");
    let mut core = WalletCore::import(paths, b"pw", Network::Mainnet, &combined).expect("imports");
    assert_eq!(
        core.mnemonic_format().expect("format"),
        MnemonicFormat::Combined3
    );

    let account = core.create_account("PQ import").await.expect("account");
    assert!(account.address.starts_with("ckb1"), "{}", account.address);
    assert_eq!(lock_args_of(&account).len(), 20);

    let digest = [0x77u8; 32];
    let signature = core
        .signer()
        .sign_digest(&account.id, &digest)
        .expect("signs");
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&signature);
    let recovered = recover(&arr, &digest).expect("recovers");
    assert_eq!(blake160(&recovered).to_vec(), lock_args_of(&account));

    let again = core.reveal_mnemonic(b"pw").expect("reveals");
    assert_eq!(again.expose(), combined);
    assert_eq!(again.word_count(), 36);
}

#[test]
fn create_refuses_an_existing_vault_and_unlock_needs_a_seed() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let (core, _phrase) =
            WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12)
                .expect("creates");
        core.lock();
    }
    assert!(matches!(
        WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12),
        Err(CoreError::AlreadyInitialised)
    ));
    assert!(matches!(
        WalletCore::import(paths, b"pw", Network::Testnet, TANK),
        Err(CoreError::AlreadyInitialised)
    ));

    let empty = tempdir().expect("tempdir");
    let empty_paths = ProfilePaths::in_dir(empty.path());
    Vault::create(&empty_paths.vault, b"pw").expect("bare vault");
    assert!(matches!(
        WalletCore::unlock(empty_paths, b"pw", Network::Testnet),
        Err(CoreError::SeedMissing)
    ));

    // A pre-existing accounts.json with no vault must also be refused —
    // create/import must not silently adopt a stale account list.
    let stale = tempdir().expect("tempdir");
    let stale_paths = ProfilePaths::in_dir(stale.path());
    std::fs::write(&stale_paths.accounts, br#"{"version":1,"accounts":[]}"#)
        .expect("writes accounts.json");
    assert!(matches!(
        WalletCore::create(
            stale_paths.clone(),
            b"pw",
            Network::Testnet,
            WordCount::Words12
        ),
        Err(CoreError::AlreadyInitialised)
    ));
    assert!(matches!(
        WalletCore::import(stale_paths, b"pw", Network::Testnet, TANK),
        Err(CoreError::AlreadyInitialised)
    ));
}

#[test]
fn bad_phrase_on_import_leaves_no_vault_file() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    assert!(matches!(
        WalletCore::import(paths.clone(), b"pw", Network::Testnet, "not a valid phrase"),
        Err(CoreError::InvalidMnemonic)
    ));
    assert!(
        !paths.vault.exists(),
        "an invalid phrase must not create vault.bin"
    );
    // A correct retry on the same paths must now succeed.
    WalletCore::import(paths, b"pw", Network::Testnet, TANK).expect("retry succeeds");
}

#[tokio::test]
async fn unlock_rejects_a_tampered_accounts_file() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let real_args = {
        let mut core =
            WalletCore::import(paths.clone(), b"pw", Network::Testnet, TANK).expect("imports");
        let account = core.create_account("Main").await.expect("account");
        core.lock();
        lock_args_of(&account)
    };

    // Swap the stored lock args for an attacker's, exactly as a local editor could.
    let text = std::fs::read_to_string(&paths.accounts).expect("reads");
    let tampered = text.replace(&hex::encode(&real_args), &"ab".repeat(20));
    assert_ne!(tampered, text, "the fixture must actually change");
    std::fs::write(&paths.accounts, tampered).expect("writes");

    let err = WalletCore::unlock(paths.clone(), b"pw", Network::Testnet)
        .expect_err("must refuse a tampered registry");
    assert!(
        matches!(err, CoreError::RegistryMismatch { .. }),
        "expected RegistryMismatch, got {err:?}"
    );

    // Restoring the file makes it open again — the check is about content, not a latch.
    std::fs::write(&paths.accounts, text).expect("restores");
    WalletCore::unlock(paths, b"pw", Network::Testnet).expect("opens again");
}

#[tokio::test]
async fn unlock_accepts_an_untouched_registry_with_several_accounts() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let (mut core, _phrase) =
            WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12)
                .expect("creates");
        for label in ["One", "Two", "Three"] {
            core.create_account(label).await.expect("account");
        }
        core.lock();
    }
    let core = WalletCore::unlock(paths, b"pw", Network::Testnet).expect("opens");
    assert_eq!(core.accounts().expect("lists").len(), 3);
}

#[tokio::test]
async fn nulling_the_derivation_does_not_smuggle_a_swapped_address_past_verification() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let mut core =
            WalletCore::import(paths.clone(), b"pw", Network::Testnet, TANK).expect("imports");
        core.create_account("Main").await.expect("account");
        core.lock();
    }
    // The full attack: swap the address AND null the derivation, so a
    // verifier that skips underivable accounts would wave it through.
    let text = std::fs::read_to_string(&paths.accounts).expect("reads");
    let mut file: serde_json::Value = serde_json::from_str(&text).expect("parses");
    let account = file["accounts"]
        .as_array_mut()
        .expect("accounts array")
        .get_mut(0)
        .expect("one account");
    account["lockArgs"] = serde_json::Value::String("ab".repeat(20));
    account["derivation"] = serde_json::Value::Null;
    let attacked = serde_json::to_string_pretty(&file).expect("serializes");
    assert_ne!(attacked, text, "the fixture must actually change");
    std::fs::write(&paths.accounts, attacked).expect("writes");

    match WalletCore::unlock(paths, b"pw", Network::Testnet) {
        // A parse rejection (`Registry`) is also a refusal.
        Err(CoreError::RegistryMismatch { .. } | CoreError::Registry(_)) => {}
        other => panic!("a nulled derivation must not be trusted, got {other:?}"),
    }
}

/// A `local_node_info` reply shaped like a real light client's, so
/// `RemoteLight::start()`'s reachability probe succeeds.
fn local_node_info_json() -> serde_json::Value {
    serde_json::json!({
        "version": "0.5.5",
        "node_id": "QmTestNode",
        "active": true,
        "addresses": [],
        "protocols": [],
        "connections": "0x0"
    })
}

fn header_json(number: &str) -> serde_json::Value {
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
async fn light_manager(
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
    use lantern_chain_backend::testing::FakeNode;

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
    use lantern_chain_backend::testing::FakeNode;

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
    use lantern_chain_backend::testing::FakeNode;

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
    use lantern_chain_backend::testing::FakeNode;

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
    let (mut core, _phrase) =
        WalletCore::create(paths, b"pw", Network::Testnet, WordCount::Words12).expect("creates");
    let result = core.sync_watched_scripts().await;
    assert!(matches!(
        result,
        Err(CoreError::Backend(
            lantern_chain_backend::BackendError::NotReady
        ))
    ));
}

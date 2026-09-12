//! End-to-end proof across all four crates. Every assertion that involves
//! key material cross-checks against `signer-secp256k1`, whose own tests
//! pin the lumos and BIP32 vectors.

use lantern_chain_backend::testing::FakeNode;
use lantern_sdk_schema::{AccountRecord, Network};
use lantern_signer_secp256k1::{SigningKey, blake160, public_key, recover, sighash_all};
use lantern_vault::{Vault, VaultError};
use lantern_wallet_core::{CoreError, MnemonicFormat, ProfilePaths, WalletCore, WordCount};
use tempfile::tempdir;

const SHANNONS_PER_CKB: u64 = 100_000_000;

/// The system `secp256k1_blake160_sighash_all` code hash, as the indexer
/// renders it in a `get_cells` reply.
const SECP_CODE_HASH: &str = "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8";

/// What the fake node answers `send_transaction` with.
const BROADCAST_HASH: &str = "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1";

/// RFC 0021's own published testnet vector — the identical string
/// `account-registry`'s encoder is pinned against. Used as the recipient so
/// the send path's address decoding is exercised against a third-party value
/// rather than against a round trip through our own encoder.
const RECIPIENT: &str = "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqgutnjd";
const RECIPIENT_ARGS: &str = "0xb39bbc0b3673c7d36450bc14cfcdad2d559c6c64";

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

    // Account resolution happens before anything else in `send`, so an
    // unknown id is refused without a backend, a builder or a signer.
    assert!(matches!(
        core.send("nope", RECIPIENT, 100 * SHANNONS_PER_CKB).await,
        Err(CoreError::AccountNotFound)
    ));
}

#[tokio::test]
async fn a_broadcast_signature_recovers_to_the_sending_accounts_lock_args() {
    // The replacement for this file's two `#[ignore]`d tests, which recovered
    // over a hardcoded `[0x5a; 32]` digest that no code path ever produced and
    // asserted a bare 65-byte signature where the API now returns an 85-byte
    // `WitnessArgs`. Removing their `#[ignore]` would not have made them mean
    // anything.
    //
    // This asserts the property they were reaching for, against the bytes that
    // actually went on the wire: take the BROADCAST transaction, re-derive the
    // RFC 0019 digest from it exactly as the on-chain lock does — zero the
    // group's witness lock field in place and hash the whole stream — recover
    // the public key from the signature it carries, and require its blake160
    // to be the args the spent cells are locked to. Nothing here reads the
    // wallet's own intermediate values.
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Main").await.expect("account");
    let args = lock_args_of(&account);

    let node = single_cell_node(&hex_args(&args), 1000 * SHANNONS_PER_CKB).await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");
    core.send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
        .await
        .expect("sends");

    assert_eq!(
        recover_blake160_from_broadcast(&broadcast_tx(&node)).to_vec(),
        args,
        "the signature on the wire must recover to the account that owns the \
         cells it spends"
    );
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
async fn quantum_purse_combined_phrase_imports() {
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

    let again = core.reveal_mnemonic(b"pw").expect("reveals");
    assert_eq!(again.expose(), combined);
    assert_eq!(again.word_count(), 36);
}

#[tokio::test]
async fn a_combined_phrase_wallet_sends_on_mainnet_and_its_signature_recovers() {
    // The second of the two rewritten `#[ignore]`d tests. Its point is that a
    // 36-word Quantum-Purse-style phrase reaches the identical signing path —
    // the entropy is longer, the BIP39 seed it expands to is still 64 bytes —
    // and that a mainnet wallet sends to a `ckb1…` recipient, not a `ckt1…`
    // one. Everything else is the testnet case.
    let dir = tempdir().expect("tempdir");
    let combined = format!("{P1} {P2} {P3}");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Mainnet,
        &combined,
    )
    .expect("imports");
    let account = core.create_account("PQ import").await.expect("account");
    let args = lock_args_of(&account);
    assert!(account.address.starts_with("ckb1"), "{}", account.address);

    let node = single_cell_node(&hex_args(&args), 1000 * SHANNONS_PER_CKB).await;
    core.attach_backend(light_manager(dir.path(), Network::Mainnet, node.url()).await)
        .expect("same network");
    // A self-transfer: the recipient is the account's own mainnet address, so
    // no second fixture address is needed and the `ckb`/`ckt` prefix check is
    // exercised from the mainnet side.
    core.send(&account.id, &account.address, 100 * SHANNONS_PER_CKB)
        .await
        .expect("sends");

    assert_eq!(
        recover_blake160_from_broadcast(&broadcast_tx(&node)).to_vec(),
        args
    );

    // ...and a testnet address must not be spendable to from a mainnet wallet.
    assert!(matches!(
        core.send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
            .await,
        Err(CoreError::Registry(_))
    ));
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

fn hex_args(args: &[u8]) -> String {
    format!("0x{}", hex::encode(args))
}

/// One `get_cells` row, shaped exactly as a live testnet reply.
fn cell_json(lock_args: &str, capacity: u64, tx_hash_fill: u8, index: u32) -> serde_json::Value {
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
fn cells_page(cells: &[serde_json::Value], last_cursor: &str) -> serde_json::Value {
    serde_json::json!({ "objects": cells, "last_cursor": last_cursor })
}

/// A node holding exactly one spendable cell under `lock_args`, whose scan
/// exhausts in a single page (`last_cursor` is the real `"0x"` sentinel).
async fn single_cell_node(lock_args: &str, capacity: u64) -> FakeNode {
    FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        .respond(
            "get_cells",
            cells_page(&[cell_json(lock_args, capacity, 0xa1, 0)], "0x"),
        )
        .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
        .start()
        .await
}

/// The transaction that actually reached the wire, as the node saw it.
fn broadcast_tx(node: &FakeNode) -> serde_json::Value {
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
fn recover_blake160_from_broadcast(tx_json: &serde_json::Value) -> [u8; 20] {
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

#[tokio::test]
async fn sending_builds_signs_and_broadcasts() {
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Main").await.expect("account");
    let args = lock_args_of(&account);

    let node = single_cell_node(&hex_args(&args), 1000 * SHANNONS_PER_CKB).await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    let hash = core
        .send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
        .await
        .expect("sends");
    assert_eq!(hex_args(hash.as_bytes()), BROADCAST_HASH);

    let tx = broadcast_tx(&node);

    let inputs = tx["inputs"].as_array().expect("inputs");
    assert_eq!(
        inputs.len(),
        1,
        "one 1000 CKB cell covers 100 CKB and a fee"
    );
    assert_eq!(
        inputs[0]["previous_output"]["tx_hash"],
        format!("0x{}", hex::encode([0xa1u8; 32])),
        "the transaction must spend the cell the node served, not an invention"
    );

    let outputs = tx["outputs"].as_array().expect("outputs");
    assert_eq!(outputs.len(), 2, "recipient and change");
    assert_eq!(
        outputs[0]["lock"]["args"], RECIPIENT_ARGS,
        "the recipient address must decode to the script that gets paid"
    );
    assert_eq!(
        outputs[0]["capacity"],
        format!("{:#x}", 100 * SHANNONS_PER_CKB)
    );
    assert_eq!(
        outputs[1]["lock"]["args"],
        hex_args(&args),
        "change must return to the sending account, not to the recipient"
    );

    // A fee must actually have been withheld: outputs summing to inputs is
    // the zero-fee bug that reaches a node as PoolRejectedTransactionByMinFeeRate.
    let change = u64::from_str_radix(
        outputs[1]["capacity"]
            .as_str()
            .expect("hex")
            .trim_start_matches("0x"),
        16,
    )
    .expect("hex");
    let fee = 1000 * SHANNONS_PER_CKB - 100 * SHANNONS_PER_CKB - change;
    assert!(fee > 0, "the transaction paid no fee at all");
    assert!(fee < SHANNONS_PER_CKB, "an absurd fee of {fee} shannons");

    let witnesses = tx["witnesses"].as_array().expect("witnesses");
    assert_eq!(
        witnesses.len(),
        inputs.len(),
        "a witness slot per input, even when empty"
    );
    let first = witnesses[0].as_str().expect("hex");
    assert_ne!(
        &first[42..],
        &"0".repeat(130),
        "the lock field must carry a signature, not the placeholder"
    );

    assert!(
        !tx["cell_deps"].as_array().expect("cell_deps").is_empty(),
        "without the secp256k1 dep group the chain answers ScriptNotFound, \
         and nothing local would notice"
    );
}

#[tokio::test]
async fn collecting_candidates_pages_until_the_scan_is_exhausted() {
    // Two cells, one per page, and a transfer that cannot be funded by either
    // alone: a pager that stopped after the first page would report
    // `InsufficientFunds` over a wallet that has the money. The second page is
    // empty with the real `"0x"` sentinel, which is what a live node returns
    // once a scan is exhausted.
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Main").await.expect("account");
    let args = hex_args(&lock_args_of(&account));

    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        .respond_sequence(
            "get_cells",
            vec![
                cells_page(
                    &[cell_json(&args, 85 * SHANNONS_PER_CKB, 0xb1, 0)],
                    "0x40aabb",
                ),
                cells_page(
                    &[cell_json(&args, 85 * SHANNONS_PER_CKB, 0xb2, 1)],
                    "0x40aacc",
                ),
                cells_page(&[], "0x"),
            ],
        )
        .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
        .start()
        .await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    core.send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
        .await
        .expect("sends");

    assert_eq!(
        node.call_count("get_cells"),
        3,
        "two pages of rows plus the empty page that ends the scan"
    );

    let tx = broadcast_tx(&node);
    let inputs = tx["inputs"].as_array().expect("inputs");
    assert_eq!(inputs.len(), 2, "neither cell funds the transfer alone");
    let witnesses = tx["witnesses"].as_array().expect("witnesses");
    assert_eq!(witnesses.len(), 2, "a slot per input");
    assert_eq!(
        witnesses[1], "0x",
        "only the group's first slot carries a signature; the rest must be \
         present and empty, because the sighash stream length-prefixes each one"
    );

    // The multi-input digest is the property a one-input test cannot reach.
    assert_eq!(
        recover_blake160_from_broadcast(&tx).to_vec(),
        lock_args_of(&account)
    );
}

#[tokio::test]
async fn a_cell_carrying_a_type_script_or_data_is_never_spent_as_plain_capacity() {
    // A plain transfer writes outputs with no type script and no data. Feeding
    // it a token cell would consume the tokens and re-issue none — the sUDT
    // script permits burning, so this is silent loss rather than a rejection.
    // The indexer also matches lock args by PREFIX by default, so a lock whose
    // args merely start with ours comes back from the same query under a
    // different script hash, which would put two script groups in one group's
    // witness and fail on chain as -52.
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Main").await.expect("account");
    let args = hex_args(&lock_args_of(&account));

    let mut token = cell_json(&args, 1000 * SHANNONS_PER_CKB, 0xc1, 0);
    token["output"]["type"] = serde_json::json!({
        "code_hash": "0xc5e5dcf215925f7ef4dfaf5f4b4f105bc321c02776d6e7d52a1db3fcd9d011a4",
        "hash_type": "type",
        "args": "0x32e555f3ff8e135cece1351a6a2971518392c1e30375c1e006ad0ce8eac07947"
    });
    token["output_data"] = serde_json::json!("0x00e1f50500000000000000000000000000");

    let mut longer_args = cell_json(&args, 1000 * SHANNONS_PER_CKB, 0xc2, 0);
    longer_args["output"]["lock"]["args"] = serde_json::json!(format!("{args}ff"));

    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        .respond("get_cells", cells_page(&[token, longer_args], "0x"))
        .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
        .start()
        .await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    let err = core
        .send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
        .await
        .expect_err("neither cell is plain capacity under this account's lock");
    assert!(
        matches!(
            err,
            CoreError::Build(lantern_tx_builder::BuildError::NoSpendableCells)
        ),
        "{err:?}"
    );
    assert_eq!(
        node.call_count("send_transaction"),
        0,
        "nothing may reach the wire"
    );
}

#[tokio::test]
async fn a_cell_whose_data_the_node_did_not_report_is_not_treated_as_empty() {
    // `IndexerCell.output_data` is an `Option`. A reply that omits the field —
    // or sends JSON `null` — carries no evidence that the cell is empty; it
    // says the node did not tell us. Treating that as "no data" accepts
    // exactly the token cells the filter above exists to reject, and the
    // divergence surfaces only as an on-chain lock error, because the signer
    // is handed input contents that do not match the cell being spent.
    //
    // The query now asks for the data explicitly, so this state should not
    // arise — but "should not arise" is a server-side default, not a
    // guarantee, and every other fixture in this file spells `"output_data":
    // "0x"` out, so nothing else covers it.
    let dir = tempdir().expect("tempdir");
    let mut core = WalletCore::import(
        ProfilePaths::in_dir(dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = core.create_account("Main").await.expect("account");
    let args = hex_args(&lock_args_of(&account));

    // Otherwise perfectly spendable: this account's exact lock, no type
    // script, ample capacity. The absent data field is the only difference
    // from the cell that funds every other test here.
    let mut omitted = cell_json(&args, 1000 * SHANNONS_PER_CKB, 0xd1, 0);
    omitted
        .as_object_mut()
        .expect("object")
        .remove("output_data");
    let mut null_data = cell_json(&args, 1000 * SHANNONS_PER_CKB, 0xd2, 0);
    null_data["output_data"] = serde_json::Value::Null;

    let node = FakeNode::builder()
        .respond("local_node_info", local_node_info_json())
        .respond("get_tip_header", header_json("0x1554ef4"))
        .respond("get_cells", cells_page(&[omitted, null_data], "0x"))
        .respond("send_transaction", serde_json::json!(BROADCAST_HASH))
        .start()
        .await;
    core.attach_backend(light_manager(dir.path(), Network::Testnet, node.url()).await)
        .expect("same network");

    let err = core
        .send(&account.id, RECIPIENT, 100 * SHANNONS_PER_CKB)
        .await
        .expect_err("a cell whose contents are unknown is not plain capacity");
    assert!(
        matches!(
            err,
            CoreError::Build(lantern_tx_builder::BuildError::NoSpendableCells)
        ),
        "{err:?}"
    );
    assert_eq!(
        node.call_count("send_transaction"),
        0,
        "nothing may reach the wire"
    );
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

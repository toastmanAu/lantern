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

#[test]
fn create_three_accounts_lock_unlock_sign_and_recover() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, phrase) =
        WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words24)
            .expect("creates");
    assert_eq!(phrase.word_count(), 24);

    let a0 = core.create_account("One").expect("account 0");
    let a1 = core.create_account("Two").expect("account 1");
    let a2 = core.create_account("Three").expect("account 2");
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

    let a3 = core.create_account("Four").expect("account 3");
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

#[test]
fn imported_phrase_yields_the_lumos_key_and_reveals_with_the_password() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let mut core = WalletCore::import(paths, b"pw", Network::Testnet, TANK).expect("imports");
    assert_eq!(
        core.mnemonic_format().expect("format"),
        MnemonicFormat::Single
    );

    let account = core.create_account("Imported").expect("account");
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

#[test]
fn quantum_purse_combined_phrase_imports_and_signs() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let combined = format!("{P1} {P2} {P3}");
    let mut core = WalletCore::import(paths, b"pw", Network::Mainnet, &combined).expect("imports");
    assert_eq!(
        core.mnemonic_format().expect("format"),
        MnemonicFormat::Combined3
    );

    let account = core.create_account("PQ import").expect("account");
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

#[test]
fn unlock_rejects_a_tampered_accounts_file() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let real_args = {
        let mut core =
            WalletCore::import(paths.clone(), b"pw", Network::Testnet, TANK).expect("imports");
        let account = core.create_account("Main").expect("account");
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

#[test]
fn unlock_accepts_an_untouched_registry_with_several_accounts() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let (mut core, _phrase) =
            WalletCore::create(paths.clone(), b"pw", Network::Testnet, WordCount::Words12)
                .expect("creates");
        for label in ["One", "Two", "Three"] {
            core.create_account(label).expect("account");
        }
        core.lock();
    }
    let core = WalletCore::unlock(paths, b"pw", Network::Testnet).expect("opens");
    assert_eq!(core.accounts().expect("lists").len(), 3);
}

#[test]
fn nulling_the_derivation_does_not_smuggle_a_swapped_address_past_verification() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    {
        let mut core =
            WalletCore::import(paths.clone(), b"pw", Network::Testnet, TANK).expect("imports");
        core.create_account("Main").expect("account");
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

#[test]
fn an_imported_wallet_scans_from_genesis_and_a_created_one_defers() {
    let imported_dir = tempdir().expect("tempdir");
    let mut imported = WalletCore::import(
        ProfilePaths::in_dir(imported_dir.path()),
        b"pw",
        Network::Testnet,
        TANK,
    )
    .expect("imports");
    let account = imported.create_account("Imported").expect("account");
    assert_eq!(
        imported.watch_from_block(&account.id).expect("known"),
        Some(0),
        "an imported wallet may have arbitrary history"
    );

    let created_dir = tempdir().expect("tempdir");
    let (mut created, _phrase) = WalletCore::create(
        ProfilePaths::in_dir(created_dir.path()),
        b"pw",
        Network::Testnet,
        WordCount::Words12,
    )
    .expect("creates");
    let account = created.create_account("Fresh").expect("account");
    assert_eq!(
        created.watch_from_block(&account.id).expect("known"),
        None,
        "a fresh wallet defers to the first sync"
    );
}

#[tokio::test]
async fn syncing_resolves_heights_and_registers_every_script() {
    use lantern_chain_backend::testing::FakeNode;

    let node = FakeNode::builder()
        .respond(
            "get_indexer_tip",
            serde_json::json!({
                "block_hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                "block_number": "0x1554ef4"
            }),
        )
        .respond(
            "get_tip_header",
            serde_json::json!({
                "compact_target": "0x1a08a97e", "dao": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "epoch": "0x1", "extra_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                "nonce": "0x0", "number": "0x1554ef4",
                "parent_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "proposals_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "timestamp": "0x1", "transactions_root": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "version": "0x0"
            }),
        )
        // `BackendManager::activate` calls `FullNode::start()`, which probes
        // `local_node_info` — the brief's mock omitted this route, and the
        // fake node returns an RPC error for any unmocked method.
        .respond(
            "local_node_info",
            serde_json::json!({
                "version": "0.5.5",
                "node_id": "QmTestNode",
                "active": true,
                "addresses": [],
                "protocols": [],
                "connections": "0x0"
            }),
        )
        .start()
        .await;

    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, _phrase) =
        WalletCore::create(paths, b"pw", Network::Testnet, WordCount::Words12).expect("creates");
    let account = core.create_account("Fresh").expect("account");
    assert_eq!(core.watch_from_block(&account.id).expect("known"), None);

    let mut manager = lantern_chain_backend::BackendManager::open(dir.path().join("backends.json"))
        .expect("manager");
    manager
        .add_profile(lantern_sdk_schema::BackendProfile {
            id: "fake".into(),
            label: "fake".into(),
            network: Network::Testnet,
            kind: lantern_sdk_schema::BackendKind::RemoteFull,
            endpoint: Some(node.url()),
        })
        .expect("adds");
    manager.activate("fake").await.expect("activates");
    core.attach_backend(manager);

    core.sync_watched_scripts().await.expect("syncs");
    assert_eq!(
        core.watch_from_block(&account.id).expect("known"),
        Some(0x0155_4ef4),
        "the deferred height resolved to the tip"
    );

    // Resolved heights persist, so a later unlock does not rescan.
    core.lock();
    let core = WalletCore::unlock(ProfilePaths::in_dir(dir.path()), b"pw", Network::Testnet)
        .expect("reopens");
    assert_eq!(
        core.watch_from_block(&account.id).expect("known"),
        Some(0x0155_4ef4)
    );
}

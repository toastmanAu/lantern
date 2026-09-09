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

    let core = WalletCore::unlock(paths, b"pw", Network::Testnet).expect("unlocks");
    let accounts = core.accounts().expect("lists");
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[1], a1);

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

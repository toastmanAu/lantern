//! A real transfer on CKB testnet.
//!
//! Two gates, deliberately: `#[ignore]` so a skipped run reports `ignored`
//! rather than `ok`, and an environment variable so it cannot fire from a
//! workflow edit alone. Running it requires both `--ignored` and the
//! variable.
//!
//! Run with:
//!   `LANTERN_LIVE_TESTNET=1` `LANTERN_LIVE_MNEMONIC`="..." \
//!     cargo test -p lantern-wallet-core --test `live_send` -- --ignored --nocapture
//!
//! The mnemonic is read from the environment and handed straight to
//! `WalletCore::import`, which zeroizes it on drop. Nothing in this file
//! ever formats, logs, or asserts against the phrase, the seed, or any
//! derived secret — only public chain data (an address, a transaction
//! hash) reaches stdout.

use lantern_chain_backend::BackendManager;
use lantern_sdk_schema::{BackendKind, BackendProfile, Network};
use lantern_wallet_core::{ProfilePaths, WalletCore};

const RPC: &str = "https://testnet.ckb.dev/";

/// Two independent checks, deliberately not folded into one `Option`: a
/// half-configured environment (the flag set, the mnemonic missing) must
/// fail loudly, not read as a quiet, successful skip.
enum Gate {
    /// Neither variable set (or the flag not exactly `"1"`): a normal,
    /// silent skip.
    Skipped,
    /// The flag is set: the mnemonic is required, in full, here.
    Enabled(String),
}

fn gate() -> Gate {
    if std::env::var("LANTERN_LIVE_TESTNET").as_deref() != Ok("1") {
        return Gate::Skipped;
    }
    let phrase = std::env::var("LANTERN_LIVE_MNEMONIC").expect(
        "LANTERN_LIVE_TESTNET=1 was set but LANTERN_LIVE_MNEMONIC was not — \
         half-configured environment refused rather than silently skipped",
    );
    Gate::Enabled(phrase)
}

#[ignore = "live: set LANTERN_LIVE_TESTNET=1 and LANTERN_LIVE_MNEMONIC, run with --ignored"]
#[tokio::test]
async fn a_real_transfer_is_accepted_by_the_testnet_pool() {
    let phrase = match gate() {
        Gate::Skipped => {
            eprintln!("skipped: LANTERN_LIVE_TESTNET=1 and LANTERN_LIVE_MNEMONIC required");
            return;
        }
        Gate::Enabled(phrase) => phrase,
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let mut wallet = WalletCore::import(paths, b"live-test-password", Network::Testnet, &phrase)
        .expect("import a funded testnet phrase");
    // `phrase` (a `String` from `std::env::var`) is not zeroized by us —
    // `WalletCore::import` only borrows it — but it is never printed,
    // logged, or included in any panic message above or below this line.
    drop(phrase);

    let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
    manager
        .add_profile(BackendProfile {
            id: "live-testnet".to_string(),
            label: "Live testnet (read-only RPC check permitted, broadcast gated)".to_string(),
            network: Network::Testnet,
            kind: BackendKind::RemoteFull,
            endpoint: Some(RPC.to_string()),
        })
        .expect("add profile");
    manager.activate("live-testnet").await.expect("activate");
    wallet.attach_backend(manager).expect("network matches");

    let account = wallet
        .create_account("live")
        .await
        .expect("derive account 0");
    wallet
        .sync_watched_scripts()
        .await
        .expect("register the watched script (no-op on a full backend)");

    // Self-transfer: no second funded party needed, and it still exercises
    // selection, fee measurement, whole-transaction signing and broadcast.
    // 200 CKB is comfortably above the 61 CKB secp change floor — an amount
    // near the floor fails for capacity reasons before the lock script
    // runs, which would mask exactly the signature validity this test
    // exists to prove.
    let amount_shannons = 200 * 100_000_000;
    let hash = wallet
        .send(&account.id, &account.address, amount_shannons)
        .await
        .expect("pool accepts the transaction");

    eprintln!("broadcast: {hash:#x}");
}

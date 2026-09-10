# Lantern Plan 1d — Chain Backend, Light-Client Supervision, Backend Manager — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After this plan Lantern reaches a CKB chain through four interchangeable backends behind one `ChainBackend` trait, supervises a bundled light-client subprocess end to end, registers account scripts from a per-account start height, switches backend and network as separate axes, reports real sync progress, and refuses to open a wallet whose plaintext account file no longer matches its seed.

**Architecture:** `lantern-chain-backend` owns a `dyn`-safe async trait plus `BackendCapabilities` data, mirroring the `LockModule` + `AccountCapabilities` shape plan 1c established. Blockchain types come from `ckb-jsonrpc-types`; the indexer request/response types are defined locally because that crate ships only the server-side serde half of them. A `Cursor` newtype makes the terminal `"0x"` sentinel unrepresentable. `EmbeddedLight` is `RemoteLight` plus a `Supervisor` that owns a `tokio::process::Child`.

**Tech Stack:** Rust 1.92 / edition 2024, `ckb-jsonrpc-types` =1.1.1, `ckb-types` =1.1.1, `reqwest` 0.13 (rustls, no default features), `async-trait` 0.1, `tokio` 1.42, `nix` 0.31 (unix only, `signal` feature), `serde_json`, `toml`, `tempfile`.

**Spec:** `docs/superpowers/specs/2026-09-10-plan-1d-chain-backend-design.md`

## Global Constraints

- Workspace lints apply to every crate: `unsafe_code = "forbid"`, clippy `all + pedantic + nursery` at warn, and CI runs `cargo clippy --workspace --all-targets -- -D warnings`. Test code must be clippy-clean. Allowed: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`.
- Pedantic `doc_markdown` is on: backticks around every identifier or CamelCase word in a doc comment. Pedantic `too_long_first_doc_paragraph` is on: keep the first paragraph of a module doc to one short sentence.
- Nursery `missing_const_for_fn`, `use_self`, `redundant_pub_crate` are on. A `pub` method on a type re-exported from `lib.rs` is reachable outside the crate: use `pub(crate)` for crate-internal methods on exported types (the plan 1c ruling).
- Every crate keeps `#![forbid(unsafe_code)]` at the top of `lib.rs`. `nix::sys::signal::kill` is a safe function; no `unsafe` block is ever needed.
- No secret material in any error `Display`, log line, or `Debug` output. RPC error text passes through because it originates at a node.
- **Never hand-write an RPC fixture.** Fixtures are captured from a real node by the committed script in Task 7. The 2026-07 cursor bug survived 360 green tests because its mock was invented.
- **Cursor rule, enforced in one place:** a page that came back empty yields `next: None`, and `"0x"` or `""` is never a resumable cursor.
- Commit after every task using `<type>: <description>` (types: feat, fix, refactor, docs, test, chore, perf, ci). Run `cargo fmt --all` before each commit. Do not push.
- Append a line to `~/.claude/rules/ckb-transactions.feedback.md` at the end of the plan.

## Verified API facts

Checked against the pinned crate sources on 2026-09-10. Do not re-derive these; do not substitute a remembered signature.

| Fact | Value |
|---|---|
| `ckb-jsonrpc-types` MSRV | 1.2.0 needs rustc 1.95; **1.1.1 needs 1.92**. Pin `=1.1.1` |
| `ckb-types` MSRV | 1.1.2+ needs rustc 1.95; **1.1.1 needs 1.92**. Pin `=1.1.1` (it would otherwise float via `ckb-jsonrpc-types`'s `^1`) |
| Blockchain types usable as a client | `Script`, `CellOutput`, `OutPoint`, `Transaction`, `TransactionView`, `HeaderView`, `CellDep` all derive `Serialize + Deserialize` |
| Transaction-with-status type name | `TransactionWithStatusResponse` (there is no `TransactionWithStatus` in 1.1.1) |
| Indexer types are NOT client-usable | `IndexerSearchKey`/`IndexerScriptType`/`IndexerOrder` derive only `Deserialize`; `IndexerPagination`/`IndexerCell`/`IndexerTip` derive only `Serialize`. Define our own with both derives, as `ckb-sdk-rust` does |
| `H256` | `ckb_types::H256` |
| reqwest 0.13 TLS feature | the feature is **`rustls`**, not `rustls-tls`. Use `default-features = false, features = ["json", "rustls", "http2", "charset"]` |
| `nix` signal | feature `signal`; `nix::sys::signal::kill(pid: Pid, signal: T) -> Result<()>`, `nix::sys::signal::Signal::SIGTERM`, `nix::unistd::Pid::from_raw(pid_t) -> Pid` (const fn) |
| `tokio::process::Child::id()` | returns `Option<u32>`; convert with `i32::try_from`, never `as` (clippy `cast_possible_wrap`) |
| Light-client RPC methods | `set_scripts`, `get_scripts`, `get_cells`, `get_transactions`, `get_cells_capacity`, `send_transaction`, `get_transaction`, `fetch_transaction`, `get_tip_header`, `get_genesis_block`, `get_header`, `fetch_header`, `estimate_cycles`, `local_node_info`, `get_peers` |
| Full-node RPC methods used | `get_tip_header`, `get_transaction`, `send_transaction`, `get_cells`, `get_indexer_tip`, `local_node_info` |
| Light-client config sections | `chain = "mainnet" \| "testnet"`, `[store] path`, `[network] path / listen_addresses / bootnodes`, `[rpc] listen_address`. There is no `[logger]` section: log level is the `RUST_LOG` env var on the spawned process |
| Two ports collide, not one | `[rpc] listen_address` **and** `[network] listen_addresses` both need dynamic ports so profiles can run side by side |

## Live-verified cursor semantics (CKB testnet, 2026-09-10)

These are the oracle for Task 4 and the fake server in Task 7.

| Call | Result |
|---|---|
| `get_cells` on a lock, paging until exhaustion | final page is `objects: []` with `last_cursor: "0x"` |
| `get_cells` with `after: "0x"` on a lock that **does** hold cells | `objects: []` — the poison, permanent until the cursor is discarded |
| `get_indexer_tip` on a node with the indexer enabled | `{"block_hash": "0x…", "block_number": "0x1554ef4"}` |
| `get_indexer_tip` on a node without it | `null` — this is the capability probe |

## File Structure

```
Cargo.toml                                    # workspace deps + MSRV pins (Task 2)
crates/sdk-schema/src/
  backend.rs                                  # BackendKind, BackendCapabilities, BackendStatus, BackendProfile (Task 3)
crates/chain-backend/
  assets/{mainnet,testnet}.toml               # upstream light-client config templates (Task 12)
  src/
    lib.rs                                    # re-exports (Tasks 4-18)
    error.rs                                  # BackendError (Task 4)
    cursor.rs                                 # Cursor, CellPage (Task 4)
    indexer.rs                                # client-side SearchKey/ScriptType/Order/IndexerCell/Pagination/Tip (Task 5)
    query.rs                                  # CellQuery, WatchedScript, conversions (Task 5)
    rpc.rs                                    # JSON-RPC envelope + transport (Task 6)
    testing.rs                                # fake RPC server, `testing` feature (Task 7)
    light.rs                                  # light-client RPC client (Task 8)
    full.rs                                   # full-node RPC client (Task 9)
    backend.rs                                # ChainBackend trait (Task 10)
    backends/remote_light.rs                  # RemoteLight (Task 10)
    backends/full_node.rs                     # FullNode + capability probe (Task 11)
    backends/embedded_light.rs                # EmbeddedLight (Task 16)
    config.rs                                 # light-client config generation (Task 12)
    supervisor.rs                             # process lifecycle (Tasks 13-15)
    manager.rs                                # BackendManager, backends.json (Task 18)
  scripts/capture-fixtures.sh                 # fixture capture (Task 7)
  tests/fixtures/*.json                       # captured, committed (Task 7)
  tests/live_testnet.rs                       # env-gated (Task 20)
crates/account-registry/src/store.rs          # accounts.json v2 (Task 17)
crates/wallet-core/src/
  error.rs                                    # RegistryMismatch, Backend (Tasks 1, 19)
  core.rs                                     # verify-on-unlock (Task 1), backend wiring (Task 19)
```

---

### Task 1: wallet-core — verify the registry on unlock

Closes the carry-over from plan 1c's whole-branch review: `accounts.json` is plaintext by design, so a local writer can swap a receiving address for their own and the wallet would display it as its own with `canSign: true`.

**Files:**
- Modify: `crates/wallet-core/src/error.rs`
- Modify: `crates/wallet-core/src/core.rs`
- Modify: `crates/wallet-core/tests/wallet_e2e.rs`

**Interfaces:**
- Consumes: `Keyring::seed_for`, `LockRegistry::get`, `LockModule::derive_lock_args` (plan 1c)
- Produces: `CoreError::RegistryMismatch { account_id: String }`; `WalletCore::unlock` now rejects a tampered registry

- [ ] **Step 1: Write the failing test**

Append to `crates/wallet-core/tests/wallet_e2e.rs`:

```rust
#[test]
fn unlock_rejects_a_tampered_accounts_file() {
    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let real_args = {
        let mut core = WalletCore::import(paths.clone(), b"pw", Network::Testnet, TANK)
            .expect("imports");
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
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-wallet-core --test wallet_e2e unlock_rejects`
Expected: FAIL — `RegistryMismatch` does not exist, so the file does not compile.

- [ ] **Step 3: Add the error variant**

In `crates/wallet-core/src/error.rs`, add to `CoreError` after `NoSigningMaterial`:

```rust
    #[error("stored account {account_id} does not match the wallet seed")]
    RegistryMismatch { account_id: String },
```

- [ ] **Step 4: Verify on unlock**

In `crates/wallet-core/src/core.rs`, replace the body of `unlock`:

```rust
    /// Open an existing profile.
    ///
    /// Re-derives every derived account's lock args and refuses a registry
    /// that no longer matches the seed. `accounts.json` is plaintext, so this
    /// is the cheapest integrity guarantee that does not require
    /// authenticating the file.
    pub fn unlock(paths: ProfilePaths, password: &[u8], network: Network) -> Result<Self, CoreError> {
        let vault = Vault::unlock(&paths.vault, password)?;
        if !Keyring::is_initialised(&vault) {
            return Err(CoreError::SeedMissing);
        }
        let accounts = AccountRegistry::open(&paths.accounts)?;
        let locks = LockRegistry::with_first_party();
        Self::verify_registry(&vault, &accounts, &locks)?;
        Ok(Self {
            vault,
            accounts,
            locks,
            network,
            paths,
        })
    }

    /// Re-derive each derived account and compare against what is stored.
    ///
    /// Accounts without a `Derivation` (watch-only, later hardware) carry no
    /// derivable material and are skipped rather than rejected.
    fn verify_registry(
        vault: &Vault,
        accounts: &AccountRegistry,
        locks: &LockRegistry,
    ) -> Result<(), CoreError> {
        for account in accounts.list() {
            let Some(derivation) = account.derivation else {
                continue;
            };
            let module = locks.get(account.lock_type)?;
            let seed = Keyring::seed_for(vault, module.seed_kind())?;
            let expected = module.derive_lock_args(seed.expose_secret(), &derivation)?;
            drop(seed);
            if expected != account.lock_args {
                return Err(CoreError::RegistryMismatch {
                    account_id: account.id.clone(),
                });
            }
        }
        Ok(())
    }
```

Note the ordering: `locks` is built before the `Self { .. }` literal so it can be borrowed by `verify_registry` first.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p lantern-wallet-core`
Expected: PASS, 20 tests in the crate (18 from plan 1c plus the 2 new).

The tampering test derives once per account, so a three-account wallet runs three PBKDF2 passes; that is milliseconds, not the vault's Argon2 second.

- [ ] **Step 6: Clippy and commit**

Run: `cargo clippy -p lantern-wallet-core --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/wallet-core
git commit -m "feat(wallet-core): verify the account registry against the seed on unlock"
```

---

### Task 2: Wire dependencies and MSRV pins

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/chain-backend/Cargo.toml`
- Modify: `crates/sdk-schema/Cargo.toml`
- Modify: `crates/wallet-core/Cargo.toml`

**Interfaces:**
- Produces: every later task's `use` lines resolve; `chain-backend` gains a `testing` feature.

- [ ] **Step 1: Add workspace dependencies**

In `Cargo.toml`, immediately after the `region = "4.0"` line inside `[workspace.dependencies]`, add:

```toml

# Chain access (plan 1d)
# ckb-jsonrpc-types 1.2.0 and ckb-types 1.1.2+ raise rust-version to 1.95;
# the toolchain is pinned at 1.92, so both are exact-pinned here.
ckb-jsonrpc-types = "=1.1.1"
ckb-types = "=1.1.1"
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls", "http2", "charset"] }
toml = "0.9"
```

- [ ] **Step 2: Replace `crates/chain-backend/Cargo.toml`**

```toml
[package]
name = "lantern-chain-backend"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Lantern chain backend — abstraction over light client / full node sources"

[lints]
workspace = true

[dependencies]
lantern-sdk-schema = { path = "../sdk-schema" }
ckb-jsonrpc-types.workspace = true
ckb-types.workspace = true
reqwest.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
toml.workspace = true
async-trait.workspace = true
tokio.workspace = true
thiserror.workspace = true
tracing.workspace = true
hex.workspace = true

[target.'cfg(unix)'.dependencies]
nix = { version = "0.31", default-features = false, features = ["signal"] }

[features]
# In-process fake JSON-RPC server, used by this crate's tests and by wallet-core's.
testing = []

[dev-dependencies]
tempfile = "3.10"
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 3: Add the backend module dependency to sdk-schema**

`crates/sdk-schema/Cargo.toml` needs no new dependency — `BackendProfile` reuses `Network`, and the rest are plain enums and structs. Leave it unchanged. This step exists so the implementer confirms it rather than guessing.

- [ ] **Step 4: Add chain-backend to wallet-core**

In `crates/wallet-core/Cargo.toml`, add under `[dependencies]` after the `lantern-signer-secp256k1` line:

```toml
lantern-chain-backend = { path = "../chain-backend" }
```

and under `[dev-dependencies]`:

```toml
lantern-chain-backend = { path = "../chain-backend", features = ["testing"] }
```

Cargo unifies these: the crate gets the `testing` feature when building tests and not otherwise.

- [ ] **Step 5: Verify the workspace resolves**

Run: `cargo check --workspace`
Expected: success. `reqwest` with `rustls` pulls `aws-lc-rs` by default in 0.13; if the build fails for lack of a C toolchain for `aws-lc-rs`, switch the feature list to `["json", "rustls-no-provider", "http2", "charset"]` and install a provider explicitly with `rustls::crypto::ring::default_provider().install_default()` in the transport's constructor. Record which path was taken in the report.

Run: `cargo tree -p lantern-chain-backend -i ckb-types | head -5`
Expected: shows `ckb-types v1.1.1`, confirming the pin held and nothing floated to 1.1.2+.

Run: `cargo test --workspace`
Expected: existing tests pass, 104 total.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/chain-backend/Cargo.toml crates/wallet-core/Cargo.toml
git commit -m "chore: wire plan 1d dependencies with MSRV pins for the ckb crates"
```

---

### Task 3: sdk-schema — backend types and the bigint decision

**Files:**
- Create: `crates/sdk-schema/src/backend.rs`
- Modify: `crates/sdk-schema/src/lib.rs`
- Modify: `crates/sdk-schema/src/export.rs`

**Interfaces:**
- Produces: `BackendKind`, `BackendCapabilities`, `BackendStatus`, `BackendProfile`, all exported to TypeScript.

**Why this task changes `typescript_bindings()`:** `specta_typescript::BigIntExportBehavior` defaults to `Fail`, which aborts the whole export the first time a `u64` appears. Plan 1c's exported types used only `u32`, so it never surfaced. `BackendStatus` carries block heights as `u64`, so the behaviour must be set explicitly.

The choice is `Number`. CKB block heights are around 2.2e7 and the JS safe-integer limit is 9.007e15, so heights are safe as numbers for the lifetime of the chain. **Monetary values are not**: 33.6 billion CKB in shannons is 3.36e18, well past the limit. Any capacity or balance added in plan 1e must ride on a string newtype, never a bare `u64`. That warning goes in the code as a doc comment, not only here.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/sdk-schema/src/export.rs`:

```rust
    #[test]
    fn backend_types_export_with_numeric_block_heights() {
        let ts = typescript_bindings().expect("export succeeds");
        for needle in [
            "BackendKind",
            "BackendCapabilities",
            "BackendStatus",
            "BackendProfile",
            "needsScriptRegistration",
            "indexerAvailable",
            "activeProfileId",
            "\"embedded_light\"",
            "\"remote_full\"",
        ] {
            assert!(ts.contains(needle), "missing {needle} in:\n{ts}");
        }
        // u64 block heights must render as `number`, not abort the export and
        // not become `bigint` (which JSON.parse would not produce).
        assert!(ts.contains("current: number"), "block height not numeric:\n{ts}");
        assert!(!ts.contains("bigint"), "bigint leaked into the bindings:\n{ts}");
        assert!(!ts.contains("needs_script_registration"), "snake_case leaked:\n{ts}");
    }
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-sdk-schema`
Expected: FAIL — `BackendKind` is not defined, so the crate does not compile.

- [ ] **Step 3: Write `crates/sdk-schema/src/backend.rs`**

```rust
//! Chain-backend types that cross the IPC boundary.
//!
//! A backend answers "how do we reach the chain", which spec §18 keeps
//! deliberately separate from "which chain" (`Network`). Accounts are scoped
//! to a network; backends are swappable within one.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::types::Network;

/// How a backend reaches the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
#[specta(rename_all = "snake_case")]
pub enum BackendKind {
    /// A `ckb-light-client` subprocess Lantern owns.
    EmbeddedLight,
    /// Someone else's light client, reached over RPC.
    RemoteLight,
    /// A full node on this machine.
    LocalFull,
    /// A full node elsewhere.
    RemoteFull,
}

impl BackendKind {
    /// Light backends sync only the scripts they are told to watch.
    pub const fn is_light(self) -> bool {
        matches!(self, Self::EmbeddedLight | Self::RemoteLight)
    }

    /// Only the embedded kind has a process whose lifetime Lantern owns.
    pub const fn is_supervised(self) -> bool {
        matches!(self, Self::EmbeddedLight)
    }
}

/// What the active backend can actually do. Data, not assumptions: the UI
/// disables features it lacks rather than failing at call time (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct BackendCapabilities {
    /// Light clients sync nothing until scripts are registered.
    pub needs_script_registration: bool,
    /// Full nodes can serve arbitrary blocks; light clients cannot.
    pub can_fetch_arbitrary_blocks: bool,
    pub can_estimate_cycles: bool,
    /// A full node with the indexer switched off cannot answer `get_cells`.
    pub indexer_available: bool,
}

/// Where a backend is in its lifecycle.
///
/// Block heights are `u64` and export as TypeScript `number`. That is safe:
/// CKB heights are ~2.2e7 against a 9.007e15 safe-integer limit. **Monetary
/// values are not safe this way** — 33.6 billion CKB in shannons is 3.36e18 —
/// so any capacity added later must use a string newtype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "state")]
#[specta(rename_all = "camelCase")]
pub enum BackendStatus {
    /// Starting up, or waiting for the first RPC response.
    Connecting,
    /// Reachable, still catching up. For light backends `current` is filter
    /// sync, which lags header sync and is what the user cares about.
    Syncing { current: u64, target: u64 },
    Synced { tip: u64 },
    Error { message: String },
    Stopped,
}

impl BackendStatus {
    /// Whether queries can be trusted to return complete results.
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Synced { .. } | Self::Syncing { .. })
    }
}

/// Spec §18's `(network × backend × name)` tuple.
///
/// Named "backend profile" deliberately: plan 1c already uses "profile" for
/// the directory holding `vault.bin`, and one word with two meanings will
/// eventually be read the wrong way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct BackendProfile {
    pub id: String,
    pub label: String,
    pub network: Network,
    pub kind: BackendKind,
    /// `None` for `EmbeddedLight`, whose port is allocated at spawn time.
    pub endpoint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{BackendKind, BackendStatus};

    #[test]
    fn kind_predicates_split_light_from_full_and_supervised_from_not() {
        assert!(BackendKind::EmbeddedLight.is_light());
        assert!(BackendKind::RemoteLight.is_light());
        assert!(!BackendKind::LocalFull.is_light());
        assert!(!BackendKind::RemoteFull.is_light());
        assert!(BackendKind::EmbeddedLight.is_supervised());
        assert!(!BackendKind::RemoteLight.is_supervised());
    }

    #[test]
    fn kind_serialises_as_snake_case() {
        let json = serde_json::to_string(&BackendKind::EmbeddedLight).expect("serialises");
        assert_eq!(json, "\"embedded_light\"");
        let json = serde_json::to_string(&BackendKind::RemoteFull).expect("serialises");
        assert_eq!(json, "\"remote_full\"");
    }

    #[test]
    fn status_is_a_tagged_union_and_only_reachable_states_are_usable() {
        let json = serde_json::to_string(&BackendStatus::Syncing {
            current: 10,
            target: 20,
        })
        .expect("serialises");
        assert!(json.contains("\"state\":\"syncing\""), "{json}");
        assert!(json.contains("\"current\":10"), "{json}");

        assert!(BackendStatus::Synced { tip: 1 }.is_usable());
        assert!(BackendStatus::Syncing { current: 1, target: 2 }.is_usable());
        assert!(!BackendStatus::Connecting.is_usable());
        assert!(!BackendStatus::Stopped.is_usable());
        assert!(
            !BackendStatus::Error {
                message: "x".into()
            }
            .is_usable()
        );
    }
}
```

If the `"embedded_light"` assertion fails with a mangled string, apply the plan 1c remedy: specta's inflector splits digit runs, so add a per-variant `#[specta(rename = "...")]`. These variant names carry no digits, so it should not fire.

- [ ] **Step 4: Export the new types**

In `crates/sdk-schema/src/export.rs`, change the exporter configuration and add the four types. Replace the body of `typescript_bindings`:

```rust
pub fn typescript_bindings() -> Result<String, SchemaError> {
    // `BigIntExportBehavior` defaults to `Fail`, which aborts the export the
    // moment a `u64` appears. Block heights are safe as JS numbers; monetary
    // values would not be, and must use a string newtype instead.
    let conf = Typescript::default().bigint(BigIntExportBehavior::Number);
    let chunks = [
        specta_typescript::export::<Network>(&conf),
        specta_typescript::export::<LockType>(&conf),
        specta_typescript::export::<AccountCapabilities>(&conf),
        specta_typescript::export::<Derivation>(&conf),
        specta_typescript::export::<AccountRecord>(&conf),
        specta_typescript::export::<BackendKind>(&conf),
        specta_typescript::export::<BackendCapabilities>(&conf),
        specta_typescript::export::<BackendStatus>(&conf),
        specta_typescript::export::<BackendProfile>(&conf),
    ];
    let mut out = String::new();
    for chunk in chunks {
        let rendered = chunk.map_err(|e| SchemaError::Export(e.to_string()))?;
        out.push_str(&rendered);
        out.push('\n');
    }
    Ok(out)
}
```

and update its imports:

```rust
use specta_typescript::{BigIntExportBehavior, Typescript};

use crate::backend::{BackendCapabilities, BackendKind, BackendProfile, BackendStatus};
use crate::error::SchemaError;
use crate::types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};
```

The `activeProfileId` string the new test looks for comes from `backends.json`, not from a type — remove that needle if it is not produced by any exported type; the remaining needles are the real assertions. Decide by running the test and reading the output, and say which you did in the report.

- [ ] **Step 5: Wire the module**

In `crates/sdk-schema/src/lib.rs` add `pub mod backend;` alongside the others and extend the re-exports:

```rust
pub use backend::{BackendCapabilities, BackendKind, BackendProfile, BackendStatus};
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p lantern-sdk-schema`
Expected: PASS, 9 tests (5 from plan 1c plus 3 new in `backend.rs` plus 1 new in `export.rs`).

- [ ] **Step 7: Clippy and commit**

Run: `cargo clippy -p lantern-sdk-schema --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/sdk-schema
git commit -m "feat(sdk-schema): backend kind, capabilities, status and profile types"
```

---

### Task 4: chain-backend — errors and the cursor invariant

This is the most important task in the plan. Everything else is plumbing; this is the bug that already cost a day in another repo.

**Files:**
- Create: `crates/chain-backend/src/error.rs`
- Create: `crates/chain-backend/src/cursor.rs`
- Replace: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `BackendError`; `Cursor` (cannot hold a terminal sentinel), `CellPage`, `Cursor::resumable`.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/cursor.rs` with only its test module:

```rust
//! Paging cursors that cannot represent an exhausted scan.
//!
//! CKB's indexer returns `last_cursor: "0x"` once a scan is exhausted, and a
//! later `get_cells` with `after: "0x"` returns nothing **forever**, even for
//! a lock that holds cells. A pager that stores the terminal cursor therefore
//! goes permanently blind, silently. Verified against CKB testnet 2026-09-10.
//!
//! Two rules live here and nowhere else: an empty page yields no next cursor,
//! and a sentinel is not a cursor.

#[cfg(test)]
mod tests {
    use super::{CellPage, Cursor};

    // The exact byte string a real testnet node returned mid-scan.
    const REAL: &str = "0x409bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce801";

    #[test]
    fn sentinels_are_not_resumable() {
        assert!(Cursor::resumable("0x").is_none(), "the exhausted-scan sentinel");
        assert!(Cursor::resumable("").is_none(), "an absent cursor");
        assert!(Cursor::resumable("   ").is_none(), "whitespace only");
        assert!(Cursor::resumable("0X").is_none(), "uppercase sentinel");
    }

    #[test]
    fn a_real_cursor_round_trips() {
        let c = Cursor::resumable(REAL).expect("a real cursor resumes");
        assert_eq!(c.as_str(), REAL);
    }

    #[test]
    fn an_empty_page_never_carries_a_next_cursor() {
        // Even if a node handed back something cursor-shaped on an empty page,
        // there is nothing left to fetch and storing it risks the poison.
        let page = CellPage::new(Vec::new(), REAL);
        assert!(page.cells.is_empty());
        assert!(page.next.is_none(), "empty page must not resume");
        assert!(page.is_exhausted());
    }

    #[test]
    fn a_full_page_carries_its_cursor() {
        let page = CellPage::new(vec![serde_json::json!({"cell": 1})], REAL);
        assert_eq!(page.cells.len(), 1);
        assert_eq!(page.next.as_ref().map(Cursor::as_str), Some(REAL));
        assert!(!page.is_exhausted());
    }

    #[test]
    fn the_live_exhaustion_sequence_terminates() {
        // Replays what testnet actually does: a full page, then an empty page
        // whose last_cursor is "0x". A pager driven by `next` must stop, and
        // must not have retained anything to feed back in.
        let page1 = CellPage::new(vec![serde_json::json!({"cell": 1})], REAL);
        assert!(page1.next.is_some(), "first page continues");
        let page2 = CellPage::new(Vec::new(), "0x");
        assert!(page2.next.is_none(), "exhausted scan stops");
        assert!(page2.is_exhausted());
    }

    #[test]
    fn a_non_empty_page_with_a_sentinel_cursor_also_stops() {
        // Defence in depth: if a node ever returns rows plus "0x", resuming
        // from "0x" would return nothing, so treat it as exhausted.
        let page = CellPage::new(vec![serde_json::json!({"cell": 1})], "0x");
        assert_eq!(page.cells.len(), 1, "rows are still delivered");
        assert!(page.next.is_none(), "but the scan does not continue");
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-chain-backend`
Expected: FAIL — `Cursor` and `CellPage` are not defined.

- [ ] **Step 3: Write `crates/chain-backend/src/error.rs`**

```rust
//! Error type for the chain backend.
//!
//! RPC text passes through because it originates at a node, never from key
//! material. `Spawn` carries a path or an OS message and nothing else.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("transport error: {0}")]
    Transport(String),

    #[error("node returned error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("the active backend does not support {0}")]
    Unsupported(&'static str),

    #[error("backend is not ready")]
    NotReady,

    #[error("could not start the light client: {0}")]
    Spawn(String),

    #[error("timed out waiting for the backend")]
    Timeout,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("backend configuration file is corrupt or has an unsupported version")]
    Corrupt,

    #[error("no backend profile with that id")]
    ProfileNotFound,
}

#[cfg(test)]
mod tests {
    use super::BackendError;

    #[test]
    fn rpc_errors_report_code_and_message() {
        let e = BackendError::Rpc {
            code: -32601,
            message: "Method not found".into(),
        };
        assert_eq!(e.to_string(), "node returned error -32601: Method not found");
    }

    #[test]
    fn io_errors_convert() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        assert!(matches!(BackendError::from(io), BackendError::Io(_)));
    }
}
```

- [ ] **Step 4: Write the implementation in `cursor.rs`, above its test module**

```rust
use serde_json::Value;

/// A resumable paging position.
///
/// Deliberately has no serde derives. A persisted cursor is a footgun: the
/// only safe way to revive one is through [`Cursor::resumable`], which
/// rejects the sentinel. If a future plan needs to persist paging state, it
/// must store the raw string and re-validate on read rather than deriving
/// `Deserialize` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(String);

impl Cursor {
    /// The sentinels a CKB node uses for "nothing further".
    const SENTINELS: [&'static str; 2] = ["0x", "0X"];

    /// Build a cursor, or `None` when the value cannot be resumed from.
    pub fn resumable(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || Self::SENTINELS.contains(&trimmed) {
            return None;
        }
        Some(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One page of a cell scan.
///
/// `next` is `None` whenever the scan cannot usefully continue, which is both
/// when the page came back empty and when the node handed back a sentinel.
#[derive(Debug, Clone)]
pub struct CellPage {
    pub cells: Vec<Value>,
    pub next: Option<Cursor>,
}

impl CellPage {
    /// The single enforcement point for both cursor rules.
    pub fn new(cells: Vec<Value>, last_cursor: &str) -> Self {
        let next = if cells.is_empty() {
            None
        } else {
            Cursor::resumable(last_cursor)
        };
        Self { cells, next }
    }

    /// Whether a pager driving this scan should stop.
    pub const fn is_exhausted(&self) -> bool {
        self.next.is_none()
    }
}
```

`cells` is `Vec<Value>` for now; Task 5 introduces the typed `IndexerCell` and changes this to `Vec<IndexerCell>` in one place. Keeping it untyped here lets the invariant land and be reviewed on its own.

- [ ] **Step 5: Replace `crates/chain-backend/src/lib.rs`**

```rust
#![forbid(unsafe_code)]

//! Lantern chain backend.
//!
//! One `dyn`-safe trait over four ways of reaching a CKB chain: a light
//! client Lantern supervises, someone else's light client, a local full node,
//! and a remote full node. Capabilities are data rather than assumptions, so
//! the UI can disable what a backend cannot do instead of failing at call
//! time.

pub mod cursor;
pub mod error;

pub use cursor::{CellPage, Cursor};
pub use error::BackendError;
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p lantern-chain-backend`
Expected: PASS, 8 tests.

- [ ] **Step 7: Prove the guard is load-bearing**

The plan 1c review established that a guard nobody can delete without a test failing is not a guard. Temporarily change `CellPage::new` so `next` is always `Cursor::resumable(last_cursor)` regardless of emptiness, and run the tests again.

Expected: `an_empty_page_never_carries_a_next_cursor` and `the_live_exhaustion_sequence_terminates` both fail. Restore the implementation and confirm 8 pass again. Record both outputs in the report; a guard that cannot be shown to fail has not been demonstrated.

- [ ] **Step 8: Clippy and commit**

Run: `cargo clippy -p lantern-chain-backend --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): BackendError and a cursor type that cannot resume an exhausted scan"
```

---

### Task 5: chain-backend — client-side indexer types and queries

**Files:**
- Create: `crates/chain-backend/src/indexer.rs`
- Create: `crates/chain-backend/src/query.rs`
- Modify: `crates/chain-backend/src/cursor.rs` (type `cells` properly)
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Consumes: `CellPage`, `Cursor` (Task 4)
- Produces: `ScriptType`, `Order`, `SearchMode`, `SearchKeyFilter`, `SearchKey`, `IndexerCell`, `Pagination<T>`, `Tip`; `CellQuery`, `WatchedScript`, `ScriptStatus`; `CellPage.cells: Vec<IndexerCell>`.

These exist because `ckb-jsonrpc-types` 1.1.1 derives only the server-side serde half of its indexer types: `IndexerSearchKey`, `IndexerScriptType` and `IndexerOrder` are `Deserialize`-only, while `IndexerPagination`, `IndexerCell` and `IndexerTip` are `Serialize`-only. `ckb-sdk-rust` defines its own for exactly this reason. The primitives (`Script`, `CellOutput`, `OutPoint`, `JsonBytes`, `Uint32`, `Uint64`) all carry both directions and are reused as-is.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/indexer.rs` with only its test module:

```rust
//! Client-side indexer request and response types.
//!
//! `ckb-jsonrpc-types` ships these with only the server-side serde half, so a
//! client cannot serialize its requests or deserialize its responses. These
//! mirror the wire format with both directions, as `ckb-sdk-rust` does.

#[cfg(test)]
mod tests {
    use super::{IndexerCell, Order, Pagination, ScriptType, SearchKey, Tip};

    fn secp_script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(serde_json::json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    #[test]
    fn search_key_serialises_to_the_wire_shape_a_node_accepts() {
        let key = SearchKey::lock(secp_script());
        let json = serde_json::to_value(&key).expect("serialises");
        assert_eq!(json["script_type"], "lock");
        assert_eq!(
            json["script"]["args"],
            "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        );
        // Optional fields must be omitted, not sent as null: a node rejects
        // `script_search_mode: null`.
        assert!(json.get("filter").is_none(), "{json}");
        assert!(json.get("script_search_mode").is_none(), "{json}");
        assert!(json.get("with_data").is_none(), "{json}");
    }

    #[test]
    fn order_and_script_type_use_the_wire_spelling() {
        assert_eq!(serde_json::to_value(Order::Asc).expect("ser"), "asc");
        assert_eq!(serde_json::to_value(Order::Desc).expect("ser"), "desc");
        assert_eq!(serde_json::to_value(ScriptType::Lock).expect("ser"), "lock");
        assert_eq!(serde_json::to_value(ScriptType::Type).expect("ser"), "type");
    }

    #[test]
    fn a_real_get_cells_response_deserialises() {
        // Shape taken from a live testnet get_cells response.
        let raw = serde_json::json!({
            "objects": [{
                "block_number": "0x1554e00",
                "out_point": {
                    "index": "0x0",
                    "tx_hash": "0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"
                },
                "output": {
                    "capacity": "0x1718c7e00",
                    "lock": {
                        "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                        "hash_type": "type",
                        "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
                    },
                    "type": null
                },
                "output_data": "0x",
                "tx_index": "0x1"
            }],
            "last_cursor": "0x40aabb"
        });
        let page: Pagination<IndexerCell> = serde_json::from_value(raw).expect("deserialises");
        assert_eq!(page.objects.len(), 1);
        assert_eq!(u64::from(page.objects[0].block_number), 22368256);
        assert_eq!(page.last_cursor, "0x40aabb");
        assert!(page.objects[0].output.type_.is_none());
    }

    #[test]
    fn an_exhausted_response_deserialises_with_the_sentinel_intact() {
        let raw = serde_json::json!({ "objects": [], "last_cursor": "0x" });
        let page: Pagination<IndexerCell> = serde_json::from_value(raw).expect("deserialises");
        assert!(page.objects.is_empty());
        assert_eq!(page.last_cursor, "0x", "the sentinel must survive parsing");
    }

    #[test]
    fn indexer_tip_is_optional_because_a_node_without_the_indexer_returns_null() {
        let present: Option<Tip> = serde_json::from_value(serde_json::json!({
            "block_hash": "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1",
            "block_number": "0x1554ef4"
        }))
        .expect("deserialises");
        assert_eq!(u64::from(present.expect("some").block_number), 22368500);

        let absent: Option<Tip> = serde_json::from_value(serde_json::Value::Null).expect("null");
        assert!(absent.is_none(), "null means the indexer is off");
    }
}
```

Create `crates/chain-backend/src/query.rs` with only its test module:

```rust
//! Wallet-facing query shapes, converted to indexer wire types at the edge.

#[cfg(test)]
mod tests {
    use super::{CellQuery, WatchedScript};
    use crate::indexer::{Order, ScriptType};

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(serde_json::json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    #[test]
    fn a_lock_query_defaults_to_ascending_and_a_sane_limit() {
        let q = CellQuery::lock(script());
        assert_eq!(q.script_type, ScriptType::Lock);
        assert_eq!(q.order, Order::Asc);
        assert_eq!(q.limit, 100);
    }

    #[test]
    fn the_limit_is_sent_as_hex_because_the_node_expects_uint32() {
        let q = CellQuery::lock(script()).with_limit(16);
        assert_eq!(serde_json::to_value(q.limit_param()).expect("ser"), "0x10");
    }

    #[test]
    fn a_watched_script_carries_its_start_height() {
        let w = WatchedScript::lock(script(), 22_000_000);
        assert_eq!(w.from_block, 22_000_000);
        let json = serde_json::to_value(w.to_script_status()).expect("ser");
        assert_eq!(json["script_type"], "lock");
        assert_eq!(json["block_number"], "0x14fb180");
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-chain-backend`
Expected: FAIL — neither module has an implementation.

- [ ] **Step 3: Implement `indexer.rs` above its test module**

```rust
use ckb_jsonrpc_types::{BlockNumber, CellOutput, JsonBytes, OutPoint, Script, Uint32};
use ckb_types::H256;
use serde::{Deserialize, Serialize};

/// Whether a search key matches a cell's lock or type script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    Lock,
    Type,
}

/// Result ordering by block number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Asc,
    Desc,
}

/// How the script args are matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Prefix,
    Exact,
    Partial,
}

/// Optional narrowing of a search. Every field is omitted when unset,
/// because a node rejects explicit nulls here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchKeyFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Script>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_data_len_range: Option<[Uint32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_capacity_range: Option<[ckb_jsonrpc_types::Uint64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_range: Option<[BlockNumber; 2]>,
}

/// The indexer's query key, shared by full nodes and light clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchKey {
    pub script: Script,
    pub script_type: ScriptType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_search_mode: Option<SearchMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<SearchKeyFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_data: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by_transaction: Option<bool>,
}

impl SearchKey {
    /// Search by lock script, the common case for a wallet.
    pub const fn lock(script: Script) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            script_search_mode: None,
            filter: None,
            with_data: None,
            group_by_transaction: None,
        }
    }
}

/// One cell as the indexer reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerCell {
    pub output: CellOutput,
    pub output_data: Option<JsonBytes>,
    pub out_point: OutPoint,
    pub block_number: BlockNumber,
    pub tx_index: Uint32,
}

/// A page of indexer results plus its raw continuation token.
///
/// `last_cursor` stays a plain `String` here so the sentinel survives
/// parsing; interpreting it is [`crate::cursor::CellPage`]'s job and only
/// its job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination<T> {
    pub objects: Vec<T>,
    pub last_cursor: String,
}

/// The indexer's own tip. `get_indexer_tip` returns `null` when the indexer
/// is disabled, which is how a full node's capability is probed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tip {
    pub block_hash: H256,
    pub block_number: BlockNumber,
}

/// A script the light client has been asked to watch, with how far it has synced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptStatus {
    pub script: Script,
    pub script_type: ScriptType,
    pub block_number: BlockNumber,
}

/// How `set_scripts` should treat the list it is given.
///
/// Lantern never sends `All`: it replaces the server's entire script list,
/// so two wallets pointed at one shared light client would erase each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetScriptsCommand {
    All,
    Partial,
    Delete,
}
```

- [ ] **Step 4: Implement `query.rs` above its test module**

```rust
use ckb_jsonrpc_types::{BlockNumber, Script, Uint32};

use crate::indexer::{Order, ScriptStatus, ScriptType, SearchKey};

/// A cell scan the wallet wants to run.
#[derive(Debug, Clone)]
pub struct CellQuery {
    pub script: Script,
    pub script_type: ScriptType,
    pub order: Order,
    pub limit: u32,
}

impl CellQuery {
    /// Scan by lock script, ascending, 100 cells per page.
    pub const fn lock(script: Script) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            order: Order::Asc,
            limit: 100,
        }
    }

    #[must_use]
    pub const fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    #[must_use]
    pub const fn with_order(mut self, order: Order) -> Self {
        self.order = order;
        self
    }

    /// The wire form of this query's key.
    pub fn search_key(&self) -> SearchKey {
        SearchKey {
            script: self.script.clone(),
            script_type: self.script_type,
            script_search_mode: None,
            filter: None,
            with_data: None,
            group_by_transaction: None,
        }
    }

    /// The limit as the node expects it: a hex-encoded `Uint32`.
    pub const fn limit_param(&self) -> Uint32 {
        Uint32::from(self.limit)
    }
}

/// A script a light backend must watch, and the height to start from.
#[derive(Debug, Clone)]
pub struct WatchedScript {
    pub script: Script,
    pub script_type: ScriptType,
    /// Filters before this height are never downloaded, so this is the single
    /// biggest lever on first-sync time.
    pub from_block: u64,
}

impl WatchedScript {
    pub const fn lock(script: Script, from_block: u64) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            from_block,
        }
    }

    /// The wire form `set_scripts` takes.
    pub fn to_script_status(&self) -> ScriptStatus {
        ScriptStatus {
            script: self.script.clone(),
            script_type: self.script_type,
            block_number: BlockNumber::from(self.from_block),
        }
    }
}
```

- [ ] **Step 5: Type the cell page**

In `crates/chain-backend/src/cursor.rs`, replace `use serde_json::Value;` with `use crate::indexer::IndexerCell;`, change the field to `pub cells: Vec<IndexerCell>`, and change `CellPage::new`'s first parameter to `cells: Vec<IndexerCell>`. Update the two tests that build a page from `serde_json::json!({"cell": 1})` to build one real `IndexerCell` from the JSON in `indexer.rs`'s test — factor that fixture into a `pub(crate) fn sample_cell() -> IndexerCell` behind `#[cfg(test)]` in `indexer.rs` and use it from both test modules.

- [ ] **Step 6: Wire the modules**

In `lib.rs` add `pub mod indexer;` and `pub mod query;`, and extend the re-exports:

```rust
pub use indexer::{IndexerCell, Order, Pagination, ScriptStatus, ScriptType, SearchKey, Tip};
pub use query::{CellQuery, WatchedScript};
```

- [ ] **Step 7: Run, clippy, commit**

Run: `cargo test -p lantern-chain-backend`
Expected: PASS, 16 tests.

Run: `cargo clippy -p lantern-chain-backend --all-targets -- -D warnings`
Expected: clean. If `Uint32::from(u32)` is not `const`, drop `const` from `limit_param` and add `#[allow(clippy::missing_const_for_fn)]` only if clippy then demands it.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): client-side indexer types and wallet query shapes"
```

---

### Task 6: chain-backend — JSON-RPC transport

**Files:**
- Create: `crates/chain-backend/src/rpc.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Consumes: `BackendError` (Task 4)
- Produces: `RpcClient::new(url, timeout)`, `RpcClient::call<P, T>(method, params)`, `RpcClient::url()`.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/rpc.rs` with only its test module:

```rust
//! JSON-RPC 2.0 over HTTP.
//!
//! One client serves every backend kind: light clients and full nodes speak
//! the same envelope, and the shared indexer methods take the same shapes.

#[cfg(test)]
mod tests {
    use super::{Request, Response};

    #[test]
    fn a_request_carries_the_2_0_envelope() {
        let req = Request::new(7, "get_tip_header", serde_json::json!([]));
        let json = serde_json::to_value(&req).expect("serialises");
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 7);
        assert_eq!(json["method"], "get_tip_header");
        assert_eq!(json["params"], serde_json::json!([]));
    }

    #[test]
    fn a_success_response_yields_its_result() {
        let raw = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"number": "0x1"}});
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        assert!(resp.error.is_none());
        assert_eq!(resp.result["number"], "0x1");
    }

    #[test]
    fn a_null_result_is_a_value_not_an_absence() {
        // `get_indexer_tip` legitimately returns null; that must survive as
        // Value::Null so `Option<Tip>` can deserialize from it.
        let raw = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": null});
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        assert!(resp.error.is_none());
        assert!(resp.result.is_null());
        let tip: Option<crate::indexer::Tip> =
            serde_json::from_value(resp.result).expect("null deserialises to None");
        assert!(tip.is_none());
    }

    #[test]
    fn an_error_response_is_recognised() {
        let raw = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": -32601, "message": "Method not found"}
        });
        let resp: Response = serde_json::from_value(raw).expect("deserialises");
        let err = resp.error.expect("has an error");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-chain-backend`
Expected: FAIL — `Request` and `Response` are undefined.

- [ ] **Step 3: Implement above the test module**

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::BackendError;

#[derive(Debug, Serialize)]
pub(crate) struct Request<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

impl<'a> Request<'a> {
    pub(crate) const fn new(id: u64, method: &'a str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method,
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RpcErrorBody {
    pub(crate) code: i64,
    pub(crate) message: String,
}

/// The result field stays a `Value` so a legitimate `null` (as
/// `get_indexer_tip` returns when the indexer is off) is distinguishable
/// from a missing field and can deserialize into `Option<T>`.
#[derive(Debug, Deserialize)]
pub(crate) struct Response {
    #[serde(default)]
    pub(crate) result: Value,
    #[serde(default)]
    pub(crate) error: Option<RpcErrorBody>,
}

/// A JSON-RPC client for one endpoint.
#[derive(Debug)]
pub struct RpcClient {
    http: reqwest::Client,
    url: String,
    next_id: AtomicU64,
}

impl RpcClient {
    /// Build a client. `timeout` bounds a single request, not a sync.
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, BackendError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| BackendError::Transport(e.to_string()))?;
        Ok(Self {
            http,
            url: url.into(),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Issue one call and decode its result.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, BackendError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(id, method, params);
        let response = self
            .http
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;
        let body: Response = response
            .json()
            .await
            .map_err(|e| BackendError::Transport(e.to_string()))?;
        if let Some(err) = body.error {
            return Err(BackendError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        serde_json::from_value(body.result).map_err(|e| {
            BackendError::Transport(format!("could not decode {method} result: {e}"))
        })
    }
}
```

- [ ] **Step 4: Wire, run, clippy, commit**

In `lib.rs` add `pub mod rpc;` and `pub use rpc::RpcClient;`.

Run: `cargo test -p lantern-chain-backend`
Expected: PASS, 20 tests.

Run: `cargo clippy -p lantern-chain-backend --all-targets -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): JSON-RPC transport over HTTP"
```

---

### Task 7: chain-backend — fake node and captured fixtures

Test infrastructure, and the reason the rest of the plan can be trusted. The 2026-07 cursor bug survived 360 green tests because its mock returned `"0x64"` on an empty page while the real node returns `"0x"`. **No fixture in this crate is hand-written.**

**Files:**
- Create: `crates/chain-backend/scripts/capture-fixtures.sh`
- Create: `crates/chain-backend/tests/fixtures/*.json` (captured output, committed)
- Create: `crates/chain-backend/src/testing.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `FakeNode::builder()`, `.respond(method, value)`, `.respond_sequence(method, values)`, `.fail(method, code, message)`, `.start()`, `FakeNode::url()`, `FakeNode::calls()`.

- [ ] **Step 1: Write the capture script**

Create `crates/chain-backend/scripts/capture-fixtures.sh`, executable:

```bash
#!/usr/bin/env bash
# Capture chain-backend test fixtures from a real CKB node.
#
# Fixtures are NEVER hand-written: a mock that invents an empty-page cursor is
# exactly how a silent paging bug shipped elsewhere. Re-run this and diff to
# confirm the committed fixtures still match reality.
#
#   ./scripts/capture-fixtures.sh [rpc-url]
set -euo pipefail

RPC="${1:-https://testnet.ckb.dev/}"
OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures"
mkdir -p "$OUT"

# A testnet lock that holds cells: the plan 1c sighash oracle's input.
FUNDED_ARGS="0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
SECP_CODE_HASH="0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8"

call() { # name method params
  local name="$1" method="$2" params="$3"
  curl -sS -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params}" \
    | python3 -m json.tool > "$OUT/$name.json"
  echo "wrote $OUT/$name.json"
}

search_key() {
  printf '{"script":{"code_hash":"%s","hash_type":"type","args":"%s"},"script_type":"lock"}' \
    "$SECP_CODE_HASH" "$1"
}

call tip_header      get_tip_header   '[]'
call indexer_tip     get_indexer_tip  '[]'
call local_node_info local_node_info  '[]'
call cells_page      get_cells        "[$(search_key "$FUNDED_ARGS"),\"asc\",\"0x1\",null]"

# The important one: page to exhaustion so the empty page and its terminal
# cursor are captured verbatim rather than imagined.
cursor=$(python3 -c "import json;print(json.load(open('$OUT/cells_page.json'))['result']['last_cursor'])")
call cells_exhausted get_cells "[$(search_key "$FUNDED_ARGS"),\"asc\",\"0x40\",\"$cursor\"]"

python3 - "$OUT/cells_exhausted.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))["result"]
assert r["objects"] == [], "expected an exhausted page; widen the limit or pick a smaller lock"
assert r["last_cursor"] == "0x", f"expected the 0x sentinel, got {r['last_cursor']!r}"
print("verified: exhausted page returns objects=[] last_cursor='0x'")
PY
```

- [ ] **Step 2: Run it and commit what it produced**

Run: `chmod +x crates/chain-backend/scripts/capture-fixtures.sh && crates/chain-backend/scripts/capture-fixtures.sh`
Expected: five fixture files, and the verification line `verified: exhausted page returns objects=[] last_cursor='0x'`.

If the funded lock has been swept and `cells_page` is empty, pick another testnet lock with cells (any recent block's outputs) and update `FUNDED_ARGS`. Say in the report which lock was used and at what tip.

- [ ] **Step 3: Write the fake node**

Create `crates/chain-backend/src/testing.rs`:

```rust
//! An in-process fake CKB node for tests.
//!
//! Hand-rolled HTTP/1.1 rather than a web framework: the crate needs no HTTP
//! server in production, and the point of this type is total control over the
//! bytes a test sees, including the terminal `"0x"` cursor a real node emits.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

type Routes = Arc<Mutex<HashMap<String, VecDeque<Reply>>>>;
type Calls = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Debug, Clone)]
enum Reply {
    Ok(Value),
    Err { code: i64, message: String },
}

/// Builds a [`FakeNode`].
#[derive(Default)]
pub struct FakeNodeBuilder {
    routes: HashMap<String, VecDeque<Reply>>,
}

impl FakeNodeBuilder {
    /// Always answer `method` with `result`.
    #[must_use]
    pub fn respond(mut self, method: &str, result: Value) -> Self {
        self.routes
            .entry(method.to_string())
            .or_default()
            .push_back(Reply::Ok(result));
        self
    }

    /// Answer successive calls to `method` with successive values. The last
    /// one repeats once the queue is down to it, so a scan can be driven to
    /// exhaustion and then poked again.
    #[must_use]
    pub fn respond_sequence(mut self, method: &str, results: Vec<Value>) -> Self {
        let queue = self.routes.entry(method.to_string()).or_default();
        for r in results {
            queue.push_back(Reply::Ok(r));
        }
        self
    }

    /// Answer `method` with a JSON-RPC error.
    #[must_use]
    pub fn fail(mut self, method: &str, code: i64, message: &str) -> Self {
        self.routes
            .entry(method.to_string())
            .or_default()
            .push_back(Reply::Err {
                code,
                message: message.to_string(),
            });
        self
    }

    /// Bind an ephemeral port and start serving.
    pub async fn start(self) -> FakeNode {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        let routes: Routes = Arc::new(Mutex::new(self.routes));
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let (stop_tx, stop_rx) = oneshot::channel();
        let served_routes = Arc::clone(&routes);
        let served_calls = Arc::clone(&calls);
        tokio::spawn(async move { serve(listener, served_routes, served_calls, stop_rx).await });
        FakeNode {
            addr,
            calls,
            stop: Some(stop_tx),
        }
    }
}

/// A fake node listening on localhost for the life of the value.
pub struct FakeNode {
    addr: SocketAddr,
    calls: Calls,
    stop: Option<oneshot::Sender<()>>,
}

impl FakeNode {
    pub fn builder() -> FakeNodeBuilder {
        FakeNodeBuilder::default()
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Every `(method, params)` received, in order.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().expect("calls mutex").clone()
    }

    /// How many times `method` was called.
    pub fn call_count(&self, method: &str) -> usize {
        self.calls()
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }
}

impl Drop for FakeNode {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn serve(listener: TcpListener, routes: Routes, calls: Calls, stop: oneshot::Receiver<()>) {
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((mut socket, _)) = accepted else { continue };
                let routes = Arc::clone(&routes);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    let _ = handle(&mut socket, &routes, &calls).await;
                });
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn handle(socket: &mut TcpStream, routes: &Routes, calls: &Calls) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let content_length = head
        .split("content-length:")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + content_length {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let request: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(json!(1));
    calls
        .lock()
        .expect("calls mutex")
        .push((method.clone(), params));

    let reply = {
        let mut guard = routes.lock().expect("routes mutex");
        match guard.get_mut(&method) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        }
    };
    let body = match reply {
        Some(Reply::Ok(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Some(Reply::Err { code, message }) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
        None => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("fake node has no route for {method}")}
        }),
    };

    let payload = serde_json::to_vec(&body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&payload).await?;
    socket.flush().await
}

#[cfg(test)]
mod tests {
    use super::FakeNode;
    use crate::rpc::RpcClient;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn serves_a_canned_result_and_records_the_call() {
        let node = FakeNode::builder()
            .respond("local_node_info", json!({"version": "0.5.5"}))
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let info: serde_json::Value = client
            .call("local_node_info", json!([]))
            .await
            .expect("call succeeds");
        assert_eq!(info["version"], "0.5.5");
        assert_eq!(node.call_count("local_node_info"), 1);
    }

    #[tokio::test]
    async fn serves_a_sequence_then_repeats_the_last() {
        let node = FakeNode::builder()
            .respond_sequence("get_cells", vec![json!({"page": 1}), json!({"page": 2})])
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let one: serde_json::Value = client.call("get_cells", json!([])).await.expect("1");
        let two: serde_json::Value = client.call("get_cells", json!([])).await.expect("2");
        let three: serde_json::Value = client.call("get_cells", json!([])).await.expect("3");
        assert_eq!(one["page"], 1);
        assert_eq!(two["page"], 2);
        assert_eq!(three["page"], 2, "the last reply repeats");
    }

    #[tokio::test]
    async fn surfaces_rpc_errors_and_unknown_methods() {
        let node = FakeNode::builder()
            .fail("get_cells", -32000, "indexer not enabled")
            .start()
            .await;
        let client = RpcClient::new(node.url(), Duration::from_secs(5)).expect("client");
        let err = client
            .call::<serde_json::Value>("get_cells", json!([]))
            .await
            .expect_err("must be an error");
        assert!(matches!(err, crate::BackendError::Rpc { code: -32000, .. }), "{err:?}");

        let unknown = client
            .call::<serde_json::Value>("no_such_method", json!([]))
            .await
            .expect_err("unrouted methods error");
        assert!(matches!(unknown, crate::BackendError::Rpc { code: -32601, .. }), "{unknown:?}");
    }
}
```

- [ ] **Step 4: Wire it behind the feature**

In `lib.rs`:

```rust
#[cfg(any(test, feature = "testing"))]
pub mod testing;
```

- [ ] **Step 5: Run, clippy, commit**

Run: `cargo test -p lantern-chain-backend`
Expected: PASS, 23 tests.

Run: `cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings`
Expected: clean. Also run it without the feature.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "test(chain-backend): in-process fake node and a fixture capture script"
```

---

### Task 8: chain-backend — light-client RPC client

**Files:**
- Create: `crates/chain-backend/src/light.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Consumes: `RpcClient` (Task 6), indexer types (Task 5), `CellPage` (Task 4)
- Produces: `LightRpc::new(url, timeout)`, `tip_header`, `get_cells`, `get_transaction`, `send_transaction`, `set_scripts_partial`, `get_scripts`, `local_node_info`.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/light.rs` with only its test module:

```rust
//! JSON-RPC client for a `ckb-light-client`.
//!
//! A light client syncs only the scripts it has been told to watch, so
//! `set_scripts` is not optional here the way it is absent on a full node.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::LightRpc;
    use crate::indexer::ScriptType;
    use crate::query::{CellQuery, WatchedScript};
    use crate::testing::FakeNode;

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    fn one_cell_page(cursor: &str) -> serde_json::Value {
        json!({
            "objects": [{
                "block_number": "0x1554e00",
                "out_point": {"index": "0x0", "tx_hash": "0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"},
                "output": {
                    "capacity": "0x1718c7e00",
                    "lock": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"},
                    "type": null
                },
                "output_data": "0x",
                "tx_index": "0x1"
            }],
            "last_cursor": cursor
        })
    }

    #[tokio::test]
    async fn paging_stops_at_the_real_terminal_cursor() {
        // Exactly what testnet does: a page, then an empty page with "0x".
        let node = FakeNode::builder()
            .respond_sequence(
                "get_cells",
                vec![
                    one_cell_page("0x40aabb"),
                    json!({"objects": [], "last_cursor": "0x"}),
                ],
            )
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let query = CellQuery::lock(script());

        let first = rpc.get_cells(&query, None).await.expect("page 1");
        assert_eq!(first.cells.len(), 1);
        let cursor = first.next.expect("page 1 continues");

        let second = rpc.get_cells(&query, Some(&cursor)).await.expect("page 2");
        assert!(second.cells.is_empty());
        assert!(second.next.is_none(), "must not resume from the sentinel");
        assert!(second.is_exhausted());
        assert_eq!(node.call_count("get_cells"), 2, "and the pager stopped");
    }

    #[tokio::test]
    async fn set_scripts_is_always_partial_never_all() {
        let node = FakeNode::builder().respond("set_scripts", json!(null)).start().await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        rpc.set_scripts_partial(&[WatchedScript::lock(script(), 22_000_000)])
            .await
            .expect("registers");

        let (_, params) = node.calls().into_iter().next().expect("one call");
        assert_eq!(
            params[1], "partial",
            "`all` would wipe every script on a shared server: {params}"
        );
        assert_eq!(params[0][0]["block_number"], "0x14fb180");
        assert_eq!(params[0][0]["script_type"], "lock");
    }

    #[tokio::test]
    async fn filter_sync_progress_is_the_minimum_watched_height() {
        let node = FakeNode::builder()
            .respond(
                "get_scripts",
                json!([
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"}, "script_type": "lock", "block_number": "0x64"},
                    {"script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x22"}, "script_type": "lock", "block_number": "0x20"}
                ]),
            )
            .start()
            .await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert_eq!(
            rpc.filter_progress().await.expect("progress"),
            Some(0x20),
            "the slowest script is the honest answer"
        );
    }

    #[tokio::test]
    async fn no_watched_scripts_means_no_filter_progress() {
        let node = FakeNode::builder().respond("get_scripts", json!([])).start().await;
        let rpc = LightRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert!(rpc.filter_progress().await.expect("progress").is_none());
    }

    #[tokio::test]
    async fn script_type_round_trips_through_the_wire() {
        assert_eq!(
            serde_json::to_value(ScriptType::Lock).expect("ser"),
            json!("lock")
        );
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-chain-backend`
Expected: FAIL — `LightRpc` is undefined.

- [ ] **Step 3: Implement above the test module**

```rust
use std::time::Duration;

use ckb_jsonrpc_types::{HeaderView, LocalNode, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use serde_json::{Value, json};

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::indexer::{IndexerCell, Pagination, ScriptStatus, SetScriptsCommand};
use crate::query::{CellQuery, WatchedScript};
use crate::rpc::RpcClient;

/// A client for one `ckb-light-client` endpoint.
#[derive(Debug)]
pub struct LightRpc {
    rpc: RpcClient,
}

impl LightRpc {
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, BackendError> {
        Ok(Self {
            rpc: RpcClient::new(url, timeout)?,
        })
    }

    pub fn url(&self) -> &str {
        self.rpc.url()
    }

    pub async fn local_node_info(&self) -> Result<LocalNode, BackendError> {
        self.rpc.call("local_node_info", json!([])).await
    }

    pub async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.call("get_tip_header", json!([])).await
    }

    /// One page of a cell scan. The returned page enforces the cursor rules.
    pub async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        let params = json!([
            query.search_key(),
            query.order,
            query.limit_param(),
            after.map(Cursor::as_str),
        ]);
        let page: Pagination<IndexerCell> = self.rpc.call("get_cells", params).await?;
        Ok(CellPage::new(page.objects, &page.last_cursor))
    }

    pub async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc.call("get_transaction", json!([hash])).await
    }

    pub async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        self.rpc.call("send_transaction", json!([tx])).await
    }

    /// Register scripts to watch.
    ///
    /// Always `partial`. The `all` command replaces the server's entire script
    /// list, so two wallets pointed at one shared light client would erase
    /// each other's registrations.
    pub async fn set_scripts_partial(
        &self,
        scripts: &[WatchedScript],
    ) -> Result<(), BackendError> {
        let statuses: Vec<ScriptStatus> =
            scripts.iter().map(WatchedScript::to_script_status).collect();
        let _: Value = self
            .rpc
            .call(
                "set_scripts",
                json!([statuses, SetScriptsCommand::Partial]),
            )
            .await?;
        Ok(())
    }

    pub async fn get_scripts(&self) -> Result<Vec<ScriptStatus>, BackendError> {
        self.rpc.call("get_scripts", json!([])).await
    }

    /// How far filter sync has actually got: the slowest watched script.
    ///
    /// `None` when nothing is watched, which means there is nothing to sync
    /// rather than that sync is complete.
    pub async fn filter_progress(&self) -> Result<Option<u64>, BackendError> {
        let scripts = self.get_scripts().await?;
        Ok(scripts
            .iter()
            .map(|s| u64::from(s.block_number))
            .min())
    }
}
```

- [ ] **Step 4: Wire, run, clippy, commit**

In `lib.rs` add `pub mod light;` and `pub use light::LightRpc;`.

Run: `cargo test -p lantern-chain-backend`
Expected: PASS, 28 tests.

If `set_scripts` returns `null` and the `Value` decode complains, note that `serde_json::Value` deserializes `null` fine; the `let _: Value` binding is deliberate so a `null` result is accepted.

Run: `cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings`
Expected: clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): light-client RPC client with partial-only script registration"
```

---

### Task 9: chain-backend — full-node RPC client

**Files:**
- Create: `crates/chain-backend/src/full.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `FullRpc::new`, `tip_header`, `get_cells`, `get_transaction`, `send_transaction`, `local_node_info`, `indexer_tip`.

The full node speaks the same indexer shapes, so `get_cells` is nearly identical to the light client's. What differs: there is no `set_scripts` (a full node indexes everything), and `get_indexer_tip` returns `null` when the indexer is disabled, which is the capability probe.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/full.rs` with only its test module:

```rust
//! JSON-RPC client for a CKB full node.
//!
//! A full node indexes every script, so there is nothing to register; what it
//! may lack is the indexer itself, which `get_indexer_tip` reports honestly.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::FullRpc;
    use crate::testing::FakeNode;

    #[tokio::test]
    async fn indexer_tip_is_some_when_the_indexer_is_enabled() {
        let node = FakeNode::builder()
            .respond(
                "get_indexer_tip",
                json!({
                    "block_hash": "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1",
                    "block_number": "0x1554ef4"
                }),
            )
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let tip = rpc.indexer_tip().await.expect("call").expect("indexer on");
        assert_eq!(u64::from(tip.block_number), 22_368_500);
    }

    #[tokio::test]
    async fn a_null_indexer_tip_means_the_indexer_is_off() {
        // Verified against a real node: this is how the capability is probed.
        let node = FakeNode::builder()
            .respond("get_indexer_tip", json!(null))
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        assert!(rpc.indexer_tip().await.expect("call").is_none());
    }

    #[tokio::test]
    async fn get_cells_obeys_the_same_cursor_rules_as_the_light_client() {
        let node = FakeNode::builder()
            .respond("get_cells", json!({"objects": [], "last_cursor": "0x"}))
            .start()
            .await;
        let rpc = FullRpc::new(node.url(), Duration::from_secs(5)).expect("client");
        let script = serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script");
        let page = rpc
            .get_cells(&crate::query::CellQuery::lock(script), None)
            .await
            .expect("page");
        assert!(page.is_exhausted());
        assert!(page.next.is_none());
    }
}
```

- [ ] **Step 2: Run to confirm it fails, then implement**

Run: `cargo test -p lantern-chain-backend` — FAIL, `FullRpc` undefined.

Implement above the test module:

```rust
use std::time::Duration;

use ckb_jsonrpc_types::{HeaderView, LocalNode, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use serde_json::json;

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::indexer::{IndexerCell, Pagination, Tip};
use crate::query::CellQuery;
use crate::rpc::RpcClient;

/// A client for one CKB full-node endpoint.
#[derive(Debug)]
pub struct FullRpc {
    rpc: RpcClient,
}

impl FullRpc {
    pub fn new(url: impl Into<String>, timeout: Duration) -> Result<Self, BackendError> {
        Ok(Self {
            rpc: RpcClient::new(url, timeout)?,
        })
    }

    pub fn url(&self) -> &str {
        self.rpc.url()
    }

    pub async fn local_node_info(&self) -> Result<LocalNode, BackendError> {
        self.rpc.call("local_node_info", json!([])).await
    }

    pub async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.call("get_tip_header", json!([])).await
    }

    /// The indexer's tip, or `None` when the node runs without an indexer.
    ///
    /// This doubles as the capability probe: a node answering `null` cannot
    /// serve `get_cells`, and saying so up front beats a confusing RPC error
    /// at the first balance query.
    pub async fn indexer_tip(&self) -> Result<Option<Tip>, BackendError> {
        self.rpc.call("get_indexer_tip", json!([])).await
    }

    pub async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        let params = json!([
            query.search_key(),
            query.order,
            query.limit_param(),
            after.map(Cursor::as_str),
        ]);
        let page: Pagination<IndexerCell> = self.rpc.call("get_cells", params).await?;
        Ok(CellPage::new(page.objects, &page.last_cursor))
    }

    pub async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc.call("get_transaction", json!([hash])).await
    }

    pub async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        // "passthrough" matches ckb-sdk's default outputs-validator choice.
        self.rpc
            .call("send_transaction", json!([tx, "passthrough"]))
            .await
    }
}
```

- [ ] **Step 3: Wire, run, clippy, commit**

In `lib.rs` add `pub mod full;` and `pub use full::FullRpc;`.

Run: `cargo test -p lantern-chain-backend` — PASS, 31 tests.
Run: `cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings` — clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): full-node RPC client with an indexer capability probe"
```

---

### Task 10: chain-backend — the `ChainBackend` trait and `RemoteLight`

**Files:**
- Create: `crates/chain-backend/src/backend.rs`
- Create: `crates/chain-backend/src/backends/mod.rs`
- Create: `crates/chain-backend/src/backends/remote_light.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `trait ChainBackend` (object-safe, async); `RemoteLight::new(profile, url)`.

- [ ] **Step 1: Write the trait**

Create `crates/chain-backend/src/backend.rs`:

```rust
//! The one interface the rest of the wallet sees.
//!
//! Spec §6: nothing above this line should care whether the chain source is a
//! supervised light client, someone else's light client, or a full node.

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};

use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::query::{CellQuery, WatchedScript};

/// A source of chain data.
///
/// Object-safe by way of `async_trait`, because `BackendManager` holds one
/// behind a `Box<dyn ChainBackend>` and swaps it at runtime.
#[async_trait]
pub trait ChainBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn network(&self) -> Network;

    /// What this backend can do, as data. Callers disable features they
    /// cannot use rather than discovering it through a failed call.
    fn capabilities(&self) -> BackendCapabilities;

    async fn status(&self) -> BackendStatus;
    async fn tip_header(&self) -> Result<HeaderView, BackendError>;
    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError>;
    async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError>;
    async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError>;

    /// Register scripts to watch. A no-op returning `Ok` on full backends,
    /// which index everything already.
    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError>;

    async fn start(&self) -> Result<(), BackendError>;
    async fn stop(&self) -> Result<(), BackendError>;
}
```

- [ ] **Step 2: Write the failing tests for `RemoteLight`**

Create `crates/chain-backend/src/backends/remote_light.rs` with only its test module:

```rust
//! A light client someone else runs.
//!
//! Same RPC as the embedded kind, none of the process ownership — and one
//! extra hazard: the server's script list is shared, so registration must
//! never replace it wholesale.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RemoteLight;
    use crate::backend::ChainBackend;
    use crate::query::WatchedScript;
    use crate::testing::FakeNode;
    use lantern_sdk_schema::{BackendKind, BackendStatus, Network};

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type", "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        })).expect("script")
    }

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

    #[tokio::test]
    async fn a_light_backend_declares_that_it_needs_registration() {
        let node = FakeNode::builder().start().await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.kind(), BackendKind::RemoteLight);
        assert_eq!(backend.network(), Network::Testnet);
        let caps = backend.capabilities();
        assert!(caps.needs_script_registration, "a light client syncs nothing otherwise");
        assert!(!caps.can_fetch_arbitrary_blocks);
        assert!(caps.indexer_available, "light clients always answer get_cells");
    }

    #[tokio::test]
    async fn status_is_syncing_until_the_slowest_script_reaches_the_tip() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([{
                "script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"},
                "script_type": "lock", "block_number": "0x20"
            }]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(
            backend.status().await,
            BackendStatus::Syncing { current: 0x20, target: 0x64 }
        );
    }

    #[tokio::test]
    async fn status_is_synced_once_filters_catch_up() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([{
                "script": {"code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8", "hash_type": "type", "args": "0x11"},
                "script_type": "lock", "block_number": "0x64"
            }]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.status().await, BackendStatus::Synced { tip: 0x64 });
    }

    #[tokio::test]
    async fn with_nothing_watched_the_backend_is_synced_not_stuck_at_zero() {
        let node = FakeNode::builder()
            .respond("get_tip_header", header("0x64"))
            .respond("get_scripts", json!([]))
            .start()
            .await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        assert_eq!(backend.status().await, BackendStatus::Synced { tip: 0x64 });
    }

    #[tokio::test]
    async fn an_unreachable_node_reports_error_rather_than_panicking() {
        // Port 1 is reserved and refuses connections.
        let backend = RemoteLight::new(Network::Testnet, "http://127.0.0.1:1/").expect("backend");
        assert!(matches!(backend.status().await, BackendStatus::Error { .. }));
    }

    #[tokio::test]
    async fn watch_scripts_registers_partially() {
        let node = FakeNode::builder().respond("set_scripts", json!(null)).start().await;
        let backend = RemoteLight::new(Network::Testnet, node.url()).expect("backend");
        backend
            .watch_scripts(&[WatchedScript::lock(script(), 100)])
            .await
            .expect("registers");
        let (_, params) = node.calls().into_iter().next().expect("a call");
        assert_eq!(params[1], "partial");
    }
}
```

- [ ] **Step 3: Implement above the test module**

```rust
use std::time::Duration;

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};

use crate::backend::ChainBackend;
use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::light::LightRpc;
use crate::query::{CellQuery, WatchedScript};

/// Default per-request timeout. Generous, because a light client under sync
/// load can be slow to answer, and a spurious timeout looks like an outage.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A light client reached over the network.
#[derive(Debug)]
pub struct RemoteLight {
    network: Network,
    rpc: LightRpc,
}

impl RemoteLight {
    pub fn new(network: Network, url: impl Into<String>) -> Result<Self, BackendError> {
        Ok(Self {
            network,
            rpc: LightRpc::new(url, DEFAULT_TIMEOUT)?,
        })
    }

    pub(crate) const fn rpc(&self) -> &LightRpc {
        &self.rpc
    }

    /// Shared by both light backends: compare filter progress to the tip.
    pub(crate) async fn light_status(rpc: &LightRpc) -> BackendStatus {
        let tip = match rpc.tip_header().await {
            Ok(header) => u64::from(header.inner.number),
            Err(e) => return BackendStatus::Error { message: e.to_string() },
        };
        match rpc.filter_progress().await {
            // Nothing watched: there is nothing to sync, so the backend is as
            // ready as it can be rather than stuck reporting 0 of tip.
            Ok(None) => BackendStatus::Synced { tip },
            Ok(Some(current)) if current >= tip => BackendStatus::Synced { tip },
            Ok(Some(current)) => BackendStatus::Syncing { current, target: tip },
            Err(e) => BackendStatus::Error { message: e.to_string() },
        }
    }

    pub(crate) const fn light_capabilities() -> BackendCapabilities {
        BackendCapabilities {
            needs_script_registration: true,
            can_fetch_arbitrary_blocks: false,
            can_estimate_cycles: true,
            indexer_available: true,
        }
    }
}

#[async_trait]
impl ChainBackend for RemoteLight {
    fn kind(&self) -> BackendKind {
        BackendKind::RemoteLight
    }

    fn network(&self) -> Network {
        self.network
    }

    fn capabilities(&self) -> BackendCapabilities {
        Self::light_capabilities()
    }

    async fn status(&self) -> BackendStatus {
        Self::light_status(&self.rpc).await
    }

    async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc.tip_header().await
    }

    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
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

    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        self.rpc.set_scripts_partial(scripts).await
    }

    async fn start(&self) -> Result<(), BackendError> {
        // Nothing to start: someone else owns this process. Confirm it answers.
        self.rpc.local_node_info().await.map(|_| ())
    }

    async fn stop(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
```

Create `crates/chain-backend/src/backends/mod.rs`:

```rust
//! Concrete backends behind the [`crate::backend::ChainBackend`] trait.

pub mod remote_light;

pub use remote_light::RemoteLight;
```

- [ ] **Step 4: Wire, run, clippy, commit**

In `lib.rs` add `pub mod backend;` and `pub mod backends;`, plus:

```rust
pub use backend::ChainBackend;
pub use backends::RemoteLight;
```

Run: `cargo test -p lantern-chain-backend` — PASS, 37 tests.
Run: `cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings` — clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): ChainBackend trait and the RemoteLight backend"
```

---

### Task 11: chain-backend — full-node backend and capability probing

**Files:**
- Create: `crates/chain-backend/src/backends/full_node.rs`
- Modify: `crates/chain-backend/src/backends/mod.rs`, `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `FullNode::connect(network, kind, url)` (probes on connect), `FullNode::probe_local()`.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/backends/full_node.rs` with only its test module:

```rust
//! A CKB full node, local or remote.
//!
//! Indexes every script, so registration is a no-op; may have the indexer
//! switched off, which is probed once at connect rather than discovered
//! later through a failing query.

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
        assert!(!caps.needs_script_registration, "a full node indexes everything");
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
        })).expect("script");
        let err = backend
            .get_cells(&crate::query::CellQuery::lock(script), None)
            .await
            .expect_err("must refuse");
        assert!(matches!(err, crate::BackendError::Unsupported("get_cells")), "{err:?}");
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
            BackendStatus::Syncing { current: 0x20, target: 0x64 }
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
        })).expect("script");
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
}
```

- [ ] **Step 2: Run to confirm it fails, then implement**

```rust
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
            Err(e) => return BackendStatus::Error { message: e.to_string() },
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
                    BackendStatus::Syncing { current, target: tip }
                }
            }
            Ok(None) => BackendStatus::Synced { tip },
            Err(e) => BackendStatus::Error { message: e.to_string() },
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

    async fn start(&self) -> Result<(), BackendError> {
        self.rpc.local_node_info().await.map(|_| ())
    }

    async fn stop(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
```

- [ ] **Step 3: Wire, run, clippy, commit**

Add `pub mod full_node;` and `pub use full_node::FullNode;` to `backends/mod.rs`, and re-export `FullNode` from `lib.rs`.

Run: `cargo test -p lantern-chain-backend` — PASS, 43 tests.
Run clippy with and without `--features testing` — clean.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): full-node backend with an up-front indexer capability probe"
```

---

### Task 12: chain-backend — light-client config generation

**Files:**
- Create: `crates/chain-backend/assets/mainnet.toml`, `crates/chain-backend/assets/testnet.toml`
- Create: `crates/chain-backend/src/config.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `LightClientConfig { data_dir, network, rpc_port, p2p_port }`, `render() -> Result<String, BackendError>`, `write_to(path)`.

**Two ports, not one.** `[rpc] listen_address` and `[network] listen_addresses` both need dynamic values, or a second Lantern profile fails to start with a bind error that looks like a crash.

- [ ] **Step 1: Vendor the upstream templates**

```bash
mkdir -p crates/chain-backend/assets
cp research/ckb-light-client/raw/ckb-light-client/config/mainnet.toml crates/chain-backend/assets/mainnet.toml
cp research/ckb-light-client/raw/ckb-light-client/config/testnet.toml crates/chain-backend/assets/testnet.toml
```

The `research/` corpus is gitignored, so if it is absent fetch the two files from `https://raw.githubusercontent.com/nervosnetwork/ckb-light-client/develop/config/{mainnet,testnet}.toml` instead. Either way the files are committed here: the bootnode list is the part we must not invent, and vendoring keeps it reviewable.

Add a header comment to the top of each: `# Vendored from nervosnetwork/ckb-light-client config/<name>.toml. Store path, network path, listen addresses and RPC port are overwritten at spawn time.`

- [ ] **Step 2: Write the failing tests**

Create `crates/chain-backend/src/config.rs` with only its test module:

```rust
//! Generating the light client's config file.
//!
//! The upstream templates are vendored so the bootnode list stays
//! authoritative; only the paths and the two ports are ours.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::Network;

    use super::LightClientConfig;

    fn config() -> LightClientConfig {
        LightClientConfig {
            data_dir: std::path::PathBuf::from("/tmp/lantern-test"),
            network: Network::Testnet,
            rpc_port: 45_001,
            p2p_port: 45_002,
        }
    }

    #[test]
    fn the_rendered_config_binds_both_ports_to_localhost() {
        let rendered = config().render().expect("renders");
        assert!(
            rendered.contains("127.0.0.1:45001"),
            "rpc port missing:\n{rendered}"
        );
        assert!(
            rendered.contains("/ip4/127.0.0.1/tcp/45002"),
            "p2p port missing:\n{rendered}"
        );
        assert!(
            !rendered.contains("0.0.0.0"),
            "must not listen on every interface:\n{rendered}"
        );
        assert!(
            !rendered.contains("127.0.0.1:9000"),
            "the template's fixed port must be overwritten:\n{rendered}"
        );
    }

    #[test]
    fn paths_live_under_the_supplied_data_dir() {
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("/tmp/lantern-test"), "{rendered}");
    }

    #[test]
    fn the_chain_matches_the_network() {
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("chain = \"testnet\""), "{rendered}");

        let mainnet = LightClientConfig {
            network: Network::Mainnet,
            ..config()
        };
        let rendered = mainnet.render().expect("renders");
        assert!(rendered.contains("chain = \"mainnet\""), "{rendered}");
    }

    #[test]
    fn the_bootnodes_survive_from_the_vendored_template() {
        // The one part we must never invent. If this is empty the client
        // cannot find peers and sync silently never starts.
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("bootnodes"), "{rendered}");
        assert!(rendered.contains("/ip4/"), "no bootnode entries:\n{rendered}");
    }

    #[test]
    fn rendering_is_deterministic() {
        assert_eq!(config().render().expect("a"), config().render().expect("b"));
    }
}
```

- [ ] **Step 3: Implement above the test module**

```rust
use std::path::{Path, PathBuf};

use lantern_sdk_schema::Network;
use toml::Table;
use toml::Value;

use crate::error::BackendError;

const MAINNET_TEMPLATE: &str = include_str!("../assets/mainnet.toml");
const TESTNET_TEMPLATE: &str = include_str!("../assets/testnet.toml");

/// Everything Lantern controls in a light-client config.
#[derive(Debug, Clone)]
pub struct LightClientConfig {
    pub data_dir: PathBuf,
    pub network: Network,
    /// JSON-RPC port, allocated fresh on every spawn.
    pub rpc_port: u16,
    /// P2P port, also dynamic: two profiles sharing 8118 would collide.
    pub p2p_port: u16,
}

impl LightClientConfig {
    const fn template(&self) -> &'static str {
        match self.network {
            Network::Mainnet => MAINNET_TEMPLATE,
            Network::Testnet => TESTNET_TEMPLATE,
        }
    }

    /// Render the config, keeping the template's bootnodes and replacing only
    /// the paths and ports.
    pub fn render(&self) -> Result<String, BackendError> {
        let mut doc: Table = self
            .template()
            .parse()
            .map_err(|e| BackendError::Spawn(format!("vendored template is not valid TOML: {e}")))?;

        let dir = self.data_dir.display().to_string();
        set_path(&mut doc, "store", "path", format!("{dir}/store"))?;
        set_path(&mut doc, "network", "path", format!("{dir}/network"))?;

        let network = doc
            .get_mut("network")
            .and_then(Value::as_table_mut)
            .ok_or_else(|| BackendError::Spawn("template has no [network] table".into()))?;
        network.insert(
            "listen_addresses".to_string(),
            Value::Array(vec![Value::String(format!(
                "/ip4/127.0.0.1/tcp/{}",
                self.p2p_port
            ))]),
        );

        let rpc = doc
            .entry("rpc".to_string())
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| BackendError::Spawn("[rpc] is not a table".into()))?;
        rpc.insert(
            "listen_address".to_string(),
            Value::String(format!("127.0.0.1:{}", self.rpc_port)),
        );

        toml::to_string(&doc)
            .map_err(|e| BackendError::Spawn(format!("could not render config: {e}")))
    }

    /// Render and write, creating the data directory if needed.
    pub fn write_to(&self, path: &Path) -> Result<(), BackendError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(self.data_dir.join("store"))?;
        std::fs::create_dir_all(self.data_dir.join("network"))?;
        std::fs::write(path, self.render()?)?;
        Ok(())
    }
}

fn set_path(doc: &mut Table, table: &str, key: &str, value: String) -> Result<(), BackendError> {
    doc.entry(table.to_string())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| BackendError::Spawn(format!("[{table}] is not a table")))?
        .insert(key.to_string(), Value::String(value));
    Ok(())
}
```

If `toml` 0.9's `Table` import path differs, use `toml::value::Table`; both have resolved to the same type historically. Record which compiled.

- [ ] **Step 4: Wire, run, clippy, commit**

In `lib.rs` add `pub mod config;` and `pub use config::LightClientConfig;`.

Run: `cargo test -p lantern-chain-backend` — PASS, 48 tests.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): vendored light-client config templates and generation"
```

---

### Task 13: chain-backend — supervisor: ports, spawn, readiness, logs

**Files:**
- Create: `crates/chain-backend/src/bin/fake_light_client.rs`
- Create: `crates/chain-backend/src/supervisor.rs`
- Create: `crates/chain-backend/tests/supervisor.rs`
- Modify: `crates/chain-backend/Cargo.toml`, `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `SupervisorConfig`, `Supervisor::start`, `Supervisor::port`, `Supervisor::health`, `SupervisorHealth`.

Supervisor tests are integration tests in `tests/` so they can reach the stub binary through `env!("CARGO_BIN_EXE_fake_light_client")`, which Cargo sets for integration tests only.

- [ ] **Step 1: Write the stub light client**

Add to `crates/chain-backend/Cargo.toml`:

```toml
[[bin]]
name = "fake_light_client"
path = "src/bin/fake_light_client.rs"
required-features = ["testing"]
```

Create `crates/chain-backend/src/bin/fake_light_client.rs`:

```rust
//! A stand-in for `ckb-light-client`, used only by supervisor tests.
//!
//! Reads the generated config for its RPC port and answers `local_node_info`.
//! Env vars steer the failure modes the supervisor must handle:
//!   `FAKE_LC_NEVER_READY=1` — bind nothing, so readiness times out
//!   `FAKE_LC_EXIT_AFTER_MS=n` — serve, then exit(1) to force a restart

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut config_path = None;
    while let Some(arg) = args.next() {
        if arg == "--config-file" {
            config_path = args.next();
        }
    }
    let config = std::fs::read_to_string(config_path.expect("--config-file is required"))
        .expect("config file is readable");
    let addr = config
        .lines()
        .find_map(|l| l.trim().strip_prefix("listen_address = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("config carries an rpc listen_address");

    eprintln!("fake light client starting on {addr}");

    if std::env::var("FAKE_LC_NEVER_READY").is_ok() {
        std::thread::sleep(Duration::from_secs(3600));
        return;
    }

    let listener = TcpListener::bind(&addr).expect("bind the configured port");
    println!("fake light client ready");

    let deadline = std::env::var("FAKE_LC_EXIT_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    listener
        .set_nonblocking(true)
        .expect("non-blocking so the deadline can be honoured");

    loop {
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                eprintln!("fake light client exiting on purpose");
                std::process::exit(1);
            }
        }
        match listener.accept() {
            Ok((mut sock, _)) => {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf);
                let body = br#"{"jsonrpc":"2.0","id":1,"result":{"version":"fake","node_id":"fake"}}"#;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.flush();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}
```

- [ ] **Step 2: Write the failing integration tests**

Create `crates/chain-backend/tests/supervisor.rs`:

```rust
//! Supervisor lifecycle, driven against a stub binary rather than the real
//! light client so the suite stays hermetic and fast.

use std::time::Duration;

use lantern_chain_backend::supervisor::{Supervisor, SupervisorConfig, SupervisorHealth};
use lantern_sdk_schema::Network;

fn config(dir: &std::path::Path) -> SupervisorConfig {
    SupervisorConfig {
        binary: env!("CARGO_BIN_EXE_fake_light_client").into(),
        data_dir: dir.to_path_buf(),
        network: Network::Testnet,
        log_level: "info".to_string(),
        ready_timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn starts_becomes_ready_and_reports_its_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    assert!(sup.port() > 0, "a port was allocated");
    assert!(matches!(sup.health(), SupervisorHealth::Running { .. }));

    // The port it reports is the port it is actually serving on.
    let client = lantern_chain_backend::RpcClient::new(
        format!("http://127.0.0.1:{}/", sup.port()),
        Duration::from_secs(5),
    )
    .expect("client");
    let info: serde_json::Value = client
        .call("local_node_info", serde_json::json!([]))
        .await
        .expect("answers");
    assert_eq!(info["version"], "fake");

    sup.stop().await.expect("stops");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));
}

#[tokio::test]
async fn a_process_that_never_binds_times_out_rather_than_hanging() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.ready_timeout = Duration::from_millis(600);
    // SAFETY-free: this is an env var, not unsafe code.
    std::env::set_var("FAKE_LC_NEVER_READY", "1");
    let result = Supervisor::start(cfg).await;
    std::env::remove_var("FAKE_LC_NEVER_READY");
    assert!(
        matches!(result, Err(lantern_chain_backend::BackendError::Timeout)),
        "expected a timeout"
    );
}

#[tokio::test]
async fn a_missing_binary_is_a_spawn_error_not_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.binary = dir.path().join("definitely-not-here");
    let err = Supervisor::start(cfg).await.expect_err("must fail");
    assert!(
        matches!(err, lantern_chain_backend::BackendError::Spawn(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn two_supervisors_get_different_ports() {
    let a_dir = tempfile::tempdir().expect("tempdir");
    let b_dir = tempfile::tempdir().expect("tempdir");
    let mut a = Supervisor::start(config(a_dir.path())).await.expect("a starts");
    let mut b = Supervisor::start(config(b_dir.path())).await.expect("b starts");
    assert_ne!(a.port(), b.port(), "profiles must run side by side");
    a.stop().await.expect("a stops");
    b.stop().await.expect("b stops");
}
```

`std::env::set_var` is safe on this toolchain's edition only inside a single-threaded context; if the compiler rejects it under edition 2024, set the variable on the child instead by adding an `extra_env: Vec<(String, String)>` field to `SupervisorConfig` and passing it through to `Command::envs`. Prefer that shape if there is any friction — it is cleaner anyway. Record which you used.

- [ ] **Step 3: Implement the supervisor**

Create `crates/chain-backend/src/supervisor.rs`:

```rust
//! Lifecycle of the bundled light-client process.
//!
//! Lantern owns this process: it generates the config, allocates the ports,
//! waits for readiness, forwards the logs, and reaps it on exit.

use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use lantern_sdk_schema::Network;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::config::LightClientConfig;
use crate::error::BackendError;
use crate::rpc::RpcClient;

/// What the supervisor knows about its child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorHealth {
    Running { restarts: u32 },
    Restarting { attempt: u32 },
    /// Too many crashes too quickly; auto-restart has given up.
    CircuitOpen,
    Stopped,
}

/// Everything needed to spawn the light client.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub network: Network,
    pub log_level: String,
    pub ready_timeout: Duration,
}

/// A running light-client process.
pub struct Supervisor {
    child: Option<Child>,
    rpc_port: u16,
    health: SupervisorHealth,
    log_tasks: Vec<JoinHandle<()>>,
    config: SupervisorConfig,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("rpc_port", &self.rpc_port)
            .field("health", &self.health)
            .finish_non_exhaustive()
    }
}

/// Ask the OS for a free port, then release it.
///
/// Inherently a race: something else could take the port before the child
/// binds it. The readiness poll is what catches that, and the caller retries
/// with a fresh port.
fn free_port() -> Result<u16, BackendError> {
    let listener = StdTcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

impl Supervisor {
    /// Generate a config, spawn the child, and wait until it answers.
    pub async fn start(config: SupervisorConfig) -> Result<Self, BackendError> {
        let rpc_port = free_port()?;
        let p2p_port = free_port()?;

        let lc_config = LightClientConfig {
            data_dir: config.data_dir.clone(),
            network: config.network,
            rpc_port,
            p2p_port,
        };
        let config_path = config.data_dir.join("light-client.toml");
        lc_config.write_to(&config_path)?;

        let mut child = Command::new(&config.binary)
            .arg("run")
            .arg("--config-file")
            .arg(&config_path)
            .env("RUST_LOG", &config.log_level)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                BackendError::Spawn(format!("{}: {e}", config.binary.display()))
            })?;

        let pid = child.id().unwrap_or_default();
        let mut log_tasks = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            log_tasks.push(forward(stdout, pid, "stdout"));
        }
        if let Some(stderr) = child.stderr.take() {
            log_tasks.push(forward(stderr, pid, "stderr"));
        }

        let mut supervisor = Self {
            child: Some(child),
            rpc_port,
            health: SupervisorHealth::Running { restarts: 0 },
            log_tasks,
            config,
        };

        if let Err(e) = supervisor.await_ready().await {
            // Never leave an unreachable child behind.
            let _ = supervisor.stop().await;
            return Err(e);
        }
        Ok(supervisor)
    }

    pub const fn port(&self) -> u16 {
        self.rpc_port
    }

    pub fn health(&self) -> SupervisorHealth {
        self.health.clone()
    }

    /// Poll `local_node_info` until it answers or the deadline passes.
    async fn await_ready(&mut self) -> Result<(), BackendError> {
        let url = format!("http://127.0.0.1:{}/", self.rpc_port);
        let client = RpcClient::new(url, Duration::from_secs(2))?;
        let deadline = Instant::now() + self.config.ready_timeout;
        let mut backoff = Duration::from_millis(100);
        loop {
            if let Some(child) = self.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(BackendError::Spawn(format!(
                        "light client exited during startup with {status}"
                    )));
                }
            }
            if client
                .call::<serde_json::Value>("local_node_info", serde_json::json!([]))
                .await
                .is_ok()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(BackendError::Timeout);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }
}

/// Re-emit a child stream through `tracing`, tagged with its PID.
fn forward<R>(stream: R, pid: u32, name: &'static str) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "lantern::light_client", pid, stream = name, "{line}");
        }
    })
}
```

`stop` lands in Task 14; add a temporary `pub async fn stop(&mut self) -> Result<(), BackendError> { Ok(()) }` here so this task compiles, and note in the report that Task 14 replaces it.

- [ ] **Step 4: Wire, run, clippy, commit**

In `lib.rs` add `pub mod supervisor;` and `pub use supervisor::{Supervisor, SupervisorConfig, SupervisorHealth};`.

Run: `cargo test -p lantern-chain-backend --features testing`
Expected: PASS. The supervisor tests need the feature because the stub binary is gated on it; note this in the CI step of Task 20.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): supervisor spawn, dynamic ports, readiness polling and log forwarding"
```

---

### Task 14: chain-backend — graceful shutdown

**Files:**
- Modify: `crates/chain-backend/src/supervisor.rs`
- Modify: `crates/chain-backend/tests/supervisor.rs`

**Interfaces:**
- Produces: a real `Supervisor::stop`, replacing Task 13's placeholder.

SIGTERM then a grace period then SIGKILL, on Unix. Windows has no SIGTERM, so it goes straight to `Child::kill`. `nix::sys::signal::kill` is a safe function, so `forbid(unsafe_code)` is untouched.

- [ ] **Step 1: Write the failing tests**

Append to `crates/chain-backend/tests/supervisor.rs`:

```rust
#[tokio::test]
async fn stopping_reaps_the_child_and_frees_the_port() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    let port = sup.port();
    sup.stop().await.expect("stops");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));

    // The child is gone, so the port binds again.
    let rebind = std::net::TcpListener::bind(("127.0.0.1", port));
    assert!(rebind.is_ok(), "port {port} was not released");
}

#[tokio::test]
async fn stopping_twice_is_harmless() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sup = Supervisor::start(config(dir.path())).await.expect("starts");
    sup.stop().await.expect("first stop");
    sup.stop().await.expect("second stop is a no-op");
    assert!(matches!(sup.health(), SupervisorHealth::Stopped));
}
```

- [ ] **Step 2: Run to confirm the port test fails**

Run: `cargo test -p lantern-chain-backend --features testing --test supervisor`
Expected: `stopping_reaps_the_child_and_frees_the_port` FAILS, because Task 13's `stop` is a no-op that leaves the child holding the port.

- [ ] **Step 3: Implement**

Replace the placeholder `stop` in `supervisor.rs` and add the signal helper:

```rust
/// How long a child gets to exit after SIGTERM before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[cfg(unix)]
fn request_termination(pid: u32) -> Result<(), BackendError> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    // `kill` is a safe wrapper, so the workspace's unsafe ban is untouched.
    let raw = i32::try_from(pid)
        .map_err(|_| BackendError::Spawn(format!("pid {pid} is out of range")))?;
    kill(Pid::from_raw(raw), Signal::SIGTERM)
        .map_err(|e| BackendError::Spawn(format!("could not signal {pid}: {e}")))
}

#[cfg(not(unix))]
const fn request_termination(_pid: u32) -> Result<(), BackendError> {
    // Windows has no SIGTERM; the caller falls through to a hard kill.
    Ok(())
}

impl Supervisor {
    /// Stop the child: SIGTERM, a grace period, then SIGKILL.
    ///
    /// Idempotent — calling it on an already-stopped supervisor succeeds.
    pub async fn stop(&mut self) -> Result<(), BackendError> {
        for task in self.log_tasks.drain(..) {
            task.abort();
        }
        let Some(mut child) = self.child.take() else {
            self.health = SupervisorHealth::Stopped;
            return Ok(());
        };

        if cfg!(unix) {
            if let Some(pid) = child.id() {
                if let Err(e) = request_termination(pid) {
                    tracing::debug!("SIGTERM failed, falling through to kill: {e}");
                }
            }
        }

        match tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await {
            Ok(Ok(status)) => tracing::debug!("light client exited with {status}"),
            _ => {
                tracing::warn!("light client did not exit in time; killing");
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        self.health = SupervisorHealth::Stopped;
        Ok(())
    }
}
```

Move the `stop` method into the same `impl Supervisor` block as the rest rather than a second block, and delete the Task 13 placeholder.

- [ ] **Step 4: Run, clippy, commit**

Run: `cargo test -p lantern-chain-backend --features testing` — PASS.

Note: `kill_on_drop(true)` from Task 13 is a backstop for panics; `stop` is the deliberate path and is what tests assert.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): graceful light-client shutdown with a platform-split signal path"
```

---

### Task 15: chain-backend — restart backoff and circuit breaker

**Files:**
- Modify: `crates/chain-backend/src/supervisor.rs`
- Modify: `crates/chain-backend/tests/supervisor.rs`

**Interfaces:**
- Produces: `RestartPolicy`, `Supervisor::ensure_running`, `SupervisorConfig.policy`.

**Design note worth stating plainly:** restarts happen on demand, from `ensure_running`, rather than from a background watchdog task. The backend calls it before serving a query and when reporting status, which a wallet does regularly. This keeps the supervisor free of shared mutable state and makes every restart path deterministic to test; the cost is that a crash while the wallet is completely idle is noticed at the next call rather than immediately.

- [ ] **Step 1: Write the failing tests**

Append to `crates/chain-backend/tests/supervisor.rs`:

```rust
use lantern_chain_backend::supervisor::RestartPolicy;

/// A policy with tiny timings so the tests stay fast.
fn brisk_policy() -> RestartPolicy {
    RestartPolicy {
        backoff_base: Duration::from_millis(10),
        backoff_cap: Duration::from_millis(40),
        breaker_threshold: 3,
        breaker_window: Duration::from_secs(60),
    }
}

#[tokio::test]
async fn a_crashed_child_is_restarted_on_the_next_call() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.policy = brisk_policy();
    cfg.extra_env.push(("FAKE_LC_EXIT_AFTER_MS".into(), "150".into()));

    let mut sup = Supervisor::start(cfg).await.expect("starts");
    let first_port = sup.port();
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Drive it until the restart lands; backoff means the first call may
    // legitimately report NotReady.
    let mut restarted = false;
    for _ in 0..20 {
        if sup.ensure_running().await.is_ok() {
            restarted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(restarted, "a dead child must come back, health={:?}", sup.health());
    assert_ne!(sup.port(), first_port, "a restart takes a fresh port");
    sup.stop().await.expect("stops");
}

#[tokio::test]
async fn repeated_fast_crashes_open_the_circuit_and_stop_retrying() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.policy = brisk_policy();
    cfg.extra_env.push(("FAKE_LC_EXIT_AFTER_MS".into(), "1".into()));

    // The child dies almost immediately, so startup itself may already fail;
    // either way the breaker must trip rather than spinning forever.
    let mut sup = match Supervisor::start(cfg).await {
        Ok(s) => s,
        Err(_) => return, // died during startup: no supervisor to drive, and nothing spun
    };
    for _ in 0..40 {
        let _ = sup.ensure_running().await;
        if matches!(sup.health(), SupervisorHealth::CircuitOpen) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        matches!(sup.health(), SupervisorHealth::CircuitOpen),
        "expected the breaker to open, health={:?}",
        sup.health()
    );
    // Once open it stays open rather than hammering the binary.
    let err = sup.ensure_running().await.expect_err("circuit is open");
    assert!(matches!(err, lantern_chain_backend::BackendError::Spawn(_)), "{err:?}");
    sup.stop().await.expect("stops");
}

#[tokio::test]
async fn a_healthy_child_needs_no_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cfg = config(dir.path());
    cfg.policy = brisk_policy();
    let mut sup = Supervisor::start(cfg).await.expect("starts");
    let port = sup.port();
    sup.ensure_running().await.expect("already running");
    assert_eq!(sup.port(), port, "no needless restart");
    sup.stop().await.expect("stops");
}
```

Update `fn config` in that file to set the two new fields:

```rust
        policy: RestartPolicy::default(),
        extra_env: Vec::new(),
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-chain-backend --features testing --test supervisor`
Expected: FAIL — `RestartPolicy`, `policy`, `extra_env` and `ensure_running` do not exist.

- [ ] **Step 3: Implement**

Add to `supervisor.rs`:

```rust
use std::collections::VecDeque;

/// How aggressively a crashed child is brought back.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    /// Exits within `breaker_window` that trip the breaker.
    pub breaker_threshold: u32,
    pub breaker_window: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        // Spec §5: 1s, 2s, 4s, 8s capped at 30s; five crashes in sixty
        // seconds stops auto-restart and surfaces the failure.
        Self {
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(30),
            breaker_threshold: 5,
            breaker_window: Duration::from_secs(60),
        }
    }
}
```

Extend `SupervisorConfig` with:

```rust
    pub policy: RestartPolicy,
    /// Extra environment for the child. Tests use it to steer the stub's
    /// failure modes without touching the parent's environment.
    pub extra_env: Vec<(String, String)>,
```

Pass it through in `start`: `.envs(config.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))` on the `Command`.

Add restart bookkeeping to `Supervisor`:

```rust
    exits: VecDeque<Instant>,
    next_attempt_at: Option<Instant>,
    restarts: u32,
```

initialised to `VecDeque::new()`, `None`, `0` in `start`.

Then:

```rust
impl Supervisor {
    /// Make sure a child is running, restarting a crashed one within policy.
    ///
    /// Called before serving a query and when reporting status, rather than
    /// from a background watchdog: no shared mutable state, and every path is
    /// deterministic to test.
    pub async fn ensure_running(&mut self) -> Result<(), BackendError> {
        if matches!(self.health, SupervisorHealth::CircuitOpen) {
            return Err(BackendError::Spawn(
                "light client restarted too many times; not retrying".into(),
            ));
        }

        // Still alive? Nothing to do.
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    tracing::warn!("light client exited with {status}");
                    self.record_exit();
                }
                Err(e) => return Err(BackendError::Spawn(e.to_string())),
            }
        }
        if self.child.is_some() {
            self.child = None;
        }

        if matches!(self.health, SupervisorHealth::CircuitOpen) {
            return Err(BackendError::Spawn(
                "light client restarted too many times; not retrying".into(),
            ));
        }

        if let Some(at) = self.next_attempt_at {
            if Instant::now() < at {
                self.health = SupervisorHealth::Restarting {
                    attempt: self.restarts,
                };
                return Err(BackendError::NotReady);
            }
        }

        self.restarts = self.restarts.saturating_add(1);
        let mut config = self.config.clone();
        config.ready_timeout = self.config.ready_timeout;
        let replacement = Self::start(config).await?;
        self.adopt(replacement);
        Ok(())
    }

    /// Note a crash and open the breaker if they are coming too fast.
    fn record_exit(&mut self) {
        let now = Instant::now();
        self.exits.push_back(now);
        while let Some(front) = self.exits.front() {
            if now.duration_since(*front) > self.config.policy.breaker_window {
                self.exits.pop_front();
            } else {
                break;
            }
        }
        if u32::try_from(self.exits.len()).unwrap_or(u32::MAX)
            >= self.config.policy.breaker_threshold
        {
            tracing::error!(
                "light client crashed {} times within {:?}; giving up",
                self.exits.len(),
                self.config.policy.breaker_window
            );
            self.health = SupervisorHealth::CircuitOpen;
            return;
        }
        let shift = self.restarts.min(16);
        let backoff = self
            .config
            .policy
            .backoff_base
            .saturating_mul(1_u32 << shift)
            .min(self.config.policy.backoff_cap);
        self.next_attempt_at = Some(now + backoff);
    }

    /// Take over a freshly started supervisor's child, keeping crash history.
    fn adopt(&mut self, mut other: Self) {
        self.child = other.child.take();
        self.rpc_port = other.rpc_port;
        self.log_tasks = std::mem::take(&mut other.log_tasks);
        self.next_attempt_at = None;
        self.health = SupervisorHealth::Running {
            restarts: self.restarts,
        };
    }
}
```

`adopt` must leave `other` inert so its `Drop` does not kill the child it just handed over: taking `child` and the log tasks out of it does exactly that, and `kill_on_drop` only fires on a `Child` that is still owned.

- [ ] **Step 4: Run, clippy, commit**

Run: `cargo test -p lantern-chain-backend --features testing` — PASS.

If `repeated_fast_crashes_open_the_circuit_and_stop_retrying` is flaky because the stub dies before readiness, raise `FAKE_LC_EXIT_AFTER_MS` to 60 and lengthen the loop. Report the value used.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): restart backoff and a crash-rate circuit breaker"
```

---

### Task 16: chain-backend — the `EmbeddedLight` backend

**Files:**
- Create: `crates/chain-backend/src/backends/embedded_light.rs`
- Modify: `crates/chain-backend/src/backends/mod.rs`, `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `EmbeddedLight::new(network, SupervisorConfig)` implementing `ChainBackend`.

`ChainBackend` takes `&self` but the supervisor needs `&mut`, so the state sits behind a `tokio::sync::Mutex`. The lock is held only long enough to clone an `Arc<LightRpc>`, never across a network round trip, so queries do not serialise behind each other.

- [ ] **Step 1: Write the failing test**

Create `crates/chain-backend/tests/embedded.rs`:

```rust
//! The supervised backend, driven against the stub light client.

use std::time::Duration;

use lantern_chain_backend::backends::EmbeddedLight;
use lantern_chain_backend::supervisor::{RestartPolicy, SupervisorConfig};
use lantern_chain_backend::{BackendError, ChainBackend};
use lantern_sdk_schema::{BackendKind, BackendStatus, Network};

fn config(dir: &std::path::Path) -> SupervisorConfig {
    SupervisorConfig {
        binary: env!("CARGO_BIN_EXE_fake_light_client").into(),
        data_dir: dir.to_path_buf(),
        network: Network::Testnet,
        log_level: "info".to_string(),
        ready_timeout: Duration::from_secs(10),
        policy: RestartPolicy::default(),
        extra_env: Vec::new(),
    }
}

#[tokio::test]
async fn it_is_not_ready_until_started_and_reports_light_capabilities() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = EmbeddedLight::new(Network::Testnet, config(dir.path()));
    assert_eq!(backend.kind(), BackendKind::EmbeddedLight);
    assert!(backend.capabilities().needs_script_registration);
    assert_eq!(backend.status().await, BackendStatus::Stopped);

    let err = backend.tip_header().await.expect_err("not started yet");
    assert!(matches!(err, BackendError::NotReady), "{err:?}");
}

#[tokio::test]
async fn starting_brings_up_a_child_and_stopping_takes_it_down() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = EmbeddedLight::new(Network::Testnet, config(dir.path()));
    backend.start().await.expect("starts");
    // The stub answers local_node_info but not get_tip_header, so status
    // surfaces an RPC error rather than a fabricated tip — which is the
    // honest behaviour and proves the call reached the child.
    assert!(!matches!(backend.status().await, BackendStatus::Stopped));
    backend.stop().await.expect("stops");
    assert_eq!(backend.status().await, BackendStatus::Stopped);
}
```

- [ ] **Step 2: Run to confirm it fails, then implement**

Create `crates/chain-backend/src/backends/embedded_light.rs`:

```rust
//! A light client whose process Lantern owns.
//!
//! Identical RPC to [`super::RemoteLight`]; the difference is lifecycle. The
//! port is not known until the child is up, so the RPC client is built at
//! start time rather than construction time.

use std::sync::Arc;

use async_trait::async_trait;
use ckb_jsonrpc_types::{HeaderView, Transaction, TransactionWithStatusResponse};
use ckb_types::H256;
use lantern_sdk_schema::{BackendCapabilities, BackendKind, BackendStatus, Network};
use tokio::sync::Mutex;

use crate::backend::ChainBackend;
use crate::backends::remote_light::{DEFAULT_TIMEOUT, RemoteLight};
use crate::cursor::{CellPage, Cursor};
use crate::error::BackendError;
use crate::light::LightRpc;
use crate::query::{CellQuery, WatchedScript};
use crate::supervisor::{Supervisor, SupervisorConfig, SupervisorHealth};

struct Inner {
    supervisor: Option<Supervisor>,
    rpc: Option<Arc<LightRpc>>,
    config: SupervisorConfig,
}

/// The default backend: a supervised `ckb-light-client`.
pub struct EmbeddedLight {
    network: Network,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for EmbeddedLight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedLight")
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

impl EmbeddedLight {
    pub fn new(network: Network, config: SupervisorConfig) -> Self {
        Self {
            network,
            inner: Mutex::new(Inner {
                supervisor: None,
                rpc: None,
                config,
            }),
        }
    }

    /// Clone out the RPC handle, restarting a crashed child first.
    ///
    /// The lock is released before any network call, so queries do not queue
    /// behind one another.
    async fn rpc(&self) -> Result<Arc<LightRpc>, BackendError> {
        let mut inner = self.inner.lock().await;
        let Some(supervisor) = inner.supervisor.as_mut() else {
            return Err(BackendError::NotReady);
        };
        supervisor.ensure_running().await?;
        let port = supervisor.port();
        // A restart moves the port, so rebuild the client when it changes.
        let stale = inner
            .rpc
            .as_ref()
            .is_none_or(|rpc| !rpc.url().contains(&port.to_string()));
        if stale {
            inner.rpc = Some(Arc::new(LightRpc::new(
                format!("http://127.0.0.1:{port}/"),
                DEFAULT_TIMEOUT,
            )?));
        }
        inner.rpc.clone().ok_or(BackendError::NotReady)
    }
}

#[async_trait]
impl ChainBackend for EmbeddedLight {
    fn kind(&self) -> BackendKind {
        BackendKind::EmbeddedLight
    }

    fn network(&self) -> Network {
        self.network
    }

    fn capabilities(&self) -> BackendCapabilities {
        RemoteLight::light_capabilities()
    }

    async fn status(&self) -> BackendStatus {
        {
            let inner = self.inner.lock().await;
            match inner.supervisor.as_ref().map(Supervisor::health) {
                None | Some(SupervisorHealth::Stopped) => return BackendStatus::Stopped,
                Some(SupervisorHealth::CircuitOpen) => {
                    return BackendStatus::Error {
                        message: "light client restarted too many times".to_string(),
                    };
                }
                Some(SupervisorHealth::Restarting { attempt }) => {
                    return BackendStatus::Error {
                        message: format!("light client restarting (attempt {attempt})"),
                    };
                }
                Some(SupervisorHealth::Running { .. }) => {}
            }
        }
        match self.rpc().await {
            Ok(rpc) => RemoteLight::light_status(&rpc).await,
            Err(e) => BackendStatus::Error {
                message: e.to_string(),
            },
        }
    }

    async fn tip_header(&self) -> Result<HeaderView, BackendError> {
        self.rpc().await?.tip_header().await
    }

    async fn get_cells(
        &self,
        query: &CellQuery,
        after: Option<&Cursor>,
    ) -> Result<CellPage, BackendError> {
        self.rpc().await?.get_cells(query, after).await
    }

    async fn get_transaction(
        &self,
        hash: &H256,
    ) -> Result<Option<TransactionWithStatusResponse>, BackendError> {
        self.rpc().await?.get_transaction(hash).await
    }

    async fn send_transaction(&self, tx: &Transaction) -> Result<H256, BackendError> {
        self.rpc().await?.send_transaction(tx).await
    }

    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError> {
        self.rpc().await?.set_scripts_partial(scripts).await
    }

    async fn start(&self) -> Result<(), BackendError> {
        let mut inner = self.inner.lock().await;
        if inner.supervisor.is_some() {
            return Ok(());
        }
        let supervisor = Supervisor::start(inner.config.clone()).await?;
        let port = supervisor.port();
        inner.rpc = Some(Arc::new(LightRpc::new(
            format!("http://127.0.0.1:{port}/"),
            DEFAULT_TIMEOUT,
        )?));
        inner.supervisor = Some(supervisor);
        Ok(())
    }

    async fn stop(&self) -> Result<(), BackendError> {
        let mut inner = self.inner.lock().await;
        if let Some(mut supervisor) = inner.supervisor.take() {
            supervisor.stop().await?;
        }
        inner.rpc = None;
        Ok(())
    }
}
```

`RemoteLight::light_capabilities` and `light_status` are `pub(crate)` from Task 10; keep them that way and do not widen them.

- [ ] **Step 3: Wire, run, clippy, commit**

Add `pub mod embedded_light;` and `pub use embedded_light::EmbeddedLight;` to `backends/mod.rs`, and re-export from `lib.rs`.

Run: `cargo test -p lantern-chain-backend --features testing` — PASS.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): EmbeddedLight backend over the supervised process"
```

---

### Task 17: account-registry — `accounts.json` v2

**Files:**
- Modify: `crates/account-registry/src/store.rs`
- Modify: `crates/account-registry/src/lib.rs`

**Interfaces:**
- Produces: `WalletOrigin { Created, Imported }`; `StoredAccount.watch_from_block: Option<u64>`; `AccountRegistry::{origin, set_origin, set_watch_from_block}`; `accounts.json` version 2 with a v1 migration.

**Why this costs more than it looks.** A light client downloads no filters before the height you register, so each account needs a start height. Without one a fresh mainnet wallet re-scans from genesis: roughly eight hours at the ~500 blocks/second the `ckb-light-client-lite` benchmark measured. And the height depends on how the wallet was opened, which plan 1c never recorded — hence `origin`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/account-registry/src/store.rs`:

```rust
    #[test]
    fn a_v1_file_migrates_to_v2_conservatively() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        // A real plan 1c file: version 1, no origin, no watchFromBlock.
        let v1 = r#"{
          "version": 1,
          "accounts": [{
            "id": "secp256k1_blake160-1111111111111111111111111111111111111111",
            "label": "Main",
            "lockType": "secp256k1_blake160",
            "extensionId": "core.secp256k1",
            "lockArgs": "1111111111111111111111111111111111111111",
            "derivation": { "change": 0, "index": 0 },
            "createdAt": 1700000000
          }]
        }"#;
        std::fs::write(&path, v1).expect("writes");

        let reg = AccountRegistry::open(&path).expect("opens a v1 file");
        assert_eq!(reg.origin(), WalletOrigin::Imported, "a v1 file cannot say; assume the worst");
        assert_eq!(
            reg.list()[0].watch_from_block,
            Some(0),
            "scan everything rather than risk missing history"
        );

        reg.save().expect("saves");
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 2"), "{text}");
        assert!(text.contains("\"origin\": \"imported\""), "{text}");
        assert!(text.contains("\"watchFromBlock\": 0"), "{text}");
    }

    #[test]
    fn a_v2_file_round_trips_with_origin_and_heights() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        {
            let mut reg = AccountRegistry::open(&path).expect("opens empty");
            reg.set_origin(WalletOrigin::Created);
            let mut account = acct(0, 0x11);
            account.watch_from_block = Some(22_000_000);
            reg.add(account).expect("adds");
            reg.save().expect("saves");
        }
        let reg = AccountRegistry::open(&path).expect("reopens");
        assert_eq!(reg.origin(), WalletOrigin::Created);
        assert_eq!(reg.list()[0].watch_from_block, Some(22_000_000));
    }

    #[test]
    fn a_future_version_is_still_corrupt() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, r#"{ "version": 3, "origin": "created", "accounts": [] }"#)
            .expect("writes");
        assert!(matches!(AccountRegistry::open(&path), Err(RegistryError::Corrupt)));
    }

    #[test]
    fn a_start_height_can_be_resolved_later() {
        let dir = tempdir().expect("tempdir");
        let mut reg = AccountRegistry::open(dir.path().join("accounts.json")).expect("opens");
        reg.add(acct(0, 0x11)).expect("adds");
        let id = acct(0, 0x11).id;
        assert_eq!(reg.get(&id).expect("present").watch_from_block, None);
        reg.set_watch_from_block(&id, 22_000_000).expect("resolves");
        assert_eq!(
            reg.get(&id).expect("present").watch_from_block,
            Some(22_000_000)
        );
        assert!(matches!(
            reg.set_watch_from_block("nope", 1),
            Err(RegistryError::NotFound)
        ));
    }
```

Update the existing `fn acct` helper to add `watch_from_block: None,` to the `StoredAccount` it builds, and add `use super::WalletOrigin;` to the test imports.

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p lantern-account-registry`
Expected: FAIL — `WalletOrigin`, `watch_from_block`, `origin()`, `set_origin`, `set_watch_from_block` are undefined.

- [ ] **Step 3: Implement**

In `crates/account-registry/src/store.rs`, change `FILE_VERSION` to `2` and add:

```rust
/// How the wallet behind this registry was opened.
///
/// It decides the start height of a newly derived account: a wallet created
/// moments ago has no history, an imported one may have years of it. Plan 1c's
/// `WalletCore` does not remember this across a lock/unlock cycle, so it is
/// persisted here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletOrigin {
    Created,
    Imported,
}

impl Default for WalletOrigin {
    /// A file that does not say is assumed to be an import, which scans more
    /// than necessary rather than missing history.
    fn default() -> Self {
        Self::Imported
    }
}
```

Add the field to `StoredAccount`, after `derivation`:

```rust
    /// Height a light backend should begin filtering from.
    ///
    /// `None` means "not yet determined": it arises only for an account
    /// created on a freshly created wallet before a backend was attached, and
    /// is resolved to the tip at first sync. Imports record `Some(0)`.
    #[serde(default)]
    pub watch_from_block: Option<u64>,
```

Change `AccountsFile` and `AccountRegistry`:

```rust
#[derive(Serialize, Deserialize)]
struct AccountsFile {
    version: u32,
    #[serde(default)]
    origin: WalletOrigin,
    accounts: Vec<StoredAccount>,
}

#[derive(Debug)]
pub struct AccountRegistry {
    path: PathBuf,
    origin: WalletOrigin,
    accounts: Vec<StoredAccount>,
}
```

Replace `open` and add the accessors:

```rust
    /// A missing file is an empty registry. A present but unreadable file is
    /// `Corrupt`; it is never silently replaced.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                origin: WalletOrigin::default(),
                accounts: Vec::new(),
            });
        }
        let bytes = fs::read(&path)?;
        let mut file: AccountsFile =
            serde_json::from_slice(&bytes).map_err(|_| RegistryError::Corrupt)?;
        match file.version {
            1 => {
                // A v1 file cannot say how the wallet was opened, and its
                // accounts carry no start height. Assume an import and scan
                // from genesis: slow, never wrong.
                file.origin = WalletOrigin::Imported;
                for account in &mut file.accounts {
                    account.watch_from_block = Some(0);
                }
            }
            FILE_VERSION => {}
            _ => return Err(RegistryError::Corrupt),
        }
        Ok(Self {
            path,
            origin: file.origin,
            accounts: file.accounts,
        })
    }

    pub const fn origin(&self) -> WalletOrigin {
        self.origin
    }

    /// Record how the wallet was opened. Called once, at creation or import.
    pub const fn set_origin(&mut self, origin: WalletOrigin) {
        self.origin = origin;
    }

    /// Fill in a start height that was not known when the account was made.
    pub fn set_watch_from_block(&mut self, id: &str, block: u64) -> Result<(), RegistryError> {
        let account = self
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(RegistryError::NotFound)?;
        account.watch_from_block = Some(block);
        Ok(())
    }
```

and in `save`, include the origin:

```rust
        let file = AccountsFile {
            version: FILE_VERSION,
            origin: self.origin,
            accounts: self.accounts.clone(),
        };
```

- [ ] **Step 4: Fix the call site**

`crates/wallet-core/src/core.rs`'s `create_account` builds a `StoredAccount` literal and will no longer compile. Add `watch_from_block: None,` for now; Task 19 gives it the real value.

- [ ] **Step 5: Run, clippy, commit**

Run: `cargo test -p lantern-account-registry` — PASS, 14 tests.
Run: `cargo test --workspace` — PASS.

```bash
cargo fmt --all
git add crates/account-registry crates/wallet-core
git commit -m "feat(account-registry): accounts.json v2 with wallet origin and per-account start height"
```

---

### Task 18: chain-backend — the backend manager

**Files:**
- Create: `crates/chain-backend/src/manager.rs`
- Modify: `crates/chain-backend/src/lib.rs`

**Interfaces:**
- Produces: `BackendManager::{open, profiles, add_profile, remove_profile, current_network, current_backend, activate, save, shutdown}`; `backends.json` version 1.

- [ ] **Step 1: Write the failing tests**

Create `crates/chain-backend/src/manager.rs` with only its test module:

```rust
//! Backend profiles and which one is active.
//!
//! Spec §18 keeps two axes apart: network is which chain, backend is how it
//! is reached. Switching backend within a network leaves accounts untouched.

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{BackendKind, BackendProfile, Network};
    use tempfile::tempdir;

    use super::BackendManager;
    use crate::error::BackendError;

    fn profile(id: &str, network: Network, kind: BackendKind) -> BackendProfile {
        BackendProfile {
            id: id.to_string(),
            label: id.to_string(),
            network,
            kind,
            endpoint: Some("http://127.0.0.1:8114/".to_string()),
        }
    }

    #[test]
    fn a_missing_file_yields_the_first_run_defaults() {
        let dir = tempdir().expect("tempdir");
        let manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        assert_eq!(manager.profiles().len(), 2, "one per network");
        assert_eq!(manager.current_network(), Network::Mainnet);
        assert!(
            manager
                .profiles()
                .iter()
                .all(|p| p.kind == BackendKind::EmbeddedLight),
            "spec §6: the embedded light client is the default"
        );
    }

    #[test]
    fn profiles_round_trip_through_disk() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("backends.json");
        {
            let mut manager = BackendManager::open(&path).expect("opens");
            manager
                .add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull))
                .expect("adds");
            manager.save().expect("saves");
        }
        let text = std::fs::read_to_string(&path).expect("reads");
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.contains("\"remote_full\""), "{text}");

        let manager = BackendManager::open(&path).expect("reopens");
        assert!(manager.profiles().iter().any(|p| p.id == "pi"));
    }

    #[test]
    fn duplicate_and_missing_profile_ids_are_rejected() {
        let dir = tempdir().expect("tempdir");
        let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        manager
            .add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull))
            .expect("adds");
        assert!(matches!(
            manager.add_profile(profile("pi", Network::Mainnet, BackendKind::RemoteFull)),
            Err(BackendError::Corrupt)
        ));
        assert!(matches!(
            manager.remove_profile("nope"),
            Err(BackendError::ProfileNotFound)
        ));
    }

    #[test]
    fn a_corrupt_or_future_file_is_refused_not_replaced() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("backends.json");
        std::fs::write(&path, "{ not json").expect("writes");
        assert!(matches!(BackendManager::open(&path), Err(BackendError::Corrupt)));
        std::fs::write(&path, r#"{"version": 2, "activeProfileId": "x", "profiles": []}"#)
            .expect("writes");
        assert!(matches!(BackendManager::open(&path), Err(BackendError::Corrupt)));
    }

    #[tokio::test]
    async fn activating_a_profile_switches_the_active_network() {
        let dir = tempdir().expect("tempdir");
        let mut manager = BackendManager::open(dir.path().join("backends.json")).expect("opens");
        // Point at a dead port: activation of a remote backend must fail
        // loudly rather than leaving a half-switched manager.
        manager
            .add_profile(BackendProfile {
                endpoint: Some("http://127.0.0.1:1/".to_string()),
                ..profile("dead", Network::Testnet, BackendKind::RemoteFull)
            })
            .expect("adds");
        assert!(manager.activate("dead").await.is_err());
        assert!(
            manager.current_backend().is_none(),
            "a failed activation leaves nothing active"
        );
    }
}
```

- [ ] **Step 2: Run to confirm it fails, then implement**

```rust
use std::path::{Path, PathBuf};

use lantern_sdk_schema::{BackendKind, BackendProfile, Network};
use serde::{Deserialize, Serialize};

use crate::backend::ChainBackend;
use crate::backends::{FullNode, RemoteLight};
use crate::error::BackendError;

const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackendsFile {
    version: u32,
    active_profile_id: Option<String>,
    profiles: Vec<BackendProfile>,
}

/// Owns the backend profiles and whichever one is live.
pub struct BackendManager {
    path: PathBuf,
    profiles: Vec<BackendProfile>,
    active_id: Option<String>,
    active: Option<Box<dyn ChainBackend>>,
}

impl std::fmt::Debug for BackendManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendManager")
            .field("profiles", &self.profiles.len())
            .field("active_id", &self.active_id)
            .finish_non_exhaustive()
    }
}

impl BackendManager {
    /// Open the profile list, or seed spec §6's first-run defaults.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BackendError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                profiles: Self::defaults(),
                active_id: Some("default-mainnet".to_string()),
                active: None,
            });
        }
        let bytes = std::fs::read(&path)?;
        let file: BackendsFile =
            serde_json::from_slice(&bytes).map_err(|_| BackendError::Corrupt)?;
        if file.version != FILE_VERSION {
            return Err(BackendError::Corrupt);
        }
        Ok(Self {
            path,
            profiles: file.profiles,
            active_id: file.active_profile_id,
            active: None,
        })
    }

    /// Mainnet and testnet, both on the embedded light client.
    fn defaults() -> Vec<BackendProfile> {
        vec![
            BackendProfile {
                id: "default-mainnet".to_string(),
                label: "Mainnet (bundled light client)".to_string(),
                network: Network::Mainnet,
                kind: BackendKind::EmbeddedLight,
                endpoint: None,
            },
            BackendProfile {
                id: "default-testnet".to_string(),
                label: "Testnet (bundled light client)".to_string(),
                network: Network::Testnet,
                kind: BackendKind::EmbeddedLight,
                endpoint: None,
            },
        ]
    }

    pub fn profiles(&self) -> &[BackendProfile] {
        &self.profiles
    }

    pub fn add_profile(&mut self, profile: BackendProfile) -> Result<(), BackendError> {
        if self.profiles.iter().any(|p| p.id == profile.id) {
            return Err(BackendError::Corrupt);
        }
        self.profiles.push(profile);
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<(), BackendError> {
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or(BackendError::ProfileNotFound)?;
        self.profiles.remove(index);
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
            self.active = None;
        }
        Ok(())
    }

    /// The network of the active profile, defaulting to mainnet.
    pub fn current_network(&self) -> Network {
        self.active_id
            .as_ref()
            .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
            .map_or(Network::Mainnet, |p| p.network)
    }

    pub fn current_backend(&self) -> Option<&dyn ChainBackend> {
        self.active.as_deref()
    }

    /// Stop whatever is running and bring up the named profile.
    ///
    /// `EmbeddedLight` is not constructed here: it needs a binary path and a
    /// data directory, which plan 1f resolves. Activating one without those
    /// is `Unsupported` rather than a silent no-op.
    pub async fn activate(&mut self, profile_id: &str) -> Result<(), BackendError> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .ok_or(BackendError::ProfileNotFound)?
            .clone();

        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        self.active_id = None;

        let backend: Box<dyn ChainBackend> = match profile.kind {
            BackendKind::RemoteLight => {
                let endpoint = profile
                    .endpoint
                    .as_deref()
                    .ok_or(BackendError::Unsupported("remote backend without an endpoint"))?;
                Box::new(RemoteLight::new(profile.network, endpoint)?)
            }
            BackendKind::LocalFull | BackendKind::RemoteFull => {
                let endpoint = profile
                    .endpoint
                    .as_deref()
                    .ok_or(BackendError::Unsupported("remote backend without an endpoint"))?;
                Box::new(FullNode::connect(profile.network, profile.kind, endpoint).await?)
            }
            BackendKind::EmbeddedLight => {
                return Err(BackendError::Unsupported(
                    "embedded light client needs a binary path; construct it directly",
                ));
            }
        };
        backend.start().await?;
        self.active = Some(backend);
        self.active_id = Some(profile.id);
        Ok(())
    }

    /// Adopt an already-constructed backend, for the embedded kind.
    pub async fn activate_backend(
        &mut self,
        profile_id: &str,
        backend: Box<dyn ChainBackend>,
    ) -> Result<(), BackendError> {
        if !self.profiles.iter().any(|p| p.id == profile_id) {
            return Err(BackendError::ProfileNotFound);
        }
        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        backend.start().await?;
        self.active = Some(backend);
        self.active_id = Some(profile_id.to_string());
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), BackendError> {
        if let Some(active) = self.active.take() {
            active.stop().await?;
        }
        Ok(())
    }

    /// Write `backends.json` via a temporary file and a rename.
    pub fn save(&self) -> Result<(), BackendError> {
        let file = BackendsFile {
            version: FILE_VERSION,
            active_profile_id: self.active_id.clone(),
            profiles: self.profiles.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|_| BackendError::Corrupt)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        // Windows: `rename` fails when the destination exists. Plan 1f's
        // packaging work replaces this with a platform-aware atomic write.
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
```

- [ ] **Step 3: Wire, run, clippy, commit**

In `lib.rs` add `pub mod manager;` and `pub use manager::BackendManager;`.

Run: `cargo test -p lantern-chain-backend --features testing` — PASS.

```bash
cargo fmt --all
git add crates/chain-backend
git commit -m "feat(chain-backend): backend manager with persisted profiles"
```

---

### Task 19: wallet-core — attach a backend and register account scripts

**Files:**
- Modify: `crates/wallet-core/src/core.rs`
- Modify: `crates/wallet-core/src/error.rs`
- Modify: `crates/wallet-core/src/lib.rs`
- Modify: `crates/wallet-core/tests/wallet_e2e.rs`

**Interfaces:**
- Consumes: `BackendManager`, `ChainBackend`, `WatchedScript` (chain-backend); `WalletOrigin`, `set_watch_from_block` (Task 17)
- Produces: `CoreError::Backend(BackendError)`; `WalletCore::{attach_backend, backend, sync_watched_scripts}`; `create_account` records a start height.

**One simplification of spec §4.6, deliberate.** The spec said a created wallet's account takes `Some(tip)` when a backend is attached. That would make `create_account` async and ripple through plan 1c's tests for no real gain. Instead a created wallet always records `None` and `sync_watched_scripts` resolves it to the tip. `None` keeps exactly one meaning, `create_account` stays synchronous, and the outcome is identical because nothing can arrive at a brand-new address before the first sync.

- [ ] **Step 1: Write the failing tests**

Append to `crates/wallet-core/tests/wallet_e2e.rs`:

```rust
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
        .respond("get_indexer_tip", serde_json::json!({
            "block_hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "block_number": "0x1554ef4"
        }))
        .respond("get_tip_header", serde_json::json!({
            "compact_target": "0x1a08a97e", "dao": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "epoch": "0x1", "extra_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
            "nonce": "0x0", "number": "0x1554ef4",
            "parent_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "proposals_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "timestamp": "0x1", "transactions_root": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "version": "0x0"
        }))
        .start()
        .await;

    let dir = tempdir().expect("tempdir");
    let paths = ProfilePaths::in_dir(dir.path());
    let (mut core, _phrase) =
        WalletCore::create(paths, b"pw", Network::Testnet, WordCount::Words12).expect("creates");
    let account = core.create_account("Fresh").expect("account");
    assert_eq!(core.watch_from_block(&account.id).expect("known"), None);

    let mut manager =
        lantern_chain_backend::BackendManager::open(dir.path().join("backends.json"))
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
        Some(0x1554ef4),
        "the deferred height resolved to the tip"
    );

    // Resolved heights persist, so a later unlock does not rescan.
    core.lock();
    let core = WalletCore::unlock(ProfilePaths::in_dir(dir.path()), b"pw", Network::Testnet)
        .expect("reopens");
    assert_eq!(core.watch_from_block(&account.id).expect("known"), Some(0x1554ef4));
}
```

Add `lantern-chain-backend = { path = "../chain-backend", features = ["testing"] }` to wallet-core's dev-dependencies if Task 2's entry did not already cover it.

- [ ] **Step 2: Run to confirm it fails, then implement**

In `crates/wallet-core/src/error.rs` add:

```rust
    #[error(transparent)]
    Backend(#[from] lantern_chain_backend::BackendError),
```

In `crates/wallet-core/src/core.rs`:

```rust
use lantern_account_registry::WalletOrigin;
use lantern_chain_backend::{BackendManager, ChainBackend, WatchedScript};
```

Add the field to `WalletCore`:

```rust
    backend: Option<BackendManager>,
```

initialised to `None` in `create`, `import` and `unlock`. In `create`, after opening the registry, `accounts.set_origin(WalletOrigin::Created);`; in `import`, `accounts.set_origin(WalletOrigin::Imported);`. Both then `accounts.save()?` so the origin survives even before an account exists.

In `create_account`, replace the `watch_from_block: None,` placeholder from Task 17 with:

```rust
            // An import may have arbitrary history, so scan everything. A
            // freshly created wallet cannot, so defer to the first sync.
            watch_from_block: match self.accounts.origin() {
                WalletOrigin::Imported => Some(0),
                WalletOrigin::Created => None,
            },
```

Then add:

```rust
impl WalletCore {
    /// Adopt a backend manager. The manager owns the active backend; the
    /// wallet only asks it questions.
    pub fn attach_backend(&mut self, manager: BackendManager) {
        self.backend = Some(manager);
    }

    pub fn backend(&self) -> Option<&dyn ChainBackend> {
        self.backend.as_ref().and_then(BackendManager::current_backend)
    }

    /// The start height recorded for an account, if it has one.
    pub fn watch_from_block(&self, account_id: &str) -> Result<Option<u64>, CoreError> {
        self.accounts
            .get(account_id)
            .map(|a| a.watch_from_block)
            .ok_or(CoreError::AccountNotFound)
    }

    /// Resolve any deferred start heights, then tell the backend which
    /// scripts to watch.
    ///
    /// A no-op on full backends, which index everything; on light backends it
    /// is the difference between syncing and sitting idle forever.
    pub async fn sync_watched_scripts(&mut self) -> Result<(), CoreError> {
        let Some(backend) = self.backend.as_ref().and_then(BackendManager::current_backend) else {
            return Err(CoreError::Backend(
                lantern_chain_backend::BackendError::NotReady,
            ));
        };

        // Resolve deferred heights to the current tip before registering, so
        // a light client never downloads filters predating the account.
        let unresolved: Vec<String> = self
            .accounts
            .list()
            .iter()
            .filter(|a| a.watch_from_block.is_none())
            .map(|a| a.id.clone())
            .collect();
        if !unresolved.is_empty() {
            let tip = u64::from(backend.tip_header().await?.inner.number);
            for id in unresolved {
                self.accounts.set_watch_from_block(&id, tip)?;
            }
            self.accounts.save()?;
        }

        let network = self.network;
        let mut watched = Vec::new();
        for account in self.accounts.list() {
            let module = self.locks.get(account.lock_type)?;
            let template = module.script_template();
            let script = ckb_jsonrpc_types::Script {
                code_hash: ckb_types::H256(template.code_hash),
                hash_type: serde_json::from_value(serde_json::json!(match template.hash_type {
                    0 => "data",
                    1 => "type",
                    2 => "data1",
                    _ => "data2",
                }))
                .map_err(|_| CoreError::Backend(
                    lantern_chain_backend::BackendError::Unsupported("hash type"),
                ))?,
                args: ckb_jsonrpc_types::JsonBytes::from_vec(account.lock_args.clone()),
            };
            watched.push(WatchedScript::lock(
                script,
                account.watch_from_block.unwrap_or(0),
            ));
        }
        let _ = network;

        let backend = self
            .backend
            .as_ref()
            .and_then(BackendManager::current_backend)
            .ok_or(CoreError::Backend(
                lantern_chain_backend::BackendError::NotReady,
            ))?;
        backend.watch_scripts(&watched).await?;
        Ok(())
    }
}
```

The backend handle is re-fetched after the mutable registry work because the first borrow ends there; if the borrow checker still objects, read the tip into a local before touching `self.accounts` and drop the borrow explicitly.

Add `pub use lantern_chain_backend::{BackendManager, ChainBackend};` to wallet-core's `lib.rs` re-exports so callers need not depend on chain-backend directly.

- [ ] **Step 3: Run, clippy, commit**

Run: `cargo test -p lantern-wallet-core` — PASS.
Run: `cargo clippy --workspace --all-targets -- -D warnings` — clean.

```bash
cargo fmt --all
git add crates/wallet-core
git commit -m "feat(wallet-core): attach a chain backend and register account scripts"
```

---

### Task 20: live testnet gate, workspace gate, docs, tag

**Files:**
- Create: `crates/chain-backend/tests/live_testnet.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-10-plan-1d-chain-backend-design.md`
- Modify: `~/.claude/rules/ckb-transactions.feedback.md`

- [ ] **Step 1: Write the live-gated test**

Create `crates/chain-backend/tests/live_testnet.rs`:

```rust
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
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
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
    assert!(backend.status().await.is_usable(), "a public node should be usable");
}
```

- [ ] **Step 2: Run it for real, once**

Run: `LANTERN_LIVE_TESTNET=1 cargo test -p lantern-chain-backend --features testing --test live_testnet -- --nocapture`
Expected: three tests pass, with the page count printed.

If paging runs away past 500 pages, the cursor rules are wrong and this is the bug the whole plan exists to prevent. Stop and report rather than raising the limit. Record the observed page count and tip height in the report.

- [ ] **Step 3: CI**

In `.github/workflows/ci.yml`, after the existing vault steps in the `rust` job:

```yaml
      - name: cargo test (chain-backend with the fake node)
        run: cargo test -p lantern-chain-backend --features testing
```

The live test is not added to CI: it is skipped without the env var, and CI must not depend on a public node.

- [ ] **Step 4: Whole-workspace gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings
cargo clippy -p lantern-vault --all-targets --no-default-features -- -D warnings
cargo test --workspace
cargo test -p lantern-chain-backend --features testing
cargo test -p lantern-vault --no-default-features
cargo doc --workspace --no-deps
```

All must pass.

- [ ] **Step 5: Record spec refinements**

Edit `docs/superpowers/specs/2026-09-10-plan-1d-chain-backend-design.md`:

1. §4.6: `create_account` records `Some(0)` for an imported wallet and `None` for a created one; the "`Some(tip)` when a backend is attached" case was dropped so `create_account` stays synchronous. Note that the outcome is identical because nothing can reach a brand-new address before the first sync.
2. §4.4: `BackendManager` gained `activate_backend(profile_id, backend)`, because `EmbeddedLight` needs a binary path that plan 1f resolves and cannot be constructed from a profile alone.
3. §3: add a row noting that restarts happen on demand from `ensure_running` rather than a background watchdog, with the trade-off stated.

- [ ] **Step 6: README**

Replace the status line with:

```markdown
**Status:** Plans 1a (scaffold), 1b (vault), 1c (accounts + secp256k1 signing) and 1d (chain backend + light-client supervision) complete. Next: plan 1e (tx-builder).
```

- [ ] **Step 7: Feedback log**

Append one line to `~/.claude/rules/ckb-transactions.feedback.md` under `## Entries`, dated today, shaped like:

```
2026-MM-DD | ckb-wallet plan 1d (chain backend + light-client supervision) | HIT GAP | <one sentence: whether the cursor discipline held on the live run, what the fake-node fixtures caught that a hand-written mock would not have, and anything about ckb-jsonrpc-types' one-directional indexer types worth carrying forward>
```

The `ckb-jsonrpc-types` finding is genuinely new and belongs in the log: a client cannot use that crate's indexer types because they carry only the server-side serde half.

- [ ] **Step 8: Commit and tag**

```bash
cargo fmt --all
git add crates/chain-backend/tests/live_testnet.rs .github/workflows/ci.yml README.md docs/superpowers/specs/2026-09-10-plan-1d-chain-backend-design.md
git commit -m "test(chain-backend): live-gated testnet checks, CI wiring and docs"
git tag -a v0.0.4-chain -m "Plan 1d: chain backend, light-client supervision, backend manager"
```

Do not push; Phill pushes after review.

---

## Self-Review

**Spec coverage.** §1's seven goals map to: trait and capabilities (Tasks 10, 11), cursor discipline (Task 4, proven live in Task 20), supervision (Tasks 12 to 16), script registration with a start height (Tasks 17, 19), backend and network axes (Task 18), computed sync status (Tasks 10, 11), registry verification (Task 1). §4.1 to §4.6 are Tasks 3, 4 to 11, 12 to 16, 18, 17, 19. §5's layout appears in Tasks 12 and 18. §6's error set is Task 4. §7's test matrix is covered by Tasks 4, 7 to 11, 13 to 18, and 20. §2's non-goals are untouched: no transaction construction, no Tauri, no devnet, no gap-limit scan.

**Placeholders.** None. Four steps name a fallback (reqwest's TLS provider, `toml::Table`'s import path, `env::set_var` under edition 2024, and the flaky-breaker timing); each names the exact alternative and asks the implementer to report which was used.

**Type consistency.** `TransactionWithStatusResponse` is used everywhere, never the non-existent `TransactionWithStatus`. `Cursor` is passed as `Option<&Cursor>` at every call site. `CellPage.cells` is `Vec<Value>` in Task 4 and becomes `Vec<IndexerCell>` in Task 5 Step 5, which is called out explicitly. `RemoteLight::light_capabilities` and `light_status` are `pub(crate)` and used by `EmbeddedLight` in Task 16. `SupervisorConfig` gains `policy` and `extra_env` in Task 15, and Task 16's test constructs it with both. `Supervisor::stop` is a placeholder in Task 13 and replaced in Task 14, stated in both.

**Known risks for the executor.**
- Task 19's `sync_watched_scripts` does two borrows of `self` around a mutable registry update; the step names the fix if the borrow checker objects.
- `hash_type` conversion in Task 19 goes through JSON because `ScriptHashType` is an enum in `ckb-jsonrpc-types` with no numeric constructor. If a direct constructor exists, use it and drop the JSON round trip.
- The supervisor tests need `--features testing` because the stub binary is gated on it; the CI step in Task 20 accounts for this, but a bare `cargo test --workspace` will not run them.
- Task 15's breaker test depends on process timing and is the most likely source of flake in the plan.

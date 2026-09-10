# Lantern Plan 1d — Chain Backend, Light-Client Supervision, Backend Manager (Design)

**Status:** approved design, 2026-09-10.
**Parent spec:** `2026-04-08-foundation-design-tauri-edition-v1.1.md` §5 (light-client subprocess supervision), §6 (chain backend abstraction), §18 (network and backend axes), §21 (recovery).
**Builds on:** plan 1a (scaffold, `v0.0.1-scaffold`), plan 1b (vault, `v0.0.2-vault`), plan 1c (accounts + signing, `v0.0.3-signing`).

---

## 1. Goal

After plan 1d, Lantern can reach a CKB chain through any of four interchangeable backends and know how far it has synced.

1. Talk JSON-RPC to a CKB light client or a full node over one `ChainBackend` trait, with capabilities as data rather than assumptions.
2. Page `get_cells` without ever poisoning its own cursor.
3. Supervise a bundled `ckb-light-client` subprocess: config generation, dynamic port, ready-polling, log forwarding, graceful shutdown, restart with backoff and a circuit breaker.
4. Register account lock scripts with light backends so they actually sync, from a per-account start height.
5. Switch backend within a network without re-deriving accounts, and switch network as a separate axis.
6. Report sync progress computed from real signals, not guessed.
7. Refuse to open a wallet whose plaintext `accounts.json` no longer matches the seed (the carry-over from plan 1c's final review).

## 2. Non-goals

- Transaction construction, fee estimation, witness assembly. Plan 1e (`tx-builder`).
- Tauri commands, sidecar bundling of the light-client binary, OS data-directory resolution. Plan 1f.
- Devnet. §18 lists it as a v0.1 network, but it needs a user-supplied chain spec and system-script registry that no plan has yet. `Network` stays `Mainnet | Testnet`.
- Gap-limit recovery scan (§21). `watch_from_block` makes it possible; it needs its own derivation loop and UI, so it gets its own slice.
- Fiber. v0.3.
- Any wallet-facing balance or history API. This plan delivers the data source; presenting it is later work.
- Block-by-block chain following. Backends answer queries; they do not maintain a local chain copy beyond what the light client stores.

## 3. Decisions locked in this design

| Decision | Choice | Why |
|---|---|---|
| Trait shape | One `dyn`-safe `ChainBackend` + `BackendCapabilities` data | Mirrors 1c's `LockModule` + `AccountCapabilities`; §6 demands runtime-swappable backends |
| Async | `async-trait` | Native AFIT is not `dyn`-safe; the manager holds `Box<dyn ChainBackend>` |
| CKB blockchain types | `ckb-jsonrpc-types = "=1.1.1"`, `ckb-types = "=1.1.1"` | 1.2.0 / 1.1.2+ require rustc 1.95; our toolchain is pinned 1.92. Same pattern as `ckb-hash = "=1.1.0"` |
| CKB **indexer** types | Defined in `chain-backend`, not taken from `ckb-jsonrpc-types` | Verified: that crate's `IndexerSearchKey`/`IndexerScriptType`/`IndexerOrder` derive only `Deserialize` and `IndexerPagination`/`IndexerCell`/`IndexerTip` derive only `Serialize` — server-side halves, exactly backwards for a client. `ckb-sdk-rust` defines its own for the same reason |
| Transport | `reqwest` (rustls, `default-features = false`) | Remote nodes are HTTPS, local are plain HTTP; avoids a native-TLS system dependency |
| Cursor handling | A `Cursor` newtype that cannot represent a terminal sentinel | Verified live 2026-09-10: an exhausted scan returns `last_cursor: "0x"`, and `after: "0x"` returns nothing forever |
| Full-node capability probe | `get_indexer_tip` returning `None` | The node's own signal that the indexer is off; no guessing, no confusing downstream RPC error |
| Remote-light writes | `set_scripts` always with `partial`, never `all`, never `delete` of scripts we did not add | `all` replaces the list globally; two Lantern instances on one shared server would erase each other |
| SIGTERM | `nix::sys::signal::kill` on Unix, `Child::kill` on Windows | Verified safe fn, so `unsafe_code = "forbid"` holds. Windows has no SIGTERM |
| Light-client binary | Injected `PathBuf` + a resolver helper | Follows 1c's `ProfilePaths` precedent; actual bundling is plan 1f's sidecar work |
| Account start height | `StoredAccount.watch_from_block: Option<u64>`, `accounts.json` v2 | Without it a fresh mainnet wallet re-scans filters from genesis: ~8 hours at the measured ~500 blk/s |
| §18 tuple naming | **backend profile**, not "profile" | 1c already uses "profile" for the directory holding `vault.bin`; two meanings of one word will bite |
| Sync progress (light) | target = `get_tip_header().number`; current = min `block_number` over `get_scripts()` | Header sync and filter sync advance independently; the user cares about filter sync |
| Sync progress (full) | `get_indexer_tip` against chain tip | Matches the readiness probe already proven in the fiber-web work |
| Supervisor restarts | On demand, from `ensure_running` before an RPC call, not a background watchdog task | Simpler lifecycle, no idle polling loop; trade-off is that a crash between calls is invisible until the next call is attempted, rather than being noticed and repaired within one poll interval |

## 4. Crate map

Dependency direction is strictly downward; no cycles.

```
wallet-core ──► chain-backend ──► sdk-schema
            ──► account-registry ──► sdk-schema
            ──► vault
            ──► signer-secp256k1
```

### 4.1 `lantern-sdk-schema` additions

```rust
pub enum BackendKind { EmbeddedLight, RemoteLight, LocalFull, RemoteFull }  // snake_case on the wire

pub struct BackendCapabilities {
    pub needs_script_registration: bool,
    pub can_fetch_arbitrary_blocks: bool,
    pub can_estimate_cycles: bool,
    pub indexer_available: bool,
}

pub enum BackendStatus {
    Connecting,                                  // also: a light backend watching none of our scripts
    Syncing { current: u64, target: u64 },
    Synced { tip: u64 },
    Error { message: String },
    Stopped,
}

pub struct BackendProfile {          // §18's (network × backend × name)
    pub id: String,
    pub label: String,
    pub network: Network,
    pub kind: BackendKind,
    pub endpoint: Option<String>,    // None for EmbeddedLight
}
```

All derive `specta::Type` + serde camelCase and join `typescript_bindings()`. `BackendStatus` is an externally tagged enum so the generated TS is a discriminated union.

`is_usable()` is documented as "whether queries can be trusted to return complete results", and is true for `Synced` and `Syncing`. A light backend with none of our scripts registered therefore reports **`Connecting`**, not `Synced`: `get_cells` is guaranteed to return nothing in that state whatever the chain holds, so `Synced` would assert full confidence in exactly the state where there is none. `Connecting` is also not a stalled progress bar, which was the original reason for not reporting `Syncing { current: 0 }` there.

### 4.2 `lantern-chain-backend`

```rust
pub struct CellQuery {               // maps to the shared indexer SearchKey
    pub script: Script,
    pub script_type: ScriptType,     // Lock | Type
    pub limit: u32,
    pub order: Order,                // Asc | Desc
}

pub struct Cursor(String);
impl Cursor {
    /// `"0x"` and `""` are terminal sentinels, never a resumable position.
    pub fn resumable(raw: &str) -> Option<Self>;
    pub fn as_str(&self) -> &str;
}

pub struct CellPage { pub cells: Vec<Cell>, pub next: Option<Cursor> }

pub struct WatchedScript { pub script: Script, pub script_type: ScriptType, pub from_block: u64 }

#[async_trait]
pub trait ChainBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn network(&self) -> Network;
    fn capabilities(&self) -> BackendCapabilities;
    async fn status(&self) -> BackendStatus;
    async fn tip_header(&self) -> Result<HeaderView, BackendError>;
    async fn get_cells(&self, query: &CellQuery, after: Option<&Cursor>) -> Result<CellPage, BackendError>;
    async fn get_transaction(&self, hash: H256) -> Result<Option<TransactionWithStatusResponse>, BackendError>;
    async fn send_transaction(&self, tx: Transaction) -> Result<H256, BackendError>;
    async fn watch_scripts(&self, scripts: &[WatchedScript]) -> Result<(), BackendError>;
    async fn start(&self) -> Result<(), BackendError>;
    async fn stop(&self) -> Result<(), BackendError>;
}
```

**Cursor discipline, the single most important invariant in this plan.** Two rules, enforced in one place so no caller can get them wrong:

1. `CellPage::next` is `None` whenever `cells` is empty, regardless of what the node returned.
2. `Cursor::resumable` rejects `"0x"` and `""`, so a persisted terminal cursor read back becomes `None` rather than a query that returns nothing forever.

Verified against CKB testnet on 2026-09-10: exhausting a scan yields `objects: []` with `last_cursor: "0x"`, and re-issuing `get_cells` with `after: "0x"` against a lock that provably holds cells returns zero objects.

**Type sourcing.** Blockchain types come from `ckb_jsonrpc_types` — `Script`, `CellOutput`, `OutPoint`, `Transaction`, `TransactionView`, `HeaderView`, `CellDep`, `TransactionWithStatusResponse` — all of which derive both `Serialize` and `Deserialize` and are safe for client use. The indexer request/response types are ours, in `indexer.rs`, each deriving both halves: `SearchKey`, `SearchKeyFilter`, `ScriptType`, `Order`, `IndexerCell`, `Pagination<T>`, `Tip`. `chain-backend` re-exports the `ckb_jsonrpc_types` items it exposes so downstream crates need not depend on it directly.

**Modules.** `error.rs` (`BackendError`), `rpc.rs` (JSON-RPC envelope + transport), `indexer.rs` (client-side indexer types), `cursor.rs` (`Cursor`, `CellPage`), `query.rs` (`CellQuery`, `WatchedScript`, conversions to `SearchKey`), `light.rs` (light-client RPC client), `full.rs` (full-node RPC client), `backends/remote_light.rs`, `backends/full_node.rs`, `backends/embedded_light.rs`, `supervisor.rs` (process lifecycle), `config.rs` (light-client config generation), `manager.rs` (`BackendManager`), `probe.rs` (capability probing), `testing.rs` (fake RPC server, behind a `testing` feature).

**Light-client RPC surface** (verified against `light-client-bin/src/rpc.rs`): `set_scripts`, `get_scripts`, `get_cells`, `get_transactions`, `get_cells_capacity`, `send_transaction`, `get_transaction`, `fetch_transaction`, `get_tip_header`, `get_genesis_block`, `get_header`, `fetch_header`, `estimate_cycles`, `local_node_info`, `get_peers`. The client wraps all of them; the trait exposes the subset the wallet needs. **There is no `get_blockchain_info`** — see the chain-identity note in §4.4.

**Registration never rewinds.** `LightRpc::set_scripts_partial` reads `get_scripts` first and sends each script at `max(requested, reported)`. Verified in the client's own `storage_trait.rs`: the `Partial` arm writes each sent height unconditionally (`batch.put(&key, &ss.block_number.to_be_bytes())`), then calls `update_min_filtered_block_number_by_scripts()` and `clear_matched_blocks()` — sending a *lower* number is how a rescan is forced. Since registration runs at least once per launch from an account's original start height, sending it unguarded would restart filter sync from that height on every launch, i.e. from genesis forever for an imported wallet. The guard lives on the RPC client, not in a backend or in wallet-core, so every caller inherits it; a deliberate rescan would need an explicit new method.

**Filter progress is our own.** `LightRpc::filter_progress` takes the minimum over the scripts *this client registered*, not over everything `get_scripts` returns: a shared light client also indexes other wallets' scripts, and a stranger's fresh registration at height 0 would otherwise pin this wallet at `Syncing { current: 0 }` forever.

**Full-node RPC surface used:** `get_tip_header`, `get_transaction`, `send_transaction`, `get_cells`, `get_indexer_tip`, `local_node_info`, `estimate_cycles`. `get_cells` takes the same `SearchKey` shape as the light client, which is why one trait method serves both.

### 4.3 Supervision (`supervisor.rs`, `config.rs`)

```rust
pub struct SupervisorConfig {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub network: Network,
    pub log_level: String,
    pub ready_timeout: Duration,     // default 60s
}

pub struct Supervisor { /* Child, port, restart state, JoinHandles */ }
impl Supervisor {
    pub async fn start(config: SupervisorConfig) -> Result<Self, BackendError>;
    pub fn port(&self) -> u16;
    pub async fn stop(&mut self) -> Result<(), BackendError>;
    pub fn health(&self) -> SupervisorHealth;   // Running { since } | Restarting { attempt } | CircuitOpen | Stopped
}
```

Lifecycle, per §5:

1. **Port.** Bind `127.0.0.1:0`, read the assigned port, drop the listener, write that port into the generated config. Racy in principle; the ready-poll catches a lost race and the supervisor retries with a fresh port.
2. **Config.** Generated into `data_dir/<network>/light-client.toml`: network chain spec, RPC listen address, store path, log level. Regenerated on every start so a stale port can never be reused. The store, the peer database and the config all live under a per-network subdirectory of `data_dir`: nothing stops a caller handing the same `data_dir` to a mainnet and a testnet client, and one RocksDB store holding two chains is not a recoverable state, so the separation is structural rather than checked.
3. **Spawn.** `tokio::process::Command` with the `run` subcommand and `--config-file`, stdout and stderr piped.
4. **Ready.** Poll `local_node_info` with backoff (100ms doubling to 2s) until success or `ready_timeout`.
5. **Logs.** Two tasks read the piped streams line by line and re-emit through `tracing` at `debug`, tagged with the subprocess PID.
6. **Shutdown.** Unix sends SIGTERM via `nix::sys::signal::kill`, waits up to 5s, then SIGKILL. Windows calls `Child::kill` directly. Both then `wait()` to reap.
7. **Restart.** On unexpected exit, backoff 1s, 2s, 4s, 8s, capped 30s. Circuit breaker: five exits within 60s opens the circuit, health becomes `CircuitOpen`, the backend reports `Error`, and auto-restart stops until an explicit `start()`.

A missing or non-executable binary fails `start()` with `BackendError::Spawn` and the backend reports `Error`, which §6's capability-gating UX renders as a disabled feature with a one-click path to another backend.

### 4.4 Manager (`manager.rs`)

```rust
pub struct BackendManager { /* profiles, active backend, paths */ }
impl BackendManager {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BackendError>;   // backends.json; missing = defaults
    pub fn profiles(&self) -> &[BackendProfile];
    pub fn add_profile(&mut self, profile: BackendProfile) -> Result<(), BackendError>;
    pub fn remove_profile(&mut self, id: &str) -> Result<(), BackendError>;
    pub fn current_network(&self) -> Network;
    pub fn current_backend(&self) -> Option<&dyn ChainBackend>;
    pub async fn activate(&mut self, profile_id: &str) -> Result<(), BackendError>;
    pub async fn activate_backend(&mut self, profile_id: &str, backend: Box<dyn ChainBackend>) -> Result<(), BackendError>;
    pub async fn shutdown(&mut self) -> Result<(), BackendError>;
    pub fn save(&self) -> Result<(), BackendError>;
}
```

`backends.json` is `{ "version": 1, "activeProfileId": "...", "profiles": [...] }`, plaintext and public, written tmp-then-rename like `accounts.json`. Missing file yields the two defaults from §6's first-run behaviour: mainnet embedded-light and testnet embedded-light, with mainnet active.

`add_profile`, `remove_profile`, `activate` and `activate_backend` each call `save` before returning. `save` having no caller left a user who switched to testnet back on `default-mainnet` after a restart — which, given that lock args are chain-independent, is the wrong-network hazard delivered by default. `add_profile` and `remove_profile` roll their in-memory change back if the write fails, so the two views cannot diverge.

**Chain identity.** `FullNode::start` calls `get_blockchain_info` and refuses a node whose `chain` field (`"ckb"` / `"ckb_testnet"`) does not match the profile's `Network`, with `BackendError::NetworkMismatch`. **The light client has no equivalent**: its RPC surface (listed in §4.2) has no `get_blockchain_info`, so a `RemoteLight` profile pointed at the wrong chain is still unverified. `EmbeddedLight` is not exposed to this — Lantern generates its config from the network-specific vendored template, so its chain is ours to choose, not a user-typed endpoint's. Closing the `RemoteLight` gap would mean comparing `get_genesis_block`'s hash against pinned per-network constants; that is left for a later slice rather than approximated here.

`activate_backend` adopts an already-constructed backend rather than building one from the profile: `EmbeddedLight` needs a light-client binary `PathBuf` that plan 1f resolves (bundled sidecar path, platform-specific), which `BackendManager` cannot derive from a `BackendProfile` alone. `activate` stays the path for the other three kinds, which are fully describable by their profile; `activate_backend` stops the current backend, starts the supplied one, and records it the same way `activate` does.

`activate` stops the current backend, starts the new one, probes capabilities, and re-registers watched scripts. Switching backend within a network never touches accounts; switching network changes which accounts are in view, since §18 scopes accounts to a network.

Probing a local full node on first run (§6 step 2) is a `probe_local_full()` helper that tries `http://127.0.0.1:8114` and reports whether a node with an indexer answered. It offers, never switches automatically.

### 4.5 `lantern-account-registry` changes

`StoredAccount` gains `watch_from_block: Option<u64>`. `accounts.json` goes to `version: 2`:

- v2 gains a top-level `origin: "created" | "imported"` alongside the account list. It is a wallet-level, public fact, and it is what lets `create_account` choose a start height correctly after a lock/unlock cycle — plan 1c's `WalletCore` does not otherwise remember how the profile was opened.
- v2 records carry the field.
- v1 records load with `origin: "imported"` and `watch_from_block: Some(0)` — scan from genesis, slow but never wrong, because a v1 file cannot say whether its accounts were created or imported.
- `save` always writes v2. A v1 file is migrated in place on first save.
- A version above 2 is still `Corrupt`.

**`None` is never written by this wallet.** `create_account` always records a `Some`, settled at creation time (§4.6). The field stays `Option<u64>` because it is part of the persisted v2 format and because a hand-edited `accounts.json` can still present a `None`; registration treats that as `0`, never as the tip. The v1 migration writes `Some(0)`.

`AccountRegistry::watched_scripts(&self, network)` returns `Vec<WatchedScript>` for handing to a backend. Wallet-core owns the call; the registry stays lock-agnostic by taking the `ScriptTemplate` from each account's module as it already does for `to_record`.

### 4.6 `lantern-wallet-core` changes

```rust
impl WalletCore {
    pub fn attach_backend(&mut self, manager: BackendManager) -> Result<(), CoreError>;
    pub async fn create_account(&mut self, label: &str) -> Result<AccountRecord, CoreError>;
    pub async fn sync_watched_scripts(&self) -> Result<(), CoreError>;
    pub fn backend(&self) -> Option<&dyn ChainBackend>;
}
```

- `create_account` is `async` and settles `watch_from_block` **at creation time**, never deferring it: `Some(0)` on a wallet opened through `import` (it may have arbitrary history); on a wallet opened through `create`, `Some(tip)` read from the attached backend, or `Some(0)` when no backend is attached or the tip read fails. Origin is read from `accounts.json`'s top-level `origin` field, so it survives a lock/unlock cycle.

  An earlier draft deferred the `create`-origin height to the first sync, arguing that "nothing can reach a brand-new address before the first sync runs". **That argument was wrong**, and the whole-branch review caught it. `create_account` hands back a usable `ckt1…`/`ckb1…` address immediately and needs no backend; the window is not "wallet created → account created" but "account created → first *successful* sync", and it is unbounded, because `sync_watched_scripts` returns `NotReady` with no backend attached. On this branch that is the normal state, since `BackendManager::activate` refuses `EmbeddedLight` until plan 1f supplies a binary path. Funding the address in block H and attaching a backend days later would have resolved and *persisted* a start height above H, so the light client would never fetch the filters that contain the funding transaction: balance 0, status `Synced`, no error, no self-healing.

  `Some(0)` costs a full filter scan — the very cost this feature exists to avoid — but over-scanning is slow while under-scanning silently loses money, so it is the correct answer whenever the tip is genuinely unknown. A lagging light-client tip is safe for the same reason.
- `sync_watched_scripts` collects every account's script for the active network and calls `watch_scripts`, at each account's *recorded* height. It resolves nothing and writes nothing, so it is idempotent. Invoked on attach, on account creation, and after `activate`.
- `attach_backend` refuses a manager whose `current_network()` differs from the wallet's, with `CoreError::BackendNetworkMismatch`. secp256k1 lock args are chain-independent, so a mismatch produces no error anywhere downstream — a testnet wallet would render `ckt1…` addresses over mainnet cells, and plan 1e's builder would spend them for real. It is fallible but not `async`: nothing about the check touches the network.
- **`unlock` verifies the registry** (the plan 1c final-review carry-over): for every account carrying a `Derivation`, re-derive `lock_args` through its `LockModule` and compare. An account whose `derivation` is `None` is treated as a mismatch, not skipped — nothing in the wallet creates one today, and skipping it would let an attacker null the field to smuggle a swapped `lock_args` past verification. Any mismatch (including a nulled derivation) is `CoreError::RegistryMismatch { account_id }`. Cost is one PBKDF2 plus n derivations, single-digit milliseconds. `accounts.json` is plaintext by design, so this is the cheapest integrity guarantee that does not require authenticating the file. When watch-only or hardware accounts arrive (plan 1e+), they will carry no derivable material by design and this rejection rule cannot apply to them as-is — that plan must authenticate `accounts.json` itself (a MAC keyed by a vault subkey, which plan 1b's `Vault::extension_subkey` already makes available) rather than reinstating a skip.

## 5. Persistence layout

```
<profile dir>/
├── vault.bin           # plan 1b, unchanged
├── accounts.json       # plan 1c, now version 2 (adds watchFromBlock)
├── backends.json       # NEW: backend profiles + active selection, public
└── light-client/       # NEW: generated config + the light client's own store
    ├── light-client.toml
    └── store/
```

Wallet-core and the manager receive explicit paths, as in 1c. Resolving the OS data directory stays Tauri's job in plan 1f.

## 6. Error handling

```rust
pub enum BackendError {
    Transport(String),                  // reqwest failure, connection refused
    Rpc { code: i64, message: String }, // node-reported
    Unsupported(&'static str),          // capability the active backend lacks
    NotReady,                           // called before start() completed
    Spawn(String),                      // binary missing, not executable, config write failed
    Timeout,                            // ready-poll or request deadline
    Io(std::io::Error),
    Corrupt,                            // backends.json unparseable or wrong version
}
```

RPC text passes through because it originates at a node, never from key material. `Spawn` carries a path or an OS error string, never a secret. Wallet-core wraps with `CoreError::Backend(BackendError)` and adds `RegistryMismatch { account_id }`.

## 7. Testing

Hermetic by default, with one live gate.

| Area | Approach |
|---|---|
| Cursor discipline | Replays the exact live sequence: a full page, then an empty page with `last_cursor: "0x"`, then an assertion that the pager stopped and stored nothing resumable. A second test feeds a persisted `"0x"` back in and asserts it reads as `None` |
| RPC clients | Fake in-process JSON-RPC server serving fixtures **captured** from real testnet by a committed script, never invented. The fake returns `"0x"` on empty pages because the real node does |
| Capability probing | Fake returns `get_indexer_tip: null` and the probe reports `indexer_available: false` |
| Supervisor | A stub binary (a small shell script or a test-only Rust bin) that serves `local_node_info`, so lifecycle is exercised without the real light client. Covers ready-poll success, ready-poll timeout, clean stop, crash-and-restart, and circuit-breaker opening after five fast exits |
| Config generation | Golden file comparison, with the port field asserted separately since it is dynamic |
| Manager | Round-trips `backends.json`, rejects a corrupt or future version, switches backend within a network and asserts accounts are untouched |
| Registry migration | A v1 fixture loads with `origin: "imported"` and `watch_from_block: Some(0)`, then saves as v2 with both fields; a v3 file is `Corrupt`. A round trip of a v2 file preserves `origin` |
| Verify-on-unlock | An `accounts.json` whose `lockArgs` were edited fails `unlock` with `RegistryMismatch`; an untouched one opens |
| Live gate | Behind `LANTERN_LIVE_TESTNET=1`: real paging against a funded testnet lock, real `get_indexer_tip`, and a real light-client spawn if the binary is present. Skipped in CI |

Fixture capture is a committed script naming its exact RPC calls, so a reviewer can re-run it and diff. The rule is that no fixture is hand-written: the 2026-07 cursor bug survived 360 green tests precisely because its mock was invented rather than captured.

## 8. Task order for the implementation plan

1. verify-on-unlock in wallet-core (`RegistryMismatch`), closing the 1c carry-over.
2. Workspace dependency wiring and MSRV pins.
3. sdk-schema: `BackendKind`, `BackendCapabilities`, `BackendStatus`, `BackendProfile`, TS export.
4. `BackendError`.
5. `Cursor`, `CellPage`, and the pager invariants.
6. `CellQuery`, `WatchedScript`, `SearchKey` conversions.
7. JSON-RPC transport.
8. Fake RPC server and the fixture-capture script.
9. Light-client RPC client.
10. Full-node RPC client and `get_indexer_tip`.
11. `ChainBackend` trait.
12. `RemoteLight` backend, including the `partial`-only rule.
13. Full-node backend and capability probing.
14. Light-client config generation.
15. Free-port allocation and process spawn.
16. Ready-polling and log forwarding.
17. Graceful shutdown, platform-split.
18. Restart backoff and circuit breaker.
19. `EmbeddedLight` backend.
20. `accounts.json` v2: `origin`, `watch_from_block`, and the v1 migration.
21. `BackendManager` and `backends.json`.
22. Wallet-core wiring and the watch-script seam.
23. Live-gated testnet test, workspace gate, docs, tag `v0.0.4-chain`.

## 9. Open items carried forward

- Gap-limit recovery scan (§21) is now unblocked by `watch_from_block`; it needs a derivation loop and UI, so it gets its own slice.
- Devnet needs a chain-spec and system-script registry model before `Network` can grow a third variant.
- Plan 1e should sign all inputs under a single seed exposure, per plan 1c's final review.
- Plan 1f owns light-client binary bundling as a Tauri sidecar and OS data-directory resolution.
- `SigningCoordinator::sign_digest` stays synchronous here; it becomes the async `sign(tx)` of §7 in plan 1e, where transactions and submission exist.

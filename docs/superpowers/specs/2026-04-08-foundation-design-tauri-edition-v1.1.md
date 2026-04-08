# Lantern — CKB Desktop Wallet, Foundation (v0.1) Design, Tauri/Rust Edition (v1.1)

**Status:** Active draft (supersedes [v1.0](./2026-04-08-foundation-design-tauri-edition.md))
**Date:** 2026-04-08
**Author:** Phill + ChatGPT (v1.0 restructure) + Claude (v1.1 amendments per [review doc](./2026-04-08-foundation-design-tauri-edition-review.md))
**Working title:** **Lantern** (placeholder — see Section 14 / open question 8)
**Repo dir:** `ckb-wallet` (will move to `lantern` if/when the name sticks)

> **What's new in v1.1:** This revision applies the 14 issues from the [review doc](./2026-04-08-foundation-design-tauri-edition-review.md) plus adds 5 sections that v1.0 was missing: a named-Rust-dependencies appendix, an update channel section, a telemetry posture section, a backup/recovery section, and a network-vs-backend axis clarification. Section 15 is also rewritten to reflect the actual 8-graph research plan (not the original 4) — all 8 graphs are complete and viewable at `http://127.0.0.1:8765/`.

---

## 1. Purpose and Positioning

A commercial-quality, community-driven desktop wallet for Nervos CKB, built as a modern replacement for the Neuron Wallet's user-facing role while remaining aligned with Nervos' decentralisation ethos. The wallet is intended to become a credible community default without depending on privileged infrastructure or business-controlled choke points.

This edition restructures the original foundation plan around a **Tauri shell with a Rust-first core** rather than an Electron-first process model. The key design decision is that the wallet should be treated as a **security application with web-based user interfaces**, not as a web desktop app that happens to manage keys.

### Explicit non-goals

- Dominating the wallet space or preventing peer wallet implementations
- Replacing developer-oriented tooling such as Neuron's dev-facing role
- Bundling a full CKB node as part of the wallet product
- Making product decisions that depend on privileged wallet-owned servers
- Treating the UI layer as trusted for key management or policy enforcement

### Guiding philosophy

Decentralisation first. Run your own node. Broadcast your own transactions. The wallet must never silently centralise trust in hosted infrastructure. Complexity belongs in code and architecture, not in the user's daily operational burden.

---

## 2. Architectural Reframe

The original foundation spec was built around Electron's main/renderer/webContents model. That product shape remains useful, but the implementation assumptions change.

In the Tauri/Rust edition:

- **Rust is the wallet core**
- **Tauri is the desktop shell and secure host boundary**
- **React/TypeScript is the UI surface**
- **Web-facing surfaces are treated as constrained clients of Rust-owned capabilities**
- **Permissions are enforced in Rust, not by frontend convention**

This is the central design pivot.

---

## 3. Roadmap Overview

The broader roadmap remains intact. This document still covers **v0.1 only**.

### v0.1 — Foundation (this spec)

- Tauri desktop shell with commercial-quality polish
- Rust-first wallet core
- `ChainBackend` abstraction (embedded light client default + remote light + local full + remote full)
- Bundled `ckb-light-client-lite` (SQLite variant) as a managed subprocess
- Multi-profile encrypted vault system
- Pluggable lock registry
- Four first-party lock modules: secp256k1, Ledger, passkey (WebAuthn), ML-DSA PQC
- Watch-only account module
- Ledger support ported or reimplemented from Neuron knowledge
- Two-tier plugin model:
  - Tier 1 connected dApps (untrusted web content)
  - Tier 2 installed extensions (permission-scoped packages)
- Minimal dApp browser surface sufficient to validate provider injection and host-controlled approvals
- Typed extension API as the public contract for community extensions
- Declarative permission model with install-time approval and runtime escalation
- Extension manifest format + bundled / registry / sideload distribution
- Internal console surface for RPC and wallet introspection
- Network selection (Mainnet / Testnet / Fiber) as first-class UI
- Send, receive, transaction history, address book
- Neuron keystore import path
- Core feature parity with Neuron except full-node bundling

### v0.2 — First-party protocol extensions (future)

- Nervos DAO
- iCKB
- NFT / Spore / CKBFS viewer and manager
- Expanded dApp browser
- Marketplace and extension UX improvements

### v0.3 — Channels and bridges (future)

- Fiber
- Perun
- Rosen / Sonami bridge-related integrations
- RGB++ depending on tooling maturity

### Deferred / research-gated

- JoyID import beyond realistic watch-only or compatibility pathways
- Dedicated Nervos node OS
- Mobile / handheld builds
- Custom light-client DB variants if SQLite proves inadequate

---

## 4. Core Architectural Principles

These remain the core invariants of the project.

1. **The wallet core has no hardcoded lock types, no hardcoded chain, and no hardcoded backend.**
2. **First-party features must use the same public extension surfaces as third parties whenever feasible.**
3. **The frontend never owns secrets.**
4. **The wallet never obscures where trust and authority come from.**
5. **Decentralisation defaults are visible in the UX.**
6. **Security is enforced at capability boundaries in Rust.**
7. **The shell and web-facing layers are clients of the wallet core, not peers to it.**

---

## 5. Process and Trust Model

The wallet is a multi-component desktop application, but the trust hierarchy is clearer than the Electron version.

```text
┌────────────────────────────────────────────────────────────┐
│ Tauri Application Host                                     │
│  - Window / webview orchestration                          │
│  - Secure bridge into Rust commands/events                 │
│  - Packaging, lifecycle, updater integration               │
└───────────────┬───────────────────────────────┬────────────┘
                │                               │
                │ invoke / events               │ subprocess / RPC
                │                               │
┌───────────────▼────────────────┐   ┌─────────▼─────────────────┐
│ Rust Wallet Core               │   │ ckb-light-client-lite     │
│                                │   │ subprocess (Rust, SQLite) │
│ - Vault manager                │   └───────────────────────────┘
│ - Account registry             │
│ - Signing coordinator          │
│ - Permission enforcement       │
│ - Extension host/runtime       │
│ - Chain backend manager        │
│ - Typed API surface            │
└───────────────┬────────────────┘
                │
                │ commands / events / scoped capabilities
                │
┌───────────────▼────────────────────────────────────────────┐
│ Frontend surfaces                                          │
│ - Main wallet UI (React/TS)                                │
│ - Extension UIs (React/TS or approved web UI model)        │
│ - Connected dApp surface (sandboxed / isolated webview)    │
└────────────────────────────────────────────────────────────┘
```

### Trust hierarchy

From highest trust to lowest trust:

1. Rust wallet core
2. Bundled first-party extension modules
3. User-installed Tier 2 extensions
4. Connected dApps / arbitrary web content

This hierarchy must be explicit in the codebase and visible in the permission system.

### Responsibilities

#### Tauri host

- Owns desktop lifecycle and top-level window/webview orchestration
- Exposes only deliberate bridges into Rust
- Applies capability scoping to windows and webviews
- Never becomes the place where wallet business logic accumulates

#### Rust wallet core

- Owns the vault and unlock state
- Owns the account registry
- Owns signing coordination
- Owns chain backend selection and lifecycle
- Owns permission evaluation and enforcement
- Owns extension registration and contribution indexing
- Owns transaction-building primitives
- Owns approval-gated actions

#### Frontend surfaces

- Render state received from Rust
- Request privileged actions through typed commands
- Never directly access secrets or bypass approval policies
- Stay replaceable and constrained

### Light client subprocess supervision (new in v1.1)

The Rust wallet core **owns the lifecycle** of the bundled `ckb-light-client-lite` subprocess. The diagram in this section shows it as a sibling for the trust model (it's a separate Rust binary, not embedded code) but supervision is unambiguously the wallet core's responsibility.

The canonical `ckb-light-client` is a long-running JSON-RPC server. Verified flow from the ckb-light-client graph:

```
main() → cli::AppConfig::load() → cli::AppConfig::execute()
                                       ↓
                              subcmds::run::execute()
                                       ↓
                       Jsonrpc HTTP Server + Storage + PendingTxs
```

The wallet-core's chain-backend manager:

1. **Spawns** the binary via `tokio::process::Command` with the `run` subcommand, a generated config file (network, port, DB path, log level), and stdout/stderr captured via `Stdio::piped()`. The chosen RPC port is allocated dynamically (find a free port, write it into the config) so multiple Lantern profiles can run side-by-side.
2. **Waits for ready** by polling `127.0.0.1:<port>/` for the `local_node_info` JSON-RPC method until it responds successfully (with timeout + exponential backoff).
3. **Holds the `Child` handle** for graceful shutdown — on app exit, sends SIGTERM, waits up to 5 seconds, then SIGKILL if still alive.
4. **Forwards subprocess logs** through Lantern's `tracing` subscriber so operators can debug light-client issues without finding a separate log file.
5. **Restarts on crash** with exponential backoff (1s, 2s, 4s, 8s, capped at 30s) and a circuit breaker (5 crashes within 60s → mark backend as `Error`, surface the failure in UI, stop auto-restarting).
6. **Exposes the subprocess as a `ChainBackend` of kind `embedded-light`** — its `status()` returns `Connecting` / `Synced` / `Syncing` / `Error` / `Stopped` based on combined process state + RPC responsiveness + sync tip lag.

Implementation lives in `crates/chain-backend/src/embedded_light.rs`. Other backend kinds (`remote-light`, `local-full`, `remote-full`) are simpler — they only need an RPC client, not subprocess management.

---

## 6. Chain Backend Abstraction

The `ChainBackend` abstraction remains one of the most important ideas in the plan and should survive unchanged in spirit.

Nothing in the wallet should care whether the active chain source is:

- bundled embedded light client
- remote light client
- local full node
- remote full node

The backend manager belongs in Rust.

### Sketch

```ts
interface ChainBackend {
  id: string
  label: string
  network: "mainnet" | "testnet" | "fiber" | string
  kind: "embedded-light" | "remote-light" | "local-full" | "remote-full"
  endpoint?: string
  capabilities: BackendCapabilities
  status: "connecting" | "synced" | "syncing" | "error" | "stopped"

  getTipHeader(): Promise<Header>
  getCells(query: CellQuery, cursor?: Cursor): Promise<Paged<Cell>>
  getTransaction(hash: string): Promise<TransactionWithStatus>
  sendTransaction(tx: Transaction): Promise<string>
  subscribeToScript(script: Script): AsyncIterator<ScriptEvent>
}
```

### v0.1 backend types

- `embedded-light` — default
- `remote-light`
- `local-full`
- `remote-full`

### First-run behavior

On first launch:

1. Start embedded light client for the default network
2. Probe for a local full node
3. Offer to switch if one is found
4. Surface the backend decision in settings rather than hiding it

### Capability-aware feature gating

When a feature requires a missing capability:

- keep the feature visible
- disable it clearly
- explain why
- give a one-click path to fixing the limitation

This is still the correct UX principle.

---

## 7. Vault, Accounts, and Signing

This section becomes stronger in the Tauri/Rust edition because secret handling clearly belongs in Rust.

### Vault

- One vault per profile
- Password → **Argon2id** via the `argon2` crate (memory-hard, side-channel-resistant)
- Encryption → **XChaCha20-Poly1305** via the `chacha20poly1305` crate, specifically `chacha20poly1305::XChaCha20Poly1305`
  - **Why XChaCha20-Poly1305 over AES-256-GCM** (changed from v1.0): XChaCha20-Poly1305 uses a 192-bit nonce, eliminating the birthday-bound nonce-reuse risk that AES-GCM (96-bit nonce) requires careful counter or RNG management to avoid. The graph-routing keystore-signing graph confirms both ciphers ship in the same `chacha20poly1305` crate (`raw/AEADs/chacha20poly1305/src/lib.rs`). Modern Rust vault libraries (notably `age`) use ChaCha20-Poly1305 family for the same reason. Vault encryption is not perf-critical, so the marginal AES-NI throughput advantage doesn't matter.
- **Nonce strategy**: fresh random 192-bit nonce per vault encryption, prepended to the ciphertext envelope. Never derived deterministically. Generated via `rand::rngs::OsRng`.
- **Versioned file format**: every encrypted vault file starts with a magic header + 1-byte format version. Format upgrades are explicit migrations, not in-place rewrites.
- **Unlock state held only in Rust memory** — see Memory Hygiene below
- Extensions can store encrypted blobs through namespaced vault APIs (each extension gets its own subkey derived from the vault master via `hkdf`)
- No plaintext secret material crosses into frontend memory

### Memory hygiene (new in v1.1)

"Held in Rust memory" is necessary but not sufficient for unlocked secret material. The vault module must enforce all of:

- **Zeroize on drop.** All sensitive in-memory types (unlocked private keys, mnemonic seed bytes, vault password, derived subkeys) implement `Zeroize + ZeroizeOnDrop` from the `zeroize` crate. `Drop` impls explicitly wipe memory before deallocation.
- **Wrapped in `secrecy::SecretBox<T>`** (not `Secret<T>` — the secrecy crate API was renamed). This prevents accidental `Debug`/`Display`/`serde` exposure and forces explicit `expose_secret()` / `expose_secret_mut()` calls at access points. Verified against `raw/iqlusioninc-crates/secrecy/src/lib.rs` in the keystore-signing graph: the modern types are `SecretBox<T>`, `SecretString`, `SecretSlice<S>`, with the `ExposeSecret` / `ExposeSecretMut` traits.
- **No `Debug` or `Display` impls** on secret-bearing types. `secrecy::SecretBox` enforces this at the type level — its `Debug` impl prints `SecretBox<...>` opaquely.
- **Memory locking via `mlock(2)`** where the OS permits. Pages holding unlocked vault state get `mlock`ed at decrypt time and `munlock`ed on lock/exit. Use the `region::lock` crate or call `libc::mlock` directly. On macOS this is best-effort (kernel may still page); on Linux the `RLIMIT_MEMLOCK` ulimit may need to be raised in the installer.
- **No serialization to disk** of the unlocked form. The vault file always holds the encrypted form; the unlocked form lives only in memory.

### Account registry

Rust owns the canonical account registry. Frontends only receive non-sensitive account records.

The canonical definition is a Rust struct in `crates/account-registry`. The TypeScript shape below is illustrative of what the generated SDK will surface to extension authors and the frontend (via `tauri-specta`):

```ts
// generated TS — canonical definition is `AccountRecord` in crates/account-registry/src/lib.rs
interface AccountRecord {
  id: string
  label: string
  lockType: string
  extensionId: string
  address: string
  publicMetadata: Record<string, unknown>
  capabilities: AccountCapabilities
}
```

```rust
// canonical Rust definition
#[derive(Serialize, Deserialize, Type)]  // Type derive from `specta`
pub struct AccountRecord {
    pub id: String,
    pub label: String,
    pub lock_type: String,
    pub extension_id: String,
    pub address: String,
    pub public_metadata: serde_json::Value,
    pub capabilities: AccountCapabilities,
}
```

`AccountRecord` deliberately holds **no secret material** — it is the public projection of an account, safe to send across the IPC boundary to any frontend or extension that has been granted `accounts.read` capability.

### Signing coordinator

There should be exactly one signing entry point in Rust. Canonical definition is an `async fn` on the `SigningCoordinator` struct in `crates/wallet-core`:

```rust
// canonical Rust definition
impl SigningCoordinator {
    pub async fn sign(
        &self,
        account_id: AccountId,
        tx: TransactionTemplate,
        ctx: SigningContext,
    ) -> Result<SignedTransaction, SigningError> { /* ... */ }
}
```

```ts
// generated TS surface (illustrative)
function sign(accountId: string, tx: TransactionTemplate, ctx: SigningContext): Promise<SignedTransaction>
```

It resolves:

- owning lock module or extension
- unlock policy
- required approval flow
- witness construction
- backend submission if approved

### First-party lock modules

- secp256k1
- Ledger
- passkey
- ML-DSA
- watch-only

Each lock type owns:

- account creation flow
- public metadata model
- signing logic or signing dispatch
- witness size data
- fee-estimation contribution

### Unlock model

Keep the hybrid model from the original plan:

- session unlock for reads
- per-signature re-authentication for writes by default
- explicit user opt-in for looser session signing where applicable

This remains one of the best parts of the original design.

---

## 8. Extension Model (Tauri/Rust Edition)

The two-tier model remains correct. The implementation wording changes.

### Tier 1 — Connected dApps

These are untrusted or minimally trusted web experiences.

Properties:

- user visits a URL or approved web source
- content is hosted in a constrained webview or equivalent isolated surface
- communication occurs only through a narrow provider bridge
- all signing and sensitive prompts are host-rendered and host-controlled
- dApp content cannot style or intercept approval UI
- no secret material or privileged capabilities are directly exposed

Tier 1 should expose only a compact provider API:

- request accounts
- sign transaction
- sign message
- get chain id / network
- subscribe to account or network changes

This is the wallet's analogue to common browser wallet connected-site behavior.

#### Multi-webview fallback (new in v1.1, addresses review issue #4)

Lantern's preferred dApp browser implementation is **multiple `WebviewWindow` instances inside one Tauri window** — each connected dApp runs in its own webview, all hosted by the same shell window with a tab UI. Tauri 2 supports this via its multi-webview API.

**However**, Tauri 2's multi-webview-per-window features are flagged as "evolving" in the official docs (verified in the tauri-ipc graph). If multi-webview-per-window proves unstable or has missing features by Lantern's v0.1 implementation window, Lantern falls back to a **one-window-per-dApp-tab model**: each connected dApp opens in its own top-level Tauri window. Same trust boundaries apply (same provider bridge, same host-controlled approval UI, same capability isolation), the UX is just slightly clunkier — users see a separate OS window per dApp instead of tabs in a single window.

This fallback is **not** a future migration risk: it's a v0.1 feature flag. The chain-backend, signing coordinator, permission gate, and provider bridge are all identical between the two modes. Only the window management code differs, and it's localized to the dApp browser surface in `apps/desktop/src-tauri/src/dapp_browser.rs`. Decision deferred until early v0.1 implementation when we can probe Tauri 2's actual stability.

### Tier 2 — Installed extensions

These are explicitly installed packages with declared contributions and permissions.

Properties:

- installed through bundled, registry, alternate registry, or sideload pathways
- contribute pages, navigation, lock types, tx builders, settings panels, console commands, and background logic
- interact with the wallet through a typed API owned by Rust
- receive only the capabilities granted by manifest and user approval
- cannot suppress host-controlled signature approval rules

### Important rule

Tier 2 extensions are still not trusted as core code simply because they are installed. They are **permissioned clients of the Rust wallet core**.

---

## 9. Extension API

The Extension API remains the long-term public contract, but it should be reframed as a Rust-owned API surface with generated or hand-maintained TypeScript bindings.

### Design goals

- typed from day one
- versioned
- capability-gated
- documented like a product, not an internal implementation detail
- dogfooded by first-party modules

### Recommended shape

- Rust defines commands, events, schemas, permission requirements, and contribution models
- TypeScript SDK exposes ergonomic wrappers for extension authors
- Frontend code never enforces security policy by itself
- Every privileged call crosses a Rust-owned permission gate

### Schema generation pipeline (new in v1.1, addresses review issue #2)

Lantern uses **`specta` + `tauri-specta`** as the Rust → TypeScript schema generation pipeline. This is the canonical pick, not "TBD".

- **`specta`** — derive macro on Rust types: `#[derive(Type)]` produces a compile-time schema
- **`tauri-specta::Builder<R>`** — collects all `#[tauri::command]` functions and `Event` types, emits TypeScript bindings
- **`specta_typescript::Typescript`** or **`specta_typescript::JSDoc`** — output formatters

The generated TS file (verified shape from `tauri-specta/examples/app/src/bindings.ts` in the tauri-ipc graph) is what extension authors and the frontend import. **One canonical Rust definition. Generated TS bindings. No hand-maintained type duplication.**

```rust
// in crates/sdk-schema or per-plugin lib.rs
use specta::Type;
use tauri_specta::{Builder, collect_commands, collect_events};

#[derive(Type, serde::Serialize, serde::Deserialize)]
pub struct AccountRecord { /* ... */ }

#[tauri::command]
#[specta::specta]
pub async fn list_accounts(filter: AccountFilter) -> Result<Vec<AccountRecord>, ApiError> { /* ... */ }

// in build.rs or main.rs
let builder = Builder::<tauri::Wry>::new()
    .commands(collect_commands![list_accounts, /* ... */])
    .events(collect_events![AccountChangedEvent]);
builder.export(specta_typescript::Typescript::default(), "../packages/extension-sdk/src/bindings.ts")?;
```

The generated `bindings.ts` provides typed `commands.listAccounts(filter)` and `events.accountChangedEvent.listen(cb)` wrappers that call `tauri::invoke` and `tauri::event::listen` under the hood with full type safety.

### tRPC replacement — responsibility map (new in v1.1, addresses review issue #3)

The original Electron spec used `tRPC` for both transport AND a stack of secondary capabilities (input/output validation, end-to-end type safety, TanStack Query integration). Tauri commands cover transport, but the rest needs explicit replacement:

| Responsibility | Old (tRPC over Electron IPC) | New (Tauri/Rust + specta) |
|---|---|---|
| Transport | `tRPC` over `ipcRenderer.invoke` | `tauri::command` + `tauri::Event` |
| Schema definition | TypeScript-first via Zod | **Rust-first via `specta::Type` derive** |
| TS type generation | tRPC inferred client types | **`tauri-specta` exports `bindings.ts`** |
| Runtime input validation (frontend) | Zod schemas | **Generated TS types** for compile-time, optional `zod` shim at the boundary if extra runtime checks are needed |
| Runtime input validation (Rust) | n/a (TS only) | `serde` deserialization rejection (any deserialize failure = HTTP 4xx equivalent) + explicit `validator` crate where business rules need to run |
| Permission gate | tRPC middleware | **`tauri::command` body checks the calling extension's capabilities** before executing — capability resolution via `tauri::AppHandle::try_state::<PermissionContext>()` |
| Query/cache integration | `@tanstack/react-query` + `@trpc/react-query` | **`@tanstack/react-query` + thin wrappers** around the generated `commands.*` invokers (`useQuery({queryKey: ['accounts'], queryFn: () => commands.listAccounts(filter)})`) |
| Subscription / live updates | tRPC subscriptions | **`tauri::Event` + `events.*.listen()`** with TanStack Query `setQueryData` from the listener |
| Error model | tRPC error formatter | Rust `Result<T, ApiError>` where `ApiError` is `#[derive(Type, Serialize)]` — generated TS gets a typed error union |

The boundary moves from "TS shapes everything, Rust is downstream" to "Rust shapes everything, TS is generated". The extension SDK author writes Rust traits and Rust commands; the TS bindings appear in `packages/extension-sdk/src/bindings.ts` as a build artifact.

### API categories

- `chain.*`
- `accounts.*`
- `vault.*`
- `registry.*`
- `ui.prompt.*`
- `ui.notify.*`
- `console.*`
- `settings.*`
- `network.*`
- `permissions.*`

### Versioning

- semantic versioning for the extension SDK
- explicit API compatibility declaration in extension manifests
- majors for breaking changes only
- deprecation policy documented early

---

## 10. Permission Model

The permission model from the original draft is strong and should remain largely intact.

### Requirements

- declarative in manifests
- reviewed at install time
- optionally escalated at runtime
- enforced at every privileged entry point
- non-bypassable for signatures

### Install-time approval

Users should see:

- what the extension can read
- what it can request
- whether it can send transactions
- whether it needs access to signing flows
- whether it requires a certain backend capability
- where it came from

### Runtime escalation

An extension may request optional permissions later, but Rust owns the approval state and enforcement.

### Non-waivable rule

Per-signature approval remains mandatory unless the user has explicitly opted into a looser session mode for applicable software keys. Ledger and passkey flows remain per-approval by nature.

---

## 11. Extension Manifest and Distribution

This area stays mostly the same conceptually.

### Manifest should declare

- id
- name
- version
- API compatibility
- author
- entrypoint or package metadata
- contributions
- required permissions
- optional permissions
- required backend capabilities
- icon and metadata

### Distribution channels

1. Bundled first-party modules
2. Default community registry
3. User-added alternate registries
4. Sideloaded packages or folders

### Key principle

Registry control must never become a centralisation choke point. Alternate registries and sideloading are not optional extras. They are part of the decentralisation posture.

---

## 12. Stack and Repository Shape

### Recommended stack

- **Tauri 2**
- **Tauri 2** — desktop shell, capability/permission system, packaging
- **Rust** — wallet core, permission enforcement, vault, backend manager, extension host
- **React 19** + **TypeScript 5+** — frontend UI surfaces (main wallet, extension UIs, dApp approval modals)
- **Vite** — frontend bundler
- **TanStack Query** — server state caching, paired with thin invokers around generated `tauri-specta` bindings
- **TanStack Router** (new in v1.1, addresses review issue #11) — type-safe routing for the multi-surface app (settings / send / receive / history / address book / network selection / dApp browser / extension UIs). File-based routes; integrates natively with TanStack Query.
- **Zustand** — local UI state (transient, not server-shaped)
- **`tauri::process::Command`** (via `tokio`) — `ckb-light-client-lite` subprocess management
- **`specta` + `tauri-specta`** — Rust → TypeScript binding generation (see Section 9)

### Repository shape

```text
ckb-wallet/                           # repo dir, will move to lantern/ if name sticks
├── apps/
│   └── desktop/                      # Tauri 2 app shell + frontend
│       ├── src/                      # React + TS frontend (TanStack Router file routes)
│       ├── src-tauri/                # Tauri shell entry (main.rs, tauri.conf.json, capabilities/)
│       └── package.json
├── crates/                           # Rust workspace — IMPLEMENTATION
│   ├── wallet-core/                  # core orchestration, supervises light-client subprocess
│   ├── vault/                        # encrypted storage (XChaCha20-Poly1305 + Argon2id + zeroize + secrecy)
│   ├── chain-backend/                # ChainBackend trait + 4 backend impls (embedded-light, remote-light, local-full, remote-full)
│   ├── extension-host/               # extension loading, manifest validation, permission evaluation
│   ├── tx-builder/                   # generic tx building, witness-size-aware fee estimation
│   ├── account-registry/             # AccountRecord storage and dispatch
│   ├── signer-secp256k1/             # secp256k1_blake160 signing implementation
│   ├── signer-ledger/                # Ledger HID transport + APDU + Nervos-app protocol
│   ├── signer-passkey/               # WebAuthn / FIDO2 / CTAP2 + libfido2
│   ├── signer-mldsa/                 # ML-DSA-44/65/87 + Falcon-512/1024 (post-quantum)
│   └── sdk-schema/                   # shared types — single source for tauri-specta to export
├── packages/                         # frontend npm workspace
│   ├── extension-sdk/                # TS SDK for extension authors (generated bindings.ts + ergonomic wrappers)
│   ├── ui/                           # shared React components, design system
│   └── tsconfig/                     # shared tsconfig presets
├── core-extensions/                  # MANIFEST + UI + DOGFOODED SDK CONSUMER for first-party features
│   ├── secp256k1/                    #   manifest.toml + ui/ + (statically links signer-secp256k1)
│   ├── ledger/                       #   manifest.toml + ui/ + (statically links signer-ledger)
│   ├── passkey/                      #   manifest.toml + ui/ + (statically links signer-passkey)
│   ├── mldsa/                        #   manifest.toml + ui/ + (statically links signer-mldsa)
│   └── watch-only/                   #   manifest.toml + ui/ (no signer crate — read-only)
├── examples/
│   └── example-extension/            # third-party extension template
├── tests/                            # (new in v1.1) integration + e2e
│   ├── integration/                  #   cross-crate Rust integration tests
│   ├── e2e/                          #   Tauri shell driven via tauri-driver / WebDriver
│   └── fixtures/
├── .github/
│   └── workflows/                    # (new in v1.1) CI matrix
│       ├── ci.yml                    #   macOS, Windows, Linux glibc, Linux musl × stable Rust
│       ├── release.yml               #   signed releases, deterministic builds
│       └── security-audit.yml        #   cargo-audit + cargo-deny
└── docs/
    └── specs/
```

### Crate vs core-extension relationship (new in v1.1, addresses review issue #5)

The repo shape above lists BOTH `crates/signer-mldsa` AND `core-extensions/mldsa`. They're not duplicates — they're two halves of the same first-party extension, designed so that **first-party signers dogfood the same extension API as third-party extensions** (Section 4 principle #2).

| Layer | Role | Example |
|---|---|---|
| `crates/signer-mldsa` | **Implementation** — Rust crate, statically linked into the wallet binary. Implements the `LockModule` trait. Owns the actual signing logic (witness layout, FIPS 204 verification, key derivation). Has no knowledge of UI or manifest concerns. | Pure Rust crate. Compiles once at build time. |
| `core-extensions/mldsa/` | **Manifest + UI surface + dogfooded SDK consumer.** Contains: `manifest.toml` (declarations: id, version, contributions, required permissions, target API version), `ui/` (TS/React frontend surfaces — account creation flow, signing approval modal), and a thin Rust `lib.rs` that registers `signer-mldsa` with the extension-host. The host treats this directory exactly like a third-party extension package — it loads the manifest, validates permissions, dispatches calls. | Manifest + UI + (pointer to crate). |
| `crates/extension-host` | **Loader / dispatcher.** Reads `manifest.toml`, validates against the current SDK version, evaluates permission grants, exposes the extension's contributions through the `tauri::command` boundary. Does not care whether the underlying signer is statically linked (first-party) or sideloaded (third-party). | Wallet core. |

**Why this split:** when a new lock type is added later by a community contributor, the only thing they need to do is write a `signer-X` crate (or, for sandboxed cases, a WASM module) and a `core-extensions/X/` package. Same shape as our first-party signers. The host doesn't have a privileged code path for built-in locks — they all flow through the same extension API.

This is the same pattern MetaMask uses internally for first-party snaps (verified against `metamask-snaps/packages/snaps-utils/` in the extension-prior-art graph: `getSnapManifest()`, `isSnapManifest()`, `validateSnapManifestLocalizations()` — manifest is loaded by the host runtime regardless of whether the snap ships in MetaMask's bundle or is installed by the user). VS Code and Obsidian use a similar split (extension manifest separate from extension code).

---

## 13. Security Posture

This section should become sharper than in the Electron draft.

### Core rules

- secrets stay in Rust
- unlock state stays in Rust
- signing stays in Rust or secure delegated device flows
- permissions are checked in Rust
- all approval UI is host-controlled
- untrusted web content never receives privileged APIs directly

### Passkeys

Passkey support remains desirable but should be treated as a dedicated research and implementation track because platform behavior, RP identity, and upgrade stability all matter.

**Linux passkey caveat (new in v1.1):** macOS has Touch ID + iCloud Keychain, Windows has Windows Hello, but **Linux passkey is fragmented** as of 2026 — no platform-default biometric pathway. Lantern's v0.1 baseline is **hardware FIDO2 keys via `libfido2` (or the `ctap-hid-fido2` Rust crate) on all three platforms**, with platform biometric paths (Touch ID, Windows Hello) added where available. Software-only passkey on Linux is **out of scope for v0.1**. Users on Linux who want a passkey-style experience plug in a YubiKey, Nitrokey, or other FIDO2 token. The graph-routing keystore-signing graph shows the relevant Rust code in `webauthn-rs/webauthn-authenticator-rs/src/ctap2/` (CTAP2 commands including `get_assertion`, `make_credential`, `bio_enrollment`).

### Ledger

Ledger support should preserve the device-confirmed model. The host must never be able to claim a signature approval the device did not actually produce. Implementation lifts directly from `ledger-app-nervos` (the LedgerHQ Nervos app C source, captured in the keystore-signing graph) for the APDU protocol shape, and from the `ledger-rs` crate stack (`ledger-apdu`, `ledger-transport`, `ledger-transport-hid`) for the host-side transport.

### ML-DSA

ML-DSA support continues to justify generic witness-size-aware transaction building. This is one of the strongest reasons not to inherit assumptions from older wallet code — and the size pressure is concrete, not abstract.

**Verified signature sizes** (from `pqcrypto-mldsa/src/ffi.rs` and `pqcrypto-falcon/src/ffi.rs` — same across CLEAN/AVX2/AArch64 backends, these are spec values per FIPS 204 / Falcon):

| Lock | Sig bytes | vs secp256k1 (65 b) | Notes |
|---|---:|---:|---|
| secp256k1 | 65 | 1× | baseline |
| ML-DSA-44 | **2,420** | 37× | NIST Level 2 |
| ML-DSA-65 | **3,309** | 51× | NIST Level 3 (Phill's primary variant) |
| ML-DSA-87 | **4,627** | 71× | NIST Level 5 |
| Falcon-512 (variable) | up to 752 | up to 12× | variable length, harder to fee-estimate |
| Falcon-padded-512 | 666 | 10× | fixed length, easier to fee-estimate |
| Falcon-1024 (variable) | up to 1,462 | up to 22× | |
| Falcon-padded-1024 | 1,280 | 20× | |

**Implications for the wallet's tx-builder:**

1. **Per-lock witness size hints are mandatory**, not optional — the `LockModule` trait must expose `witness_size_hint(&self, ctx: &SigningContext) -> WitnessSize`, where `WitnessSize` is one of `Fixed(usize)` or `Variable { min: usize, max: usize }`.
2. **Cell capacity must cover the witness.** A cell locked with ML-DSA-65 needs ~3.3KB more cell capacity reserved for its witness compared to a secp256k1 cell. The tx-builder refuses to construct outputs whose owning lock's witness wouldn't fit in the output's capacity.
3. **CKB max-tx-size is 512KB.** Multi-input ML-DSA transactions hit this limit faster than secp256k1 transactions. The tx-builder enforces a pre-broadcast check.
4. **Fee estimation must include the actual witness bytes**, not a secp256k1 baseline. Variable-length Falcon witnesses are estimated at the `max` value to avoid undercharging.
5. **Padded variants are preferred** over variable-length variants in v0.1 because fee predictability matters more for UX than the small bandwidth saving.

The graph-routing **ckb-wallet-locks graph** also captures Phill's `ckb-mldsa-lock` repo design notes (`raw/ckb-mldsa-lock/docs/falcon-investigation-2026-04-08.md`) and the per-variant lock_args layouts.

### dApp containment

Tier 1 dApps must remain constrained to a narrow API and isolated rendering environment. Their host relationship should feel closer to a browser wallet permission model than to plugin-level trust.

### Extension containment

Tier 2 extensions must be namespaced in storage and permissioned in access. They should never be able to freely inspect one another's data or silently gain account-level power.

---

## 14. Tauri-Specific Open Questions

These replace several Electron-specific open questions from the original draft.

1. Exact Tauri 2 feature set and stable multi-webview/window design to target
2. Best extension runtime model: host-managed extension bundles, separate web assets, or hybrid package format
3. Passkey integration details across macOS, Windows, and Linux
4. Whether Tier 1 connected dApps should use an internal isolated webview model, external browser mediation, or both
5. Best Rust-to-TS schema generation approach for the extension SDK
6. Exact Neuron keystore import compatibility range
7. Fee estimation contribution interface for variable witness sizes
8. Final naming

---

## 15. Research Prerequisites — 8 Knowledge Graphs (rewritten in v1.1, addresses review issue #13)

v1.0 specified 4 research graphs. The actual research plan grew to 8 during the foundation phase, all complete as of 2026-04-08. They live as graphify-generated knowledge graphs in `/home/phill/ckb-wallet/research/<name>/graphify-out/` and are viewable at **`http://127.0.0.1:8765/`** under the "ckb-wallet research" section.

| # | Graph | Nodes / Edges / Communities | Domain tier | What it covers |
|---|---|---|---|---|
| 1 | **neuron-port** | 1,849 / 2,931 / 208 | task | Neuron Electron wallet backend (UI excluded) — pattern reference for the Rust port: tx construction, signing coordinator, IPC handler taxonomy, exception model, hardware signer abstraction, sync engine split (full vs light) |
| 2 | **ckb-ecosystem-locks** | 2,308 / 4,161 / 203 | task | secp256k1 system scripts, omnilock, pw-lock, joyid, anyone-can-pay, ckb-mldsa-lock (toastmanAu), 27 RFCs (0017, 0019, 0021, 0022, 0023, 0024, 0026, 0042, 0044, 0052, etc.), 24 RFC technical diagrams via vision extraction. The complete on-chain lock landscape Lantern must support. |
| 3 | **fiber-payment-channels** | 9,785 / 19,936 / 212 | task | Full Fiber Network Rust codebase — channel actor, TLC state machine, RPC API surface, gossip protocol, biscuit auth, watchtower, CCH cross-chain hub. Plus Phill's wyltek-fiber-client research docs. **Reusable for FiberQuest and any future state-channel work.** Captures the open problem of post-quantum payment channels (Fiber uses MuSig2 for 2-of-2 funding which doesn't translate to ML-DSA). |
| 4 | **extension-platform-prior-art** | 10,673 / 15,843 / 384 | domain | tauri + tauri-specta + MetaMask snaps (sdk + controllers) + VS Code samples + obsidian-api. The pattern catalog for our Tier 1/Tier 2 model. |
| 5 | **ckb-light-client** | 1,089 / 1,471 / 146 | task | Canonical ckb-light-client Rust source (`light-client-bin`, `light-client-lib`, WASM bindings), RFC 0044, the STORAGE_REFACTORING design notes, plus Phill's `ckb-light-client-lite` SQLite findings. Used to ground the Section 5 subprocess supervision contract. |
| 6 | **tauri-ipc-and-permissions** | 8,104 / 11,367 / 369 | domain | Tauri 2 IPC layer + ACL types (Permission, PermissionSet, Capability, Scope) + tauri-specta core + capability scoping per window/webview + tauri-docs security pages. The exact machinery Lantern's extension boundary sits on top of. |
| 7 | **keystore-signing** | 3,564 / 5,296 / 189 | domain | Ledger app-nervos + ledger-rs transport stack + webauthn-rs (passkey) + pqcrypto-mldsa + pqcrypto-falcon + iqlusioninc bip32 + secrecy + signatory + age + chacha20poly1305 + aes-gcm. **Reusable across ckb-esp32-signer, FiberQuest passkey support, and any future hardware wallet work.** |
| 8 | **ckb-tx-construction** | 5,820 / 9,569 / 282 | domain | ckb-sdk-rust (THE Rust SDK Lantern's tx-builder will mirror) + ccc + lumos common-scripts. Witness construction, fee/capacity balancing, address encoding, cell_dep resolution. |

**Total:** ~43,200 nodes, ~70,500 edges, ~1,993 communities across 8 graphs.

**Cross-project reuse:** Graphs 4, 6, 7, 8 are tagged `domain` tier in `~/.claude/graphs.json` — they'll be picked up by `graph-routing` for any future work in those domains, not just Lantern.

**Routing infrastructure:** the `graph-routing` skill can query all 8 via `~/.claude/skills/graph-routing/scripts/graph-call.sh` (which now has both HTTP and file adapters as of v1.1 work). Trajectory logs of graph-routing usage are appended to `~/.claude/shared/trajectories/graph-routing/<date>.jsonl` for future model fine-tuning.

---

## 16. Non-scope Reminders

Still not part of v0.1:

- Nervos DAO / iCKB / NFT / Spore / CKBFS / RGB++
- Fiber / Perun / Rosen integrations
- advanced dApp browser features
- mobile and handheld ports
- custom light-client DB variants
- full-node bundling

---

## 17. Implementation Sequencing

After approval of this Tauri/Rust foundation spec:

1. Build the four research graphs
2. Resolve Tauri-specific open questions
3. Write a detailed implementation plan
4. Scaffold the repository around Rust core crates and the Tauri desktop shell
5. Build in the order:
   - vault
   - chain backend manager
   - embedded light-client supervision
   - account registry
   - signing coordinator
   - extension API schemas
   - extension host
   - shell UI
   - first-party lock modules
   - Neuron import path
   - polish
6. Review after each milestone

---

## 18. Network and Backend Axes (new in v1.1)

Section 6 (ChainBackend) and Section 12 (network selection feature) describe two related but **orthogonal** axes that need to be modeled separately:

| Axis | Question | v0.1 values |
|---|---|---|
| **Network** | Which CKB chain? | `mainnet`, `testnet`, `devnet` (each has its own genesis, address prefix, system script registry) |
| **Backend** | How do we reach that chain? | `embedded-light`, `remote-light`, `local-full`, `remote-full` |

A **profile** is the cartesian product: `(network × backend × name)`. For example, a user might have:
- `default-mainnet` = (mainnet, embedded-light, "Default")
- `pi-node` = (mainnet, remote-full, "Pi Node @192.168.1.10")
- `testnet-dev` = (testnet, remote-light, "Testnet via my server")

**Accounts are scoped to a network**, not a backend. An address generated on testnet is invalid on mainnet (different prefix) — the address generation flow embeds the network into the address. The vault stores accounts grouped by network so testnet accounts never accidentally appear in a mainnet view.

**Backends are swappable within a network** without re-deriving accounts. Switching from `embedded-light` to `local-full` on mainnet keeps the same accounts visible, just changes the data source.

The chain-backend manager exposes `current_network()` and `current_backend()` separately. UI surfaces them as separate selectors in settings, with sensible defaults (mainnet + embedded-light on first run).

---

## 19. Update Channel (new in v1.1)

Lantern uses **Tauri's built-in updater** with these constraints:

- **Update manifests are signed** with a project-controlled Ed25519 key. Public key is compiled into the binary at release time. Update bodies are verified before any disk write.
- **Off by default for major versions.** Minor and patch updates can be opt-in auto-applied (still with verification). Major updates always prompt.
- **Manual approval prompt by default** for v0.1 — users see a dialog with version, changelog summary, and signature verification status before updating.
- **Update server is project-controlled and CDN-fronted** (e.g. Cloudflare R2 + a small static manifest endpoint). No third-party update infrastructure.
- **Rollback capability**: previous version's binary is kept on disk for one update cycle so users can roll back if a release is broken. Vault format migrations are explicitly versioned and have downgrade guards.
- **Update channel selection**: stable / beta / nightly. Default is stable. Channel choice is per-installation, not global.

**Not in v0.1**: differential updates, P2P update distribution, mandatory updates.

---

## 20. Telemetry Posture (new in v1.1)

**Off by default. Opt-in only. No silent collection. Ever.**

Lantern is decentralisation-first (Section 1 guiding philosophy). Telemetry that ships data off-device without explicit consent contradicts that posture. Concrete rules:

- **No network calls on first run** other than the user-facing chain backend (for embedded-light, that's only the CKB p2p network it joins via the subprocess). No analytics ping. No "phone home".
- **Crash reports**: if enabled (opt-in via settings, default OFF), crash dumps are captured locally only and the user must explicitly export and send them. Lantern does NOT operate a crash collection backend.
- **Usage metrics**: not collected. Period.
- **Update check**: the only outbound call Lantern makes by default is the signed update manifest fetch. Update check frequency is configurable (daily / weekly / manual / never).
- **Anonymous network telemetry inside the light client subprocess** (peer counts, sync lag) is captured locally for diagnostics — exposed via the wallet UI to the user, never uploaded.
- **Bug reports** via project issue tracker are explicit user-initiated actions. Lantern provides a "copy diagnostic bundle" button that creates a sanitized JSON for the user to paste — no automatic upload.

If a future Lantern version proposes any opt-in telemetry, it must be:
- Documented in the changelog and the privacy section
- Disabled by default
- Per-feature opt-in (no "enable all telemetry" megaswitch)
- Disclosed precisely (which fields, which destination, retention policy)

---

## 21. Backup and Recovery (new in v1.1)

### Seed phrase backup

- On wallet creation: BIP39 mnemonic (12 or 24 words, user choice — default 24) is shown **once** with explicit user confirmation steps (typed re-entry of selected words) before the wallet is unlocked.
- **Re-display gated behind password re-entry** in settings. Vault password unlock alone is not enough — the user must re-enter the password specifically to view the seed.
- Never logged. Never serialized to disk in plaintext. Never copied to clipboard automatically (manual copy only, with a 30-second clipboard auto-clear).

### Vault file backup

- Vault file format is documented (magic header, version byte, encrypted blob with prepended nonce, no separate key file). Users can back up the vault file directly via OS file copy.
- Vault file is portable across Lantern installations on different machines: copy `~/.config/lantern/vault-<profile>.bin` to another machine, install Lantern, point it at the file, enter the password.
- Vault file format is version-stable within a major version. Migrations from older formats are explicit.

### Recovery from seed

- A seed phrase regenerates the same address set on any Lantern installation. The HD derivation path is BIP44-compatible for secp256k1 accounts (`m/44'/309'/0'/0/i` for CKB) and follows the lock module's documented derivation for ML-DSA / Falcon / passkey accounts.
- Recovery into an empty wallet repopulates accounts up to the first address with no on-chain history (gap limit of 20).
- **ML-DSA and Falcon accounts are NOT recoverable from a BIP39 seed alone** — their key generation is randomized and the vault stores the actual private key. PQ accounts must be backed up via vault file copy. This is documented prominently in the account creation flow.

### Hardware key recovery

- Ledger accounts are not stored in the vault — only the public key and derivation path are. Recovery is "plug in the same Ledger with the same seed and re-derive". The vault contains zero secret material for hardware accounts.
- Passkey / FIDO2 key accounts: same model. Public credential ID and metadata stored, private key stays on the authenticator. Recovery is "plug in the same authenticator".

---

## 22. Named Rust Dependencies (new in v1.1, leading candidates)

This appendix names specific crates Lantern is **expected** to use, to close most of v1.0's open questions cheaply. Treat as leading candidates — locked in unless implementation reveals a blocker.

### Tauri shell + frontend bridge

| Crate | Purpose |
|---|---|
| `tauri = "2"` | Desktop shell, capability/permission system, packaging |
| `tauri-build` | Build-time shell setup |
| `specta` | Rust type schema derive |
| `tauri-specta` | Rust → TypeScript binding generation, command + event collector |
| `specta-typescript` | TypeScript output formatter for specta |

### Async + error + logging

| Crate | Purpose |
|---|---|
| `tokio` (full features) | Async runtime, process management, channels |
| `async-trait` | Async fns in `ChainBackend` and `LockModule` traits |
| `anyhow` | Error handling at the wallet-core boundary only |
| `thiserror` | Per-crate typed errors |
| `tracing` | Structured logging across all crates |
| `tracing-subscriber` | Log subscriber, also receives subprocess stdout/stderr |

### Vault crypto + memory hygiene

| Crate | Purpose |
|---|---|
| `argon2` | Argon2id password KDF |
| `chacha20poly1305` | XChaCha20-Poly1305 AEAD for vault encryption |
| `rand` | OS RNG via `rand::rngs::OsRng` for nonces and key generation |
| `zeroize` | `Zeroize + ZeroizeOnDrop` on sensitive types |
| `secrecy` | `SecretBox<T>`, `SecretString`, `ExposeSecret` trait — opaque secret wrapping |
| `region` (or `libc::mlock`) | Memory page locking for unlocked vault state |
| `hkdf` | Per-extension subkey derivation from vault master |

### HD wallets, mnemonics, signing

| Crate | Purpose |
|---|---|
| `bip32` (iqlusioninc) | BIP32 HD derivation — verified in keystore graph |
| `bip39` | Mnemonic seed phrases |
| `signature` | Generic signature trait |
| `signatory` (iqlusioninc) | Signature trait abstraction over secp256k1 / ed25519 / nistp256 (alternative to using each crate directly) |

### Hardware wallet stack (signer-ledger)

| Crate | Purpose |
|---|---|
| `ledger-transport-hid` | Ledger HID transport (host side) |
| `ledger-apdu` | APDU command construction |
| `ledger-zondax-generic` | Generic Ledger app helpers |
| (custom) | Nervos-app protocol layer — port from `ledger-app-nervos` C source captured in keystore graph |

### Passkey / FIDO2 (signer-passkey)

| Crate | Purpose |
|---|---|
| `webauthn-authenticator-rs` | CTAP2 client for hardware FIDO2 keys (the host side, not the RP side) |
| `ctap-hid-fido2` | Lower-level CTAP-HID transport (alternative to webauthn-authenticator-rs) |
| `libfido2-sys` (or system `libfido2`) | Native libfido2 bindings as a fallback transport on Linux |

### Post-quantum (signer-mldsa)

| Crate | Purpose |
|---|---|
| `pqcrypto-mldsa` | ML-DSA-44 / 65 / 87 — verified signature byte sizes 2420 / 3309 / 4627 |
| `pqcrypto-falcon` | Falcon-512 / 1024 + padded variants |
| `pqcrypto-traits` | Common keypair / signature trait surface |

### CKB tx construction

| Crate | Purpose |
|---|---|
| `ckb-sdk` | The Rust SDK Lantern's tx-builder mirrors |
| `ckb-types` | Canonical Rust types matching RFC 0019 |
| `ckb-jsonrpc-types` | JSON-RPC wire types for talking to full nodes / light client |
| `molecule` | Molecule codec for wire format |
| `ckb-hash` | Blake2b helpers |

### Storage (non-vault)

| Crate | Purpose |
|---|---|
| `sqlx` (sqlite feature) | Cached chain data, account records, transaction history |
| Or `rusqlite` if simpler — TBD during scaffolding |

### Frontend (TypeScript / npm packages)

| Package | Purpose |
|---|---|
| `react` 19+ | UI |
| `typescript` 5+ | Type system (strict mode) |
| `@tanstack/react-query` | Server state caching |
| `@tanstack/react-router` | Type-safe routing (chosen in v1.1 — see Section 12) |
| `zustand` | Local UI state |
| `vite` | Bundler |
| (generated) `bindings.ts` | From `tauri-specta` — single source of truth for IPC types |

---

## 23. Decision Summary

This project should be planned as:

**Lantern = Tauri 2 shell + Rust wallet core + React/TypeScript frontend surfaces**

That is the cleanest expression of the wallet's actual trust and product model.

The purpose of this restructure is not merely to replace Electron. It is to make the architecture reflect the real nature of the product:

**a Rust security application with extensible web-based user interfaces.**

---

## 24. Changelog

### v1.1 (2026-04-08, Claude)

**Reference:** [`2026-04-08-foundation-design-tauri-edition-review.md`](./2026-04-08-foundation-design-tauri-edition-review.md) — 14 issues + 5 missing sections enumerated by Claude as a review pass on v1.0.

**Amendments applied** (cross-referenced to review issue numbers):

- **#1** Added Rust-canonical notes alongside TS interface sketches in Sections 6 and 7
- **#2** Section 9 now names `specta` + `tauri-specta` as the canonical schema-gen pipeline (no longer "TBD") with code example, verified in tauri-ipc graph
- **#3** Section 9 has a tRPC-replacement responsibility map covering transport / schema / validation / cache / permission
- **#4** Section 8 has explicit one-window-per-tab fallback for the dApp browser if Tauri 2 multi-webview proves unstable
- **#5** Section 12 has a 3-row table explaining the `crates/signer-X` (implementation) vs `core-extensions/X/` (manifest+UI+SDK consumer) split, attributing the pattern to MetaMask snaps prior art
- **#6** Section 5 has explicit light-client subprocess supervision contract (spawn / ready / shutdown / restart), grounded in ckb-light-client graph
- **#7** Section 7 vault crypto switched from AES-256-GCM to XChaCha20-Poly1305 with explicit nonce strategy (192-bit random per encryption), verified in keystore-signing graph
- **#8** Section 7 has new "Memory hygiene" subsection naming `zeroize`, `secrecy::SecretBox<T>`, `mlock`, no Debug/Display rule. Corrected the secrecy crate API name (it's `SecretBox<T>`, not `Secret<T>`) via graph verification.
- **#9** Section 13 ML-DSA subsection has verified signature byte sizes (ML-DSA-44=2420, 65=3309, 87=4627; Falcon variants 666–1462), implications for tx-builder, and a witness-size hint trait requirement
- **#10** Section 13 Passkeys subsection has Linux passkey caveat — hardware FIDO2 only on Linux for v0.1
- **#11** Section 12 stack list now names TanStack Router; repo shape lists frontend route layout
- **#12** Section 12 repo shape now includes `tests/{integration,e2e,fixtures}` and `.github/workflows/{ci,release,security-audit}.yml`
- **#13** Section 15 rewritten to enumerate the actual 8 graphs (was 4), with viewer URLs and completion status
- **#14** Working name set: **Lantern** (placeholder, can be replaced)

**New sections added** (5):

- **§18 Network and Backend Axes** — clarifies that network (mainnet/testnet/devnet) and backend (embedded-light/remote-light/local-full/remote-full) are orthogonal
- **§19 Update Channel** — Tauri updater with signed manifests, off-by-default for majors, project-controlled CDN
- **§20 Telemetry Posture** — off by default, opt-in only, no silent collection
- **§21 Backup and Recovery** — seed phrase + vault file + recovery flow + ML-DSA/Falcon backup caveat
- **§22 Named Rust Dependencies** — leading-candidate crate list closing most of v1.0's open questions

### v1.0 (2026-04-08, ChatGPT)

Initial Tauri/Rust restructure of the original Electron foundation spec. See [historical v1.0](./2026-04-08-foundation-design-tauri-edition.md) for the original document.

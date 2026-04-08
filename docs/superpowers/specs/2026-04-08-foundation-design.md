# CKB Desktop Wallet — Foundation (v0.1) Design

> ⚠️ **SUPERSEDED 2026-04-08** — this Electron-flavoured edition has been replaced by [`2026-04-08-foundation-design-tauri-edition.md`](./2026-04-08-foundation-design-tauri-edition.md). Kept for historical reference and as a record of the rejected alternative. The Tauri/Rust edition retains the product/security ideas but rewrites Sections 4 (process model), 8–9 (extension API), 11 (stack), and 12 (security) around a Rust-first core. Do not implement from this version.

**Status:** ⛔ Superseded — historical
**Date:** 2026-04-08
**Author:** Phill + Claude (brainstorming session)
**Working title:** `ckb-wallet` (final name TBD)

---

## 1. Purpose and Positioning

A commercial-quality, community-driven desktop wallet for Nervos CKB, built as a modern replacement for the Neuron Wallet's user-facing role — *not* as a business-owned product. The intent is to become a candidate "default" community wallet as the Nervos Foundation decentralises, the way MetaMask and Phantom are defaults in their ecosystems despite not being core-team wallets.

**Explicit non-goals:**

- Dominating the wallet space. The architecture must actively welcome competing wallet implementations as peer extensions rather than crowd them out.
- Replacing developer-focused tooling. Neuron will likely remain a dev-oriented wallet; this project targets all users.
- Bundling a full CKB node. A dedicated Nervos node OS is a separate future project.
- Profit-driven design decisions. Every UX choice must be defensible on community-interest grounds.

**Guiding philosophy:** Decentralisation first. Run your own node. Broadcast your own transactions. The wallet must never silently route user activity through privileged servers. Complexity belongs in the code, not the user's daily flow.

---

## 2. Roadmap Overview

The full feature set Phill described spans ~18 independent subsystems. They are decomposed across three versions plus a deferred tier. **This spec covers v0.1 (Foundation) only.** Later versions are listed here so nothing is lost, but they are *out of scope for this document*.

### v0.1 — Foundation (this spec)

- Electron shell with commercial-quality polish
- `ChainBackend` abstraction (embedded light client default + remote light + local full + remote full)
- Bundled ckb-light-client-lite (SQLite variant) as a managed subprocess
- Multi-profile encrypted vault system
- Pluggable lock registry driven by extensions
- Four first-party lock extensions: secp256k1, Ledger, passkey (WebAuthn), ML-DSA PQC
- Watch-only account extension
- Ledger hardware wallet support (ported from Neuron)
- Two-tier plugin model: Tier 1 iframe dApp sandbox + Tier 2 `webContents` extension host
- **Minimal dApp browser tab** (URL bar, single tab, back/forward, iframe sandbox, CKB provider shim, approval modal) — sufficient to validate the Tier 1 mechanism end-to-end; full-featured browser (multi-tab, bookmarks, history, session keys, fee sponsorship) is v0.2
- Extension API (tRPC-based) as the public contract for community extensions
- Declarative permission model with install-time approval and runtime escalation
- Extension manifest format + bundled/registry/sideload distribution
- Internal console tab (bitcoin-core style) for RPC and wallet introspection
- Network selection (Mainnet / Testnet / Fiber) as a first-class UI concept
- Send, receive, transaction history, address book
- Neuron keystore import path
- Core feature parity with Neuron **except** full node integration

### v0.2 — First-party plugins (future)

- Nervos DAO extension
- iCKB extension
- NFT / Spore / CKBFS viewer + manager extension
- dApp browser tab upgrades: multi-tab, bookmarks, history, session keys, fee sponsorship hooks
- Plugin marketplace UI improvements

### v0.3 — Channels and bridges (future)

- Fiber payment channels extension
- Perun channels extension
- Rosen / Sonami Ergo bridge extension
- RGB++ extension (dependency: RGB++ TS tooling maturity)

### Deferred / research-gated

- JoyID "import" (technically infeasible for credential transfer; best-effort is watch-only today, same-passkey-lock account creation for a comparable address)
- Dedicated Nervos node OS (separate project)
- Mobile / handheld ports (Nervos Launcher is the separate handheld track)
- Custom light-client DB variant if SQLite proves inadequate under real load

---

## 3. Architectural Principles

These are the invariants every design choice in this spec and the implementation must preserve.

1. **The core wallet has no hardcoded lock types, no hardcoded chain, no hardcoded backend.** Everything is registered through extensions or the `ChainBackend` abstraction. This is the single most important discipline in the codebase.
2. **First-party features are built as extensions against the same API third parties use.** Constant dogfooding. The API cannot be second-class for the community.
3. **The user's keys are never visible to the wallet core.** Core owns only encrypted storage; lock extensions handle all secret material through the storage primitive.
4. **The wallet never lies about where power comes from.** If a feature requires a full node and the user is on a light client, it is *visible, explained, and one click from acting*, not hidden.
5. **Decentralisation defaults.** Autodetect local full nodes. Offer user-chosen backends prominently. Never route through privileged servers without disclosure.
6. **Security is enforced at capability boundaries, not by convention.** Permissions are checked at the tRPC procedure layer. Sandboxes are real process boundaries.

---

## 4. Process Model

The wallet is a multi-process Electron application with four kinds of processes:

```
┌──────────────────────────────────────────────────────────────┐
│ Electron Main (Node.js)                                      │
│  - Vault / keystore primitive                                │
│  - Account registry                                          │
│  - Signing coordinator                                       │
│  - Chain backend service (manages light-client subprocess)   │
│  - Extension host (spawns, manages extension webContents)    │
│  - tRPC router (the Extension API)                           │
└────────┬──────────────────────┬──────────────────────┬───────┘
         │                      │                      │
         │ IPC (tRPC)            │ IPC (tRPC)           │ stdio + JSON-RPC
         │                      │                      │
┌────────▼───────────┐  ┌───────▼────────────┐  ┌──────▼────────────┐
│ Shell Renderer     │  │ Extension          │  │ ckb-light-client- │
│ (React + Vite)     │  │ webContents × N    │  │ lite subprocess   │
│                    │  │ (Tier 2)           │  │ (Rust, SQLite)    │
│ Main wallet UI,    │  │                    │  └───────────────────┘
│ routing, layout,   │  │ Each extension     │
│ dApp browser tab   │  │ runs in its own    │
│ containing:        │  │ webContents with a │
│ ┌────────────────┐ │  │ contextIsolation=  │
│ │ iframe (Tier 1)│ │  │ true preload       │
│ │ sandboxed dApp │ │  │ bridging tRPC      │
│ └────────────────┘ │  │                    │
└────────────────────┘  └────────────────────┘
```

**Main process responsibilities:**

- Owns the vault (single source of encrypted secrets)
- Owns the account registry and dispatches signing requests
- Owns the `ChainBackend` registry and the light-client subprocess lifecycle
- Owns the tRPC router that implements the Extension API
- Spawns and supervises extension `webContents` instances
- Loads and validates extension manifests, enforces permissions

**Shell renderer responsibilities:**

- Main wallet UI (accounts list, send/receive, history, settings, console)
- Hosts the dApp browser tab, which is itself a `webContents` containing sandboxed `<iframe>` elements for Tier-1 dApps
- Injects a CKB provider shim into dApp iframes over `postMessage`
- All data comes from the main process over tRPC

**Extension webContents (Tier 2):**

- Each installed extension runs in its own `webContents` with `contextIsolation: true`, `nodeIntegration: false`
- A curated preload script exposes a typed tRPC client as `window.wallet` via `contextBridge`
- Extensions can contribute full pages, which are rendered by embedding their `webContents` into slots in the shell UI
- Extensions declare what they contribute in their manifest and the shell consumes that declaratively

**Light client subprocess:**

- Bundled `ckb-light-client-lite` binary (SQLite variant) shipped per-platform in the installer
- Spawned by the chain backend service on startup when the active backend is `embedded-light-client`
- Communication: localhost JSON-RPC
- Lifecycle: health checks, restart on crash, clean shutdown on app quit
- Logs surfaced to the internal console tab

---

## 5. Chain Backend Abstraction

A `ChainBackend` is the sole interface any code in the wallet uses to talk to a CKB network. The core principle: **nothing else in the wallet knows or cares whether the other end is a light client, a full node, local, or remote.**

### Interface (TypeScript sketch)

```ts
interface ChainBackend {
  id: string;                       // user-assigned or autogenerated
  label: string;                    // shown in UI
  network: "mainnet" | "testnet" | "fiber" | string;
  kind: "embedded-light" | "remote-light" | "local-full" | "remote-full";
  endpoint?: string;                // URL or undefined for embedded
  capabilities: BackendCapabilities;
  status: "connecting" | "synced" | "syncing" | "error" | "stopped";

  getTipHeader(): Promise<Header>;
  getCells(query: CellQuery, cursor?: Cursor): Promise<Paged<Cell>>;
  getTransaction(hash: string): Promise<TransactionWithStatus>;
  sendTransaction(tx: Transaction): Promise<string>;
  subscribeToScript(script: Script): AsyncIterator<ScriptEvent>;
  // ... etc
}

interface BackendCapabilities {
  canScanChain: boolean;          // full-node only
  canFullIndex: boolean;
  supportsSubscriptions: boolean;
  supportsGetBlockByNumber: boolean;
  // extensible as CKB RPC grows
}
```

### First-class backend types (v0.1)

| Kind | Default? | Notes |
|---|---|---|
| `embedded-light` | yes | Bundled ckb-light-client-lite subprocess, SQLite variant |
| `remote-light` | no | User-provided URL to a light client (e.g. household NUC) |
| `local-full` | no | Autodetected on first run via probe of `127.0.0.1:8114` |
| `remote-full` | no | User-provided URL to a full node |

### First-run behaviour

On first launch, the wallet:

1. Starts the embedded light client subprocess for Mainnet
2. Probes `127.0.0.1:8114` and, if a CKB full node responds, shows a one-time prompt:
   > *"A CKB full node is running on this machine. Use it instead of the bundled light client? Using your own node improves privacy and supports decentralisation."*
3. Populates the Network settings page with whichever backends were configured

### Capability-aware feature gating

When a feature requires a capability the current backend lacks (e.g. scanning the entire chain for all cells matching a lock), the UI surface for that feature:

- Remains **visible** (not hidden)
- Is disabled with a tooltip explaining why
- Provides a one-click path: *"Switch to a full-node backend"* → opens Network settings scrolled to the relevant section
- Or: *"This extension needs a full node. Add one?"*

This is the architectural expression of Principle #4.

---

## 6. Account and Keystore Model

### Core responsibilities (minimal)

The core wallet owns exactly three things related to keys and accounts:

1. **Encrypted vault primitive.** Password → Argon2id → AES-256-GCM. One vault file per profile. The vault is a generic encrypted key-value store; lock extensions ask the vault to persist blobs tagged with an account ID. The core never sees plaintext private key material of any kind.
2. **Account registry.** A list of records of the shape:
   ```ts
   interface AccountRecord {
     id: string;                   // stable UUID
     label: string;                // user-editable
     lockType: string;             // e.g. "core.secp256k1"
     extensionId: string;          // which extension owns it
     address: string;              // display-ready CKB address
     publicMetadata: Record<string, unknown>;  // xpub, credentialId, device path, etc.
     capabilities: AccountCapabilities;        // canSign, isWatchOnly, requiresDevice
   }
   ```
3. **Signing coordinator.** A single entry point: `sign(accountId, tx, context)`. It resolves the owning extension and dispatches. The coordinator enforces the unlock policy and the per-tx approval flow.

### Lock extensions (first-party)

| Extension | Account Type | Key Material | Witness Size |
|---|---|---|---|
| `@core/secp256k1` | HD wallet, BIP-32/44 path `m/44'/309'/0'/0/i` | Encrypted seed in vault | 65 bytes |
| `@core/ledger` | Hardware wallet | xpub + derivation path (no secret stored) | 65 bytes, signed on device |
| `@core/passkey` | WebAuthn credential (domain-bound to wallet origin) | Credential ID + public key; signing via OS platform authenticator | ~70–100 bytes (P-256 DER) |
| `@core/mldsa` | Post-quantum ML-DSA-44/65 | Encrypted ML-DSA private key in vault | ~2.4 KB |
| `@core/watch-only` | View-only address of any lock type | Address only, no secret | N/A — cannot sign |

Each extension provides its own account creation UI, its own signing implementation, and its own witness builder. The transaction builder is generic over witness size, which is **the reason we're writing a new tx builder rather than porting Neuron's** — Neuron assumes 65-byte witnesses in fee estimation.

### Profile model: multiple profiles, separate vaults

- A **profile** is a password-protected vault with its own account list, address book, settings, and installed extensions.
- Users can create, switch between, and delete profiles from a header menu.
- Each profile stores its data in a separate subdirectory under user data.
- Importing a Neuron keystore creates a new profile by default (preserving separation).
- Profiles are the unit of compartmentalisation — "Personal," "Business," "Hot," "Cold" — and this is first-class UX, not hidden in config.

### Unlock model: hybrid

- **Session unlock** for reads: enter vault password once, auto-lock timer (default 15 min, configurable 5 min / 15 min / 1 hour / never)
- **Per-signature re-authentication** for writes:
  - Ledger accounts: device confirmation (already per-tx by nature)
  - Passkey accounts: WebAuthn prompt (already per-tx by nature)
  - Software + PQC accounts: password or biometric re-prompt, with opt-in "remember for N minutes" defaulting **off**
- Users who want pure session-style signing can opt in explicitly in settings; default posture is safer than Neuron.

---

## 7. Two-Tier Plugin Model

The wallet has *two distinct plugin systems* with different trust models. Conflating them is the single most common architectural mistake in this space; separating them is how VS Code, Obsidian, and Figma all succeeded.

### Tier 1 — Connected dApps (zero trust)

- Any website the user visits in the dApp browser tab
- Loaded inside a sandboxed `<iframe>` with `sandbox="allow-scripts allow-forms"` and strict CSP
- Communication with the wallet via `postMessage` only
- A **CKB provider shim** is injected into the iframe, exposing a small API (request accounts, sign transaction, sign message, get chain id, subscribe to account changes)
- Every signing request triggers a host-rendered approval modal that the dApp cannot touch, style, or interact with
- No install step; the user navigates to a URL
- Zero persistent state beyond what the dApp stores in its own origin's localStorage

### Tier 2 — Extensions (explicit install, earned trust)

- Runs in its own Electron `webContents` with `contextIsolation: true`, `nodeIntegration: false`, and a curated preload
- The preload exposes a single `window.wallet` object: a typed tRPC client bridged to the main process
- Extensions can contribute:
  - **Navigation items** (new items in the sidebar, opening a page the extension renders)
  - **Full pages** (the extension's `webContents` is embedded in a host-provided slot)
  - **Lock types** (registered at extension load, appear in account creation flows)
  - **Transaction builder contributions** (e.g. "Deposit to Nervos DAO" for the DAO extension)
  - **Cell dep resolvers**
  - **Settings panels**
  - **Internal console commands**
  - **Background tasks** (lifecycle managed by the host)
- Each extension declares what it contributes in `manifest.json`; the shell consumes the manifest declaratively and wires the contributions into the UI

### Trust gradient

```
UNTRUSTED                 USER-INSTALLED              BUNDLED
(Tier 1)                  (Tier 2 — community)        (Tier 2 — first-party)
any dApp URL              iCKB, community locks       secp256k1, Ledger, passkey, ML-DSA, watch-only
iframe sandbox            webContents + preload       webContents + preload
minimal CKB provider      full Extension API          full Extension API
every signature prompts   declared permissions        declared permissions
```

**Every first-party feature is a Tier-2 extension.** The core ships an empty shell plus a bundled set of `@core/*` extensions that are installed on first run. There is no privileged "core functionality that isn't an extension" — if it signs, queries chains, or contributes UI, it is an extension.

This guarantees:

- The Extension API is dogfooded relentlessly — its weaknesses surface in our own code before a community dev hits them.
- Third-party extensions are never second-class.
- Removing a first-party feature later is just `uninstall`, not a core refactor.
- **Another wallet team can ship their wallet logic as a peer extension adapter**, providing their own account type, signing flow, and UI, at full fidelity. This is the architectural expression of Principle #1 and the "wallets in harmony" vision.

---

## 8. Extension API

The Extension API is our long-term public contract with the community. Its stability is second in importance only to the security of the vault.

### Transport: tRPC over Electron IPC

- Main process defines a tRPC router (`@wallet/extension-api`)
- Renderer and extension webContents each host a tRPC client bridged via `contextBridge` in their preload scripts
- Type definitions flow end-to-end: extension authors get full autocomplete and compile-time checking against our API
- `@wallet/extension-api` is published to npm as the public SDK; community devs `pnpm add @wallet/extension-api` and are authoring against typed interfaces from line one

### Versioning

- Semver on `@wallet/extension-api`
- Extensions declare `"apiVersion": "^0.1.0"` in their manifest
- Wallet refuses to load extensions targeting an incompatible major version
- Breaking changes only at majors; minors add capabilities; patches fix bugs
- A deprecation policy similar to VS Code: deprecated APIs work for two majors before removal

### Permission-gated procedures

Every tRPC procedure in the Extension API is tagged with required permissions. The router middleware checks the caller's manifest at every invocation. Calling a procedure without the permission throws a typed error; extensions handle it or the shell surfaces it as a "this extension needs additional permissions" prompt.

### Top-level API surface (v0.1, partial enumeration)

- `chain.*` — read state, query cells, get tx, send tx, subscribe
- `accounts.*` — list accounts, get account, request signature
- `vault.*` — store/retrieve extension-owned encrypted blobs
- `registry.lockTypes.register` — contribute a lock type
- `registry.navigation.register` — contribute a sidebar item
- `registry.page.register` — contribute a full page
- `registry.txBuilder.register` — contribute tx-builder operations
- `ui.prompt.*` — show host-rendered modals (approval, confirmation, input)
- `ui.notify.*` — show host-rendered notifications
- `console.registerCommand` — contribute an internal-console command
- `settings.register` — contribute a settings panel
- `network.currentBackend`, `network.onBackendChanged`

The full surface is the subject of an API spec that sits alongside the implementation. This document lists the categories, not the complete signature set.

---

## 9. Permission Model

Declarative, install-time-approved, runtime-escalatable. Android-style permission prompts are a familiar and effective mental model and will be used deliberately.

### Manifest declaration

```json
{
  "permissions": [
    "chain.read",
    "chain.send",
    "accounts.sign",
    "vault.store",
    "ui.prompt",
    "navigation.contribute"
  ],
  "optionalPermissions": [
    "accounts.list"
  ],
  "requiresCapabilities": ["scanChain"]
}
```

### Install-time approval

Before installing an extension, the shell displays:

> **Install "Cool Extension" v1.2.0?**
>
> This extension will be able to:
> - Read your chain state and cells
> - Send transactions (you will still approve each one individually)
> - Request signatures from your accounts (you will still approve each one individually)
> - Store encrypted data in your vault
> - Add a sidebar tab
>
> This extension requires: a full-node chain backend
>
> Author: ... · Source: ... · Manifest verified · [Install] [Cancel]

Users approve the entire permission set at install time. Optional permissions can be granted later on first use.

### Runtime escalation

An extension can request a permission at runtime:

```ts
await window.wallet.permissions.request("accounts.list");
```

The user sees a prompt, the permission is persisted, the extension is notified.

### Enforcement

Enforcement happens in the tRPC router middleware. Every procedure is declared with its required permission, and the middleware rejects calls that don't satisfy it. Permissions are not a UI convention — they are a real capability boundary.

**Per-signature approval is non-waivable.** Even if an extension has `accounts.sign`, every individual signature triggers the host-rendered approval modal. Extensions cannot suppress this. The only exception is when the user has opted into session signing in settings, and even then only for software and PQC accounts (Ledger and passkey are always per-tx by nature).

---

## 10. Extension Manifest and Distribution

### Manifest format

A `manifest.json` file in the root of each extension, alongside `package.json`. A published JSON Schema provides IDE autocomplete for extension authors and install-time validation.

```json
{
  "$schema": "https://ckb-wallet.org/schemas/extension-manifest.v1.json",
  "id": "community.author.name",
  "name": "Human Readable Name",
  "version": "1.2.0",
  "apiVersion": "^0.1.0",
  "author": { "name": "...", "url": "..." },
  "description": "...",
  "homepage": "...",
  "repository": "...",
  "license": "MIT",
  "entry": "dist/main.js",
  "contributes": {
    "navigation": [...],
    "pages": [...],
    "lockTypes": [...],
    "txBuilders": [...],
    "consoleCommands": [...],
    "settings": [...]
  },
  "permissions": [...],
  "optionalPermissions": [...],
  "requiresCapabilities": [...],
  "icon": "icon.png"
}
```

### Distribution channels (hybrid)

All three mechanisms exist simultaneously, with escalating friction that matches the trust level:

1. **Bundled.** First-party `@core/*` extensions are shipped inside the installer and installed on first run. No registry roundtrip.
2. **Default community registry.** A GitHub repository (`ckb-wallet/extensions-registry`) containing one JSON file per listed extension (name, repo, manifest hash, maintainer). Community devs open a PR to list their extension. Review is **curated for safety only** — working manifest, no obvious malware, buildable — *not* editorially judged. Users browse the registry in-app and install with one click.
3. **User-addable alternate registries.** Users can add additional registry URLs in settings (like apt sources, Flatpak remotes). A group of developers can run their own registry for in-development or niche extensions without depending on ours. This is the architectural guarantee that we never hold a monopoly.
4. **Sideload from URL or local folder.** Power users can paste a URL to a built extension bundle or drop a folder onto the extensions page to install. Confirmation dialog makes the risk explicit: *"Sideloaded extensions are not reviewed. Only install extensions from sources you trust."*

This mirrors Flatpak and VS Code and embodies Principle #5 (decentralisation defaults).

---

## 11. Stack and Repository Shape

### Stack

- **Electron** (latest stable)
- **TypeScript, strict mode, `"noUncheckedIndexedAccess": true`**
- **Vite** for both main and renderer
- **React** for UI in the shell and first-party extensions
- **shadcn/ui** as the component system (copy-paste, owned by us, themeable)
- **tRPC over Electron IPC** for main↔renderer and Extension API
- **Zustand** for local UI state
- **TanStack Query** for async state in the renderer
- **Node.js** for main process (LTS)
- **Rust** for the light-client subprocess (reusing existing `ckb-light-client-lite`)

### Repository layout (monorepo, pnpm workspaces + Turborepo)

```
ckb-wallet/
├── apps/
│   └── shell/                 # Electron main + renderer
├── packages/
│   ├── extension-api/         # tRPC router, types, host — the public SDK
│   ├── chain-backend/         # ChainBackend interface + implementations
│   ├── light-client-client/   # TS client for ckb-light-client-lite
│   ├── vault/                 # Encrypted storage primitive
│   ├── ui/                    # Shared shadcn-based component library
│   └── tsconfig/              # Shared TypeScript configs
├── core-extensions/
│   ├── secp256k1/
│   ├── ledger/
│   ├── passkey/
│   ├── mldsa/
│   └── watch-only/
├── examples/
│   └── example-extension/     # Template third parties copy
├── docs/
│   └── superpowers/specs/     # This directory
├── turbo.json
├── pnpm-workspace.yaml
└── package.json
```

**Scaffolding helper:** `pnpm create @wallet/extension my-extension` produces a starter using the template in `examples/`. This is the primary path for third-party extension authors.

---

## 12. Security Posture

Security-sensitive areas of the foundation and the disciplines that apply to them.

### Vault

- Argon2id for password KDF, parameters tuned for ~500ms on target hardware
- AES-256-GCM for encryption
- Vault file format versioned; future algorithm upgrades handled via re-encryption on unlock
- No plaintext private keys in memory longer than a single sign operation
- Vault unlock state kept only in main process memory; never crosses IPC

### IPC and sandboxing

- `contextIsolation: true`, `nodeIntegration: false` on every renderer and extension webContents
- Preload scripts are the only code that crosses the main/renderer boundary
- Extension preloads expose only the tRPC bridge, nothing else
- Strict CSP on all renderer and extension pages

### Ledger integration

- Device-side confirmation is treated as the source of truth; host can never claim a signature was approved without the device attesting
- Firmware version quirks handled per-device (reference: Neuron's existing handling)
- HID reconnect logic must be robust to sleep/wake cycles

### Passkey (WebAuthn) lock

- Credentials are domain-bound to the wallet's RP ID (the Electron app's origin or a platform-specific equivalent)
- The wallet RP ID is stable across installs so credentials survive upgrades
- Platform authenticator preferred; cross-platform authenticators (YubiKey etc.) supported
- WebAuthn on Electron has known quirks (RP ID scoping, Touch ID vs Windows Hello vs Linux differences) — handled in the passkey extension with per-platform code paths

### ML-DSA PQC lock

- Uses Phill's existing `ckb-mldsa-lock` v2 (v2-C variant currently deployed) and v2-rust sibling when available
- ML-DSA-44 and ML-DSA-65 both supported; v0.1 defaults to ML-DSA-44
- Private keys stored in vault; signing in main process, never in renderer
- Witness sizing (~2.4 KB) is **the reason** the tx builder must be generic over signature size

### dApp iframe sandbox

- `sandbox="allow-scripts allow-forms"` only
- No `allow-same-origin` (forces dApp origin to be unique)
- CSP restricts what the iframe can fetch
- `postMessage` handler validates origin and message shape

### Extension sandbox

- Extensions cannot read each other's vault blobs (namespaced per extension ID)
- Extensions cannot observe other extensions' accounts unless they hold a permission that grants it
- Extension permissions are enforced in the tRPC middleware, not at the UI layer

### Principle: no silent privileged servers

No code path in the foundation routes user transactions, queries, or signatures through a privileged server owned by the wallet project. If a future feature requires a relay (e.g. fee sponsorship in v0.2), it is disclosed, user-configurable, and swappable.

---

## 13. Research Prerequisites (Knowledge Graphs)

Before implementation begins, four dense knowledge graphs are constructed to serve as reference material throughout the project lifecycle. This applies Phill's graph-first navigation rule and pays compound interest over the months of build work ahead. Graph construction happens **after this spec is approved and before scaffolding begins.**

### Graph 1 — `neuron-port`

**Purpose:** reference for every piece of Neuron we are porting, vendoring, or learning from.

**Must answer:**

- How does Neuron's Ledger transport work end-to-end, from user click to signed tx?
- What is the Neuron keystore file format, and how do we import it losslessly?
- How does Neuron build transactions, estimate fees, and size witness placeholders?
- How does Neuron handle sUDT, Nervos DAO, and multisig?
- What is Neuron's IPC shape and what should we avoid repeating?
- Which Neuron strings, CSV formats, and schemas should we preserve for continuity?
- What parts of Neuron's UX should we deliberately deviate from and why?

**Sources:** Neuron repository, release notes, issue tracker, any available architecture docs.

### Graph 2 — `ckb-ecosystem-locks-and-protocols`

**Purpose:** the reference our lock registry, tx builders, and capability detection consult at runtime and during development.

**Must answer:**

- For every lock script (secp256k1-blake160, multisig, anyone-can-pay, omnilock with each flag, JoyID WebAuthn lock, ckb-mldsa-lock v2 and v2-rust, future additions): cell dep tx hash per network, hash type, witness layout, script hash, known deployed addresses
- For every protocol (xUDT, sUDT, Nervos DAO, Spore, CKBFS, iCKB, RGB++): on-chain shapes, cell structure, type scripts, interaction patterns
- Witness size implications per lock (critical for fee estimation)
- Relevant CKB RFCs and their status

**Sources:** Phill's existing CKB knowledge base (109 source files structured), ckb-docs, nervosnetwork/rfcs, the lock script repositories directly, deployment tx hashes from ckb-docs and Explorer.

### Graph 3 — `channels-and-bridges`

**Purpose:** research insurance for v0.3. Surfaced early so the v0.1 Extension API does not accidentally foreclose on something Fiber, Perun, or Rosen will need.

**Must answer:**

- Fiber: protocol RFC, channel state machine, RPC shape, invoice flow, payment routing
- Perun on CKB: protocol, maturity, current implementations, differences from Fiber
- Rosen Bridge: architecture, attestation model, supported chains, permit scheme
- Sonami repositories: current state of their Ergo bridge work, any CKB-side components
- Extension API implications: state observers, long-running background tasks, external event ingestion

**Sources:** Fiber RFC and docs, Phill's existing Fiber node lessons, Perun CKB repositories, Rosen Bridge docs, Sonami GitHub organisation.

### Graph 4 — `extension-platform-prior-art`

**Purpose:** directly informs Extension API and passkey lock design.

**Must answer:**

- VS Code Extension API: versioning discipline, contribution points, activation events, sandboxing, marketplace model
- Obsidian plugin API: what's simpler, what's the sandbox story, how are permissions handled
- Figma plugin model: constrained UI protocol, sandbox
- Tempo accounts SDK (`tempoxyz/accounts`): domain-bound passkeys, session keys, fee payer pattern, dialog reference implementation, wagmi connector
- WebAuthn on Electron: RP ID scoping, platform authenticator access, Touch ID / Windows Hello / Linux gotchas, prior CKB-side WebAuthn work (JoyID lock)

**Sources:** VS Code docs and source, Obsidian plugin developer docs, Figma plugin docs, tempoxyz/accounts repository, Electron WebAuthn issue tracker, JoyID lock script source.

### Graph construction approach

- Graphs 1, 2, and 4 are built locally via the `graphify` skill against existing repo clones and docs
- Graph 3 is partially built locally and partially via an Argus research ticket for the less-documented pieces (Perun on CKB maturity, Sonami Ergo bridge current state)
- All four graphs are served through the `graph-routing` workflow throughout implementation
- Graphs are versioned; regenerated at major milestones or when upstream changes significantly

---

## 14. Open Questions (resolve during graph construction and plan writing)

1. **Exact Electron version target** — latest stable at scaffold time; revisit if WebAuthn gotchas on a specific version force a pin.
2. **Neuron keystore import format version range** — determined by Graph 1.
3. **Extension marketplace UI in v0.1 or v0.2?** — v0.1 ships with bundled extensions, sideload, and registry-by-URL. Full in-app browse UI can slip to v0.2 if schedule tightens.
4. **Passkey credential portability story** — can a user export a credential reference for backup, or is credential loss terminal? Answered by Graph 4.
5. **ML-DSA-44 vs ML-DSA-65 default** — provisionally ML-DSA-44 (smaller sigs, still post-quantum secure); confirm with current lock deployment status.
6. **Fee estimation algorithm for variable-witness signing** — likely a contribution-point from lock extensions rather than a fixed formula in core; confirmed during implementation plan.
7. **Name.** "ckb-wallet" is a placeholder. A proper name is chosen before the scaffold lands.

---

## 15. Non-scope reminders

The following are **explicitly not** part of v0.1 and should not creep in during implementation:

- Nervos DAO, iCKB, NFT/Spore/CKBFS, RGB++ (v0.2 extensions)
- Fiber, Perun, Rosen (v0.3 extensions)
- dApp browser *features* beyond the basic provider injection (session keys, fee sponsorship are v0.2)
- Mobile, handheld, or web builds
- Custom light-client DB variants
- In-app extension marketplace browse UI (may slip to v0.2)
- Full node integration as a shipped subprocess (not ever — that is the future Nervos node OS's job)

---

## 16. Implementation sequencing (high-level; detailed plan is a separate document)

After this spec is approved:

1. Build the four research graphs (Section 13)
2. Revise this spec if graphs reveal issues
3. Write the implementation plan via the `writing-plans` skill
4. Scaffold the monorepo per Section 11
5. Build the foundation in the order: vault → chain backend → light client subprocess → account registry → signing coordinator → tRPC router → extension host → shell UI → first-party lock extensions → Neuron keystore import → polish
6. Each milestone gets its own review gate

---

## 17. Changelog

- **2026-04-08** — Initial draft (Phill + Claude brainstorming session)

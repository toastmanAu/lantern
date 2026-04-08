# CKB Desktop Wallet — Foundation (v0.1) Design, Tauri/Rust Edition (v1.0)

> ⚠️ **SUPERSEDED 2026-04-08** — this is **v1.0** of the Tauri/Rust edition. Replaced by [`2026-04-08-foundation-design-tauri-edition-v1.1.md`](./2026-04-08-foundation-design-tauri-edition-v1.1.md), which incorporates the 14 review issues from [`-review.md`](./2026-04-08-foundation-design-tauri-edition-review.md) plus 5 additional sections. Kept here for historical record. **Do not implement from this version.**

**Status:** ⛔ Superseded — historical (v1.0)
**Date:** 2026-04-08
**Author:** Phill + ChatGPT (restructured from the original foundation draft)
**Working title:** `ckb-wallet` (final name TBD)

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
- Password → Argon2id
- Encryption → AES-256-GCM
- Versioned file format
- Unlock state held only in Rust memory
- Extensions can store encrypted blobs through namespaced vault APIs
- No plaintext secret material crosses into frontend memory

### Account registry

Rust owns the canonical account registry. Frontends only receive non-sensitive account records.

```ts
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

### Signing coordinator

There should be exactly one signing entry point in Rust, conceptually similar to:

```ts
sign(accountId, tx, context)
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
- **Rust** for wallet core, permission enforcement, vault, backend manager, and extension host logic
- **React**
- **TypeScript**
- **Vite**
- **TanStack Query**
- **Zustand** for local UI state
- **Rust subprocess integration** for `ckb-light-client-lite`
- Optional generated TS SDK for extensions

### Repository shape

```text
ckb-wallet/
├── apps/
│   └── desktop/                 # Tauri app shell + frontend
├── crates/
│   ├── wallet-core/             # core orchestration
│   ├── vault/                   # encrypted storage
│   ├── chain-backend/           # backend abstractions and adapters
│   ├── extension-host/          # extension loading and permission logic
│   ├── tx-builder/              # generic tx building and witness sizing
│   ├── account-registry/        # account records and dispatch
│   ├── signer-secp256k1/
│   ├── signer-ledger/
│   ├── signer-passkey/
│   ├── signer-mldsa/
│   └── sdk-schema/              # shared API schemas
├── packages/
│   ├── extension-sdk/           # TS SDK for extensions
│   ├── ui/                      # shared React components
│   └── tsconfig/
├── core-extensions/
│   ├── secp256k1/
│   ├── ledger/
│   ├── passkey/
│   ├── mldsa/
│   └── watch-only/
├── examples/
│   └── example-extension/
└── docs/
    └── specs/
```

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

### Ledger

Ledger support should preserve the device-confirmed model. The host must never be able to claim a signature approval the device did not actually produce.

### ML-DSA

ML-DSA support continues to justify generic witness-size-aware transaction building. This remains one of the strongest reasons not to inherit assumptions from older wallet code.

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

## 15. Research Prerequisites

The same four research graph tracks remain excellent and should still be built:

1. `neuron-port`
2. `ckb-ecosystem-locks-and-protocols`
3. `channels-and-bridges`
4. `extension-platform-prior-art`

However, Graph 4 should now explicitly emphasize:

- Tauri plugin and capability model
- system webview implications
- passkey/WebAuthn behavior in Tauri contexts
- prior art from VS Code, Obsidian, Figma, and wallet SDKs

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

## 18. Decision Summary

This project should be planned as:

**Tauri shell + Rust wallet core + React/TypeScript frontend surfaces**

That is the cleanest expression of the wallet's actual trust and product model.

The purpose of this restructure is not merely to replace Electron. It is to make the architecture reflect the real nature of the product:

**a Rust security application with extensible web-based user interfaces.**

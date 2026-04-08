# Review notes — Foundation v0.1 (Tauri/Rust edition)

**Date:** 2026-04-08
**Reviewer:** Claude (sonnet 4.6)
**Spec under review:** [`2026-04-08-foundation-design-tauri-edition.md`](./2026-04-08-foundation-design-tauri-edition.md)

## Verdict

Sound restructure. Every architectural decision in the Tauri/Rust edition is correct for what this product actually is — a Rust security application with web-based user interfaces, not a web desktop app that happens to manage keys. The chat log (`WalletPivotChat.txt`) shows a sound reasoning trail with honest tradeoff analysis. Best moment to pivot is exactly *now*, before any code is written.

The product/security ideas from the original spec all survive intact: ChainBackend abstraction, multi-profile vault, pluggable lock registry, two-tier extension model, declarative permissions, capability gating, no privileged servers, host-rendered approvals.

What follows are issues, gaps, and clarifications that should land in a v1.1 of the Tauri spec before moving to implementation planning. None are blockers — the spec is ship-quality as a foundation. These are the "second-pass tightening" items.

---

## Issues

### 1. TypeScript interface sketches in a Rust-first spec

**Where:** Section 6 (`ChainBackend`), Section 7 (`AccountRecord`, `AccountCapabilities`), Section 7 (`sign(accountId, tx, context)`).

**Problem:** Code sketches are written in TypeScript despite the doc explicitly saying these live in Rust. A reader could come away assuming the canonical definition is TS.

**Fix:** One-line note before each TS sketch — *"Canonical definition is a Rust trait/struct in the relevant crate; the TypeScript shape shown here is illustrative of what the generated SDK will surface to extension authors and the frontend."* Or rewrite the sketches in Rust trait syntax (`async fn`, `Result<…, Error>`).

### 2. No schema generation tool named

**Where:** Section 9 ("Recommended shape"), Section 14 open question 5.

**Problem:** Spec says "Rust defines commands, events, schemas, permission requirements" and "TypeScript SDK exposes ergonomic wrappers" but doesn't name a generation pipeline. Open question 5 acknowledges this is TBD, but the gap leaves readers assuming hand-written bindings.

**Fix:** Name [`specta`](https://github.com/oscartbeaumont/specta) + [`tauri-specta`](https://github.com/oscartbeaumont/tauri-specta) as the leading candidate. They're built specifically for the Rust → Tauri command → TypeScript pipeline, support type-safe events, and integrate with serde. Alternatives worth listing: `ts-rs`, `schemars` + JSON Schema → TS, hand-written. Specta is the natural choice; lock it in as a recommendation even if the open question stays open.

### 3. tRPC's secondary value isn't replaced

**Where:** Section 9 / Section 12.

**Problem:** Removing tRPC removed not just the transport but also: input/output validation, end-to-end type safety, and the TanStack Query integration. Tauri commands cover transport. The spec doesn't say what covers the rest.

**Fix:** One paragraph mapping the responsibilities:
- **Transport:** `tauri::command` + `tauri::Event`
- **Schema generation:** `specta` / `tauri-specta`
- **Frontend validation:** generated TS types + optional `zod` at the boundary if runtime checks are needed
- **Query cache & React integration:** TanStack Query with thin wrappers around `invoke()`
- **Permission gate:** Rust `#[command]` body checks the calling extension's capabilities before executing

### 4. Tauri 2 multi-webview is "evolving" — no fallback story for the dApp browser

**Where:** Section 8 (Tier 1), Section 14 open question 4.

**Problem:** The dApp browser depends on multi-webview-per-window. The chat log explicitly notes Tauri's multi-webview features are flagged as evolving in their docs. If that doesn't stabilise on the v0.1 timeline, the dApp browser blocks.

**Fix:** Add an explicit fallback to Section 8: *"If multi-webview-per-window proves unstable on Tauri 2 by the v0.1 implementation window, the wallet falls back to a one-window-per-dApp-tab model. Same trust boundaries apply, the UX is just slightly clunkier (each connected dApp opens in its own window instead of a tab)."*

### 5. Crate vs core-extension relationship is ambiguous

**Where:** Section 12 repo shape.

**Problem:** Both `crates/signer-secp256k1` (and friends) AND `core-extensions/secp256k1` (and friends) exist. Are these the same thing? Different things? One depends on the other?

**Likely intent (best read):** The Rust crate is the *implementation*; the `core-extensions/<name>/` directory is the *manifest, contribution declarations, UI surface, and dogfooded extension package* that points at the crate. So a first-party signer is shipped as a core extension consuming its own Rust crate, which is exactly what "first-party features dogfood the extension API" means.

**Fix:** State this relationship explicitly. One sentence in Section 12 plus a brief example: *"Each first-party signer has a Rust crate (`crates/signer-mldsa`, the implementation) and a paired core extension package (`core-extensions/mldsa`, the manifest + UI + dogfooded SDK consumer). The crate is what ships in the binary; the core-extension package declares how the wallet shell discovers and routes to it."*

### 6. Light client supervision isn't named

**Where:** Section 5 diagram, Section 17 build order item 3.

**Problem:** The diagram shows ckb-light-client-lite as a sibling subprocess of the Rust core, but doesn't say *who supervises it*. Section 17 mentions "embedded light-client supervision" as a build step but doesn't bind the responsibility to a crate or component.

**Fix:** Add to Section 5: *"The Rust wallet core spawns, monitors, and restarts the ckb-light-client-lite subprocess. The chain-backend manager exposes its lifecycle as a `ChainBackend` of kind `embedded-light`. Crash handling, graceful shutdown on app exit, and stdout/stderr capture for diagnostics all live in the core."*

### 7. Vault crypto: nonce strategy missing

**Where:** Section 7.

**Problem:** Argon2id + AES-256-GCM specified but no nonce/IV strategy. AES-GCM with reused or predictable nonces is catastrophic — entire keystreams leak.

**Fix:** Either:
- **Option A (safer):** Switch to **XChaCha20-Poly1305** — 192-bit random nonce, no birthday-bound risk, available via the `chacha20poly1305` crate, used by `age` and other modern Rust crypto
- **Option B (specify carefully):** Stick with AES-256-GCM but mandate "fresh 96-bit random nonce per encryption, prepended to ciphertext, never reused under the same key"

I recommend Option A. AES-GCM is faster on x86 with hardware AES, but vault encryption isn't perf-critical and the safety margin matters more.

### 8. Memory hygiene for unlocked secrets isn't named

**Where:** Section 7 ("Unlock state held only in Rust memory"), Section 13.

**Problem:** "In Rust memory" is necessary but not sufficient. Secrets need:
- **Zeroization on drop** — `zeroize` crate, `ZeroizeOnDrop` trait on sensitive types
- **Memory locking** — `mlock(2)` or `region::lock` to prevent paging to swap
- **No serde Debug/Display impls** — to prevent accidental logging

**Fix:** Add a paragraph to Section 7 or Section 13:
*"All sensitive in-memory types (unlocked private keys, mnemonic seeds, passwords) implement `Zeroize + ZeroizeOnDrop`. Secret-bearing pages are `mlock`ed where the OS permits. Secret types do not implement `Debug` or `Display` and are wrapped in `secrecy::Secret<T>` to prevent accidental logging."*

### 9. ML-DSA witness sizes aren't called out as a concrete pressure on the tx-builder

**Where:** Section 13.

**Problem:** Section 13 says "ML-DSA support continues to justify generic witness-size-aware transaction building." True but understated. ML-DSA-44 / -65 / -87 produce signatures of **2.4 KB / 3.3 KB / 4.6 KB** respectively. By CKB standards these are *enormous* — secp256k1 sigs are 65 bytes, multisig is ~340 bytes. Witness size impacts:
- Tx fee (proportional to size)
- Cell capacity required to hold the locked output (must cover the witness)
- Max tx size limits

**Fix:** Add concrete numbers and one sentence: *"Each lock module advertises a witness-size hint (constant or formula). The tx-builder uses this to compute fee, cell capacity requirements, and to refuse tx layouts that would exceed CKB's max-tx-size with the chosen lock(s)."*

### 10. Linux passkey is the weakest link

**Where:** Section 13, Section 14 open question 3.

**Problem:** "Passkey support across macOS, Windows, and Linux" papers over the fact that Linux passkey is fragmented today. macOS has Touch ID + iCloud Keychain. Windows has Windows Hello. Linux has libfido2 + a hardware key (yubikey, etc.) OR a software authenticator like KeePassXC, but no platform-default biometric pathway.

**Fix:** State the truth in Section 14 question 3: *"Linux passkey support is the weakest of the three. The wallet should support hardware FIDO2 keys via libfido2 on all three platforms as a baseline, then add platform biometric paths (Touch ID, Windows Hello) where available. Software-only passkey on Linux is out of scope for v0.1."*

### 11. No router named in the frontend stack

**Where:** Section 12.

**Problem:** TanStack Query, Zustand, React, Vite — all named. No router. The wallet has settings, send/receive, history, address book, network selection, dApp browser, multiple extension UIs — that's a router-driven app.

**Fix:** Name **TanStack Router** (natural pair with TanStack Query, type-safe routes, file-based or code-based — fits the "typed everything" theme) or **React Router** (safer default, more familiar). Either works. Pick one to avoid bikeshedding later.

### 12. Test layout and CI not in the repo shape

**Where:** Section 12.

**Problem:** Rust unit tests live next to source (default) — fine. But:
- Where do **integration tests spanning crates** go? Top-level `tests/` or per-crate `tests/`?
- Where do **end-to-end tests** that drive the Tauri shell live? Playwright? WebDriver? `tauri-driver`?
- No `.github/workflows/` mentioned. CI for macOS, Windows, Linux musl, Linux glibc, signed releases, deterministic builds — all need to be planned in, not bolted on.

**Fix:** Add to repo shape:
```
tests/
├── e2e/                    # Tauri shell + frontend driven via tauri-driver
├── integration/            # cross-crate Rust integration tests
└── fixtures/
.github/
└── workflows/              # CI matrix: macOS, Win, Linux glibc, Linux musl
```
And one paragraph naming the test stack: cargo test for unit/integration, tauri-driver + WebDriver (or Playwright with the Tauri webview) for e2e.

### 13. Spec still says "4 research graphs"

**Where:** Section 15.

**Problem:** The chat log + this spec carry over the original 4-graph plan from the previous foundation spec. The actual research plan is now **8 graphs** (decided in this session):

1. neuron-port
2. ckb-ecosystem-locks
3. fiber-payment-channels (was "channels-and-bridges" — pivoted to fiber-only since force-bridge and godwoken are obsolete)
4. extension-platform-prior-art (now Tauri-flavoured: must include Tauri plugin/capability model + system webview implications + Tauri passkey behaviour, in addition to MetaMask Snaps / VS Code / Obsidian / Figma)
5. ckb-light-client
6. tauri-ipc-and-permissions (was "trpc-and-ipc-security" — renamed for the new stack: Tauri commands, events, permissions, capabilities, specta, sandbox)
7. keystore-signing (Ledger CKB, WebAuthn/passkey, ML-DSA / FIPS 204, vault patterns)
8. ckb-tx-construction (ccc, lumos, RFCs 0019/0022, cell model, witnesses)

**Fix:** Rewrite Section 15 to enumerate the 8 graphs. Note that Graphs 1, 2, 3 are already complete and live at `http://127.0.0.1:8765/` under "ckb-wallet research".

### 14. Wallet still has no real product name

**Where:** Section 14 open question 8.

**Problem:** Calling it "ckb-wallet" indefinitely is harmful to identity. People will keep calling it "the wallet" or "ckb-wallet" and it will become the actual name by inertia.

**Fix:** Set a *temporary working name* in the spec right now — even one Phill doesn't love. The act of having a name beats the absence of one. Once a real candidate emerges, search-and-replace. Some directions to consider for placeholder candidates:
- Directional / philosophical: *Pulse, Cadence, Lattice, Lantern, Beacon, Compass*
- Botanical / agricultural (echoing CKB's "common knowledge base" + Nervos ecosystem of "Granary", "Lina" etc): *Granary, Sprout, Loam, Yield, Bramble*
- Concrete / architectural: *Foundry, Anvil, Workshop, Atrium*
- Light / clarity (matches the light-client-default decision): *Lumen, Lantern, Glint, Spark*

Pick one as a placeholder, lock it down later. *"Lantern"* would be my low-cost suggestion — it's evocative of the light-client default, short, memorable, not collision-y in the wallet space, and easy to replace if it doesn't stick.

---

## Things I'd suggest *adding* to the spec

These aren't issues with what's there — they're sections that are missing.

### A. Named Rust dependencies (high-confidence picks)

The spec is implementation-agnostic on purpose, but a "leading candidates" appendix would close a lot of open questions cheaply:

- **Tauri shell:** `tauri = "2"`
- **Schema gen:** `specta`, `tauri-specta`
- **Vault crypto:** `argon2`, `chacha20poly1305`, `rand`, `zeroize`, `secrecy`
- **HD wallet / mnemonic:** `bip32`, `bip39`, `tiny-bip32`, or `slip-10`
- **Ledger:** `ledger-transport-hidapi` + a wallet-specific protocol crate (port from Neuron's logic)
- **Passkey / WebAuthn:** `webauthn-rs` for platform side; `ctap-hid-fido2` or `libfido2` for hardware FIDO2
- **ML-DSA:** `pqc-mldsa` / `fips204` (verify what exists in Rust today — Phill's mldsa-lock-v2 already uses something — graph it)
- **CKB tx building:** ccc-rust if it exists, or roll our own per RFC 0019/0022 (Graph 8 will tell us)
- **Light client subprocess management:** `tokio::process::Command`, `tracing` for log capture
- **Async:** `tokio` (everywhere), `async_trait` for `ChainBackend`
- **Errors:** `thiserror` for crate errors, `anyhow` only at the wallet-core boundary
- **Logging:** `tracing` + `tracing-subscriber`
- **Storage (non-vault):** `sqlx` with sqlite for cached chain data and account records, or `rusqlite` if simpler
- **Frontend state:** Zustand (named), TanStack Query (named), TanStack Router (recommended add)

### B. Update channel architecture

Tauri has a built-in updater. The spec mentions packaging but not update flow. Worth one paragraph: signed update manifests, signature verification with a project-controlled key, manual approval prompt by default, opt-in auto-update for non-major versions.

### C. Telemetry posture

For a "decentralisation-first, no privileged servers" wallet, telemetry must be **off by default and opt-in only**. Worth stating explicitly so it never sneaks in. If any metrics are collected, they must be local-only or explicitly user-uploaded.

### D. Backup and recovery story

Mnemonic seed backup is implied by the BIP39 / vault model but not specified. Hardware key recovery is also implied but not described. v0.1 should at minimum specify:
- Seed phrase shown once on creation
- Re-display gated behind password re-entry
- Vault file format documented (so users can backup the file directly)
- Recovery from seed produces the same address set as the original wallet

### E. Network selection and chain ID handling

Section 12 mentions "Network selection (Mainnet / Testnet / Fiber)" as a feature but Section 6 only describes backend selection. These are different axes:
- **Network** = which CKB chain (mainnet, testnet, devnet)
- **Backend** = how to reach it (embedded light, remote light, local full, remote full)

The wallet needs both. A profile is (network × backend), and accounts are scoped to networks (so addresses don't leak between mainnet/testnet). Worth one paragraph clarifying the model.

---

## Summary

The Tauri/Rust restructure is the right call. Do it. The 14 issues above are tightening passes, not blockers — the spec can carry forward into implementation planning as-is and these can be addressed in a v1.1 of the foundation doc once Graphs 4–8 are complete.

The most important items to address before scaffolding code:
1. **#7 (vault nonce strategy)** — security-critical, easy to fix now, costly to fix later
2. **#8 (memory hygiene)** — same reasoning
3. **#5 (crate vs core-extension)** — unblocks the repo scaffold
4. **#13 (8 graphs not 4)** — keeps research aligned with spec

Everything else is polish.

# Lantern

> A community-driven desktop wallet for Nervos CKB.
> Tauri 2 shell + Rust core + React/TypeScript frontend.
> Working title — final name TBD.

**Status:** Plans 1a (scaffold), 1b (vault) and 1c (accounts + secp256k1 signing) complete. Next: plan 1d (chain backend).

## Documents

- **Active spec:** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md)
- **Spec review notes:** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-review.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-review.md)
- **Historical (Tauri v1.0, superseded):** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition.md)
- **Historical (Electron, superseded):** [`docs/superpowers/specs/2026-04-08-foundation-design.md`](./docs/superpowers/specs/2026-04-08-foundation-design.md)
- **Implementation plans:** [`docs/superpowers/plans/`](./docs/superpowers/plans/)

## Research

8 knowledge graphs cover the design surface. See [Section 15 of the spec](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md#15-research-prerequisites--8-knowledge-graphs-rewritten-in-v11-addresses-review-issue-13) for the full list.

Source corpora live under `research/<name>/raw/` (gitignored — clone-on-demand). Graphs are viewable at `http://127.0.0.1:8765/` under "ckb-wallet research".

## Building

**Prerequisites:**
- Rust 1.88+ via [rustup](https://rustup.rs) (the `rust-toolchain.toml` will pin to 1.92 stable)
- Node.js 22 LTS (see `.nvmrc`)
- pnpm 10+
- On Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf`
- On macOS: Xcode Command Line Tools
- On Windows: WebView2 Runtime (preinstalled on Windows 11)

**One-time setup:**

```bash
git clone https://github.com/toastmanAu/lantern.git
cd lantern
pnpm install
```

**Run the dev shell:**

```bash
pnpm dev
```

This launches Vite + the Tauri shell. A window opens titled "Lantern" with a placeholder page. Hot-reload works on the React frontend; the Rust shell rebuilds on `Cargo.toml`/`.rs` changes.

**Run all tests:**

```bash
cargo test --workspace      # Rust tests
pnpm -r test                # Frontend tests
```

**Build a release artifact:**

```bash
pnpm tauri build
```

Output appears under `apps/desktop/src-tauri/target/release/bundle/`.

## Roadmap

- **v0.1 — Foundation** — secp256k1 wallet, embedded light client, vault, extension host skeleton, minimal UI. Testnet → mainnet.
- **v0.2** — first-party protocol extensions (Nervos DAO, iCKB, NFT/Spore/CKBFS viewer).
- **v0.3** — channels and bridges (Fiber, Perun).

## Implementation philosophy

Decentralisation first. Run your own node. Broadcast your own transactions. Secrets stay in Rust. Permissions enforced at capability boundaries. First-party features dogfood the same extension API as third parties.

See spec Section 4 (Core Architectural Principles).

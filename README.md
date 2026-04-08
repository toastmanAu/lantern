# Lantern

> A community-driven desktop wallet for Nervos CKB.
> Tauri 2 shell + Rust core + React/TypeScript frontend.
> Working title — final name TBD.

**Status:** Foundation design complete (v1.1). Implementation about to begin.

## Documents

- **Active spec:** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md)
- **Spec review notes:** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-review.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-review.md)
- **Historical (Tauri v1.0, superseded):** [`docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition.md`](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition.md)
- **Historical (Electron, superseded):** [`docs/superpowers/specs/2026-04-08-foundation-design.md`](./docs/superpowers/specs/2026-04-08-foundation-design.md)
- **Implementation plans:** [`docs/superpowers/plans/`](./docs/superpowers/plans/)

## Research

8 knowledge graphs cover the design surface. See [Section 15 of the spec](./docs/superpowers/specs/2026-04-08-foundation-design-tauri-edition-v1.1.md#15-research-prerequisites--8-knowledge-graphs-rewritten-in-v11-addresses-review-issue-13) for the full list.

Source corpora live under `research/<name>/raw/` (gitignored — clone-on-demand). Graphs are viewable at `http://127.0.0.1:8765/` under "ckb-wallet research".

## Roadmap

- **v0.1 — Foundation** — secp256k1 wallet, embedded light client, vault, extension host skeleton, minimal UI. Testnet → mainnet.
- **v0.2** — first-party protocol extensions (Nervos DAO, iCKB, NFT/Spore/CKBFS viewer).
- **v0.3** — channels and bridges (Fiber, Perun).

## Implementation philosophy

Decentralisation first. Run your own node. Broadcast your own transactions. Secrets stay in Rust. Permissions enforced at capability boundaries. First-party features dogfood the same extension API as third parties.

See spec Section 4 (Core Architectural Principles).

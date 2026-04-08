# Lantern tests

Layout (per spec section 12):

```text
tests/
├── integration/   cross-crate Rust integration tests (cargo test, may need real subprocess / network)
├── e2e/           end-to-end tests driving the full Tauri shell (tauri-driver / WebDriver / Playwright TBD plan 1f)
└── fixtures/      shared test data (mock blocks, sample vault files, recorded RPC responses)
```

## Running

- **Unit tests** (per crate): `cargo test --workspace`
- **Integration tests**: `cargo test --workspace --test '*' -- --ignored` (most need a real network or subprocess and are gated behind `--ignored` so plain `cargo test` stays fast)
- **E2E tests**: TBD — wired up in plan 1f when the first user-facing flow exists

Test data fixtures should never include real secrets. Use the `lantern-vault` crate's test helpers to generate ephemeral test vaults.

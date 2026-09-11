# Plan 1e — Transaction Builder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build, sign and broadcast a plain CKB capacity transfer from a single account, proven by CKB testnet pool acceptance.

**Architecture:** A pure `tx-builder` crate with no I/O and no dependency on `chain-backend` computes capacity floors, selects inputs largest-first, and resolves a fee/selection fixpoint by *measuring* the serialised transaction rather than estimating it. `wallet-core` owns every side effect: fetching candidates, dispatching to an async whole-transaction `LockModule::sign`, and broadcasting.

**Tech Stack:** Rust 1.92 / edition 2024, `ckb-types` + `ckb-jsonrpc-types` (`=1.1.1`), `async-trait`, `thiserror`, `blake2b` via the existing `signer-secp256k1` primitives.

**Spec:** `docs/superpowers/specs/2026-09-12-plan-1e-tx-builder-design.md`

## Global Constraints

- Rust 1.92, edition 2024. `unsafe_code = "forbid"` workspace-wide.
- Clippy `all` + `pedantic` + `nursery` must pass under `-D warnings`. New fallible public functions need `# Errors` rustdoc.
- No secret material in any `Display`, `Debug`, log line, or serialised file.
- No new third-party dependencies. `ckb-types` and `ckb-jsonrpc-types` are added to `tx-builder` only, as the documented exception in spec §3.2.
- `tx-builder` contains no `async` and no I/O, and must not depend on `lantern-chain-backend`.
- Transaction size is always **measured** from the serialised molecule, never computed by summing field widths (spec §3.3).
- Capacity floors are always computed from the actual script, never hardcoded (spec §5.2).
- Integration test files under `tests/` carry `#![cfg(feature = "testing")]` as their first line where they use the fake node.
- Live network tests carry `#[ignore]` **and** an environment gate, so a skipped run reports `ignored` and never `ok`.
- Conventional-commit messages.

## Type ownership (spec §3.1)

`sdk-schema` owns the signing vocabulary: `WitnessSize`, `SigningRequest`, `InputContext`, `SigningGroup`, `SignedWitness`, and the `LockModule` trait. `tx-builder` owns `TransferRequest`, `TransferPlan`, `BuildError`, and imports the rest. Do not move these.

## Reference facts, verified in-tree

Use these rather than re-deriving them:

- A `WitnessArgs` carrying only a lock of `L` bytes serialises to **`20 + L`** bytes: 4 (total size) + 12 (three u32 offsets) + 4 (lock length) + L. For secp256k1's 65-byte signature that is 85, pinned by `crates/signer-secp256k1/src/sighash.rs` with the header comment `total 0x55, offsets 0x10/0x55/0x55, lock len 0x41`.
- `sighash_all(tx_hash: &[u8; 32], first_witness: &[u8], others: &[&[u8]]) -> [u8; 32]` already exists in `crates/signer-secp256k1/src/sighash.rs` and is **molecule-agnostic — it takes bytes**. Plan 1e does not reimplement the digest; it assembles the inputs to it correctly for more than one input.
- Occupied capacity of a script in bytes is `32 (code_hash) + 1 (hash_type) + args.len()`. A cell's floor adds 8 bytes for the capacity field itself. 1 byte of occupied space costs 1 CKB = 100_000_000 shannons. secp256k1 with 20-byte args: `8 + 53 = 61` bytes = 61 CKB.
- `ScriptHashType` discriminants are `Data = 0, Type = 1, Data1 = 2, Data2 = 4`. **There is no 3.**

---

### Task 1: Capacity floors and units

**Files:**
- Create: `crates/tx-builder/src/capacity.rs`
- Modify: `crates/tx-builder/src/lib.rs`
- Modify: `crates/tx-builder/Cargo.toml`

**Interfaces:**
- Produces: `SHANNONS_PER_CKB: u64`; `script_occupied_bytes(&Script) -> u64`; `min_capacity(lock: &Script, type_: Option<&Script>, data_len: u64) -> u64` returning **shannons**.

- [ ] **Step 1: Add the dependencies**

In `crates/tx-builder/Cargo.toml`, under `[dependencies]`, add:

```toml
ckb-jsonrpc-types.workspace = true
ckb-types.workspace = true
```

This is the documented exception in spec §3.2. Do not add `lantern-chain-backend`.

Under `[dev-dependencies]` add:

```toml
hex.workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `crates/tx-builder/src/capacity.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::{min_capacity, script_occupied_bytes, SHANNONS_PER_CKB};
    use ckb_jsonrpc_types::{JsonBytes, Script, ScriptHashType};
    use ckb_types::H256;

    fn script(args_len: usize) -> Script {
        Script {
            code_hash: H256([0u8; 32]),
            hash_type: ScriptHashType::Type,
            args: JsonBytes::from_vec(vec![0u8; args_len]),
        }
    }

    #[test]
    fn a_script_occupies_its_code_hash_hash_type_and_args() {
        assert_eq!(script_occupied_bytes(&script(20)), 53);
        assert_eq!(script_occupied_bytes(&script(32)), 65);
        assert_eq!(script_occupied_bytes(&script(0)), 33);
    }

    #[test]
    fn a_secp256k1_cell_floor_is_61_ckb() {
        // 8 capacity field + 32 code_hash + 1 hash_type + 20 args.
        // The number every CKB wallet trips over exactly once.
        assert_eq!(min_capacity(&script(20), None, 0), 61 * SHANNONS_PER_CKB);
    }

    #[test]
    fn a_32_byte_args_pq_cell_floor_is_73_ckb() {
        assert_eq!(min_capacity(&script(32), None, 0), 73 * SHANNONS_PER_CKB);
    }

    #[test]
    fn a_type_script_and_data_raise_the_floor() {
        // 8 + 53 (lock) + 53 (type) + 10 (data) = 124
        assert_eq!(
            min_capacity(&script(20), Some(&script(20)), 10),
            124 * SHANNONS_PER_CKB
        );
    }
}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder capacity`
Expected: FAIL to compile — `min_capacity`, `script_occupied_bytes`, `SHANNONS_PER_CKB` are undefined.

- [ ] **Step 4: Implement**

Prepend to `crates/tx-builder/src/capacity.rs`:

```rust
//! Cell capacity floors.
//!
//! A CKB cell must hold enough capacity to store itself. The floor is
//! therefore a property of the cell's scripts and data, never a constant —
//! a PQ lock with 32-byte args needs 12 CKB more than secp256k1. Computing
//! it from the script is what stops the `-302 InsufficientCellCapacity`
//! class of failure, which surfaces only on-chain.

use ckb_jsonrpc_types::Script;

/// Shannons in one CKB.
pub const SHANNONS_PER_CKB: u64 = 100_000_000;

/// Bytes a script occupies: code hash, hash type, args.
#[must_use]
pub fn script_occupied_bytes(script: &Script) -> u64 {
    32 + 1 + script.args.as_bytes().len() as u64
}

/// Minimum capacity, in shannons, that a cell with these scripts and data
/// must hold to be valid.
///
/// The 8 bytes are the capacity field itself, which a cell must also pay for.
#[must_use]
pub fn min_capacity(lock: &Script, type_: Option<&Script>, data_len: u64) -> u64 {
    let bytes = 8
        + script_occupied_bytes(lock)
        + type_.map_or(0, script_occupied_bytes)
        + data_len;
    bytes * SHANNONS_PER_CKB
}
```

In `crates/tx-builder/src/lib.rs`, replace the placeholder test module with:

```rust
//! Lantern transaction builder.
//!
//! Pure: no I/O, no async, no dependency on `chain-backend`. Given candidate
//! cells and parameters it returns an unsigned transaction plan. Every
//! decision that can be wrong — capacity floors, the fee fixpoint, witness
//! sizing, the size ceiling — is a pure function reachable from a test with
//! no node.

#![forbid(unsafe_code)]

pub mod capacity;

pub use capacity::{min_capacity, script_occupied_bytes, SHANNONS_PER_CKB};
```

- [ ] **Step 5: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder capacity`
Expected: PASS, 4 tests.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): cell capacity floors computed from the script"
```

---

### Task 2: `WitnessSize` replaces `witness_lock_len`

**Files:**
- Modify: `crates/sdk-schema/src/lock.rs`
- Modify: `crates/signer-secp256k1/src/lock.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `WitnessSize::{Fixed(usize), Variable { min: usize, max: usize }}` with `for_fee_estimate(&self) -> usize`; `LockModule::witness_size(&self) -> WitnessSize` replacing `witness_lock_len`.

This is a breaking trait change. Both the real secp implementation and the `Fake` test implementation inside `lock.rs` must be updated in the same commit or the crate will not compile.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `crates/sdk-schema/src/lock.rs`:

```rust
#[test]
fn fee_estimation_uses_the_worst_case_witness_size() {
    assert_eq!(WitnessSize::Fixed(65).for_fee_estimate(), 65);
    // Falcon-512 ranges 666..=1462. Charging the minimum would get the
    // transaction rejected by the pool; charging the maximum costs a
    // rounding error. Always the maximum.
    assert_eq!(
        WitnessSize::Variable { min: 666, max: 1462 }.for_fee_estimate(),
        1462
    );
}
```

Add `WitnessSize` to that module's `use super::{...}` list.

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-sdk-schema witness`
Expected: FAIL to compile — `WitnessSize` is undefined.

- [ ] **Step 3: Implement the enum and change the trait**

In `crates/sdk-schema/src/lock.rs`, add above the `LockModule` trait:

```rust
/// How many bytes a lock module's signature occupies in the witness.
///
/// Post-quantum schemes make this a real question: Falcon-512 signatures
/// range 666 to 1462 bytes, so a single number cannot describe them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessSize {
    Fixed(usize),
    Variable { min: usize, max: usize },
}

impl WitnessSize {
    /// The size fee estimation must use.
    ///
    /// Always the worst case. Undercharging gets the transaction rejected
    /// by the pool; overcharging costs a rounding error.
    #[must_use]
    pub const fn for_fee_estimate(&self) -> usize {
        match self {
            Self::Fixed(n) => *n,
            Self::Variable { max, .. } => *max,
        }
    }
}
```

Replace the trait method:

```rust
    /// Size of the witness lock placeholder for fee estimation.
    fn witness_size(&self) -> WitnessSize;
```

Update the `Fake` implementation in the same file's test module — it currently returns `1` from `witness_lock_len`:

```rust
        fn witness_size(&self) -> WitnessSize {
            WitnessSize::Fixed(1)
        }
```

- [ ] **Step 4: Update the secp256k1 implementation**

In `crates/signer-secp256k1/src/lock.rs`, replace the `witness_lock_len` implementation with:

```rust
    fn witness_size(&self) -> WitnessSize {
        // RFC 0019: a recoverable signature is 65 bytes — r (32), s (32),
        // recovery id (1). Fixed, so fee estimation is exact rather than
        // conservative.
        WitnessSize::Fixed(65)
    }
```

Add `WitnessSize` to that file's imports from `lantern_sdk_schema`.

- [ ] **Step 5: Run the workspace to catch every other caller**

Run: `cargo test --workspace`
Expected: PASS. If anything else referenced `witness_lock_len`, fix it now and note it in your report — the brief did not anticipate it.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/sdk-schema crates/signer-secp256k1
git commit -m "feat(sdk-schema): witness size becomes an enum for variable-length signatures"
```

---

### Task 3: Witness placeholder construction

**Files:**
- Create: `crates/tx-builder/src/witness.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Consumes: `WitnessSize` (Task 2).
- Produces: `placeholder_witness(size: WitnessSize) -> Vec<u8>` returning serialised `WitnessArgs` bytes; `EMPTY_WITNESS: &[u8]` (the zero-length witness for non-first slots).

- [ ] **Step 1: Write the failing test**

Create `crates/tx-builder/src/witness.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::placeholder_witness;
    use lantern_sdk_schema::WitnessSize;

    #[test]
    fn a_65_byte_lock_serialises_to_85_bytes() {
        // Pinned independently by signer-secp256k1's sighash vector, whose
        // header comment reads: total 0x55, offsets 0x10/0x55/0x55,
        // lock len 0x41. Two sources agreeing is the point.
        let w = placeholder_witness(WitnessSize::Fixed(65));
        assert_eq!(w.len(), 85);
        assert_eq!(&w[..4], &[0x55, 0x00, 0x00, 0x00], "total size u32 LE");
        assert_eq!(&w[4..8], &[0x10, 0x00, 0x00, 0x00], "lock offset 16");
        assert_eq!(&w[16..20], &[0x41, 0x00, 0x00, 0x00], "lock length 65");
        assert!(w[20..].iter().all(|b| *b == 0), "lock body is zeroed");
    }

    #[test]
    fn the_serialised_length_is_always_twenty_plus_the_lock() {
        for len in [0usize, 1, 65, 666, 1462, 3309] {
            assert_eq!(
                placeholder_witness(WitnessSize::Fixed(len)).len(),
                20 + len,
                "4 total + 12 offsets + 4 lock length + body"
            );
        }
    }

    #[test]
    fn a_variable_size_placeholder_uses_the_maximum() {
        let w = placeholder_witness(WitnessSize::Variable { min: 666, max: 1462 });
        assert_eq!(w.len(), 20 + 1462);
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder witness`
Expected: FAIL to compile — `placeholder_witness` is undefined.

- [ ] **Step 3: Implement**

Prepend to `crates/tx-builder/src/witness.rs`:

```rust
//! Witness placeholders for fee measurement.
//!
//! The placeholder must be the exact size of the real signature, because
//! the transaction is measured with placeholders in place and broadcast
//! with real signatures. For `Fixed` sizes the two are identical; for
//! `Variable` the placeholder is the maximum, so the measured size is an
//! upper bound and the fee is never short.

use ckb_types::packed::{Bytes, BytesOpt, WitnessArgs};
use ckb_types::prelude::*;
use lantern_sdk_schema::WitnessSize;

/// A witness slot carrying nothing. Non-first slots of a script group use
/// this; they must be PRESENT, because the sighash stream length-prefixes
/// every slot and a missing one shifts every byte after it.
pub const EMPTY_WITNESS: &[u8] = &[];

/// Serialised `WitnessArgs` whose lock field is zeroed to the size fee
/// estimation must assume.
#[must_use]
pub fn placeholder_witness(size: WitnessSize) -> Vec<u8> {
    let lock_len = size.for_fee_estimate();
    let lock = Bytes::new_builder()
        .set(vec![0u8.into(); lock_len])
        .build();
    WitnessArgs::new_builder()
        .lock(BytesOpt::new_builder().set(Some(lock)).build())
        .build()
        .as_bytes()
        .to_vec()
}
```

If the `ckb-types` builder API differs from the above, adapt the construction but **do not change the assertions** — the tests define the required output and are pinned against an independent source. Report any API difference you hit.

Add to `crates/tx-builder/src/lib.rs`:

```rust
pub mod witness;

pub use witness::{placeholder_witness, EMPTY_WITNESS};
```

Add to `crates/tx-builder/Cargo.toml` dependencies:

```toml
lantern-sdk-schema = { path = "../sdk-schema" }
```

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder witness`
Expected: PASS, 3 tests.

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): witness placeholders sized for fee measurement"
```

---

### Task 4: Transaction size by measurement

**Files:**
- Create: `crates/tx-builder/src/size.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Produces: `measure(tx: &ckb_types::packed::Transaction) -> usize`; `MAX_TX_SIZE: usize`.

- [ ] **Step 1: Pin the size ceiling from consensus**

Search the vendored CKB sources for the consensus transaction-size limit:

```bash
grep -rn "MAX_BLOCK_BYTES\|max_block_bytes" /home/phill/ckb-wallet/research/ | head
```

Use the value you find and cite its file and line in a code comment. If you cannot find an authoritative value, use `512_000` — the figure the foundation spec §13 cites — and say plainly in your report that you could not verify it from source, so the next reader knows the provenance is weaker than the rest of the file.

- [ ] **Step 2: Write the failing test**

Create `crates/tx-builder/src/size.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::measure;
    use ckb_types::packed::Transaction;
    use ckb_types::prelude::*;

    #[test]
    fn size_is_the_serialised_length_plus_the_block_prefix() {
        let tx = Transaction::default();
        assert_eq!(measure(&tx), tx.as_slice().len() + 4);
    }

    #[test]
    fn adding_a_witness_grows_the_measurement_by_the_witness_and_its_framing() {
        let bare = Transaction::default();
        let before = measure(&bare);

        let with_witness = bare
            .clone()
            .as_advanced_builder()
            .witness(vec![0u8; 85].pack())
            .build()
            .data();
        let after = measure(&with_witness);

        // 85 bytes of witness, plus molecule framing for the new item.
        // The exact framing is the serialiser's business — the point is
        // that the measurement tracks it without us modelling it.
        assert!(
            after >= before + 85,
            "witness bytes must be counted: {before} -> {after}"
        );
    }
}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder size`
Expected: FAIL to compile — `measure` is undefined.

- [ ] **Step 4: Implement**

Prepend to `crates/tx-builder/src/size.rs`:

```rust
//! Transaction size, by measurement.
//!
//! This module deliberately does not compute size by summing field widths.
//! Every fee under-count in this project's history is an arithmetic model
//! diverging from what was actually serialised — the 560-byte JoyID gap,
//! the 2431-byte mint gap, the 384-byte loss through a clone path. If the
//! artifact measured is the artifact broadcast, that divergence cannot
//! exist.

use ckb_types::packed::Transaction;
use ckb_types::prelude::*;

/// Consensus ceiling on a single transaction's serialised size.
// Provenance: see Task 4 Step 1 — cite the file and line you verified.
pub const MAX_TX_SIZE: usize = 512_000;

/// Serialised size as the pool measures it.
///
/// The `+ 4` is the size prefix a transaction carries in block
/// serialisation.
#[must_use]
pub fn measure(tx: &Transaction) -> usize {
    tx.as_slice().len() + 4
}
```

Add to `crates/tx-builder/src/lib.rs`:

```rust
pub mod size;

pub use size::{measure, MAX_TX_SIZE};
```

- [ ] **Step 5: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder size`
Expected: PASS, 2 tests.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): measure transaction size from the serialised molecule"
```

---

### Task 5: Fee math

**Files:**
- Create: `crates/tx-builder/src/fee.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Produces: `fee_for(size: usize, rate: u64) -> u64`; `DEFAULT_FEE_RATE: u64`.

- [ ] **Step 1: Write the failing test**

Create `crates/tx-builder/src/fee.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::{fee_for, DEFAULT_FEE_RATE};

    #[test]
    fn at_the_pool_minimum_the_fee_equals_the_byte_count() {
        // 1000 shannons per 1000 bytes: one shannon per byte.
        assert_eq!(fee_for(1000, 1000), 1000);
        assert_eq!(fee_for(383, 1000), 383);
    }

    #[test]
    fn the_fee_always_rounds_up() {
        // Rounding down is how a transaction lands one shannon under the
        // pool minimum and is rejected with a message about fee rates.
        assert_eq!(fee_for(1, 1), 1);
        assert_eq!(fee_for(1001, 1), 2);
        assert_eq!(fee_for(999, 1), 1);
    }

    #[test]
    fn a_higher_rate_scales_linearly() {
        assert_eq!(fee_for(1000, 1200), 1200);
        assert_eq!(fee_for(2000, 1200), 2400);
    }

    #[test]
    fn the_default_rate_is_the_pool_minimum() {
        // We measure the transaction exactly rather than estimating it, so
        // the usual 20% drift buffer buys nothing. See fee.rs's module doc.
        assert_eq!(DEFAULT_FEE_RATE, 1000);
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder fee`
Expected: FAIL to compile — `fee_for` and `DEFAULT_FEE_RATE` are undefined.

- [ ] **Step 3: Implement**

Prepend to `crates/tx-builder/src/fee.rs`:

```rust
//! Fee arithmetic.
//!
//! The rate is shannons per 1000 bytes. The pool minimum is 1000, i.e. one
//! shannon per byte.
//!
//! `DEFAULT_FEE_RATE` is that minimum rather than a padded figure, and this
//! is a consequence of measuring instead of estimating: a placeholder is
//! the exact size of a `Fixed` signature and the maximum for a `Variable`
//! one, so the measured size is an upper bound on what gets broadcast and
//! the fee can never fall short. The customary 20% buffer exists to absorb
//! estimation drift that this builder does not have.

/// Shannons per 1000 bytes. The CKB pool's minimum.
pub const DEFAULT_FEE_RATE: u64 = 1000;

/// Fee in shannons for a transaction of `size` bytes, rounded up.
#[must_use]
pub const fn fee_for(size: usize, rate: u64) -> u64 {
    (size as u64).saturating_mul(rate).div_ceil(1000)
}
```

Add to `crates/tx-builder/src/lib.rs`:

```rust
pub mod fee;

pub use fee::{fee_for, DEFAULT_FEE_RATE};
```

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder fee`
Expected: PASS, 4 tests.

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): fee arithmetic at the pool minimum rate"
```

---

### Task 6: Candidate ordering

**Files:**
- Create: `crates/tx-builder/src/select.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Consumes: `InputContext` — **not yet defined**. For this task only, define the ordering over a minimal local trait bound so the task is independently testable; Task 9 introduces the real `InputContext` and Task 10 wires it. Order by `(capacity descending, out_point ascending)`.
- Produces: `order_candidates<T: Candidate>(candidates: &[T]) -> Vec<usize>`; `trait Candidate { fn capacity(&self) -> u64; fn tie_break(&self) -> &[u8]; }`.

- [ ] **Step 1: Write the failing test**

Create `crates/tx-builder/src/select.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::{order_candidates, Candidate};

    struct Cell {
        capacity: u64,
        id: Vec<u8>,
    }

    impl Candidate for Cell {
        fn capacity(&self) -> u64 {
            self.capacity
        }
        fn tie_break(&self) -> &[u8] {
            &self.id
        }
    }

    fn cell(capacity: u64, id: u8) -> Cell {
        Cell { capacity, id: vec![id] }
    }

    #[test]
    fn largest_capacity_comes_first() {
        let cells = [cell(100, 1), cell(300, 2), cell(200, 3)];
        assert_eq!(order_candidates(&cells), vec![1, 2, 0]);
    }

    #[test]
    fn equal_capacities_break_ties_deterministically() {
        // Without a total order, selection varies run to run — which makes
        // a golden serialisation vector impossible and tests flaky for
        // reasons that look like real bugs.
        let cells = [cell(100, 9), cell(100, 1), cell(100, 5)];
        assert_eq!(order_candidates(&cells), vec![1, 2, 0]);
    }

    #[test]
    fn an_empty_candidate_set_orders_to_nothing() {
        let cells: [Cell; 0] = [];
        assert!(order_candidates(&cells).is_empty());
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder select`
Expected: FAIL to compile — `order_candidates` and `Candidate` are undefined.

- [ ] **Step 3: Implement**

Prepend to `crates/tx-builder/src/select.rs`:

```rust
//! Candidate ordering: largest capacity first.
//!
//! Fewest inputs means the smallest transaction and the lowest fee. It also
//! means the multi-input path is rare in a consolidated wallet — which is
//! exactly why the tests that exercise it must construct fragmented wallets
//! deliberately rather than hope to encounter one.

/// Something with a capacity that can be ordered deterministically.
pub trait Candidate {
    fn capacity(&self) -> u64;
    /// Bytes used only to break capacity ties, so ordering is total.
    fn tie_break(&self) -> &[u8];
}

/// Indices into `candidates`, largest capacity first, ties broken by
/// `tie_break` ascending.
///
/// The tie-break is not cosmetic: without a total order the same wallet
/// produces different transactions on different runs, which defeats a
/// golden serialisation vector and makes failures irreproducible.
#[must_use]
pub fn order_candidates<T: Candidate>(candidates: &[T]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|a, b| {
        candidates[*b]
            .capacity()
            .cmp(&candidates[*a].capacity())
            .then_with(|| candidates[*a].tie_break().cmp(candidates[*b].tie_break()))
    });
    order
}
```

Add to `crates/tx-builder/src/lib.rs`:

```rust
pub mod select;

pub use select::{order_candidates, Candidate};
```

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder select`
Expected: PASS, 3 tests.

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): deterministic largest-first candidate ordering"
```

---

### Task 7: Signing vocabulary in `sdk-schema`

**Files:**
- Create: `crates/sdk-schema/src/signing.rs`
- Modify: `crates/sdk-schema/src/lib.rs`
- Modify: `crates/sdk-schema/Cargo.toml`

**Interfaces:**
- Produces: `InputContext`, `SigningGroup`, `SigningRequest`, `SignedWitness`. These live in `sdk-schema` because `LockModule` (Task 8) consumes them; `tx-builder` imports them.

- [ ] **Step 1: Add the dependency**

`sdk-schema` needs `ckb-jsonrpc-types` for `Script` and `OutPoint`, and `ckb-types` for `Transaction`. Add both to `crates/sdk-schema/Cargo.toml` under `[dependencies]`:

```toml
ckb-jsonrpc-types.workspace = true
ckb-types.workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `crates/sdk-schema/src/signing.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::{SigningGroup, SigningRequest};
    use crate::types::Derivation;

    fn group(indices: Vec<usize>) -> SigningGroup {
        SigningGroup {
            lock_hash: [7u8; 32],
            input_indices: indices,
            derivation: Derivation { change: 0, index: 0 },
        }
    }

    #[test]
    fn a_groups_witness_slot_is_its_lowest_input_index() {
        // RFC 0019 puts the signature in the first witness of the group.
        assert_eq!(group(vec![2, 5, 9]).witness_index(), Some(2));
        assert_eq!(group(vec![]).witness_index(), None);
    }

    #[test]
    fn a_request_knows_every_index_its_module_may_write() {
        let req = SigningRequest {
            tx: ckb_types::packed::Transaction::default(),
            inputs: Vec::new(),
            groups: vec![group(vec![0, 3]), group(vec![1])],
        };
        let owned = req.owned_indices();
        assert!(owned.contains(&0) && owned.contains(&1));
        assert!(
            !owned.contains(&2),
            "an index outside the module's groups is not its to write"
        );
    }
}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p lantern-sdk-schema signing`
Expected: FAIL to compile — the module and its types are undefined.

- [ ] **Step 4: Implement**

Prepend to `crates/sdk-schema/src/signing.rs`:

```rust
//! What a lock module receives when asked to sign.
//!
//! A module gets the whole resolved transaction, not a digest. Every
//! non-trivial signer needs it: a post-quantum lock rebuilds its own
//! message stream from the inputs, a hardware wallet renders the
//! transaction on-device, a passkey shows it for approval. Handing over a
//! precomputed digest instead is what produced the code-46 divergence in a
//! sibling project, where a builder reported empty input contents that were
//! correct only while the lock held nothing but pure change cells.

use ckb_jsonrpc_types::{OutPoint, Script};
use ckb_types::packed::Transaction;
use std::collections::BTreeSet;

use crate::types::Derivation;

/// A resolved input: what the cell being spent actually contains.
#[derive(Debug, Clone)]
pub struct InputContext {
    pub out_point: OutPoint,
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
    pub data: Vec<u8>,
}

/// One RFC 0019 script group: the inputs sharing a lock script hash.
///
/// The lock script runs once per group and only the group's first witness
/// carries the signature, so ten inputs from one address produce one
/// signature rather than ten.
#[derive(Debug, Clone)]
pub struct SigningGroup {
    pub lock_hash: [u8; 32],
    /// Ascending. The lowest is the group's witness slot.
    pub input_indices: Vec<usize>,
    pub derivation: Derivation,
}

impl SigningGroup {
    /// The witness slot that carries this group's signature.
    #[must_use]
    pub fn witness_index(&self) -> Option<usize> {
        self.input_indices.iter().min().copied()
    }
}

/// Everything a module needs to produce witnesses.
#[derive(Debug, Clone)]
pub struct SigningRequest {
    /// Unsigned, with witness slots already padded to `inputs.len()` and
    /// each group's first slot carrying a placeholder-sized `WitnessArgs`.
    pub tx: Transaction,
    /// Resolved, index-aligned with `tx`'s inputs.
    pub inputs: Vec<InputContext>,
    /// The groups this module is being asked to sign.
    pub groups: Vec<SigningGroup>,
}

impl SigningRequest {
    /// Every witness index this module is permitted to write.
    ///
    /// The orchestrator rejects anything outside this set. With third-party
    /// extension signers the alternative is a buggy or hostile module
    /// overwriting another lock's witness.
    #[must_use]
    pub fn owned_indices(&self) -> BTreeSet<usize> {
        self.groups
            .iter()
            .flat_map(|g| g.input_indices.iter().copied())
            .collect()
    }
}

/// One witness a module produced, and where it belongs.
#[derive(Debug, Clone)]
pub struct SignedWitness {
    pub index: usize,
    pub witness: Vec<u8>,
}
```

Add to `crates/sdk-schema/src/lib.rs`:

```rust
pub mod signing;

pub use signing::{InputContext, SignedWitness, SigningGroup, SigningRequest};
```

- [ ] **Step 5: Run it to confirm it passes**

Run: `cargo test -p lantern-sdk-schema signing`
Expected: PASS, 2 tests.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/sdk-schema
git commit -m "feat(sdk-schema): signing request carries the resolved transaction"
```

---

### Task 8: `LockModule::sign` becomes async and grouped

**Files:**
- Modify: `crates/sdk-schema/src/lock.rs`
- Modify: `crates/sdk-schema/Cargo.toml`

**Interfaces:**
- Consumes: `SigningRequest`, `SignedWitness` (Task 7).
- Produces: `async fn sign(&self, seed: &[u8], req: &SigningRequest) -> Result<Vec<SignedWitness>, LockError>` replacing `sign_digest`. The trait gains `#[async_trait]`.

The real secp256k1 implementation lands in Task 10. This task changes the contract and keeps the workspace compiling by updating the `Fake` test implementation and leaving `signer-secp256k1` with a temporary implementation that returns `LockError`. **That temporary is removed in Task 10** — say so in a code comment so a reader does not mistake it for finished work.

- [ ] **Step 1: Add the dependency**

In `crates/sdk-schema/Cargo.toml`:

```toml
async-trait.workspace = true
```

- [ ] **Step 2: Write the failing test**

Add to `mod tests` in `crates/sdk-schema/src/lock.rs`:

```rust
#[tokio::test]
async fn a_module_signs_from_a_request_rather_than_a_digest() {
    let fake = Fake;
    let req = SigningRequest {
        tx: ckb_types::packed::Transaction::default(),
        inputs: Vec::new(),
        groups: vec![SigningGroup {
            lock_hash: [0u8; 32],
            input_indices: vec![0],
            derivation: Derivation { change: 0, index: 0 },
        }],
    };
    let out = fake.sign(b"seed", &req).await.expect("signs");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].index, 0, "the group's witness slot");
}
```

Add `tokio` with the `macros` and `rt-multi-thread` features to `crates/sdk-schema/Cargo.toml` under `[dev-dependencies]`, and add the needed imports to the test module.

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p lantern-sdk-schema sign`
Expected: FAIL to compile — `LockModule` has no `sign`.

- [ ] **Step 4: Change the trait**

In `crates/sdk-schema/src/lock.rs`, add `use async_trait::async_trait;` and `use crate::signing::{SignedWitness, SigningRequest};`, mark the trait `#[async_trait]`, and replace `sign_digest` with:

```rust
    /// Produce witnesses for the groups in `req`.
    ///
    /// The module receives every group it owns in a single call, so it
    /// exposes the seed exactly once no matter how many inputs are being
    /// signed. Returning a witness for an index outside `req.owned_indices()`
    /// is a contract violation and the caller rejects it.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] if key derivation or signing fails.
    async fn sign(
        &self,
        seed: &[u8],
        req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError>;
```

Update the `Fake` implementation in the test module:

```rust
    #[async_trait]
    impl LockModule for Fake {
        // ... existing methods unchanged ...

        async fn sign(
            &self,
            _: &[u8],
            req: &SigningRequest,
        ) -> Result<Vec<SignedWitness>, LockError> {
            Ok(req
                .groups
                .iter()
                .filter_map(|g| g.witness_index())
                .map(|index| SignedWitness { index, witness: vec![0xAB] })
                .collect())
        }
    }
```

- [ ] **Step 5: Keep `signer-secp256k1` compiling**

In `crates/signer-secp256k1/src/lock.rs`, replace the `sign_digest` implementation with a temporary:

```rust
    async fn sign(
        &self,
        _seed: &[u8],
        _req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError> {
        // TEMPORARY — Task 10 of plan 1e implements this. Left returning an
        // error rather than a plausible-looking empty Vec, so that anything
        // wiring this up before then fails loudly instead of producing an
        // unsigned transaction that looks signed.
        Err(LockError::Unsupported)
    }
```

Use whichever `LockError` variant exists for "not implemented"; if none fits, add one rather than reusing a misleading variant, and say so in your report. Mark the crate's existing `sign_digest` tests `#[ignore]` with a reason naming Task 10 — do **not** delete them; Task 10 restores them.

- [ ] **Step 6: Run it to confirm it passes**

Run: `cargo test --workspace`
Expected: PASS, with the `signer-secp256k1` signing tests reported as `ignored`.

- [ ] **Step 7: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/sdk-schema crates/signer-secp256k1
git commit -m "feat(sdk-schema): signing becomes async, whole-transaction and grouped"
```

---

### Task 9: `TransferRequest`, `TransferPlan`, `BuildError`

**Files:**
- Create: `crates/tx-builder/src/types.rs`
- Create: `crates/tx-builder/src/error.rs`
- Modify: `crates/tx-builder/src/select.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Consumes: `InputContext` (Task 7), `WitnessSize` (Task 2), `Candidate` (Task 6).
- Produces: `TransferRequest`, `TransferPlan`, `BuildError`, and `impl Candidate for InputContext`.

- [ ] **Step 1: Write the failing test**

Create `crates/tx-builder/src/error.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::BuildError;

    #[test]
    fn every_error_names_the_number_that_would_fix_it() {
        let e = BuildError::InsufficientFunds {
            needed: 6_100_000_500,
            available: 6_000_000_000,
            shortfall: 100_000_500,
        };
        let text = e.to_string();
        assert!(text.contains("100000500"), "shortfall must be stated: {text}");
    }

    #[test]
    fn change_below_floor_does_not_claim_insufficient_funds() {
        // The user HAS the money. Saying "insufficient funds" would be a
        // lie, and would send them to a faucet to fix a problem a different
        // send amount solves.
        let e = BuildError::ChangeBelowFloor {
            change: 4_000_000_000,
            floor: 6_100_000_000,
        };
        let text = e.to_string().to_lowercase();
        assert!(!text.contains("insufficient"), "{text}");
        assert!(text.contains("change") || text.contains("leftover"), "{text}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-tx-builder error`
Expected: FAIL to compile — `BuildError` is undefined.

- [ ] **Step 3: Implement the errors**

Prepend to `crates/tx-builder/src/error.rs`:

```rust
//! Build failures.
//!
//! Every variant names the quantity that would resolve it. "Insufficient
//! funds" on its own tells a user nothing they can act on.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BuildError {
    #[error("amount {amount} shannons is below the recipient lock's minimum cell capacity of {floor}")]
    AmountBelowFloor { amount: u64, floor: u64 },

    #[error("need {needed} shannons but only {available} are available, short {shortfall}")]
    InsufficientFunds {
        needed: u64,
        available: u64,
        shortfall: u64,
    },

    #[error(
        "the leftover change of {change} shannons is below the {floor} a cell must hold; \
         send a different amount or consolidate first"
    )]
    ChangeBelowFloor { change: u64, floor: u64 },

    #[error("transaction is {size} bytes, over the {limit} byte limit")]
    TransactionTooLarge { size: usize, limit: usize },

    #[error("no spendable cells")]
    NoSpendableCells,
}
```

- [ ] **Step 4: Implement the types**

Create `crates/tx-builder/src/types.rs`:

```rust
//! The builder's inputs and outputs.

use ckb_jsonrpc_types::{CellDep, Script};
use ckb_types::packed::Transaction;
use lantern_sdk_schema::{InputContext, SigningGroup, WitnessSize};

use crate::select::Candidate;

/// What to build.
#[derive(Debug, Clone)]
pub struct TransferRequest {
    /// Every cell the account can spend. Order is irrelevant; the builder
    /// imposes its own.
    pub candidates: Vec<InputContext>,
    /// Where the capacity goes.
    pub recipient: Script,
    /// How much, in shannons.
    pub amount: u64,
    /// Where leftover capacity returns to.
    pub change_lock: Script,
    /// The owning lock module's witness size. `wallet-core` reads this from
    /// the module, which is what keeps this crate free of the trait.
    pub witness_size: WitnessSize,
    /// Cell deps the lock script needs. Omitting these fails only on-chain,
    /// as `ScriptNotFound`.
    pub cell_deps: Vec<CellDep>,
    /// Shannons per 1000 bytes.
    pub fee_rate: u64,
}

/// What to sign and broadcast.
#[derive(Debug, Clone)]
pub struct TransferPlan {
    /// Unsigned, witness slots padded, group slot placeholder-sized.
    pub tx: Transaction,
    /// Resolved, index-aligned with `tx`'s inputs.
    pub inputs: Vec<InputContext>,
    /// Always exactly one group in plan 1e: a transfer spends from one
    /// account, so every input shares a lock.
    pub groups: Vec<SigningGroup>,
    pub fee: u64,
    /// `None` when the selection covered the amount and fee exactly.
    pub change: Option<u64>,
    /// Measured size with placeholders in place.
    pub size: usize,
}

impl Candidate for InputContext {
    fn capacity(&self) -> u64 {
        self.capacity
    }

    fn tie_break(&self) -> &[u8] {
        // The out point's tx hash is a total, stable discriminator.
        self.out_point.tx_hash.as_bytes()
    }
}
```

If `OutPoint`'s `tx_hash` does not expose `as_bytes()` in `ckb-jsonrpc-types` 1.1.1, use whatever byte view it does provide — the requirement is a stable total order, not a particular accessor. Report what you used.

Add to `crates/tx-builder/src/lib.rs`:

```rust
pub mod error;
pub mod types;

pub use error::BuildError;
pub use types::{TransferPlan, TransferRequest};
```

- [ ] **Step 5: Run it to confirm it passes**

Run: `cargo test -p lantern-tx-builder`
Expected: PASS.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): transfer request, plan and error types"
```

---

### Task 10: secp256k1 signs a multi-input group

**Files:**
- Modify: `crates/signer-secp256k1/src/lock.rs`
- Modify: `crates/signer-secp256k1/Cargo.toml`

**Interfaces:**
- Consumes: `SigningRequest`, `SignedWitness`, `sighash_all` (existing).
- Produces: a real `LockModule::sign` for `Secp256k1Lock`.

This is the task the plan exists for. Read `crates/signer-secp256k1/src/sighash.rs` in full before starting — `sighash_all(tx_hash, first_witness, others)` already exists, takes bytes, and is pinned against a real testnet transaction. Do not reimplement it.

**The algorithm, per RFC 0019, for one group:**

1. Compute the transaction hash from `req.tx`'s raw part.
2. Take the group's witness slot — its lowest input index.
3. `first_witness` is that slot's current bytes, which the builder already set to a placeholder-sized `WitnessArgs` with a zeroed lock.
4. `others` is, in order: the remaining witnesses of the group (ascending by index), then **every witness at an index at or beyond `tx.inputs.len()`**.
5. `digest = sighash_all(tx_hash, first_witness, &others)`.
6. Sign the digest with the key derived for the group's `derivation`.
7. The output witness is a `WitnessArgs` identical to the placeholder but with the real 65-byte signature in the lock field.

Step 4 is where one-input transactions differ from every other transaction, and why plan 1c's vector cannot exercise this.

- [ ] **Step 1: Write the failing test**

Add to `crates/signer-secp256k1/src/lock.rs`'s test module:

```rust
#[tokio::test]
async fn signing_a_two_input_group_writes_only_the_groups_first_slot() {
    let module = Secp256k1Lock;
    let seed = [7u8; 64];
    let req = two_input_request();

    let out = module.sign(&seed, &req).await.expect("signs");

    assert_eq!(out.len(), 1, "one signature per group, not per input");
    assert_eq!(out[0].index, 0, "the group's lowest input index");
    assert_eq!(out[0].witness.len(), 85, "WitnessArgs with a 65-byte lock");
    assert_ne!(
        &out[0].witness[20..],
        &[0u8; 65][..],
        "the lock field must carry a real signature, not the placeholder"
    );
}

#[tokio::test]
async fn the_second_input_of_a_group_changes_the_digest() {
    // The whole point of RFC 0019's sighash-all: every witness in the group
    // is committed to. If the second input's witness slot were ignored, a
    // signature would transfer between transactions that differ only there.
    let module = Secp256k1Lock;
    let seed = [7u8; 64];

    let a = module.sign(&seed, &two_input_request()).await.expect("a");
    let b = module
        .sign(&seed, &two_input_request_with_altered_second_witness())
        .await
        .expect("b");

    assert_ne!(
        a[0].witness, b[0].witness,
        "a witness the digest does not commit to is a witness an attacker can change"
    );
}
```

Write the two helper constructors in the same test module. `two_input_request` builds a `SigningRequest` whose `tx` has two inputs under one lock, witness slots padded to 2, slot 0 holding `placeholder_witness(WitnessSize::Fixed(65))` and slot 1 empty, and one `SigningGroup` with `input_indices: vec![0, 1]`. The altered variant differs only in slot 1's bytes.

Add `lantern-tx-builder` to `crates/signer-secp256k1/Cargo.toml` `[dev-dependencies]` for `placeholder_witness`, and `tokio` with `macros` and `rt-multi-thread`.

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-signer-secp256k1 two_input`
Expected: FAIL — the temporary from Task 8 returns an error.

- [ ] **Step 3: Implement**

Replace the temporary `sign` in `crates/signer-secp256k1/src/lock.rs`:

```rust
    async fn sign(
        &self,
        seed: &[u8],
        req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError> {
        let tx_hash = transaction_hash(&req.tx);
        let input_count = req.tx.raw().inputs().len();
        let witnesses = witness_bytes(&req.tx);

        let mut signed = Vec::with_capacity(req.groups.len());
        for group in &req.groups {
            let Some(first) = group.witness_index() else {
                continue;
            };

            // RFC 0019: the rest of the group, then every witness beyond
            // the input count. Both parts are length-prefixed into the
            // digest, so a missing slot shifts every byte after it — which
            // is invisible at one input and wrong at two.
            let mut others: Vec<&[u8]> = group
                .input_indices
                .iter()
                .filter(|i| **i != first)
                .map(|i| witnesses[*i].as_slice())
                .collect();
            others.extend(
                witnesses
                    .iter()
                    .skip(input_count)
                    .map(std::vec::Vec::as_slice),
            );

            let digest = crate::sighash::sighash_all(&tx_hash, &witnesses[first], &others);
            let signature = self.sign_digest_internal(seed, &group.derivation, &digest)?;

            signed.push(SignedWitness {
                index: first,
                witness: witness_with_lock(&signature),
            });
        }
        Ok(signed)
    }
```

Add the three helpers to the same file. `transaction_hash` blake2b-hashes `tx.raw().as_slice()` with the `ckb-default-hash` personalisation — reuse whatever the crate already uses for blake160 rather than introducing a second hasher. `witness_bytes` extracts each witness as `Vec<u8>`. `witness_with_lock` builds an 85-byte `WitnessArgs` carrying the signature.

Rename the existing per-digest signing routine to `sign_digest_internal` and keep it private — it remains correct and is now an implementation detail. Un-ignore the tests you marked in Task 8 Step 5 and point them at it.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test -p lantern-signer-secp256k1`
Expected: PASS, including the previously ignored tests.

- [ ] **Step 5: Falsifiability probe — required, report the result**

The second test claims to prove the digest commits to the whole group. Verify it can fail: temporarily change `others` to include only the witnesses beyond `input_count`, dropping the rest of the group. `the_second_input_of_a_group_changes_the_digest` must FAIL. Restore, confirm green, and report what you saw. If it still passes, say so as the headline — the test does not pin what it claims.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/signer-secp256k1
git commit -m "feat(signer-secp256k1): sign a multi-input script group under one seed exposure"
```

---

### Task 11: A third-oracle multi-input sighash vector

**Files:**
- Modify: `crates/signer-secp256k1/src/sighash.rs`

**Interfaces:**
- Consumes: `sighash_all`.
- Produces: a pinned vector proving the multi-input digest against an independent source.

Plan 1c pinned a one-input transaction from testnet block 22356763. That vector cannot exercise witness padding. This task adds a two-input one.

- [ ] **Step 1: Find a real multi-input secp256k1 transaction on testnet**

Query a public node for a committed transaction with two or more inputs, all under `secp256k1_blake160_sighash_all`:

```bash
curl -s -X POST https://testnet.ckb.dev/ -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","method":"get_transaction","params":["<TX_HASH>"],"id":1}' | head -c 2000
```

Find a candidate by browsing a testnet explorer for a multi-input transfer. Record the transaction hash, its raw fields, and its witnesses in your report.

- [ ] **Step 2: Recompute its digest independently**

Recover the signer's public key hash from the signature and compare it to the lock args of the inputs. If the recovered hash matches, the digest you computed is the one the chain accepted — that is the independent oracle, and it does not depend on our own code being right.

Write this as a test in `crates/signer-secp256k1/src/sighash.rs`:

```rust
#[test]
fn a_real_two_input_testnet_transaction_recovers_to_its_own_lock_args() {
    // Oracle: this transaction is committed on CKB testnet, so the chain
    // accepted these signatures. If our digest is right, recovering the
    // public key from the signature yields the blake160 the inputs are
    // locked to. Nothing here trusts our own signing path.
    //
    // tx: <HASH>  block: <NUMBER>
    let tx_hash = hex!("...");
    let first_witness = hex!("...");
    let other_witnesses: Vec<Vec<u8>> = vec![hex!("...").to_vec()];
    let expected_lock_args = hex!("...");

    let others: Vec<&[u8]> = other_witnesses.iter().map(Vec::as_slice).collect();
    let mut zeroed = first_witness.to_vec();
    zeroed[20..85].fill(0);

    let digest = sighash_all(&tx_hash, &zeroed, &others);
    let recovered = recover_blake160(&digest, &first_witness[20..85]);

    assert_eq!(recovered, expected_lock_args);
}
```

Implement `recover_blake160` as a test helper using the crate's existing secp256k1 dependency: parse the 65-byte recoverable signature, recover the public key, serialise it compressed, blake2b it, take the first 20 bytes.

- [ ] **Step 3: Run it**

Run: `cargo test -p lantern-signer-secp256k1 two_input_testnet`
Expected: PASS.

**If it fails, stop and report rather than adjusting the test.** A failure here means either the digest construction is wrong or the transaction was not what it appeared to be — and both are findings worth more than a green suite. Record which transaction you used so the next person can check your work.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/signer-secp256k1
git commit -m "test(signer-secp256k1): pin the multi-input digest against a committed testnet transaction"
```

---

### Task 12: `build_transfer` — the fixpoint

**Files:**
- Create: `crates/tx-builder/src/build.rs`
- Modify: `crates/tx-builder/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1, 3, 4, 5, 6, 9.
- Produces: `build_transfer(req: &TransferRequest) -> Result<TransferPlan, BuildError>`.

- [ ] **Step 1: Write the failing tests**

Create `crates/tx-builder/src/build.rs` with only a test module covering:

```rust
#[cfg(test)]
mod tests {
    use super::build_transfer;
    use crate::capacity::{min_capacity, SHANNONS_PER_CKB};
    use crate::{BuildError, TransferRequest};
    // plus the helpers described below

    #[test]
    fn an_amount_below_the_recipients_floor_is_refused_before_building() {
        // 30 CKB to a secp address. This exact mistake reached the chain
        // twice in a sibling project and came back as -302, after the
        // transaction had been built and broadcast.
        let req = request(vec![1000], 30 * SHANNONS_PER_CKB);
        assert!(matches!(
            build_transfer(&req),
            Err(BuildError::AmountBelowFloor { .. })
        ));
    }

    #[test]
    fn a_single_large_cell_covers_amount_fee_and_change() {
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.inputs.len(), 1);
        assert!(plan.change.is_some());
        let change = plan.change.expect("some");
        assert_eq!(
            change,
            1000 * SHANNONS_PER_CKB - 100 * SHANNONS_PER_CKB - plan.fee
        );
    }

    #[test]
    fn selection_grows_until_the_change_clears_the_floor() {
        // 150 CKB available across two cells, sending 100. One cell alone
        // leaves change below the 61 CKB floor, so the builder must take
        // the second rather than give up or burn it.
        let req = request(vec![120, 30], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.inputs.len(), 2, "one cell alone cannot make viable change");
        let floor = min_capacity(&req.change_lock, None, 0);
        assert!(plan.change.expect("some") >= floor);
    }

    #[test]
    fn a_wallet_that_cannot_make_viable_change_says_so_precisely() {
        // Enough to pay, not enough to leave a spendable remainder.
        let req = request(vec![120], 100 * SHANNONS_PER_CKB);
        match build_transfer(&req) {
            Err(BuildError::ChangeBelowFloor { change, floor }) => {
                assert!(change < floor);
            }
            other => panic!("expected ChangeBelowFloor, got {other:?}"),
        }
    }

    #[test]
    fn too_little_money_is_insufficient_funds_not_a_change_problem() {
        let req = request(vec![70], 100 * SHANNONS_PER_CKB);
        match build_transfer(&req) {
            Err(BuildError::InsufficientFunds { shortfall, .. }) => {
                assert!(shortfall > 0);
            }
            other => panic!("expected InsufficientFunds, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_wallet_is_its_own_error() {
        let req = request(vec![], 100 * SHANNONS_PER_CKB);
        assert!(matches!(
            build_transfer(&req),
            Err(BuildError::NoSpendableCells)
        ));
    }

    #[test]
    fn witness_slots_are_padded_to_the_input_count() {
        // At one input this assertion is trivially satisfied and proves
        // nothing. At three it is the guard against the -52 class of
        // failure, where a missing slot shifts the sighash stream.
        let req = request(vec![80, 80, 80], 150 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(plan.inputs.len() >= 2, "the point of this test");
        assert_eq!(
            plan.tx.witnesses().len(),
            plan.inputs.len(),
            "every input needs a witness slot, even an empty one"
        );
    }

    #[test]
    fn the_fee_covers_the_measured_size_at_the_requested_rate() {
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(
            plan.fee >= crate::fee::fee_for(plan.size, req.fee_rate),
            "fee {} under-pays for {} bytes",
            plan.fee,
            plan.size
        );
    }

    #[test]
    fn all_inputs_form_one_group_in_a_single_account_transfer() {
        let req = request(vec![80, 80], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].input_indices.len(), plan.inputs.len());
    }
}
```

Write a `request(capacities_in_ckb: Vec<u64>, amount: u64) -> TransferRequest` helper that builds candidates all under one 20-byte-args secp-shaped lock, with distinct out points, `WitnessSize::Fixed(65)`, one cell dep, and `DEFAULT_FEE_RATE`.

- [ ] **Step 2: Run them to confirm they fail**

Run: `cargo test -p lantern-tx-builder build`
Expected: FAIL to compile — `build_transfer` is undefined.

- [ ] **Step 3: Implement**

Prepend to `crates/tx-builder/src/build.rs`:

```rust
//! Assembling a transfer.
//!
//! Fee depends on size, size depends on input count, and input count
//! depends on fee. The loop below resolves that fixpoint, and terminates
//! because a `CellOutput`'s capacity is a fixed-width u64 — the change
//! VALUE does not affect serialised size, only whether a change output
//! exists does. So for a given input count and change-presence the size is
//! fully determined, each added input increases it by a known positive
//! amount, and every iteration consumes one candidate.
//!
//! That argument holds because size is measured rather than modelled. An
//! arithmetic estimate could drift from the serialiser and invalidate it
//! with no test noticing.

use crate::capacity::min_capacity;
use crate::error::BuildError;
use crate::fee::fee_for;
use crate::select::order_candidates;
use crate::size::{measure, MAX_TX_SIZE};
use crate::types::{TransferPlan, TransferRequest};

/// Build an unsigned transfer.
///
/// # Errors
///
/// See [`BuildError`]. Every variant names the quantity that would resolve it.
pub fn build_transfer(req: &TransferRequest) -> Result<TransferPlan, BuildError> {
    let recipient_floor = min_capacity(&req.recipient, None, 0);
    if req.amount < recipient_floor {
        return Err(BuildError::AmountBelowFloor {
            amount: req.amount,
            floor: recipient_floor,
        });
    }
    if req.candidates.is_empty() {
        return Err(BuildError::NoSpendableCells);
    }
    let change_floor = min_capacity(&req.change_lock, None, 0);

    let order = order_candidates(&req.candidates);
    let mut taken: Vec<usize> = Vec::new();
    let mut available: u64 = 0;

    for &next in &order {
        taken.push(next);
        available = available.saturating_add(req.candidates[next].capacity);

        // Exact: no change output at all, so a smaller transaction and a
        // smaller fee. Check before the with-change shape, or a selection
        // that lands exactly is never recognised.
        let exact_tx = assemble(req, &taken, None);
        let exact_size = measure(&exact_tx);
        let exact_fee = fee_for(exact_size, req.fee_rate);
        if available == req.amount.saturating_add(exact_fee) {
            return finish(req, &taken, exact_tx, exact_fee, None, exact_size);
        }

        let change_tx = assemble(req, &taken, Some(0));
        let change_size = measure(&change_tx);
        let change_fee = fee_for(change_size, req.fee_rate);
        let needed = req
            .amount
            .saturating_add(change_fee)
            .saturating_add(change_floor);
        if available >= needed {
            let change = available - req.amount - change_fee;
            let tx = assemble(req, &taken, Some(change));
            return finish(req, &taken, tx, change_fee, Some(change), change_size);
        }
    }

    // Exhausted. Distinguish "cannot afford it" from "cannot make change".
    let tx = assemble(req, &taken, Some(0));
    let fee = fee_for(measure(&tx), req.fee_rate);
    let needed = req.amount.saturating_add(fee);
    if available < needed {
        return Err(BuildError::InsufficientFunds {
            needed,
            available,
            shortfall: needed - available,
        });
    }
    Err(BuildError::ChangeBelowFloor {
        change: available - needed,
        floor: change_floor,
    })
}
```

Write `assemble(req, taken, change) -> Transaction` and `finish(...) -> Result<TransferPlan, BuildError>` in the same file.

`assemble` builds the packed transaction: cell deps from `req.cell_deps`, no header deps, one input per taken index, a recipient output at `req.amount`, an optional change output at the given value under `req.change_lock`, empty outputs data for each output, and witnesses padded to the input count with slot 0 holding `placeholder_witness(req.witness_size)` and the rest `EMPTY_WITNESS`.

`finish` checks `size <= MAX_TX_SIZE`, returning `TransactionTooLarge` otherwise, and otherwise assembles the `TransferPlan` including the single `SigningGroup` over all taken indices. Compute the group's `lock_hash` from the candidates' shared lock script; they are all the same in a single-account transfer, so take it from the first.

- [ ] **Step 4: Run them to confirm they pass**

Run: `cargo test -p lantern-tx-builder build`
Expected: PASS, 9 tests.

- [ ] **Step 5: Falsifiability probes — required, report all three**

1. Remove the `change_floor` term from `needed` in the growth check. `selection_grows_until_the_change_clears_the_floor` must FAIL. Restore.
2. Change witness padding in `assemble` to emit a single witness rather than one per input. `witness_slots_are_padded_to_the_input_count` must FAIL. Restore.
3. Return `fee_for(exact_size, ...)` from the with-change branch, i.e. under-count by the change output. `the_fee_covers_the_measured_size_at_the_requested_rate` must FAIL. Restore.

If any probe passes, report it as the headline rather than adjusting the test.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy -p lantern-tx-builder --all-targets -- -D warnings
git add crates/tx-builder
git commit -m "feat(tx-builder): resolve the selection and fee fixpoint by measurement"
```

---

### Task 13: A fee property test

**Files:**
- Create: `crates/tx-builder/tests/fee_properties.rs`

**Interfaces:**
- Consumes: `build_transfer`, `fee_for`, `measure`.

Fee under-payment is the single most repeated failure in this project's history — across JoyID, CCC, and an sUDT builder. A handful of examples does not cover it; the property does.

- [ ] **Step 1: Write the test**

Create `crates/tx-builder/tests/fee_properties.rs`:

```rust
//! The one invariant that must hold for every transfer this builder makes.

use lantern_tx_builder::{build_transfer, fee_for, measure, BuildError};

/// A deterministic pseudo-random generator, so a failure is reproducible
/// from its seed. A flaky property test that cannot be replayed is worse
/// than no property test.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        self.0 >> 16
    }
    fn in_range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[test]
fn the_fee_never_under_pays_for_the_measured_size() {
    let mut rng = Lcg(0x1E_1E_1E_1E);
    let mut built = 0;

    for case in 0..500 {
        let wallet: Vec<u64> = (0..rng.in_range(1, 8))
            .map(|_| rng.in_range(62, 5_000))
            .collect();
        let amount_ckb = rng.in_range(61, 4_000);
        let rate = rng.in_range(1000, 3000);

        let req = /* build a TransferRequest from wallet, amount_ckb, rate */;

        match build_transfer(&req) {
            Ok(plan) => {
                built += 1;
                assert!(
                    plan.fee >= fee_for(plan.size, req.fee_rate),
                    "case {case}: fee {} under-pays for {} bytes at rate {}",
                    plan.fee, plan.size, req.fee_rate
                );
                assert_eq!(
                    plan.size,
                    measure(&plan.tx),
                    "case {case}: reported size disagrees with the transaction"
                );
            }
            Err(BuildError::InsufficientFunds { .. })
            | Err(BuildError::ChangeBelowFloor { .. })
            | Err(BuildError::AmountBelowFloor { .. }) => {}
            Err(other) => panic!("case {case}: unexpected {other:?}"),
        }
    }

    assert!(
        built > 100,
        "only {built} of 500 cases built a transaction — the generator is \
         producing mostly-unbuildable inputs and proving little"
    );
}
```

Reuse the `request` helper shape from Task 12, exported from a small `tests/common/mod.rs` or duplicated locally — duplication is acceptable here rather than widening the crate's public API for tests.

Note the final assertion: a property test whose inputs are almost all rejected passes while exercising nothing. Counting successes is what stops that.

- [ ] **Step 2: Run it**

Run: `cargo test -p lantern-tx-builder --test fee_properties`
Expected: PASS.

- [ ] **Step 3: Falsifiability probe — required**

Temporarily make `fee_for` round down (`/ 1000` instead of `div_ceil(1000)`). The property test must FAIL for at least one case. Restore and report. Rounding down is a one-shannon error that only shows up on certain sizes, which is exactly what a property test is for and an example test is not.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/tx-builder
git commit -m "test(tx-builder): the fee never under-pays for the measured size"
```

---

### Task 14: `wallet-core` sends

**Files:**
- Modify: `crates/wallet-core/src/core.rs`
- Modify: `crates/wallet-core/src/error.rs`
- Modify: `crates/wallet-core/Cargo.toml`

**Interfaces:**
- Consumes: `build_transfer`, `TransferRequest`, `LockModule::sign`, `ChainBackend::{get_cells, send_transaction}`.
- Produces: `WalletCore::send(&mut self, account_id: &str, recipient: &str, amount: u64) -> Result<H256, CoreError>`; `CoreError::{Build, WitnessOutOfRange}`.

- [ ] **Step 1: Write the failing test**

Add to `crates/wallet-core/tests/wallet_e2e.rs`:

```rust
#[tokio::test]
async fn sending_builds_signs_and_broadcasts() {
    let node = FakeNode::builder()
        .respond("local_node_info", node_info())
        .respond("get_tip_header", header("0x1554ef4"))
        .respond("get_cells", cells_page_with_one_large_cell())
        .respond("send_transaction", json!("0xabc..."))
        .start()
        .await;

    // ... create wallet, create account, attach a RemoteLight backend ...

    let hash = core.send(&account.id, RECIPIENT_ADDRESS, 100 * 100_000_000)
        .await
        .expect("sends");
    assert_eq!(hash.to_string(), "0xabc...");

    let (_, params) = node
        .calls()
        .into_iter()
        .find(|(m, _)| m == "send_transaction")
        .expect("broadcast happened");

    let tx = &params[0];
    assert_eq!(params[1], "passthrough", "non-standard locks must not be rejected");
    assert_eq!(
        tx["witnesses"].as_array().expect("array").len(),
        tx["inputs"].as_array().expect("array").len(),
        "a witness slot per input, even when empty"
    );
    let first = tx["witnesses"][0].as_str().expect("hex");
    assert_ne!(
        &first[42..], &"0".repeat(130),
        "the lock field must carry a signature, not the placeholder"
    );
}

#[tokio::test]
async fn a_module_cannot_write_a_witness_it_does_not_own() {
    // With third-party extension signers, the alternative to this check is
    // one module silently overwriting another lock's witness.
    let req = /* a SigningRequest whose groups cover index 0 only */;
    let rogue = vec![SignedWitness { index: 3, witness: vec![0xFF] }];
    assert!(matches!(
        apply_witnesses(req.tx.clone(), &req, rogue),
        Err(CoreError::WitnessOutOfRange { index: 3 })
    ));
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p lantern-wallet-core sending`
Expected: FAIL to compile — `send` and `apply_witnesses` are undefined.

- [ ] **Step 3: Implement**

Add to `crates/wallet-core/src/error.rs`:

```rust
    #[error(transparent)]
    Build(#[from] lantern_tx_builder::BuildError),

    #[error("lock module returned a witness for index {index}, which is not in its script groups")]
    WitnessOutOfRange { index: usize },
```

Add `lantern-tx-builder = { path = "../tx-builder" }` to `crates/wallet-core/Cargo.toml`.

In `crates/wallet-core/src/core.rs`, add:

```rust
    /// Build, sign and broadcast a transfer from one account.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Backend`] if the chain is unreachable,
    /// [`CoreError::Build`] if the transfer cannot be constructed, and
    /// [`CoreError::WitnessOutOfRange`] if the lock module returns a
    /// witness outside the groups it was asked to sign.
    pub async fn send(
        &mut self,
        account_id: &str,
        recipient: &str,
        amount: u64,
    ) -> Result<H256, CoreError> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or(CoreError::AccountNotFound)?
            .clone();
        let module = self.locks.get(account.lock_type)?;
        let backend = self
            .backend
            .as_ref()
            .and_then(BackendManager::current_backend)
            .ok_or(CoreError::Backend(BackendError::NotReady))?;

        let candidates = collect_candidates(backend, &account, self.network).await?;
        let request = TransferRequest {
            candidates,
            recipient: decode_address(recipient, self.network)?,
            amount,
            change_lock: lock_script_for(&account, module)?,
            witness_size: module.witness_size(),
            cell_deps: module.cell_deps(self.network),
            fee_rate: lantern_tx_builder::DEFAULT_FEE_RATE,
        };
        let plan = lantern_tx_builder::build_transfer(&request)?;

        let signing = SigningRequest {
            tx: plan.tx.clone(),
            inputs: plan.inputs.clone(),
            groups: plan.groups.clone(),
        };
        let seed = Keyring::seed_for(&self.vault, module.seed_kind())?;
        let witnesses = module.sign(seed.expose_secret(), &signing).await?;
        drop(seed);

        let signed = apply_witnesses(plan.tx, &signing, witnesses)?;
        backend.send_transaction(&signed.into()).await.map_err(Into::into)
    }
```

And the free function:

```rust
/// Splice a module's witnesses into the transaction, refusing any index the
/// module was not asked to sign.
fn apply_witnesses(
    tx: Transaction,
    req: &SigningRequest,
    witnesses: Vec<SignedWitness>,
) -> Result<Transaction, CoreError> {
    let owned = req.owned_indices();
    let mut slots: Vec<Vec<u8>> = witness_bytes(&tx);
    for w in witnesses {
        if !owned.contains(&w.index) {
            return Err(CoreError::WitnessOutOfRange { index: w.index });
        }
        slots[w.index] = w.witness;
    }
    Ok(rebuild_with_witnesses(tx, slots))
}
```

`collect_candidates` pages `get_cells` for the account's lock using the `CellQuery`/`Cursor` API from plan 1d, collecting `InputContext` values. Page until exhausted — and do not persist the cursor anywhere; `Cursor` deliberately has no serde derives for that reason. `module.cell_deps(network)` is a new `LockModule` method returning the lock's `CellDep` list; add it with the secp256k1 mainnet and testnet dep group out points, and implement it for the `Fake` module as an empty vec.

`decode_address` parses a CKB2021 bech32m address to a `Script` — `account-registry` already encodes them, so put the decoder beside that encoder and reuse its constants rather than writing a second copy of the HRP and format rules.

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo test -p lantern-wallet-core`
Expected: PASS.

- [ ] **Step 5: Falsifiability probe — required**

Temporarily remove the `owned.contains` check in `apply_witnesses`. `a_module_cannot_write_a_witness_it_does_not_own` must FAIL. Restore and report.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/wallet-core crates/sdk-schema crates/signer-secp256k1 crates/account-registry
git commit -m "feat(wallet-core): build, sign and broadcast a transfer"
```

---

### Task 15: A golden serialisation vector

**Files:**
- Create: `crates/tx-builder/tests/golden.rs`

**Interfaces:**
- Consumes: `build_transfer`.

- [ ] **Step 1: Write the test**

Create `crates/tx-builder/tests/golden.rs`:

```rust
//! One transfer, pinned byte for byte.
//!
//! Unit tests assert properties; this asserts the artifact. If a
//! dependency's serialiser changes behaviour, or an ordering rule drifts,
//! a property test may still pass while the bytes on the wire differ. That
//! is the failure this catches, and it is why the expected value is
//! transcribed from an independent encoding rather than from our own
//! output.

use lantern_tx_builder::build_transfer;

#[test]
fn a_fixed_transfer_serialises_to_known_bytes() {
    let req = /* a fully deterministic TransferRequest: fixed out points,
                 fixed lock args, 1000 CKB input, 100 CKB amount,
                 DEFAULT_FEE_RATE */;
    let plan = build_transfer(&req).expect("builds");
    let bytes = hex::encode(plan.tx.as_slice());

    assert_eq!(bytes, GOLDEN, "serialised transfer changed");
    assert_eq!(plan.size, plan.tx.as_slice().len() + 4);
}

const GOLDEN: &str = "...";
```

- [ ] **Step 2: Derive the golden value independently**

Do **not** produce `GOLDEN` by printing what the builder emits and pasting it back — that asserts the code agrees with itself and would pass against any bug. Instead:

Construct the same transaction with `ckb-sdk-rust`'s or `ckb-cli`'s transaction builder from the vendored sources in `/home/phill/ckb-wallet/research/ckb-tx-construction/`, serialise it, and use that hex. If the two disagree, **investigate before changing either** — one of them is wrong, and finding out which is the entire value of this test.

Record in your report which tool produced the golden value and how you invoked it.

If you genuinely cannot produce an independent encoding, say so plainly and skip this task rather than writing a self-referential assertion. A test that cannot fail is worse than a missing one, because it reads as coverage.

- [ ] **Step 3: Run it**

Run: `cargo test -p lantern-tx-builder --test golden`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/tx-builder
git commit -m "test(tx-builder): pin a transfer's serialisation against an independent encoder"
```

---

### Task 16: Live broadcast, CI, docs

**Files:**
- Create: `crates/wallet-core/tests/live_send.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-12-plan-1e-tx-builder-design.md`

- [ ] **Step 1: Write the live test**

Create `crates/wallet-core/tests/live_send.rs`:

```rust
//! A real transfer on CKB testnet.
//!
//! Two gates, deliberately: `#[ignore]` so a skipped run reports `ignored`
//! rather than `ok`, and an environment variable so it cannot fire from a
//! workflow edit alone. Running it requires both `--ignored` and the
//! variable.
//!
//! Run with:
//!   LANTERN_LIVE_TESTNET=1 LANTERN_LIVE_MNEMONIC="..." \
//!     cargo test -p lantern-wallet-core --test live_send -- --ignored --nocapture

const RPC: &str = "https://testnet.ckb.dev/";

fn enabled() -> Option<String> {
    if std::env::var("LANTERN_LIVE_TESTNET").as_deref() != Ok("1") {
        return None;
    }
    std::env::var("LANTERN_LIVE_MNEMONIC").ok()
}

#[ignore = "live: set LANTERN_LIVE_TESTNET=1 and LANTERN_LIVE_MNEMONIC, run with --ignored"]
#[tokio::test]
async fn a_real_transfer_is_accepted_by_the_testnet_pool() {
    let Some(phrase) = enabled() else {
        eprintln!("skipped: LANTERN_LIVE_TESTNET=1 and LANTERN_LIVE_MNEMONIC required");
        return;
    };

    // Import the funded wallet, attach a RemoteFull backend at RPC,
    // sync watched scripts, create or reuse account 0, and send a small
    // amount back to the account's own address — a self-transfer needs no
    // second funded party and still exercises the entire path.
    //
    // Send 200 CKB, comfortably above the 61 CKB floor. Amounts near the
    // floor fail for capacity reasons BEFORE the lock script runs, which
    // masks exactly the signature validity this test exists to prove.

    // ... build, send, assert the hash is returned ...

    eprintln!("broadcast: {hash:#x}");
}
```

Record the transaction hash in your report.

- [ ] **Step 2: Run it against real testnet, once**

Ask the coordinator for the environment values rather than inventing them. If they are not available, leave the test written and gated, run nothing, and say so — the task is complete either way, but do not report a live pass that did not happen.

If the pool rejects the transaction, **stop and report the exact error**. A `PoolRejectedTransactionByMinFeeRate` means the fee math is wrong; a `-302` means a capacity floor is wrong; a `-52` means the multi-input witness assembly is wrong. Each is the finding this plan was built to surface, and none should be worked around by adjusting a constant until it passes.

- [ ] **Step 3: CI**

In `.github/workflows/ci.yml`, in the `rust` job after the existing chain-backend steps:

```yaml
      - name: cargo test (tx-builder)
        run: cargo test -p lantern-tx-builder
```

The live test is not added to CI. It is doubly gated and must never depend on a public node from a workflow.

- [ ] **Step 4: Docs**

In `README.md`, replace the status line:

```markdown
**Status:** Plans 1a (scaffold), 1b (vault), 1c (accounts + secp256k1 signing), 1d (chain backend + light-client supervision) and 1e (transaction builder) complete. Next: plan 1f (Tauri commands + extension host).
```

Append a "Refinements during implementation" section to the 1e spec recording anything that diverged from the design, with the reason. If nothing diverged, say that explicitly — a spec that claims to match reality earns its authority by being checked.

- [ ] **Step 5: Full gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p lantern-chain-backend --all-targets --features testing -- -D warnings
cargo clippy -p lantern-vault --all-targets --no-default-features -- -D warnings
cargo test --workspace
cargo test -p lantern-chain-backend --features testing
cargo test -p lantern-chain-backend
cargo test -p lantern-vault --no-default-features
cargo doc --workspace --no-deps
```

All must pass. Confirm `pgrep -af fake_light_client` shows nothing.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-core .github/workflows/ci.yml README.md docs/superpowers/specs
git commit -m "test(wallet-core): live-gated testnet transfer, CI wiring and docs"
```

Do not tag. Tagging happens at branch-finishing, after the whole-branch review.

---

## Self-Review

**Spec coverage.** §1's seven goals map to: selection and change (Task 12), measured fee (Tasks 4, 5, 12, 13), computed floors (Task 1), whole-transaction signing (Tasks 7, 8, 10), one seed exposure (Task 8's single call, Task 14's single `seed_for`), actionable refusals (Task 9), pool acceptance (Task 16). §3.1's layout is Tasks 1–12; §3.2's dependency exception is Task 1 Step 1; §3.3 is Task 4. §4.1 is Task 2, §4.2 is Tasks 7–8 and 10, §4.3 is Tasks 10 and 12. §5 is Task 12; §5.1's termination argument is that task's module doc; §5.2 is Task 1. §6 is Task 9. §7.1's three tiers are Tasks 12/13/15, 14, and 16; §7.2's four required tests are Tasks 11, 13, 15 and Task 12's witness-padding test; §7.3's probes are named in Tasks 10, 12, 13, 14. §8's funded key is Task 16 Step 2.

**Placeholders.** Three steps deliberately defer a value to the implementer with a defined fallback and a reporting requirement: the consensus size ceiling (Task 4 Step 1), the multi-input testnet transaction (Task 11 Step 1), and the golden encoding (Task 15 Step 2). Each names where to look, what to do if the search fails, and what to report — these are research steps, not gaps.

**Type consistency.** `WitnessSize` is introduced in Task 2 and consumed in Tasks 3, 9, 10, 12. `InputContext`/`SigningGroup`/`SigningRequest`/`SignedWitness` are introduced in Task 7 and consumed in Tasks 8, 9, 10, 12, 14. `Candidate` is introduced in Task 6 and implemented for `InputContext` in Task 9 — which is the one ordering dependency that would break if tasks ran out of order, so Task 6's interface block says so. `build_transfer` is defined in Task 12 and consumed in Tasks 13, 14, 15. `measure`/`fee_for`/`min_capacity`/`order_candidates`/`placeholder_witness` all keep the names they are given.

**Known risks for the executor.**
- Task 3's `ckb-types` builder API is written from the molecule layout rather than from a compiled example. The assertions are pinned independently and must not be weakened; adapt the construction if the API differs.
- Task 11 depends on finding a suitable public transaction. If none is findable, report rather than substituting a self-generated vector.
- Task 14's `decode_address` is the only genuinely new cryptographic-adjacent code outside the signer; it must reuse `account-registry`'s existing encoder constants rather than restating the format.

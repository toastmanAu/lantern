# Plan 1e — Transaction Builder Design

**Status:** approved 2026-09-12, ready for implementation planning
**Extends:** the foundation spec
`2026-04-08-foundation-design-tauri-edition-v1.1.md` §13 (witness sizing) and
the lock-module contract shipped in plan 1c.

---

## 1. Goal

Give Lantern the ability to spend. Plan 1e builds, signs and broadcasts a plain
CKB capacity transfer from a single account, and proves it by having the CKB
testnet pool accept one.

Seven things must be true when this lands:

1. A transfer selects input cells, sizes witnesses, computes a fee the pool
   accepts, and produces a change cell that is spendable.
2. Fee estimation is derived from the bytes actually sent, not from an
   arithmetic model of them.
3. Every capacity floor is computed from the real script, never hardcoded.
4. A lock module receives the whole resolved transaction, not a digest.
5. Signing every input of an account costs exactly one seed exposure.
6. The builder refuses, with an actionable error, any transaction the chain
   would reject for capacity or size reasons.
7. A real transaction is accepted by the testnet pool and its hash recorded.

## 2. Non-goals

Out of scope, each deferred to a named later plan:

- **sUDT / xUDT token transfers** — type scripts and token change cells. A
  later plan; it stresses cell-data handling, a different axis from the
  witness-size generality this plan exists to establish.
- **Nervos DAO** deposit and withdraw. Later plan. Note for whoever writes it:
  the phase-2 lock period is measured from the **deposit** block, not the
  withdraw block — recorded as a real bug in `ckb-transactions.feedback.md`
  (2026-06-16).
- **Multi-account sends.** The signing model is grouped so this is additive
  rather than a rewrite, but 1e spends from one account only.
- **Tauri commands and UI.** Plan 1f.
- **Hardware and PQ signers.** Plans 2, 4, 5. Plan 1e shapes the trait they
  implement and ships only `signer-secp256k1`.
- **Fee-rate estimation from the node.** CKB's fee market is effectively flat;
  the pool minimum is the operative floor. A caller-supplied rate with a
  conservative default is sufficient and avoids depending on an RPC whose
  semantics vary between node versions.

## 3. Architecture

### 3.1 The pure/impure boundary

`crates/tx-builder` performs no I/O, contains no `async`, and does not depend
on `chain-backend`. It receives candidate cells and parameters and returns an
unsigned transaction plan. `wallet-core` owns every side effect: fetching
candidates, dispatching to the lock module, and broadcasting.

```
crates/tx-builder/src/
  lib.rs        re-exports
  types.rs      TransferRequest, TransferPlan
  capacity.rs   occupied-capacity floor for a script; shannon/CKB units
  witness.rs    WitnessArgs placeholder construction from a WitnessSize
  size.rs       transaction byte size, by measurement
  fee.rs        fee math and the selection/fee fixpoint
  select.rs     largest-first candidate selection
  build.rs      build_transfer(request) -> Result<TransferPlan, BuildError>
  error.rs      BuildError
```

**Type ownership.** `WitnessSize`, `SigningRequest`, `InputContext`,
`SigningGroup` and `SignedWitness` belong to `sdk-schema`, alongside the
`LockModule` trait that consumes them (§4). `tx-builder` imports them; it owns
only `TransferRequest`, `TransferPlan` and `BuildError`. A `TransferPlan`
carries the pieces `wallet-core` assembles into a `SigningRequest` — the plan
is what the builder produces, the request is what the signer consumes, and
keeping them distinct is what lets `tx-builder` stay free of the trait.

The split follows a lesson already recorded in this project's feedback log
(2026-06-22, subcell): separating pure assembly from the I/O-bearing wrapper is
what made the assembly testable at unit speed. Every decision that can be
wrong here — capacity floors, the fee fixpoint, witness sizing, the size
ceiling — is a pure function reachable from a test with no node.

### 3.2 Dependency exception

`tx-builder` depends on `ckb-types` and `ckb-jsonrpc-types` directly. This is a
deliberate, documented exception to plan 1d's ruling that downstream crates
consume chain types through `chain-backend`'s re-exports.

That ruling existed to stop crates pinning chain types independently, and the
workspace `Cargo.toml` already guarantees version consistency. Routing a pure
crate's type dependency through `chain-backend` would pull `reqwest` and
`tokio` into it to obtain type definitions, defeating the boundary in §3.1.
`wallet-core` continues to use the re-exports.

### 3.3 Measurement, not estimation

`size.rs` constructs the real molecule `Transaction` with correctly sized
witness placeholders and takes `.as_slice().len() + 4`. It does not sum field
widths.

This is the structural answer to the most repeated failure in this project's
history. Every under-count recorded in `ckb-transactions.md` §1 — the 560-byte
JoyID gap, the 2431-byte mint gap, the 384-byte loss through CCC's
`completeFee` clone path — is an arithmetic estimate diverging from what was
serialised. If the artifact measured is the artifact broadcast, that divergence
cannot exist.

## 4. The lock-module contract

Two changes to `LockModule` in `crates/sdk-schema`.

### 4.1 Witness size becomes an enum

Replacing `fn witness_lock_len(&self) -> usize`:

```rust
pub enum WitnessSize {
    Fixed(usize),
    Variable { min: usize, max: usize },
}

impl WitnessSize {
    /// The value fee estimation must use.
    pub const fn for_fee_estimate(&self) -> usize;
}

fn witness_size(&self) -> WitnessSize;
```

`for_fee_estimate` returns the maximum for `Variable`. Undercharging gets the
transaction rejected by the pool; overcharging costs a rounding error. Falcon
signatures are variable-length (666–1462 bytes), which is why the foundation
spec §13 requires this shape and prefers padded variants for v0.1.

### 4.2 Signing becomes async, whole-transaction and grouped

Replacing `fn sign_digest(&self, seed, derivation, digest) -> Result<Vec<u8>>`:

```rust
pub struct SigningRequest {
    /// Unsigned, with witness slots padded to inputs.len() and the group's
    /// first slot carrying a placeholder-sized WitnessArgs.
    pub tx: Transaction,
    /// Resolved, index-aligned with tx.inputs.
    pub inputs: Vec<InputContext>,
    /// The script groups this module is being asked to sign.
    pub groups: Vec<SigningGroup>,
}

pub struct InputContext {
    pub out_point: OutPoint,
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
    pub data: Vec<u8>,
}

pub struct SigningGroup {
    pub lock_hash: [u8; 32],
    /// Ascending. The first is the group's witness slot.
    pub input_indices: Vec<usize>,
    pub derivation: Derivation,
}

pub struct SignedWitness { pub index: usize, pub witness: Vec<u8> }

async fn sign(&self, seed: &[u8], req: &SigningRequest)
    -> Result<Vec<SignedWitness>, LockError>;
```

Four properties this establishes:

**Groups, not a flat index list.** RFC 0019 runs a lock script once per script
group — the inputs sharing a lock hash — and only the group's first witness
carries the signature. Ten inputs from one address produce one signature, not
ten. Modelling groups from the start is required even though 1e has exactly
one, because the sighash digest is defined per group.

**Resolved input contents.** Each `InputContext` carries the input's real lock,
type and data. A module that reconstructs its own signing stream — ML-DSA and
any cobuild-style lock — needs the truth. The wyltek-wallet code-46 bug
(feedback log, 2026-06-22) was a builder handing its PQ signer hardcoded empty
input contents: correct only while the lock held nothing but pure change cells,
and silently wrong the moment a token or message cell was selected. Here there
is nothing to misreport.

**Modules return only what they own.** `wallet-core` rejects any returned index
outside the module's own groups. With third-party extension signers, the
alternative is a buggy or hostile module overwriting another lock's witness.

**One seed exposure.** A single call carries every group, so the module loops
internally. This satisfies plan 1c's carry-over structurally rather than by a
discipline a future caller must remember.

### 4.3 Witness padding

Witness slots are padded to `inputs.len()` before any digest is computed.

Plan 1c's RFC 0019 implementation was proven against a **one-input**
transaction, where padding is unobservable. The feedback log's 2026-05-23 entry
is this exact bug from the other side: a multi-input builder whose digests
verified locally and failed on-chain with `-52`, because each missing witness
slot shifted the stream by a `u64_le(0)` length prefix. One-input transactions
are immune; the bug appears at two.

## 5. The build pipeline

`build_transfer(TransferRequest) -> Result<TransferPlan, BuildError>`

The request carries candidates, recipient script, amount, change lock, the
owning module's `WitnessSize` and cell deps, and a fee rate. Extracting the
module-derived values in `wallet-core` is what keeps `tx-builder` free of the
`LockModule` trait.

1. **Validate before building.** Reject `amount < min_capacity(recipient)`
   naming the floor. This is the `-302 InsufficientCellCapacity` trap — hit
   twice in the feedback log, at 5 CKB and at 30 CKB — caught before a
   transaction exists rather than by the pool.
2. **Sort candidates by capacity, descending.**
3. **Fixpoint.** Each iteration assembles the real transaction for the current
   selection, measures it (§3.3), computes `fee = ceil(size × rate / 1000)`,
   and derives `change = inputs − amount − fee`. Take another candidate while
   `change < 0`, or while `0 < change < min_capacity(change_lock)`. Terminate
   on `change == 0` exactly, or `change ≥ floor`. Exhausted candidates is
   `InsufficientFunds` or `ChangeBelowFloor` (§6).
4. **Size ceiling.** Check the measured size against CKB's maximum transaction
   size. The implementer must pin this constant from consensus rather than
   from this document; the foundation spec cites 512KB.
5. **Cell deps** for the lock script are attached from the module's list.
   Omitting them fails only on-chain, as `ScriptNotFound` — recorded twice in
   the feedback log, including a subcell spend path where 224 passing mock
   tests missed it.

### 5.1 Why the fixpoint terminates

Fee depends on size, size depends on input count, and input count depends on
fee. The loop is nonetheless strictly progressing:

A `CellOutput`'s capacity is a fixed-width `u64`, so the change **value** does
not affect serialised size — only whether a change output **exists** does. For
a fixed input count and change-presence, size is fully determined, and each
additional input increases it by a known, positive amount. Each iteration
consumes one candidate, so the loop is bounded by the candidate count.

This termination argument is a property of the real serialiser because size is
measured rather than modelled. An arithmetic estimate could drift from the
serialiser and invalidate the argument without any test noticing.

### 5.2 Capacity floors

A cell must hold enough capacity to store itself: 8 bytes for the capacity
field plus the serialised lock script (32 code_hash + 1 hash_type + args), plus
any type script and data. For `secp256k1_blake160` with 20-byte args this is 61
bytes, hence 61 CKB. A 32-byte-args PQ lock is 73 CKB.

`capacity.rs` computes this from the script. It is never hardcoded — the
feedback log's 2026-04-24 entry records the same requirement reached from the
other direction ("computed via `lock.occupiedSize`, not hard-coded").

## 6. Error model

Every variant names the quantity that would resolve it.

| Variant | Meaning |
|---|---|
| `AmountBelowFloor { amount, floor }` | The recipient's lock cannot hold that little |
| `InsufficientFunds { needed, available, shortfall }` | Balance does not cover amount plus fee |
| `ChangeBelowFloor { change, floor }` | Affordable, but the leftover would be unspendable |
| `TransactionTooLarge { size, limit }` | Exceeds the consensus size ceiling |
| `NoSpendableCells` | Nothing to select from |

`ChangeBelowFloor` is deliberately distinct from `InsufficientFunds`. The user
has the money, the amount and fee are covered, and the remainder lands below
the floor — reporting "insufficient funds" would be false. The message must say
the leftover is below the minimum a cell can hold, and that a different amount
or a consolidation resolves it.

Capacities, lock args and scripts are public chain data, so no variant risks
leaking secret material. This remains subject to the standing rule that no
secret appears in any `Display`, `Debug`, or log line.

## 7. Testing

### 7.1 Three tiers

**Pure unit tests** carry the weight, since the crate is pure: capacity floors,
the fee fixpoint, selection, and size measurement, driven with adversarial
inputs — fragmented wallets, exact change, change at floor−1 / floor / floor+1,
PQ-sized witnesses, and the size ceiling.

**Fake-node integration** exercises `wallet-core`'s send path end to end —
fetch, build, sign, broadcast — asserting what actually reaches
`send_transaction` on the wire.

**A live gated broadcast** spends real testnet CKB and records the hash. It
follows plan 1d's convention: `#[ignore]` plus a `LANTERN_LIVE_TESTNET=1`
environment gate, so a skipped run reports `ignored` rather than `ok` and can
never be mistaken for a live pass in CI. Broadcasting requires both the
environment variable and `-- --ignored`, so no CI edit can turn it on by
itself.

### 7.2 Tests this project's history requires

- **Multi-input sighash**, at least two inputs in one group, checked against a
  third oracle — a real multi-input testnet transaction decoded and its digest
  recomputed. Plan 1c's vector was one input, which cannot exercise §4.3.
- **A fee property test:** across randomised wallets and amounts, the fee is
  never below `measured_size × rate`. Fee under-payment is the most repeated
  failure in the feedback log, across JoyID, CCC, and the sUDT builder.
- **A golden serialisation vector**, byte-compared against a known-good
  encoding produced independently. The 2026-08-15 entry records why: a
  memorised CKB constant was wrong, and only a third oracle distinguished that
  from a bad implementation.
- **Cell-dep presence** asserted on the built transaction, since its absence
  fails only on-chain.

### 7.3 Falsifiability

Every guard gets a mutation probe named in its task brief before
implementation: break the property, confirm the named test goes red, restore,
confirm green. Plan 1d produced eight tests that asserted less than their names
claimed — including one that asserted a bug as its specification — and probes
found all of them. Specifying probes up front is cheaper than discovering
hollow tests afterwards, and is now standing practice.

## 8. Dependencies and open items

**A funded secp256k1 testnet address is required** before the live broadcast
test can run. The Quantum Purse throwaway recorded in memory is SPHINCS+ and
cannot sign here. The test is written and gated regardless; only the run
depends on the key. No transaction is ever broadcast autonomously.

**Carried in from plan 1d**, relevant to whoever implements the send path: a
supervised light-client restart currently leaves `EmbeddedLight` reporting
`Connecting` for the rest of the session, because the registration list is
per-client state and the client is rebuilt when the port moves. It fails
closed. The fix belongs to plan 1f, but a send path that gates on
`is_usable()` will observe it.

## 9. Decision log

| Decision | Alternative rejected | Why |
|---|---|---|
| Plain transfer only | Tokens and/or DAO in 1e | Keeps the plan on the witness-size axis that makes this wallet distinctive; token support stresses cell data instead |
| Async, whole-transaction signing | Batched `sign_digest` | A digest-only interface is what produced code-46; every non-trivial signer needs the transaction, and changing the trait later touches five implementors |
| Largest-first selection | Smallest-first; pluggable strategy | Fewest inputs, smallest transaction, lowest fee. A second strategy has no caller yet |
| Grow selection past sub-floor change | Fold change into the fee | The floor is ~61 CKB, not dust; silently burning that is a bad outcome, unlike Bitcoin where sub-dust change is worth less than its own bytes |
| Pure core, orchestrator in wallet-core | Builder owns the backend | Fee and capacity math is testable without a node; the crates stay independently evolvable |
| Measure size | Compute size analytically | Removes the entire under-count class recorded in §1 of the CKB ruleset |
| Direct `ckb-types` dependency | Route through `chain-backend` | Would drag `reqwest`/`tokio` into a pure crate for type definitions |
| Live broadcast ends the plan | Stop at a signed transaction | Only pool acceptance exercises min-fee-rate, capacity and witness validity — the gap this project's log records most often |

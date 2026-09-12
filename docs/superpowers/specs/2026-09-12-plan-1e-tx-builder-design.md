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

## 10. Refinements during implementation

A great deal diverged from this document as written, all of it discovered by
implementation and settled by controller ruling during the plan's execution
(the full reasoning for each lives in the plan's SDD ledger). None of it
changes §1's goals or §9's decisions; each is recorded here because a spec
that claims to match reality earns its authority by being checked.

**§3.3's size ceiling: this document was right, an intermediate ruling was
wrong, and the module no longer restates the formula.** `MAX_TX_SIZE` is
`512_000` — `ckb_types::core::tx_pool::TRANSACTION_SIZE_LIMIT`,
`ckb-types-1.1.1/src/core/tx_pool.rs:309`, referenced through the `ckb-types`
`tx-builder` already depends on rather than copied as a literal.

An earlier ruling during implementation set the constant to `597_000`
(`MAX_BLOCK_BYTES`) on the stated premise that `512_000` had *no reachable
provenance in the pinned crates*. That premise was false. The constant was
there the whole time, in a crate already in this crate's dependency graph, and
its own doc comment states the semantics we want exactly: "The maximum size of
the tx-pool to accept transactions. The ckb consensus does not limit the size
of a single transaction, but if the size of the transaction is close to the
limit of the block, it may cause the transaction to fail to be packed." The
search that reported nothing looked at `ckb-chain-spec` and `ckb-constant` and
did not look at `ckb-types`; the ruling that followed it is recorded here as
mistaken rather than quietly reversed, because a spec's authority comes from
being checked and this is the check.

What survives from that reasoning is the observation, which is correct and
worth keeping: **CKB consensus imposes no per-transaction byte ceiling.**
There are two real limits and neither is a per-transaction consensus rule —
`MAX_BLOCK_BYTES` (`597_000`, `ckb-chain-spec-1.1.1/src/consensus.rs:83`,
RFC 0020) bounds a whole block, and `TRANSACTION_SIZE_LIMIT` bounds what a
pool will accept. The pool's is the tighter one and it is the first thing a
broadcast meets, so that is where we guard: a transaction above it never
reaches a block at all, whatever consensus would have allowed.

The caveat that remains points the other way. A node operator can configure a
*lower* `max_tx_size` than the default, which this crate cannot see, so
`TransactionTooLarge` is a necessary condition for acceptance and not a
sufficient one — unreached in plan 1e either way, where a plain transfer is on
the order of 1 KB.

Separately, `size.rs`'s `measure` does not build the sum §3.3 describes
(`.as_slice().len() + 4`). It delegates to
`ckb_types::packed::Transaction::serialized_size_in_block()` — the identical
function CKB's own transaction verifier uses to compute the size its
fee-rate check divides by (`ckb-gen-types`'s
`serialized_size_in_block`, consumed by `ckb-verification`'s
`TransactionVerifier`). This makes §3.3's promise — the artifact measured is
the artifact the pool will price — true by construction rather than by a
constant this crate maintains, and removes the last hand-written arithmetic
from the sizing path. The `+ 4` was correct all along; it is molecule's
`NUMBER_SIZE`, the offset entry a transaction occupies in a block's
transaction `dynvec`, not a number this crate needed to know once the delegate
was in place.

**§4.2's `SigningGroup`/selection ordering needed a total order, and
`Candidate::tie_break` changed shape to give it one.** As specced,
`tie_break` returned `&[u8]` (a transaction hash). Two cells from the *same*
transaction — an entirely ordinary shape, produced by any transfer paying two
outputs to one address — tie under that comparator, and the tie then falls
through to `sort_by`'s stability, which resolves it by *input order*: a
property of the indexer's paging, not of this crate. `Candidate::tie_break`
now returns `(&[u8], u32)` — the tx hash together with the output index —
which uniquely identifies an `OutPoint` and is therefore genuinely total,
still without allocating. §5's fixpoint and §7.2's golden vector both depend
on selection order being deterministic from the candidates alone, which is
the property this closes.

**§4.2's `SigningGroup` needed a `Derivation` that a builder could get right by
construction.** As specced, `SigningGroup.derivation` had no producer in
`tx-builder` and would have been left `{0, 0}` for `wallet-core` to overwrite
— a discipline a future caller must remember, which is exactly the failure
mode §4.2's "one seed exposure" property is meant to make structural rather
than conventional. `TransferRequest` now carries a `derivation: Derivation`
field, and `build_transfer` fills every group from it, so a `TransferPlan` for
a non-default account can never carry a knowingly-wrong key index.

**§3.2's dependency exception did not go far enough, and this document's
"only" was unimplementable as written.** §3.2 states the `ckb-types` /
`ckb-jsonrpc-types` exception for `tx-builder` alone. But §3.1 itself assigns
`SigningRequest` (carrying a `Transaction`), `InputContext` (carrying
`OutPoint`, `Script`) and the `LockModule` trait to `sdk-schema`, and those
types cannot be declared without the same dependency — so the literal word
"only" contradicts §3.1's own type-ownership assignment. Both crates are
added to `sdk-schema` (§4.2's types) and to `signer-secp256k1` (whose `sign`
implementation needs packed transaction types), and the exception's intent —
no *new* third-party crate enters the workspace, and chain types stay
version-consistent via the workspace pin — holds in both cases: all three
crates use the same already-pinned `"=1.1.1"`, and no new `[[package]]` entry
appears in the lockfile.

**§4 gained a method §4 did not specify: `LockModule::cell_deps(network) ->
Vec<CellDep>`.** Needed once `wallet-core`'s send path had to supply
`TransferRequest.cell_deps` from something other than a hardcoded constant —
a lock module is the only thing that knows what its own script needs as a
dependency, and hardcoding it in `wallet-core` would have broken the moment a
second lock type existed.

**§4.2's grouped-signing contract turned out to need one more invariant than
written: the secp256k1 module refuses to sign unless the group's first
witness slot is byte-for-byte its own expected placeholder.** The digest RFC
0019 defines is taken over whatever bytes sit in that slot; on chain, the lock
recomputes the same digest from the *broadcast* witness with its lock zeroed
in place. Nothing in the contract as specced required those two to match, and
three distinct builder mistakes — a shorter or longer placeholder, one
carrying stray `input_type`/`output_type` fields, or an empty slot entirely —
would each have produced a wrong signature that only fails on chain, with
nothing local to catch it. The signer now checks equality against its own
placeholder before signing and refuses otherwise. The corollary the PQ plans
must reconcile: this check is unsatisfiable as written for a *variable*-length
lock (§4.1), because `for_fee_estimate()` returns the maximum and a real
signature under that maximum would be shorter than the placeholder it is
checked against. The reconciliation is to pad the real signature up to the
placeholder's length on chain, never to shrink the placeholder to the
signature — sizing the lock to the signature is circular against a
self-committing digest.

**§3.1's "resolved input contents" property needed a filter at the one call
site that collects them, and the indexer query needed an explicit flag it did
not carry.** `wallet-core`'s candidate collection now filters to cells under
exactly the sending lock with no type script and no data — otherwise an sUDT
or notification cell could be selected as if it were plain capacity, which is
the exact production failure this project's feedback log already records
under a different codebase (lock error 46, 2026-06-22). That filter is only
as good as the data it is filtering: `CellQuery`'s wire request now sends
`with_data: Some(true)` explicitly rather than relying on an unstated indexer
default, and a cell whose `output_data` the node did not return is treated as
unknown-and-excluded, never as confirmed-empty-and-accepted — a cell can fail
to be selected this way, which fails closed (a smaller apparent balance)
rather than open.

**§7.2's fee property test needed an oracle independent of the function it
tests, and the spec's own wording was the thing that caught the gap.**
§7.2 requires the fee to be checked against `measured_size × rate`
independently; an initial version compared `plan.fee` against a call back into
`fee_for(plan.size, req.fee_rate)`, which is `build_transfer`'s own internal
call restated — a tautology that cannot detect a rounding-direction bug in
`fee_for`, the exact class of bug this test exists to catch. Forcing `fee_for`
to floor instead of `div_ceil` produced zero failures under that oracle. The
test now asserts the relation directly, `plan.fee * 1000 >= plan.size * rate`
in `u128`, with no call into `fee_for` at all — which is exactly §7.2's
sentence turned into an assertion, and which fails immediately (468 of 469
generated cases) when `fee_for` is forced to floor.

**§7 gained a signing entry point §7 did not name: `WalletCore::send`
replaces `SigningCoordinator`.** `SigningCoordinator` (from plan 1c) is
removed rather than kept alongside `send`: a public "sign whatever request I
hand you" surface over an unlocked vault is a foot-gun once a caller can ask
the wallet to build and sign its own transfer, and `send` is the single
signing entry point §7 already describes in spirit.

**What is not proven.** Every claim above is checked against a pure function,
a recorded testnet vector, or an independent encoder — never a broadcast. No
transaction produced by this crate and `wallet-core::send` has been submitted
to a live node or accepted by a pool; `crates/wallet-core/tests/live_send.rs`
exists, is gated behind `#[ignore]` and an environment variable, and has not
been run, because no funded secp256k1 testnet key is available to this
project's automation. This remains true until someone runs it by hand with a
funded key, per §8.

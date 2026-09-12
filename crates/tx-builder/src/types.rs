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
        // The out point's tx hash: stable (a 32-byte view borrowed straight
        // from `self`, so it never allocates) and deterministic across runs.
        // It does not distinguish two candidates that are different outputs
        // of the *same* transaction (equal tx_hash, different index) — an
        // owned tx_hash+index composite would, but `Candidate::tie_break`
        // returns `&[u8]` borrowed from `self`, and `InputContext` (Task 7,
        // sdk-schema) has no field already holding that concatenation. Two
        // same-tx candidates colliding here only matters when their
        // capacities also tie, and `order_candidates`'s sort is stable, so
        // the result stays deterministic for a given input order even then.
        self.out_point.tx_hash.as_bytes()
    }
}

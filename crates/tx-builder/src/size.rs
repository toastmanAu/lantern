//! Transaction size, by measurement.
//!
//! This module deliberately does not compute size by summing field widths.
//! Every fee under-count in this project's history is an arithmetic model
//! diverging from what was actually serialised — the 560-byte `JoyID` gap,
//! the 2431-byte mint gap, the 384-byte loss through a clone path. If the
//! artifact measured is the artifact broadcast, that divergence cannot
//! exist.

use ckb_types::packed::Transaction;

/// Practical ceiling on a single transaction's serialised size.
///
/// Provenance: `MAX_BLOCK_BYTES` in
/// `ckb-chain-spec-1.1.1/src/consensus.rs:83`
/// (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`),
/// `pub const MAX_BLOCK_BYTES: u64 = TWO_IN_TWO_OUT_BYTES * TWO_IN_TWO_OUT_COUNT`
/// = 597 * `1_000` = `597_000`. Confirmed against RFC 0020 (vendored at
/// `research/ckb-ecosystem-locks/raw/rfcs/rfcs/0020-ckb-consensus-protocol/`,
/// line 366: `| block size limit | MAX_BLOCK_BYTES | 597000 |`).
///
/// This governs the maximum serialised **block** size, not a transaction —
/// searching the vendored research tree and the pinned `ckb-chain-spec`
/// and `ckb-constant` crates (both at workspace-pinned version 1.1.1) turned
/// up no dedicated per-transaction consensus byte ceiling; CKB consensus
/// scripts do not enforce one directly. `MAX_BLOCK_BYTES` is the nearest
/// real limit: any single transaction that will ever be included in a block
/// must fit under it, so it is a correct (if loose) practical ceiling, not
/// a cited per-tx consensus rule. Treat this as a conservative upper bound.
pub const MAX_TX_SIZE: usize = 597_000;

/// Serialised size as the pool measures it.
///
/// Delegates to `Transaction::serialized_size_in_block`, defined at
/// `ckb-gen-types-1.1.1/src/extension/serialized_size.rs:42`
/// (`TransactionReader::serialized_size_in_block`, `self.as_slice().len() +
/// molecule::NUMBER_SIZE`) and exposed on the owned `Transaction` by the
/// `impl_serialized_size_for_entity!` macro at line 46 of the same file.
/// `molecule::NUMBER_SIZE` (4 bytes) is the offset entry a transaction
/// occupies in a block's transaction dynvec — not a constant we keep in
/// sync ourselves.
///
/// This is the same function CKB's own `SizeVerifier` calls to check a
/// transaction against the block byte ceiling: see
/// `ckb-verification-1.1.1/src/transaction_verifier.rs:315`,
/// `self.transaction.data().serialized_size_in_block()`. Delegating means
/// our measurement is the pool's measurement by definition, not by a
/// formula we hope stays in sync with it.
#[must_use]
pub fn measure(tx: &Transaction) -> usize {
    tx.serialized_size_in_block()
}

#[cfg(test)]
mod tests {
    use super::measure;
    use ckb_types::packed::Transaction;
    use ckb_types::prelude::*;

    #[test]
    fn a_default_transaction_measures_to_seventy_two_bytes() {
        // Pinned by direct observation of `measure` against a known input,
        // the way `witness.rs`'s `a_65_byte_lock_serialises_to_85_bytes`
        // pins 85 rather than restating the formula under test.
        let tx = Transaction::default();
        assert_eq!(measure(&tx), 72);
    }

    #[test]
    fn adding_a_witness_grows_the_measurement_by_the_witness_and_its_framing() {
        let bare = Transaction::default();
        let before = measure(&bare);

        let with_witness = bare
            .as_advanced_builder()
            .witness([0u8; 85].pack())
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

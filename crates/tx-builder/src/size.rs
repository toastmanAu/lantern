//! Transaction size, by measurement.
//!
//! This module deliberately does not compute size by summing field widths.
//! Every fee under-count in this project's history is an arithmetic model
//! diverging from what was actually serialised — the 560-byte `JoyID` gap,
//! the 2431-byte mint gap, the 384-byte loss through a clone path. If the
//! artifact measured is the artifact broadcast, that divergence cannot
//! exist.

use ckb_types::packed::Transaction;

/// The largest transaction a CKB tx-pool will accept, in serialised bytes.
///
/// Provenance: `TRANSACTION_SIZE_LIMIT` in
/// `ckb-types-1.1.1/src/core/tx_pool.rs:309`
/// (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`),
/// `pub const TRANSACTION_SIZE_LIMIT: u64 = 512 * 1_000`. Not restated:
/// referenced through the same `ckb-types` this crate already depends on, so
/// a version bump that moved it would fail to compile rather than leave a
/// stale number behind.
///
/// Its own doc comment states the semantics exactly: *"The maximum size of
/// the tx-pool to accept transactions. The ckb consensus does not limit the
/// size of a single transaction, but if the size of the transaction is close
/// to the limit of the block, it may cause the transaction to fail to be
/// packed."*
///
/// Note on consensus: there is indeed **no per-transaction consensus byte
/// ceiling**. The nearest consensus figure is `MAX_BLOCK_BYTES` — 597,000,
/// `ckb-chain-spec-1.1.1/src/consensus.rs:83`, RFC 0020's `| block size
/// limit | MAX_BLOCK_BYTES | 597000 |` — which bounds a whole *block*. So
/// there are two real limits, and the pool's is the tighter one: a
/// transaction over 512,000 bytes is refused entry to the pool and therefore
/// never reaches a block at all, whatever consensus would have permitted. We
/// guard at the tighter one, because the first thing a broadcast meets is
/// the pool.
///
/// The remaining caveat is in the other direction: a node operator can
/// configure a *lower* `max_tx_size`, which this crate cannot see, so
/// [`crate::BuildError::TransactionTooLarge`] is a necessary condition for
/// acceptance and not a sufficient one.
///
/// The cast is `u64 -> usize`. `usize::try_from` is not available in a
/// `const`, so the bound is checked by the const block instead: a target
/// where the limit did not fit fails to compile rather than wrapping to a
/// smaller ceiling.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the const assertion in the initialiser rules it out at compile time"
)]
pub const MAX_TX_SIZE: usize = {
    const { assert!(ckb_types::core::tx_pool::TRANSACTION_SIZE_LIMIT <= usize::MAX as u64) };
    ckb_types::core::tx_pool::TRANSACTION_SIZE_LIMIT as usize
};

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
    use super::{MAX_TX_SIZE, measure};
    use ckb_types::packed::Transaction;
    use ckb_types::prelude::*;

    #[test]
    fn the_size_ceiling_is_the_tx_pools_limit_and_is_tighter_than_a_blocks() {
        // `MAX_BLOCK_BYTES`, 597_000, `ckb-chain-spec-1.1.1/src/consensus.rs:83`
        // and RFC 0020's `| block size limit | MAX_BLOCK_BYTES | 597000 |`.
        // It bounds a whole BLOCK; the pool's limit bounds one transaction.
        const MAX_BLOCK_BYTES: usize = 597_000;

        // The literal is restated here deliberately rather than compared back
        // to the constant it is defined from, which would assert nothing: a
        // `ckb-types` bump that moved `TRANSACTION_SIZE_LIMIT` should fail
        // here and be looked at, not be adopted silently.
        assert_eq!(MAX_TX_SIZE, 512_000);

        // And it must stay the tighter of the two real limits: a transaction
        // between them would clear consensus and still never be relayed,
        // because the pool refuses it first.
        const {
            assert!(
                MAX_TX_SIZE < MAX_BLOCK_BYTES,
                "guarding at the block ceiling would admit transactions no pool accepts"
            );
        }
    }

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

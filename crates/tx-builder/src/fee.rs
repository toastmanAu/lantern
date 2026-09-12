//! Fee arithmetic.
//!
//! The rate is shannons per 1000 bytes. The pool minimum is 1000, i.e. one
//! shannon per byte.
//!
//! `DEFAULT_FEE_RATE` is that minimum rather than a padded figure. On the
//! **serialisation axis** that is sound, and it is a consequence of measuring
//! instead of estimating: a placeholder is the exact size of a `Fixed`
//! signature and the maximum for a `Variable` one, so the measured size is an
//! upper bound on what gets broadcast and the byte term of the fee can never
//! fall short. The customary 20% buffer exists to absorb estimation drift
//! that this builder does not have. That is the whole of what the 20% is not
//! needed for — it is not a claim that fee under-payment is impossible.
//!
//! # CKB prices weight, not bytes
//!
//! A pool orders transactions by *weight*, and weight has two axes:
//!
//! ```text
//! get_transaction_weight(tx_size, cycles)
//!     = max(tx_size, cycles × DEFAULT_BYTES_PER_CYCLES)
//! ```
//!
//! (`ckb-types-1.1.1/src/core/tx_pool.rs:298`, with
//! `DEFAULT_BYTES_PER_CYCLES = 0.000_170_571_4` at line 279.) A transaction
//! whose script cycles outweigh its bytes is priced on **cycles**, a quantity
//! this crate cannot measure: `tx-builder` never executes a script, and by
//! design has no backend to ask.
//!
//! So the precise statement is: measuring eliminates the under-count class on
//! the size axis, and says nothing about the cycle axis. For the secp256k1
//! lock the size term wins in the shapes this crate builds, but not by much —
//! the exact-landing no-change transfer is 355 bytes against a cycle weight
//! somewhere in the region of 290–410, and nothing here pins secp's real
//! cycle count, because nothing here can. For the Falcon and ML-DSA locks
//! this design is meant to generalise to, verification cycles dominate and a
//! purely size-derived fee would under-pay.
//!
//! The measurement that would settle it is a real one: the live broadcast
//! should record the `cycles` a node reports back from `get_transaction`, and
//! that number is what a cycle-aware fee model must be built against. Until
//! then this crate prices the axis it can see and states that it is one of
//! two.

/// Shannons per 1000 bytes. The CKB pool's minimum.
pub const DEFAULT_FEE_RATE: u64 = 1000;

/// Fee in shannons for a transaction of `size` bytes, rounded up.
#[must_use]
pub const fn fee_for(size: usize, rate: u64) -> u64 {
    (size as u64).saturating_mul(rate).div_ceil(1000)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FEE_RATE, fee_for};

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
        // We measure the transaction exactly rather than estimating its
        // SIZE, so the usual 20% drift buffer buys nothing on that axis.
        // It buys nothing on the cycle axis either — a margin chosen against
        // bytes does not track a weight computed from cycles. See fee.rs's
        // module doc for what is and is not ruled out.
        assert_eq!(DEFAULT_FEE_RATE, 1000);
    }
}

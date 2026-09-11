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
        // We measure the transaction exactly rather than estimating it, so
        // the usual 20% drift buffer buys nothing. See fee.rs's module doc.
        assert_eq!(DEFAULT_FEE_RATE, 1000);
    }
}

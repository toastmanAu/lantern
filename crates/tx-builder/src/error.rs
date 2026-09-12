//! Build failures.
//!
//! Every variant names the quantity that would resolve it. "Insufficient
//! funds" on its own tells a user nothing they can act on.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BuildError {
    #[error(
        "amount {amount} shannons is below the recipient lock's minimum cell capacity of {floor}"
    )]
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
        assert!(
            text.contains("100000500"),
            "shortfall must be stated: {text}"
        );
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
        assert!(
            text.contains("change") || text.contains("leftover"),
            "{text}"
        );
    }

    #[test]
    fn amount_below_floor_names_the_amount_and_the_floor() {
        let e = BuildError::AmountBelowFloor {
            amount: 500_000_000,
            floor: 6_100_000_000,
        };
        let text = e.to_string();
        assert!(text.contains("500000000"), "amount must be stated: {text}");
        assert!(text.contains("6100000000"), "floor must be stated: {text}");
    }

    #[test]
    fn transaction_too_large_names_the_size_and_the_limit() {
        let e = BuildError::TransactionTooLarge {
            size: 600_000,
            limit: 512_000,
        };
        let text = e.to_string();
        assert!(text.contains("600000"), "size must be stated: {text}");
        assert!(text.contains("512000"), "limit must be stated: {text}");
    }

    #[test]
    fn no_spendable_cells_states_the_condition() {
        // No quantity to name here — the honest assertion is that the
        // message says what's wrong, not that it invents a number.
        let text = BuildError::NoSpendableCells.to_string();
        assert!(!text.is_empty());
        assert!(text.to_lowercase().contains("no spendable cells"), "{text}");
    }
}

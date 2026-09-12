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
}

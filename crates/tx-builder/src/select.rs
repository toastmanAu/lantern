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

#[cfg(test)]
mod tests {
    use super::{Candidate, order_candidates};

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
        Cell {
            capacity,
            id: vec![id],
        }
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

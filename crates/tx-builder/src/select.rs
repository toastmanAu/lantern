//! Candidate ordering: largest capacity first.
//!
//! Fewest inputs means the smallest transaction and the lowest fee. It also
//! means the multi-input path is rare in a consolidated wallet — which is
//! exactly why the tests that exercise it must construct fragmented wallets
//! deliberately rather than hope to encounter one.

/// Something with a capacity that can be ordered deterministically.
pub trait Candidate {
    fn capacity(&self) -> u64;
    /// Used only to break capacity ties, so ordering is total: bytes plus a
    /// trailing discriminator, so two candidates that share the byte prefix
    /// (e.g. two outputs of the same transaction, which share `tx_hash`)
    /// still compare unequal as long as the discriminator differs (e.g.
    /// their output `index`). Rust orders tuples lexicographically, so no
    /// special comparator is needed.
    fn tie_break(&self) -> (&[u8], u32);
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
            .then_with(|| candidates[*a].tie_break().cmp(&candidates[*b].tie_break()))
    });
    order
}

#[cfg(test)]
mod tests {
    use super::{Candidate, order_candidates};

    struct Cell {
        capacity: u64,
        id: Vec<u8>,
        index: u32,
    }

    impl Candidate for Cell {
        fn capacity(&self) -> u64 {
            self.capacity
        }
        fn tie_break(&self) -> (&[u8], u32) {
            (&self.id, self.index)
        }
    }

    fn cell(capacity: u64, id: u8) -> Cell {
        Cell {
            capacity,
            id: vec![id],
            index: 0,
        }
    }

    fn cell_at(capacity: u64, id: u8, index: u32) -> Cell {
        Cell {
            capacity,
            id: vec![id],
            index,
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

    #[test]
    fn two_same_tx_outputs_with_equal_capacity_order_the_same_regardless_of_input_order() {
        // Same tx_hash (same `id`), same capacity, different output index —
        // exactly the case that collided when tie_break was tx_hash alone.
        // The result must not depend on which order the candidates arrived
        // in (e.g. from indexer paging), which is what a genuine total
        // order guarantees and a partial one does not.
        let forward = [cell_at(100, 7, 0), cell_at(100, 7, 1)];
        let reversed = [cell_at(100, 7, 1), cell_at(100, 7, 0)];

        // Whichever candidate carries the lower output index sorts first,
        // regardless of which slot it occupies in the input slice.
        assert_eq!(order_candidates(&forward), vec![0, 1]);
        assert_eq!(order_candidates(&reversed), vec![1, 0]);
    }
}

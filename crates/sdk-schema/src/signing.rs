//! What a lock module receives when asked to sign.
//!
//! A module gets the whole resolved transaction, not a digest. Every
//! non-trivial signer needs it: a post-quantum lock rebuilds its own
//! message stream from the inputs, a hardware wallet renders the
//! transaction on-device, a passkey shows it for approval. Handing over a
//! precomputed digest instead is what produced the code-46 divergence in a
//! sibling project, where a builder reported empty input contents that were
//! correct only while the lock held nothing but pure change cells.

use ckb_jsonrpc_types::{OutPoint, Script};
use ckb_types::packed::Transaction;
use std::collections::BTreeSet;

use crate::types::Derivation;

/// A resolved input: what the cell being spent actually contains.
#[derive(Debug, Clone)]
pub struct InputContext {
    pub out_point: OutPoint,
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
    pub data: Vec<u8>,
}

/// One RFC 0019 script group: the inputs sharing a lock script hash.
///
/// The lock script runs once per group and only the group's first witness
/// carries the signature, so ten inputs from one address produce one
/// signature rather than ten.
#[derive(Debug, Clone)]
pub struct SigningGroup {
    pub lock_hash: [u8; 32],
    /// Ascending. The lowest is the group's witness slot.
    pub input_indices: Vec<usize>,
    pub derivation: Derivation,
}

impl SigningGroup {
    /// The witness slot that carries this group's signature.
    #[must_use]
    pub fn witness_index(&self) -> Option<usize> {
        self.input_indices.iter().min().copied()
    }
}

/// Everything a module needs to produce witnesses.
#[derive(Debug, Clone)]
pub struct SigningRequest {
    /// Unsigned, with witness slots already padded to `inputs.len()` and
    /// each group's first slot carrying a placeholder-sized `WitnessArgs`.
    pub tx: Transaction,
    /// Resolved, index-aligned with `tx`'s inputs.
    pub inputs: Vec<InputContext>,
    /// The groups this module is being asked to sign.
    pub groups: Vec<SigningGroup>,
}

impl SigningRequest {
    /// Every witness index this module is permitted to write.
    ///
    /// The orchestrator rejects anything outside this set. With third-party
    /// extension signers the alternative is a buggy or hostile module
    /// overwriting another lock's witness.
    #[must_use]
    pub fn owned_indices(&self) -> BTreeSet<usize> {
        self.groups
            .iter()
            .flat_map(|g| g.input_indices.iter().copied())
            .collect()
    }
}

/// One witness a module produced, and where it belongs.
#[derive(Debug, Clone)]
pub struct SignedWitness {
    pub index: usize,
    pub witness: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::{SigningGroup, SigningRequest};
    use crate::types::Derivation;

    fn group(indices: Vec<usize>) -> SigningGroup {
        SigningGroup {
            lock_hash: [7u8; 32],
            input_indices: indices,
            derivation: Derivation {
                change: 0,
                index: 0,
            },
        }
    }

    #[test]
    fn a_groups_witness_slot_is_its_lowest_input_index() {
        // RFC 0019 puts the signature in the first witness of the group.
        assert_eq!(group(vec![2, 5, 9]).witness_index(), Some(2));
        assert_eq!(group(vec![]).witness_index(), None);
    }

    #[test]
    fn a_request_knows_every_index_its_module_may_write() {
        let req = SigningRequest {
            tx: ckb_types::packed::Transaction::default(),
            inputs: Vec::new(),
            groups: vec![group(vec![0, 3]), group(vec![1])],
        };
        let owned = req.owned_indices();
        assert!(owned.contains(&0) && owned.contains(&1));
        assert!(
            !owned.contains(&2),
            "an index outside the module's groups is not its to write"
        );
    }
}

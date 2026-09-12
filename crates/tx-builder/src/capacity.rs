//! Cell capacity floors.
//!
//! A CKB cell must hold enough capacity to store itself. The floor is
//! therefore a property of the cell's scripts and data, never a constant —
//! a PQ lock with 32-byte args needs 12 CKB more than secp256k1. Computing
//! it from the script is what stops the `-302 InsufficientCellCapacity`
//! class of failure, which surfaces only on-chain.

use ckb_jsonrpc_types::Script;

/// Shannons in one CKB.
pub const SHANNONS_PER_CKB: u64 = 100_000_000;

/// Bytes a script occupies: code hash, hash type, args.
#[must_use]
pub fn script_occupied_bytes(script: &Script) -> u64 {
    32 + 1 + script.args.as_bytes().len() as u64
}

/// Minimum capacity, in shannons, that a cell with these scripts and data
/// must hold to be valid.
///
/// The 8 bytes are the capacity field itself, which a cell must also pay for.
#[must_use]
pub fn min_capacity(lock: &Script, type_: Option<&Script>, data_len: u64) -> u64 {
    let bytes = 8 + script_occupied_bytes(lock) + type_.map_or(0, script_occupied_bytes) + data_len;
    bytes * SHANNONS_PER_CKB
}

#[cfg(test)]
mod tests {
    use super::{SHANNONS_PER_CKB, min_capacity, script_occupied_bytes};
    use ckb_jsonrpc_types::{JsonBytes, Script, ScriptHashType};
    use ckb_types::H256;

    fn script(args_len: usize) -> Script {
        Script {
            code_hash: H256([0u8; 32]),
            hash_type: ScriptHashType::Type,
            args: JsonBytes::from_vec(vec![0u8; args_len]),
        }
    }

    #[test]
    fn a_script_occupies_its_code_hash_hash_type_and_args() {
        assert_eq!(script_occupied_bytes(&script(20)), 53);
        assert_eq!(script_occupied_bytes(&script(32)), 65);
        assert_eq!(script_occupied_bytes(&script(0)), 33);
    }

    #[test]
    fn a_secp256k1_cell_floor_is_61_ckb() {
        // 8 capacity field + 32 code_hash + 1 hash_type + 20 args.
        // The number every CKB wallet trips over exactly once.
        assert_eq!(min_capacity(&script(20), None, 0), 61 * SHANNONS_PER_CKB);
    }

    #[test]
    fn a_32_byte_args_pq_cell_floor_is_73_ckb() {
        assert_eq!(min_capacity(&script(32), None, 0), 73 * SHANNONS_PER_CKB);
    }

    #[test]
    fn a_type_script_and_data_raise_the_floor() {
        // 8 + 53 (lock) + 53 (type) + 10 (data) = 124
        assert_eq!(
            min_capacity(&script(20), Some(&script(20)), 10),
            124 * SHANNONS_PER_CKB
        );
    }
}

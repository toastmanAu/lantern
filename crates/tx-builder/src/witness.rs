//! Witness placeholders for fee measurement.
//!
//! The placeholder must be the exact size of the real signature, because
//! the transaction is measured with placeholders in place and broadcast
//! with real signatures. For `Fixed` sizes the two are identical; for
//! `Variable` the placeholder is the maximum, so the measured size is an
//! upper bound and the fee is never short.

use ckb_types::packed::{Bytes, BytesOpt, WitnessArgs};
use ckb_types::prelude::*;
use lantern_sdk_schema::WitnessSize;

/// A witness slot carrying nothing. Non-first slots of a script group use
/// this; they must be PRESENT, because the sighash stream length-prefixes
/// every slot and a missing one shifts every byte after it.
pub const EMPTY_WITNESS: &[u8] = &[];

/// Serialised `WitnessArgs` whose lock field is zeroed to the size fee
/// estimation must assume.
#[must_use]
pub fn placeholder_witness(size: WitnessSize) -> Vec<u8> {
    let lock_len = size.for_fee_estimate();
    let lock = Bytes::new_builder().set(vec![0u8.into(); lock_len]).build();
    WitnessArgs::new_builder()
        .lock(BytesOpt::new_builder().set(Some(lock)).build())
        .build()
        .as_bytes()
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::placeholder_witness;
    use lantern_sdk_schema::WitnessSize;

    #[test]
    fn a_65_byte_lock_serialises_to_85_bytes() {
        // Pinned independently by signer-secp256k1's sighash vector, whose
        // header comment reads: total 0x55, offsets 0x10/0x55/0x55,
        // lock len 0x41. Two sources agreeing is the point.
        let w = placeholder_witness(WitnessSize::Fixed(65));
        assert_eq!(w.len(), 85);
        assert_eq!(&w[..4], &[0x55, 0x00, 0x00, 0x00], "total size u32 LE");
        assert_eq!(&w[4..8], &[0x10, 0x00, 0x00, 0x00], "lock offset 16");
        assert_eq!(&w[16..20], &[0x41, 0x00, 0x00, 0x00], "lock length 65");
        assert!(w[20..].iter().all(|b| *b == 0), "lock body is zeroed");
    }

    #[test]
    fn the_serialised_length_is_always_twenty_plus_the_lock() {
        for len in [0usize, 1, 65, 666, 1462, 3309] {
            assert_eq!(
                placeholder_witness(WitnessSize::Fixed(len)).len(),
                20 + len,
                "4 total + 12 offsets + 4 lock length + body"
            );
        }
    }

    #[test]
    fn a_variable_size_placeholder_uses_the_maximum() {
        let w = placeholder_witness(WitnessSize::Variable {
            min: 666,
            max: 1462,
        });
        assert_eq!(w.len(), 20 + 1462);
    }
}

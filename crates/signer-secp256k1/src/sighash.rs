//! The `secp256k1_blake160_sighash_all` signing message (RFC 0019), exactly
//! as ckb-sdk-rust's `generate_message` computes it:
//!
//! `blake2b_256(tx_hash ‖ u64le(len(w0)) ‖ w0 ‖ Σ (u64le(len(wi)) ‖ wi))`
//!
//! `w0` is the first witness of the script group with its lock field
//! already replaced by `witness_lock_len` zero bytes by the caller.
//! `others` are the remaining witnesses of the group followed by every
//! witness beyond the input count. This module takes bytes; molecule
//! layout is the transaction builder's concern (plan 1e).

fn len_prefix(bytes: &[u8]) -> [u8; 8] {
    u64::try_from(bytes.len())
        .expect("witness length fits in u64")
        .to_le_bytes()
}

/// RFC 0019 signing message. See the module doc for the byte layout.
pub fn sighash_all(tx_hash: &[u8; 32], first_witness: &[u8], others: &[&[u8]]) -> [u8; 32] {
    let mut hasher = ckb_hash::new_blake2b();
    hasher.update(tx_hash);
    hasher.update(&len_prefix(first_witness));
    hasher.update(first_witness);
    for witness in others {
        hasher.update(&len_prefix(witness));
        hasher.update(witness);
    }
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::sighash_all;
    use crate::hash::blake160;
    use crate::sign::recover;

    // CKB testnet block 22356763, a one-input secp256k1_blake160 transfer.
    const TX_HASH: &str = "03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4";
    const WITNESS: &str = "5500000010000000550000005500000041000000a0d661612ec85fc91e2569cd41d25d7704ed44c0042a79e78dc4dcb8e76fd1a15861db4e81cf318be154818199843007f00085d25e3a778494f9f770a4647a7801";
    const LOCK_ARGS: &str = "72f72b0cafd31de5072b10e84fc6c9d7d7596db7";

    fn h32(s: &str) -> [u8; 32] {
        let v = hex::decode(s).expect("hex");
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    #[test]
    fn on_chain_signature_recovers_to_input_lock_args() {
        let witness = hex::decode(WITNESS).expect("hex");
        assert_eq!(witness.len(), 85);
        assert_eq!(
            &witness[..20],
            &hex::decode("5500000010000000550000005500000041000000").expect("hex")[..],
            "WitnessArgs header: total 0x55, offsets 0x10/0x55/0x55, lock len 0x41"
        );
        let mut signature = [0u8; 65];
        signature.copy_from_slice(&witness[20..85]);

        let mut zeroed = witness.clone();
        zeroed[20..85].fill(0);

        let digest = sighash_all(&h32(TX_HASH), &zeroed, &[]);
        let pk = recover(&signature, &digest).expect("real signature recovers");
        assert_eq!(hex::encode(blake160(&pk)), LOCK_ARGS);
    }

    #[test]
    fn other_witnesses_change_the_digest_and_are_order_sensitive() {
        let tx = [7u8; 32];
        let w0 = [1u8; 85];
        let a = [2u8; 10];
        let b = [3u8; 12];
        let none = sighash_all(&tx, &w0, &[]);
        let ab = sighash_all(&tx, &w0, &[&a, &b]);
        let ba = sighash_all(&tx, &w0, &[&b, &a]);
        assert_ne!(none, ab);
        assert_ne!(ab, ba);
    }
}

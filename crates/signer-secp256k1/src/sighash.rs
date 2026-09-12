//! The `secp256k1_blake160_sighash_all` signing message (RFC 0019), exactly
//! as ckb-sdk-rust's `generate_message` computes it:
//!
//! `blake2b_256(tx_hash ‖ u64le(len(w0)) ‖ w0 ‖ Σ (u64le(len(wi)) ‖ wi))`
//!
//! `w0` is the first witness of the script group with its lock field
//! already replaced by `witness_size().for_fee_estimate()` zero bytes by
//! the caller.
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

    // CKB testnet block 22,388,663 (0x1559fb7), tx index 1: a three-input
    // secp256k1_blake160_sighash_all transfer, all three inputs locked to the
    // same args (one script group). Found by walking blocks back from the
    // tip (2026-09-12) and resolving each input's previous output.
    const TX2_HASH: &str = "fb34f7b143c634b18da16e41947779586236ac09b8ce5c3749147e626f9a11dc";
    const TX2_WITNESS0: &str = "5500000010000000550000005500000041000000874bd6abecfb2187b7d67dfaa02d0f7916b4529333cdb622beb2e051f6df54d2760223af63dd64cd4edb937e7390d5333f2e09e7aca59a10394f538c2f138b1600";
    const TX2_WITNESS1: &str = "10000000100000001000000010000000";
    const TX2_WITNESS2: &str = "10000000100000001000000010000000";
    const TX2_LOCK_ARGS: &str = "b12e3692d401c331f6d1f1efcb24d510296c4a6a";

    /// Recover the signer's `blake160` from a 65-byte recoverable signature
    /// (`r ‖ s ‖ recid`) over `digest`. Not this crate's public surface: a
    /// test-only oracle helper built from the crate's already-independently-
    /// verified `sign::recover` (RFC 6979/secp256k1 round trip, `sign.rs`)
    /// and `hash::blake160` (blake2b personalisation, `hash.rs`), so nothing
    /// new is reimplemented here — only composed for this test's purpose.
    fn recover_blake160(digest: &[u8; 32], signature: &[u8]) -> [u8; 20] {
        let mut sig = [0u8; 65];
        sig.copy_from_slice(signature);
        let pk = recover(&sig, digest).expect("real signature recovers");
        blake160(&pk)
    }

    #[test]
    fn a_real_multi_input_testnet_transaction_recovers_to_its_own_lock_args() {
        // Oracle: this transaction is committed on CKB testnet (tx
        // TX2_HASH, block 22,388,663), so the chain accepted these
        // signatures. If our digest is right, recovering the public key
        // from the signature yields the blake160 all three inputs are
        // locked to. Nothing here trusts our own signing path — sighash_all
        // is the only piece of this crate's own code under test.
        //
        // The brief asked for a two-input vector; this transaction has
        // three, all under one script group. Left as three rather than
        // trimmed to two: it exercises the same "others" concatenation with
        // one more witness than the minimum, which is a strict superset of
        // the required coverage.
        let tx_hash = h32(TX2_HASH);
        let first_witness = hex::decode(TX2_WITNESS0).expect("hex");
        let other_witnesses: Vec<Vec<u8>> = vec![
            hex::decode(TX2_WITNESS1).expect("hex"),
            hex::decode(TX2_WITNESS2).expect("hex"),
        ];
        let expected_lock_args = hex::decode(TX2_LOCK_ARGS).expect("hex");

        assert_eq!(first_witness.len(), 85);
        let others: Vec<&[u8]> = other_witnesses.iter().map(Vec::as_slice).collect();
        let mut zeroed = first_witness.clone();
        zeroed[20..85].fill(0);

        let digest = sighash_all(&tx_hash, &zeroed, &others);
        let recovered = recover_blake160(&digest, &first_witness[20..85]);

        assert_eq!(recovered.to_vec(), expected_lock_args);
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

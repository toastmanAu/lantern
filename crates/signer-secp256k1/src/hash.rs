//! CKB hashing helpers built on `ckb-hash`, the consensus implementation
//! (blake2b-256, personalisation `ckb-default-hash`).

use crate::key::PublicKey;

/// First 20 bytes of `blake2b_256(pubkey)`: the lock args of a
/// `secp256k1_blake160` cell.
pub fn blake160(pubkey: &PublicKey) -> [u8; 20] {
    let full = ckb_hash::blake2b_256(pubkey.as_bytes());
    let mut out = [0u8; 20];
    out.copy_from_slice(&full[..20]);
    out
}

#[cfg(test)]
mod tests {
    use super::blake160;
    use crate::key::PublicKey;

    #[test]
    fn ckb_hash_personalisation_matches_consensus_blank_hash() {
        // Guards against a mis-personalised blake2b. This constant is the
        // network's hash of the empty string.
        assert_eq!(
            hex::encode(ckb_hash::blake2b_256(b"")),
            "44f4c69744d5f8c55d642062949dcae49bc4e7ef43d388c5a12f42b5633d163e"
        );
    }

    #[test]
    fn blake160_matches_independent_blake2b() {
        // Python: hashlib.blake2b(pub, digest_size=32, person=b"ckb-default-hash").digest()[:20]
        let pk = hex::decode("0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3")
            .expect("hex");
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&pk);
        let pk = PublicKey::from_bytes(arr).expect("valid point");
        assert_eq!(
            hex::encode(blake160(&pk)),
            "02e830bd6fe19912ffb7b0b134cbe53178b9e8f1"
        );
    }
}

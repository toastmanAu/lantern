//! Recoverable ECDSA over a 32-byte digest in the layout the
//! `secp256k1_blake160_sighash_all` lock reads: 64-byte compact signature
//! followed by one recovery-id byte.

use secp256k1::Message;
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};

use crate::error::SignerError;
use crate::key::{PublicKey, SigningKey};

/// Sign a digest. Output layout is `r ‖ s ‖ recid`.
pub fn sign_recoverable(key: &SigningKey, digest: &[u8; 32]) -> [u8; 65] {
    let mut sk = key.to_secp();
    let sig = RecoverableSignature::sign_ecdsa_recoverable(Message::from_digest(*digest), &sk);
    sk.non_secure_erase();
    let (recid, compact) = sig.serialize_compact();
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&compact);
    out[64] = u8::from(recid);
    out
}

/// Recover the public key that produced `signature` over `digest`.
pub fn recover(signature: &[u8; 65], digest: &[u8; 32]) -> Result<PublicKey, SignerError> {
    let recid = RecoveryId::try_from(i32::from(signature[64]))
        .map_err(|_| SignerError::InvalidSignature)?;
    let sig = RecoverableSignature::from_compact(&signature[..64], recid)
        .map_err(|_| SignerError::InvalidSignature)?;
    let pk = sig
        .recover_ecdsa(Message::from_digest(*digest))
        .map_err(|_| SignerError::InvalidSignature)?;
    Ok(PublicKey::from_secp(&pk))
}

#[cfg(test)]
mod tests {
    use super::{recover, sign_recoverable};
    use crate::error::SignerError;
    use crate::key::{SigningKey, public_key};

    fn key() -> SigningKey {
        let v = hex::decode("fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498")
            .expect("hex");
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&v);
        SigningKey::from_bytes(arr).expect("valid")
    }

    #[test]
    fn signature_is_deterministic_and_recovers_the_signer() {
        let key = key();
        let digest = [0x42u8; 32];
        let a = sign_recoverable(&key, &digest);
        let b = sign_recoverable(&key, &digest);
        assert_eq!(a, b, "RFC 6979 nonces make signing deterministic");
        assert!(a[64] <= 3, "recid byte must be 0..=3, got {}", a[64]);
        let recovered = recover(&a, &digest).expect("recovers");
        assert_eq!(recovered, public_key(&key));
    }

    #[test]
    fn wrong_digest_does_not_recover_the_signer() {
        let key = key();
        let sig = sign_recoverable(&key, &[0x42u8; 32]);
        let other = recover(&sig, &[0x43u8; 32]);
        assert!(other.ok().is_none_or(|pk| pk != public_key(&key)));
    }

    #[test]
    fn bad_recovery_id_is_rejected() {
        let key = key();
        let mut sig = sign_recoverable(&key, &[0x42u8; 32]);
        sig[64] = 4;
        assert_eq!(
            recover(&sig, &[0x42u8; 32]).err(),
            Some(SignerError::InvalidSignature)
        );
    }
}

//! Key types. `SigningKey` is the only secret-bearing type in this crate;
//! it zeroizes on drop and has no `Debug`, `Clone`, or `Serialize`.
//!
//! Known residue: libsecp256k1's `SecretKey` is `Copy` and does not zeroize
//! on drop, so every call that needs one creates a transient copy on the
//! stack and calls `non_secure_erase` on it before returning. Copies made
//! by the compiler when passing arrays by value are outside our control.
//! The residue list is not exhaustive: `hmac`/`sha2` hasher state (which
//! absorbs parent key and seed bytes during BIP32 derivation) is not
//! zeroized either — those crates give us no API to do so.

use secp256k1::{PublicKey as SecpPublicKey, SecretKey};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::SignerError;

/// A 32-byte secp256k1 secret scalar. Validated at construction.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SigningKey([u8; 32]);

impl SigningKey {
    /// Accepts the bytes by value and wipes the argument slot on both the
    /// success and rejection paths, so rejected key bytes never linger on
    /// the stack past this call.
    pub fn from_bytes(mut bytes: [u8; 32]) -> Result<Self, SignerError> {
        let probe = SecretKey::from_secret_bytes(bytes);
        let key = Self(bytes);
        bytes.zeroize();
        probe.map_or_else(
            |_| Err(SignerError::InvalidKey), // `key` drops here and zeroizes
            |mut valid| {
                valid.non_secure_erase();
                Ok(key)
            },
        )
    }

    /// Transient libsecp256k1 key. Callers must `non_secure_erase` it.
    pub(crate) fn to_secp(&self) -> SecretKey {
        SecretKey::from_secret_bytes(self.0).expect("validated at construction")
    }
}

/// A 33-byte compressed SEC1 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey([u8; 33]);

impl PublicKey {
    pub const fn as_bytes(&self) -> &[u8; 33] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 33]) -> Result<Self, SignerError> {
        SecpPublicKey::from_slice(&bytes).map_err(|_| SignerError::InvalidKey)?;
        Ok(Self(bytes))
    }

    pub(crate) fn from_secp(pk: &SecpPublicKey) -> Self {
        Self(pk.serialize())
    }
}

/// Compressed public key for a signing key.
pub fn public_key(key: &SigningKey) -> PublicKey {
    let mut sk = key.to_secp();
    let pk = SecpPublicKey::from_secret_key(&sk);
    sk.non_secure_erase();
    PublicKey::from_secp(&pk)
}

#[cfg(test)]
mod tests {
    use super::{PublicKey, SigningKey, public_key};
    use crate::error::SignerError;

    fn h32(s: &str) -> [u8; 32] {
        let v = hex::decode(s).expect("hex");
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    #[test]
    fn public_key_matches_lumos_vector() {
        // lumos keychain.test.ts: m/44'/309'/0'/0/0 from the BIP32 TV1 seed
        let key = SigningKey::from_bytes(h32(
            "fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498",
        ))
        .expect("valid key");
        let pk = public_key(&key);
        assert_eq!(
            hex::encode(pk.as_bytes()),
            "0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3"
        );
    }

    #[test]
    fn zero_and_overflow_keys_are_rejected() {
        assert_eq!(
            SigningKey::from_bytes([0u8; 32]).err(),
            Some(SignerError::InvalidKey)
        );
        assert_eq!(
            SigningKey::from_bytes([0xffu8; 32]).err(),
            Some(SignerError::InvalidKey)
        );
    }

    #[test]
    fn public_key_from_bytes_validates_point() {
        let good =
            hex::decode("0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3")
                .expect("hex");
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&good);
        assert!(PublicKey::from_bytes(arr).is_ok());
        arr[0] = 0x05;
        assert_eq!(
            PublicKey::from_bytes(arr).err(),
            Some(SignerError::InvalidKey)
        );
    }
}

//! BIP32 hardened and normal child derivation, written directly on
//! `hmac`/`sha2` and libsecp256k1's `add_tweak` so the crate carries one
//! curve implementation. Verified against BIP-0032 test vector 1 and the
//! lumos `m/44'/309'/0'` vectors.
//!
//! Only private derivation exists; there is no xpub export.

use hmac::{Hmac, Mac};
use secp256k1::{PublicKey as SecpPublicKey, Scalar, SecretKey};
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::SignerError;
use crate::key::SigningKey;

type HmacSha512 = Hmac<Sha512>;

/// Hardened-index bit.
pub const HARDENED: u32 = 0x8000_0000;
/// SLIP-0044 coin type for CKB.
pub const CKB_COIN_TYPE: u32 = 309;
const PURPOSE_BIP44: u32 = 44;

/// BIP44 chain branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// Receiving addresses (`.../0/i`).
    External,
    /// Change addresses (`.../1/i`).
    Internal,
}

impl Branch {
    pub const fn index(self) -> u32 {
        match self {
            Self::External => 0,
            Self::Internal => 1,
        }
    }
}

impl TryFrom<u32> for Branch {
    type Error = SignerError;

    fn try_from(value: u32) -> Result<Self, SignerError> {
        match value {
            0 => Ok(Self::External),
            1 => Ok(Self::Internal),
            _ => Err(SignerError::DerivationOverflow),
        }
    }
}

/// A private node in the key tree. Zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ExtendedKey {
    pub key: [u8; 32],
    pub chain_code: [u8; 32],
}

fn hmac_sha512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    let mut tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    tag.as_mut_slice().zeroize();
    out
}

fn split(i: &[u8; 64]) -> Result<ExtendedKey, SignerError> {
    let mut node = ExtendedKey {
        key: [0u8; 32],
        chain_code: [0u8; 32],
    };
    node.key.copy_from_slice(&i[..32]);
    node.chain_code.copy_from_slice(&i[32..]);
    let mut probe = SecretKey::from_secret_bytes(node.key).map_err(|_| SignerError::InvalidKey)?;
    probe.non_secure_erase();
    Ok(node)
}

fn master_from_seed(seed: &[u8]) -> Result<ExtendedKey, SignerError> {
    if !(16..=64).contains(&seed.len()) {
        return Err(SignerError::InvalidSeedLength);
    }
    let i = hmac_sha512(b"Bitcoin seed", &[seed]);
    split(&i)
}

fn derive_child(parent: &ExtendedKey, index: u32) -> Result<ExtendedKey, SignerError> {
    let i = if index & HARDENED == 0 {
        let mut sk =
            SecretKey::from_secret_bytes(parent.key).map_err(|_| SignerError::InvalidKey)?;
        let pk = SecpPublicKey::from_secret_key(&sk).serialize();
        sk.non_secure_erase();
        hmac_sha512(&parent.chain_code, &[&pk, &index.to_be_bytes()])
    } else {
        hmac_sha512(
            &parent.chain_code,
            &[&[0u8], &parent.key, &index.to_be_bytes()],
        )
    };
    let mut left = [0u8; 32];
    left.copy_from_slice(&i[..32]);
    let tweak = Scalar::from_be_bytes(left).map_err(|_| SignerError::InvalidKey)?;
    left.zeroize();
    let parent_sk =
        SecretKey::from_secret_bytes(parent.key).map_err(|_| SignerError::InvalidKey)?;
    let mut child_sk = parent_sk
        .add_tweak(&tweak)
        .map_err(|_| SignerError::InvalidKey)?;
    let mut child = ExtendedKey {
        key: child_sk.to_secret_bytes(),
        chain_code: [0u8; 32],
    };
    child_sk.non_secure_erase();
    child.chain_code.copy_from_slice(&i[32..]);
    Ok(child)
}

/// Walk an arbitrary path from the seed. Hardened levels carry the
/// `HARDENED` bit.
pub fn derive_path(seed: &[u8], path: &[u32]) -> Result<ExtendedKey, SignerError> {
    let mut node = master_from_seed(seed)?;
    for &index in path {
        node = derive_child(&node, index)?;
    }
    Ok(node)
}

/// Derive the signing key at `m/44'/309'/0'/branch/index`.
pub fn derive_ckb_key(seed: &[u8], branch: Branch, index: u32) -> Result<SigningKey, SignerError> {
    if index >= HARDENED {
        return Err(SignerError::DerivationOverflow);
    }
    let path = [
        PURPOSE_BIP44 | HARDENED,
        CKB_COIN_TYPE | HARDENED,
        HARDENED,
        branch.index(),
        index,
    ];
    let node = derive_path(seed, &path)?;
    SigningKey::from_bytes(node.key)
}

#[cfg(test)]
mod tests {
    use super::{Branch, HARDENED, derive_ckb_key, derive_path};
    use crate::error::SignerError;
    use crate::key::public_key;

    const TV1_SEED: &str = "000102030405060708090a0b0c0d0e0f";
    const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

    fn seed(s: &str) -> Vec<u8> {
        hex::decode(s).expect("hex")
    }

    #[test]
    fn bip32_test_vector_1_chain() {
        let seed = seed(TV1_SEED);
        let cases: [(&[u32], &str, &str); 6] = [
            (
                &[],
                "e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35",
                "873dff81c02f525623fd1fe5167eac3a55a049de3d314bb42ee227ffed37d508",
            ),
            (
                &[HARDENED],
                "edb2e14f9ee77d26dd93b4ecede8d16ed408ce149b6cd80b0715a2d911a0afea",
                "47fdacbd0f1097043b78c63c20c34ef4ed9a111d980047ad16282c7ae6236141",
            ),
            (
                &[HARDENED, 1],
                "3c6cb8d0f6a264c91ea8b5030fadaa8e538b020f0a387421a12de9319dc93368",
                "2a7857631386ba23dacac34180dd1983734e444fdbf774041578e9b6adb37c19",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED],
                "cbce0d719ecf7431d88e6a89fa1483e02e35092af60c042b1df2ff59fa424dca",
                "04466b9cc8e161e966409ca52986c584f07e9dc81f735db683c3ff6ec7b1503f",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED, 2],
                "0f479245fb19a38a1954c5c7c0ebab2f9bdfd96a17563ef28a6a4b1a2a764ef4",
                "cfb71883f01676f587d023cc53a35bc7f88f724b1f8c2892ac1275ac822a3edd",
            ),
            (
                &[HARDENED, 1, 2 | HARDENED, 2, 1_000_000_000],
                "471b76e389e528d6de6d816857e012c5455051cad6660850e58372a6c3e6e7c8",
                "c783e67b921d2beb8f6b389cc646d7263b4145701dadd2161548a8b078e65e9e",
            ),
        ];
        for (path, key, chain) in cases {
            let node = derive_path(&seed, path).expect("derives");
            assert_eq!(hex::encode(node.key), key, "key at {path:?}");
            assert_eq!(hex::encode(node.chain_code), chain, "chain at {path:?}");
        }
    }

    #[test]
    fn lumos_ckb_account_node_from_tv1_seed() {
        let node = derive_path(&seed(TV1_SEED), &[44 | HARDENED, 309 | HARDENED, HARDENED])
            .expect("derives");
        assert_eq!(
            hex::encode(node.key),
            "bb39d218506b30ca69b0f3112427877d983dd3cd2cabc742ab723e2964d98016"
        );
        assert_eq!(
            hex::encode(node.chain_code),
            "37e85a19f54f0a242a35599abac64a71aacc21e3a5860dd024377ffc7e6827d8"
        );
    }

    #[test]
    fn lumos_first_receiving_key_from_tv1_seed() {
        let key = derive_ckb_key(&seed(TV1_SEED), Branch::External, 0).expect("derives");
        assert_eq!(
            hex::encode(public_key(&key).as_bytes()),
            "0331b3c0225388c5010e3507beb28ecf409c022ef6f358f02b139cbae082f5a2a3"
        );
        let node = derive_path(
            &seed(TV1_SEED),
            &[44 | HARDENED, 309 | HARDENED, HARDENED, 0, 0],
        )
        .expect("derives");
        assert_eq!(
            hex::encode(node.key),
            "fcba4708f1f07ddc00fc77422d7a70c72b3456f5fef3b2f68368cdee4e6fb498"
        );
        assert_eq!(
            hex::encode(node.chain_code),
            "c4b7aef857b625bbb0497267ed51151d090f81737f4f22a0ac3673483b927090"
        );
    }

    #[test]
    fn lumos_tank_planet_seed_chain() {
        let seed = seed(TANK_SEED);
        assert_eq!(seed.len(), 64);
        let m = derive_path(&seed, &[]).expect("derives");
        assert_eq!(
            hex::encode(m.key),
            "37d25afe073a6ba17badc2df8e91fc0de59ed88bcad6b9a0c2210f325fafca61"
        );
        let acct = derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED]).expect("derives");
        assert_eq!(
            hex::encode(acct.key),
            "2925f5dfcbee3b6ad29100a37ed36cbe92d51069779cc96164182c779c5dc20e"
        );
        let external =
            derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED, 0]).expect("derives");
        assert_eq!(
            hex::encode(external.key),
            "047fae4f38b3204f93a6b39d6dbcfbf5901f2b09f6afec21cbef6033d01801f1"
        );
        let first =
            derive_path(&seed, &[44 | HARDENED, 309 | HARDENED, HARDENED, 0, 0]).expect("derives");
        assert_eq!(
            hex::encode(first.key),
            "848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14"
        );
    }

    #[test]
    fn rejects_bad_seed_length_and_hardened_index() {
        assert_eq!(
            derive_ckb_key(&[0u8; 15], Branch::External, 0).err(),
            Some(SignerError::InvalidSeedLength)
        );
        assert_eq!(
            derive_ckb_key(&seed(TV1_SEED), Branch::External, HARDENED).err(),
            Some(SignerError::DerivationOverflow)
        );
        assert_eq!(
            Branch::try_from(2).err(),
            Some(SignerError::DerivationOverflow)
        );
        assert_eq!(Branch::try_from(1), Ok(Branch::Internal));
    }
}

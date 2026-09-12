//! `LockModule` implementation for `secp256k1_blake160_sighash_all`.

use async_trait::async_trait;
use lantern_sdk_schema::{
    AccountCapabilities, Derivation, LockError, LockModule, LockType, ScriptTemplate, SeedKind,
    SignedWitness, SigningRequest, WitnessSize,
};

use crate::error::SignerError;
use crate::hash::blake160;
use crate::hd::{Branch, derive_ckb_key};
use crate::key::{SigningKey, public_key};
use crate::sign::sign_recoverable;

/// Code hash of the system `secp256k1_blake160_sighash_all` script. The same
/// value on mainnet and testnet (it is a type-id hash, not a data hash).
pub const SECP256K1_BLAKE160_CODE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9,
    0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];

/// `ScriptHashType::Type`.
pub const HASH_TYPE_TYPE: u8 = 0x01;

const BIP39_SEED_LEN: usize = 64;

/// The first-party secp256k1 lock module. Carries no state.
#[derive(Debug, Default, Clone, Copy)]
pub struct Secp256k1Lock;

fn key_for(seed: &[u8], derivation: Derivation) -> Result<SigningKey, LockError> {
    if seed.len() != BIP39_SEED_LEN {
        return Err(LockError::InvalidSeed);
    }
    let branch = Branch::try_from(derivation.change).map_err(|_| LockError::InvalidDerivation)?;
    derive_ckb_key(seed, branch, derivation.index).map_err(|e| match e {
        SignerError::DerivationOverflow => LockError::InvalidDerivation,
        SignerError::InvalidSeedLength => LockError::InvalidSeed,
        other => LockError::Signing(other.to_string()),
    })
}

#[async_trait]
impl LockModule for Secp256k1Lock {
    fn lock_type(&self) -> LockType {
        LockType::Secp256k1Blake160
    }

    fn extension_id(&self) -> &'static str {
        "core.secp256k1"
    }

    fn capabilities(&self) -> AccountCapabilities {
        AccountCapabilities {
            can_sign: true,
            hardware: false,
        }
    }

    fn script_template(&self) -> ScriptTemplate {
        ScriptTemplate {
            code_hash: SECP256K1_BLAKE160_CODE_HASH,
            hash_type: HASH_TYPE_TYPE,
        }
    }

    fn seed_kind(&self) -> SeedKind {
        SeedKind::Bip39Seed
    }

    fn witness_size(&self) -> WitnessSize {
        // RFC 0019: a recoverable signature is 65 bytes — r (32), s (32),
        // recovery id (1). Fixed, so fee estimation is exact rather than
        // conservative.
        WitnessSize::Fixed(65)
    }

    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError> {
        let key = key_for(seed, *derivation)?;
        Ok(blake160(&public_key(&key)).to_vec())
    }

    async fn sign(
        &self,
        _seed: &[u8],
        _req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError> {
        // TEMPORARY — Task 10 of plan 1e ("secp256k1 signs a multi-input
        // group") implements this for real, replacing this stub with a
        // request-shaped RFC 0019 sighash-all signer built on
        // `sign_digest` below (soon renamed `sign_digest_internal`). Left
        // returning an error rather than a plausible-looking empty `Vec`,
        // so that anything wiring this up before Task 10 lands fails
        // loudly instead of producing an unsigned transaction that looks
        // signed.
        Err(LockError::Signing(
            "Secp256k1Lock::sign is not implemented yet — Task 10 of plan 1e replaces this stub"
                .to_string(),
        ))
    }
}

impl Secp256k1Lock {
    /// Sign a precomputed 32-byte digest for `derivation`.
    ///
    /// Not part of `LockModule` — the trait's `sign` takes a whole
    /// `SigningRequest` (Task 8 of plan 1e). This routine is the
    /// implementation detail that will back it once Task 10 lands; until
    /// then it is exercised only by this crate's own `#[ignore]`d tests.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] if the seed has the wrong shape or the
    /// derivation is out of range.
    // TEMPORARY — only the `#[ignore]`d test below calls this until Task 10
    // wires it up as `sign_digest_internal` behind `LockModule::sign`. `self`
    // is unused because `Secp256k1Lock` carries no state; a trait impl would
    // be exempt from that lint, but this is a plain inherent method now that
    // it is no longer required by `LockModule`.
    #[allow(dead_code, clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    fn sign_digest(
        &self,
        seed: &[u8],
        derivation: Derivation,
        digest: &[u8; 32],
    ) -> Result<Vec<u8>, LockError> {
        let key = key_for(seed, derivation)?;
        Ok(sign_recoverable(&key, digest).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{Derivation, LockError, LockModule, LockType, SeedKind, WitnessSize};

    use super::{HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock};
    use crate::hash::blake160;
    use crate::key::{SigningKey, public_key};
    use crate::sign::recover;

    const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

    fn d(change: u32, index: u32) -> Derivation {
        Derivation { change, index }
    }

    #[test]
    fn identity() {
        let m = Secp256k1Lock;
        assert_eq!(m.lock_type(), LockType::Secp256k1Blake160);
        assert_eq!(m.extension_id(), "core.secp256k1");
        assert_eq!(m.seed_kind(), SeedKind::Bip39Seed);
        assert_eq!(m.witness_size(), WitnessSize::Fixed(65));
        assert!(m.capabilities().can_sign);
        assert!(!m.capabilities().hardware);
        let t = m.script_template();
        assert_eq!(t.code_hash, SECP256K1_BLAKE160_CODE_HASH);
        assert_eq!(t.hash_type, HASH_TYPE_TYPE);
        assert_eq!(
            hex::encode(t.code_hash),
            "9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8"
        );
    }

    #[test]
    fn lock_args_match_the_lumos_derived_key() {
        let seed = hex::decode(TANK_SEED).expect("hex");
        let args = Secp256k1Lock
            .derive_lock_args(&seed, &d(0, 0))
            .expect("derives");
        let key_bytes =
            hex::decode("848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14")
                .expect("hex");
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&key_bytes);
        let expected = blake160(&public_key(&SigningKey::from_bytes(arr).expect("valid")));
        assert_eq!(args, expected.to_vec());
    }

    #[test]
    #[ignore = "sign_digest is not reachable through LockModule until Task 10 of \
                plan 1e implements Secp256k1Lock::sign; restored there as \
                sign_digest_internal"]
    fn signature_recovers_to_lock_args() {
        let seed = hex::decode(TANK_SEED).expect("hex");
        let digest = [0x99u8; 32];
        let sig = Secp256k1Lock
            .sign_digest(&seed, d(0, 3), &digest)
            .expect("signs");
        assert_eq!(sig.len(), 65);
        let mut arr = [0u8; 65];
        arr.copy_from_slice(&sig);
        let pk = recover(&arr, &digest).expect("recovers");
        let args = Secp256k1Lock
            .derive_lock_args(&seed, &d(0, 3))
            .expect("derives");
        assert_eq!(blake160(&pk).to_vec(), args);
    }

    #[test]
    fn rejects_wrong_seed_shape_and_bad_branch() {
        assert_eq!(
            Secp256k1Lock.derive_lock_args(&[0u8; 32], &d(0, 0)).err(),
            Some(LockError::InvalidSeed)
        );
        let seed = hex::decode(TANK_SEED).expect("hex");
        assert_eq!(
            Secp256k1Lock.derive_lock_args(&seed, &d(2, 0)).err(),
            Some(LockError::InvalidDerivation)
        );
        assert_eq!(
            Secp256k1Lock
                .derive_lock_args(&seed, &d(0, 0x8000_0000))
                .err(),
            Some(LockError::InvalidDerivation)
        );
    }
}

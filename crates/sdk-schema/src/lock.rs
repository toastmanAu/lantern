//! The lock-module contract. Every first-party and third-party signer
//! implements `LockModule`; `wallet-core` dispatches through it and never
//! names a concrete scheme.

use crate::error::LockError;
use crate::types::{AccountCapabilities, Derivation, LockType};

/// The script a lock module's accounts are locked by. `hash_type` follows
/// the CKB `ScriptHashType` encoding (`0x00` data, `0x01` type, `0x02`
/// data1, `0x04` data2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptTemplate {
    pub code_hash: [u8; 32],
    pub hash_type: u8,
}

/// Which master secret a module derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    /// The 64-byte BIP39 seed (PBKDF2 over the phrase). Used by BIP32 schemes.
    Bip39Seed,
    /// The raw mnemonic entropy (16 to 96 bytes). Used by hash-based
    /// post-quantum schemes that HKDF over it, as Quantum Purse does.
    RawEntropy,
}

/// How many bytes a lock module's signature occupies in the witness.
///
/// Post-quantum schemes make this a real question: Falcon-512 signatures
/// range 666 to 1462 bytes, so a single number cannot describe them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitnessSize {
    Fixed(usize),
    Variable { min: usize, max: usize },
}

impl WitnessSize {
    /// The size fee estimation must use.
    ///
    /// Always the worst case. Undercharging gets the transaction rejected
    /// by the pool; overcharging costs a rounding error.
    #[must_use]
    pub const fn for_fee_estimate(&self) -> usize {
        match self {
            Self::Fixed(n) => *n,
            Self::Variable { max, .. } => *max,
        }
    }
}

/// Object-safe contract for a lock family.
///
/// `seed` is a borrowed slice of whatever `seed_kind` asked for. Implementors
/// must not retain it. `sign_digest` returns the bytes destined for the
/// witness lock field, whatever size the scheme needs.
pub trait LockModule: Send + Sync {
    fn lock_type(&self) -> LockType;
    fn extension_id(&self) -> &'static str;
    fn capabilities(&self) -> AccountCapabilities;
    fn script_template(&self) -> ScriptTemplate;
    fn seed_kind(&self) -> SeedKind;
    /// Size of the witness lock placeholder for fee estimation.
    fn witness_size(&self) -> WitnessSize;
    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError>;
    fn sign_digest(
        &self,
        seed: &[u8],
        derivation: &Derivation,
        digest: &[u8; 32],
    ) -> Result<Vec<u8>, LockError>;
}

#[cfg(test)]
mod tests {
    use super::{LockModule, ScriptTemplate, SeedKind, WitnessSize};
    use crate::error::LockError;
    use crate::types::{AccountCapabilities, Derivation, LockType};

    struct Fake;

    impl LockModule for Fake {
        fn lock_type(&self) -> LockType {
            LockType::Secp256k1Blake160
        }
        fn extension_id(&self) -> &'static str {
            "test.fake"
        }
        fn capabilities(&self) -> AccountCapabilities {
            AccountCapabilities {
                can_sign: true,
                hardware: false,
            }
        }
        fn script_template(&self) -> ScriptTemplate {
            ScriptTemplate {
                code_hash: [0; 32],
                hash_type: 1,
            }
        }
        fn seed_kind(&self) -> SeedKind {
            SeedKind::RawEntropy
        }
        fn witness_size(&self) -> WitnessSize {
            WitnessSize::Fixed(1)
        }
        fn derive_lock_args(&self, seed: &[u8], _: &Derivation) -> Result<Vec<u8>, LockError> {
            Ok(seed.to_vec())
        }
        fn sign_digest(
            &self,
            _: &[u8],
            _: &Derivation,
            d: &[u8; 32],
        ) -> Result<Vec<u8>, LockError> {
            Ok(d.to_vec())
        }
    }

    #[test]
    fn fee_estimation_uses_the_worst_case_witness_size() {
        assert_eq!(WitnessSize::Fixed(65).for_fee_estimate(), 65);
        // Falcon-512 ranges 666..=1462. Charging the minimum would get the
        // transaction rejected by the pool; charging the maximum costs a
        // rounding error. Always the maximum.
        assert_eq!(
            WitnessSize::Variable {
                min: 666,
                max: 1462
            }
            .for_fee_estimate(),
            1462
        );
    }

    #[test]
    fn trait_is_object_safe() {
        let module: Box<dyn LockModule> = Box::new(Fake);
        let derivation = Derivation {
            change: 0,
            index: 0,
        };
        assert_eq!(
            module.derive_lock_args(&[1, 2], &derivation),
            Ok(vec![1, 2])
        );
        assert_eq!(module.seed_kind(), SeedKind::RawEntropy);
    }
}

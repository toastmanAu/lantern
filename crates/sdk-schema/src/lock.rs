//! The lock-module contract. Every first-party and third-party signer
//! implements `LockModule`; `wallet-core` dispatches through it and never
//! names a concrete scheme.

use async_trait::async_trait;

// Re-exported: `cell_deps` names it in the trait's own signature, so every
// implementor needs the type and should not have to depend on
// `ckb-jsonrpc-types` directly to get it.
pub use ckb_jsonrpc_types::CellDep;

use crate::error::LockError;
use crate::signing::{SignedWitness, SigningRequest};
use crate::types::{AccountCapabilities, Derivation, LockType, Network};

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
/// must not retain it.
#[async_trait]
pub trait LockModule: Send + Sync {
    fn lock_type(&self) -> LockType;
    fn extension_id(&self) -> &'static str;
    fn capabilities(&self) -> AccountCapabilities;
    fn script_template(&self) -> ScriptTemplate;
    fn seed_kind(&self) -> SeedKind;
    /// Size of the witness lock placeholder for fee estimation.
    fn witness_size(&self) -> WitnessSize;

    /// The cell deps this module's lock script needs on `network`.
    ///
    /// Per network, because a system script's dep group lives at a different
    /// out point on each chain — one chain's does not exist on the other.
    /// Omitting these, or carrying the wrong chain's, fails **only** on chain,
    /// as `ScriptNotFound`: the transaction builds, signs and serialises
    /// perfectly without them.
    fn cell_deps(&self, network: Network) -> Vec<CellDep>;

    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError>;

    /// Produce witnesses for the groups in `req`.
    ///
    /// The module receives every group it owns in a single call, so it
    /// exposes the seed exactly once no matter how many inputs are being
    /// signed. Returning a witness for an index outside `req.owned_indices()`
    /// is a contract violation and the caller rejects it.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] if key derivation or signing fails.
    async fn sign(
        &self,
        seed: &[u8],
        req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError>;
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::{CellDep, LockModule, ScriptTemplate, SeedKind, WitnessSize};
    use crate::error::LockError;
    use crate::signing::{SignedWitness, SigningGroup, SigningRequest};
    use crate::types::{AccountCapabilities, Derivation, LockType, Network};

    struct Fake;

    #[async_trait]
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
        fn cell_deps(&self, _: Network) -> Vec<CellDep> {
            Vec::new()
        }
        fn derive_lock_args(&self, seed: &[u8], _: &Derivation) -> Result<Vec<u8>, LockError> {
            Ok(seed.to_vec())
        }

        async fn sign(
            &self,
            _: &[u8],
            req: &SigningRequest,
        ) -> Result<Vec<SignedWitness>, LockError> {
            Ok(req
                .groups
                .iter()
                .filter_map(SigningGroup::witness_index)
                .map(|index| SignedWitness {
                    index,
                    witness: vec![0xAB],
                })
                .collect())
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

    #[tokio::test]
    async fn a_module_signs_from_a_request_rather_than_a_digest() {
        let fake = Fake;
        // Two groups, each with non-monotonic input_indices that do not
        // start at 0. This is deliberate: with a single group of [0] (an
        // earlier version of this test), a `Fake` that ignores `req` and
        // always returns one canned witness at index 0 is indistinguishable
        // from a correct implementation. Here, per group, the true minimum
        // input index, the group's position in `groups`, and the first
        // element of `input_indices` are all different (mins {2, 4} vs.
        // positions {0, 1} vs. firsts {7, 9}), so the asserted output can
        // only come from actually reading each group's `input_indices` and
        // taking its minimum.
        let req = SigningRequest {
            tx: ckb_types::packed::Transaction::default(),
            inputs: Vec::new(),
            groups: vec![
                SigningGroup {
                    lock_hash: [0u8; 32],
                    input_indices: vec![7, 2, 5],
                    derivation: Derivation {
                        change: 0,
                        index: 0,
                    },
                },
                SigningGroup {
                    lock_hash: [1u8; 32],
                    input_indices: vec![9, 4],
                    derivation: Derivation {
                        change: 0,
                        index: 1,
                    },
                },
            ],
        };
        let out = fake.sign(b"seed", &req).await.expect("signs");
        assert_eq!(out.len(), 2, "one witness per group, not a fixed count");
        assert_eq!(out[0].index, 2, "the first group's minimum input index");
        assert_eq!(out[1].index, 4, "the second group's minimum input index");
    }
}

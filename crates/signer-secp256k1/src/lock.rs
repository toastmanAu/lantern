//! `LockModule` implementation for `secp256k1_blake160_sighash_all`.

use async_trait::async_trait;
use ckb_jsonrpc_types::{CellDep, DepType, OutPoint};
use ckb_types::H256;
use ckb_types::packed::{Byte, Bytes, BytesOpt, Transaction, WitnessArgs};
use ckb_types::prelude::*;
use lantern_sdk_schema::{
    AccountCapabilities, Derivation, LockError, LockModule, LockType, Network, ScriptTemplate,
    SeedKind, SignedWitness, SigningRequest, WitnessSize,
};

use crate::error::SignerError;
use crate::hash::blake160;
use crate::hd::{Branch, derive_ckb_key};
use crate::key::{SigningKey, public_key};
use crate::sighash::sighash_all;
use crate::sign::sign_recoverable;

/// Code hash of the system `secp256k1_blake160_sighash_all` script. The same
/// value on mainnet and testnet (it is a type-id hash, not a data hash).
pub const SECP256K1_BLAKE160_CODE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9,
    0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];

/// `ScriptHashType::Type`.
pub const HASH_TYPE_TYPE: u8 = 0x01;

/// Out point of the mainnet genesis dep group carrying the system
/// `secp256k1_blake160_sighash_all` script and its `secp256k1_data` cell.
///
/// Chain data, not a derivation: each chain's genesis block produced its own,
/// so the two are unrelated values and neither exists on the other chain.
pub const SECP256K1_DEP_GROUP_MAINNET: [u8; 32] = [
    0x71, 0xa7, 0xba, 0x8f, 0xc9, 0x63, 0x49, 0xfe, 0xa0, 0xed, 0x3a, 0x5c, 0x47, 0x99, 0x2e, 0x3b,
    0x40, 0x84, 0xb0, 0x31, 0xa4, 0x22, 0x64, 0xa0, 0x18, 0xe0, 0x07, 0x2e, 0x81, 0x72, 0xe4, 0x6c,
];

/// The same dep group on testnet. See [`SECP256K1_DEP_GROUP_MAINNET`].
pub const SECP256K1_DEP_GROUP_TESTNET: [u8; 32] = [
    0xf8, 0xde, 0x3b, 0xb4, 0x7d, 0x05, 0x5c, 0xdf, 0x46, 0x0d, 0x93, 0xa2, 0xa6, 0xe1, 0xb0, 0x5f,
    0x74, 0x32, 0xf9, 0x77, 0x7c, 0x8c, 0x47, 0x4a, 0xbf, 0x4e, 0xec, 0x1d, 0x4a, 0xee, 0xd4, 0xbb,
];

const BIP39_SEED_LEN: usize = 64;

/// A recoverable secp256k1 signature: r (32), s (32), recovery id (1).
const SIGNATURE_LEN: usize = 65;

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
        WitnessSize::Fixed(SIGNATURE_LEN)
    }

    fn cell_deps(&self, network: Network) -> Vec<CellDep> {
        let tx_hash = match network {
            Network::Mainnet => SECP256K1_DEP_GROUP_MAINNET,
            Network::Testnet => SECP256K1_DEP_GROUP_TESTNET,
        };
        vec![CellDep {
            out_point: OutPoint {
                tx_hash: H256(tx_hash),
                index: 0u32.into(),
            },
            // The genesis cell is a dep GROUP: it lists the script binary and
            // the `secp256k1_data` table it needs. `Code` would offer the VM
            // the group cell's own contents instead.
            dep_type: DepType::DepGroup,
        }]
    }

    fn derive_lock_args(&self, seed: &[u8], derivation: &Derivation) -> Result<Vec<u8>, LockError> {
        let key = key_for(seed, *derivation)?;
        Ok(blake160(&public_key(&key)).to_vec())
    }

    async fn sign(
        &self,
        seed: &[u8],
        req: &SigningRequest,
    ) -> Result<Vec<SignedWitness>, LockError> {
        let tx_hash = transaction_hash(&req.tx);
        let input_count = req.tx.raw().inputs().len();
        let witnesses = witness_bytes(&req.tx);

        // The builder pads the witness slots out to the input count before
        // handing the request over. A request that did not is malformed, and
        // the two ways of carrying on from here are both worse than
        // refusing: indexing past the end panics inside a signer, and
        // quietly dropping the missing slots produces a digest that verifies
        // locally and fails on chain with -52.
        if witnesses.len() < input_count {
            return Err(LockError::Signing(format!(
                "transaction has {input_count} inputs but only {} witness slots",
                witnesses.len()
            )));
        }

        let mut signed = Vec::with_capacity(req.groups.len());
        for group in &req.groups {
            let Some(first) = group.witness_index() else {
                continue;
            };
            if let Some(out_of_range) = group.input_indices.iter().find(|i| **i >= input_count) {
                return Err(LockError::Signing(format!(
                    "script group names input {out_of_range}, but the transaction has \
                     {input_count} inputs"
                )));
            }

            // The signed bytes and the broadcast bytes must be the same
            // witness, differing in the lock field alone: on chain the lock
            // re-derives this digest from the BROADCAST witness with its lock
            // field zeroed. Emitting a witness rebuilt from this slot's own
            // `WitnessArgs` makes that true by construction for any shape the
            // builder chooses — an `input_type` field it carries is carried
            // through rather than dropped.
            let placeholder = WitnessArgs::from_slice(&witnesses[first]).map_err(|_| {
                LockError::Signing(format!("witness slot {first} is not a WitnessArgs"))
            })?;

            // ...and the slot must already be this module's placeholder, with
            // its lock zeroed to the signature's own length. Rebuilding from
            // the slot cannot fix a slot that is the wrong shape to begin
            // with: a lock of the wrong length, or one the builder never
            // zeroed, leaves the on-chain zeroing reproducing different bytes
            // from the ones hashed here. Checked before the seed is touched.
            if witness_with_lock(&placeholder, &[0u8; SIGNATURE_LEN]) != witnesses[first] {
                return Err(LockError::Signing(format!(
                    "witness slot {first} is not this module's placeholder: its lock field \
                     must be {SIGNATURE_LEN} zero bytes"
                )));
            }

            // RFC 0019: the rest of the group, then every witness beyond the
            // input count. Both parts are length-prefixed into the digest, so
            // a missing slot shifts every byte after it — which is invisible
            // at one input and wrong at two.
            //
            // Sorted rather than trusted: `input_indices` is documented
            // ascending, but nothing enforces it, and an out-of-order group
            // would stream the witnesses in the wrong order for a digest that
            // is only observably wrong once the transaction reaches a node.
            let mut rest: Vec<usize> = group
                .input_indices
                .iter()
                .copied()
                .filter(|i| *i != first)
                .collect();
            rest.sort_unstable();

            let mut others: Vec<&[u8]> =
                rest.into_iter().map(|i| witnesses[i].as_slice()).collect();
            others.extend(witnesses.iter().skip(input_count).map(Vec::as_slice));

            let digest = sighash_all(&tx_hash, &witnesses[first], &others);
            let signature = self.sign_digest_internal(seed, group.derivation, &digest)?;

            signed.push(SignedWitness {
                index: first,
                witness: witness_with_lock(&placeholder, &signature),
            });
        }
        Ok(signed)
    }
}

/// The CKB transaction hash: `blake2b_256` of the raw part, which covers the
/// inputs, outputs and cell deps but not the witnesses.
///
/// Uses `ckb_hash::blake2b_256`, the same function [`crate::hash::blake160`]
/// calls, so this crate has exactly one blake2b configuration and `hash.rs`'s
/// personalisation test guards both. A second hasher configured slightly
/// differently passes every unit test and fails on chain.
fn transaction_hash(tx: &Transaction) -> [u8; 32] {
    ckb_hash::blake2b_256(tx.raw().as_slice())
}

/// Each witness slot's *unwrapped* bytes, in slot order.
///
/// `raw_data()`, not `as_slice()`: the signing stream length-prefixes the
/// witness content itself, and `as_slice()` would hand it molecule's own
/// 4-byte length header as well, shifting every byte after it.
fn witness_bytes(tx: &Transaction) -> Vec<Vec<u8>> {
    tx.witnesses()
        .into_iter()
        .map(|w| w.raw_data().to_vec())
        .collect()
}

/// `placeholder` with `signature` in its lock field and every other field
/// left exactly as it was.
///
/// Building from the placeholder rather than from scratch is what makes the
/// broadcast witness and the signed bytes differ in the lock field and nowhere
/// else. Rebuilt from scratch, a placeholder carrying an `input_type` would be
/// hashed with that field and broadcast without it — a signature that is
/// confidently wrong, and only says so as an on-chain -52.
fn witness_with_lock(placeholder: &WitnessArgs, signature: &[u8]) -> Vec<u8> {
    let lock = Bytes::new_builder()
        .set(signature.iter().copied().map(Byte::new).collect())
        .build();
    placeholder
        .clone()
        .as_builder()
        .lock(BytesOpt::new_builder().set(Some(lock)).build())
        .build()
        .as_bytes()
        .to_vec()
}

impl Secp256k1Lock {
    /// Sign a precomputed 32-byte digest for `derivation`.
    ///
    /// Private: `LockModule::sign` takes a whole `SigningRequest` and derives
    /// the RFC 0019 digest itself, so this is the curve-math half of that and
    /// not an entry point. Nothing outside this crate can hand in a digest of
    /// its own choosing.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] if the seed has the wrong shape or the
    /// derivation is out of range.
    // `self` is unused, and unit-sized, because `Secp256k1Lock` carries no
    // state; it is kept so the call site reads as the module signing, matching
    // the trait method it backs.
    #[allow(clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    fn sign_digest_internal(
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
    //! The two tests that reach items this module keeps private.
    //!
    //! Everything else lives in `tests/lock.rs`, which exercises the same
    //! module strictly through its public API. These two cannot: they name
    //! `transaction_hash` and `Secp256k1Lock::sign_digest_internal`, and
    //! widening either to `pub` to satisfy a test would widen the API of a
    //! signer. `sign_digest_internal`'s own doc says why it is private —
    //! nothing outside this crate may hand it a digest of its own choosing —
    //! and a test is not a reason to give that up.
    //!
    //! The price is that `TANK_SEED`, `d`, `tx_with` and `tx_hash_of` exist
    //! here and in `tests/lock.rs` both. They are fixture plumbing with no
    //! assertions in them, so the duplication cannot drift into disagreement
    //! about a property.

    use ckb_types::packed::{
        BytesVec, CellInput, CellInputVec, OutPoint, RawTransaction, Transaction,
    };
    use ckb_types::prelude::*;
    use lantern_sdk_schema::{Derivation, LockModule, WitnessSize};
    use lantern_tx_builder::{EMPTY_WITNESS, placeholder_witness};

    use super::{Secp256k1Lock, transaction_hash};
    use crate::hash::blake160;
    use crate::sign::recover;

    const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

    fn d(change: u32, index: u32) -> Derivation {
        Derivation { change, index }
    }

    /// A transaction with `input_count` distinct inputs and exactly the
    /// witness slots given.
    ///
    /// Only those two parts matter to RFC 0019: the transaction hash covers
    /// the raw part (inputs included, witnesses not), and the witness slots
    /// are streamed in separately afterwards. Outputs are left empty so that
    /// a change in the digest can only have come from an input or a witness.
    fn tx_with(input_count: u32, witnesses: &[&[u8]]) -> Transaction {
        let inputs: Vec<CellInput> = (0..input_count)
            .map(|i| {
                CellInput::new_builder()
                    .previous_output(
                        OutPoint::new_builder()
                            .tx_hash([0xABu8; 32].pack())
                            .index(i)
                            .build(),
                    )
                    .build()
            })
            .collect();
        Transaction::new_builder()
            .raw(
                RawTransaction::new_builder()
                    .inputs(CellInputVec::new_builder().set(inputs).build())
                    .build(),
            )
            .witnesses(
                BytesVec::new_builder()
                    .set(witnesses.iter().map(|w| w.pack()).collect())
                    .build(),
            )
            .build()
    }

    #[test]
    fn signature_recovers_to_lock_args() {
        let seed = hex::decode(TANK_SEED).expect("hex");
        let digest = [0x99u8; 32];
        let sig = Secp256k1Lock
            .sign_digest_internal(&seed, d(0, 3), &digest)
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

    fn tx_hash_of(tx: &Transaction) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(tx.calc_tx_hash().as_slice());
        out
    }

    #[test]
    fn the_transaction_hash_matches_ckb_types_own_calculation() {
        // Step 1 against an independent oracle: `calc_tx_hash` is ckb-types'
        // own implementation, not a constant copied from this one. A
        // differently personalised blake2b would agree with itself here and
        // disagree with the network.
        let tx = tx_with(
            2,
            &[&placeholder_witness(WitnessSize::Fixed(65)), EMPTY_WITNESS],
        );
        assert_eq!(transaction_hash(&tx), tx_hash_of(&tx));
    }
}

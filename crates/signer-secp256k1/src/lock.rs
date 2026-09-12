//! `LockModule` implementation for `secp256k1_blake160_sighash_all`.

use async_trait::async_trait;
use ckb_types::packed::{Byte, Bytes, BytesOpt, Transaction, WitnessArgs};
use ckb_types::prelude::*;
use lantern_sdk_schema::{
    AccountCapabilities, Derivation, LockError, LockModule, LockType, ScriptTemplate, SeedKind,
    SignedWitness, SigningRequest, WitnessSize,
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
                witness: witness_with_lock(&signature),
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

/// A `WitnessArgs` carrying `signature` in its lock field and nothing else:
/// byte-for-byte the placeholder this module's [`LockModule::witness_size`]
/// asked the builder to measure, with the zeroes replaced by the signature.
fn witness_with_lock(signature: &[u8]) -> Vec<u8> {
    let lock = Bytes::new_builder()
        .set(signature.iter().copied().map(Byte::new).collect())
        .build();
    WitnessArgs::new_builder()
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
    use ckb_types::packed::{
        BytesVec, CellInput, CellInputVec, OutPoint, RawTransaction, Transaction,
    };
    use ckb_types::prelude::*;
    use lantern_sdk_schema::{
        Derivation, LockError, LockModule, LockType, SeedKind, SigningGroup, SigningRequest,
        WitnessSize,
    };
    use lantern_tx_builder::{EMPTY_WITNESS, placeholder_witness};

    use super::{HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock, transaction_hash};
    use crate::hash::blake160;
    use crate::key::{SigningKey, public_key};
    use crate::sighash::sighash_all;
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

    fn group(indices: Vec<usize>, derivation: Derivation) -> SigningGroup {
        SigningGroup {
            lock_hash: [0u8; 32],
            input_indices: indices,
            derivation,
        }
    }

    fn request(tx: Transaction, groups: Vec<SigningGroup>) -> SigningRequest {
        SigningRequest {
            tx,
            // This module derives its digest from the molecule transaction
            // alone, so it never reads the resolved inputs. A post-quantum
            // lock that rebuilds its own message stream would.
            inputs: Vec::new(),
            groups,
        }
    }

    /// Two inputs under one lock: slot 0 carries the builder's placeholder,
    /// slot 1 is present but empty. Slot 1 must still be committed to.
    fn two_input_request() -> SigningRequest {
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        request(
            tx_with(2, &[&w0, EMPTY_WITNESS]),
            vec![group(vec![0, 1], d(0, 0))],
        )
    }

    /// Identical to [`two_input_request`] except for the bytes in slot 1.
    /// Nothing in the raw part changes, so the transaction hash is the same
    /// and only the witness stream can move the digest.
    fn two_input_request_with_altered_second_witness() -> SigningRequest {
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let w1: &[u8] = &[0xCD; 8];
        request(tx_with(2, &[&w0, w1]), vec![group(vec![0, 1], d(0, 0))])
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

    #[tokio::test]
    async fn signing_a_two_input_group_writes_only_the_groups_first_slot() {
        let module = Secp256k1Lock;
        let seed = [7u8; 64];
        let req = two_input_request();

        let out = module.sign(&seed, &req).await.expect("signs");

        assert_eq!(out.len(), 1, "one signature per group, not per input");
        assert_eq!(out[0].index, 0, "the group's lowest input index");
        assert_eq!(out[0].witness.len(), 85, "WitnessArgs with a 65-byte lock");
        assert_ne!(
            &out[0].witness[20..],
            &[0u8; 65][..],
            "the lock field must carry a real signature, not the placeholder"
        );
    }

    #[tokio::test]
    async fn the_second_input_of_a_group_changes_the_digest() {
        // The whole point of RFC 0019's sighash-all: every witness in the
        // group is committed to. If the second input's witness slot were
        // ignored, a signature would transfer between transactions that
        // differ only there.
        let module = Secp256k1Lock;
        let seed = [7u8; 64];

        let a = module.sign(&seed, &two_input_request()).await.expect("a");
        let b = module
            .sign(&seed, &two_input_request_with_altered_second_witness())
            .await
            .expect("b");

        assert_ne!(
            a[0].witness, b[0].witness,
            "a witness the digest does not commit to is a witness an attacker can change"
        );
    }

    /// The 65-byte signature a module wrote into a witness slot.
    fn signature_in(witness: &[u8]) -> [u8; 65] {
        assert_eq!(witness.len(), 85, "WitnessArgs with a 65-byte lock");
        let mut sig = [0u8; 65];
        sig.copy_from_slice(&witness[20..]);
        sig
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

    #[tokio::test]
    async fn the_written_signature_recovers_over_the_hand_assembled_stream() {
        // Steps 3 to 7 against a stream assembled by hand rather than by the
        // code under test: transaction hash, then the group's own slot, then
        // slot 1. If the implementation streamed anything else — a molecule
        // length header, the wrong slot, no second slot at all — recovery
        // lands on a different public key and this fails.
        let seed = [7u8; 64];
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let req = two_input_request();

        let expected = sighash_all(&tx_hash_of(&req.tx), &w0, &[EMPTY_WITNESS]);
        let out = Secp256k1Lock.sign(&seed, &req).await.expect("signs");

        let pk = recover(&signature_in(&out[0].witness), &expected).expect("recovers");
        assert_eq!(
            blake160(&pk).to_vec(),
            Secp256k1Lock
                .derive_lock_args(&seed, &d(0, 0))
                .expect("derives"),
            "the signature must be over the RFC 0019 digest, by the group's own key"
        );
    }

    #[tokio::test]
    async fn a_witness_beyond_the_input_count_is_committed_to() {
        // The second half of step 4, and the half no two-input transaction
        // with two slots can exercise. Trailing witnesses carry things like
        // CKBFS content; a digest blind to them is a digest an attacker can
        // rewrite around.
        let module = Secp256k1Lock;
        let seed = [7u8; 64];
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let groups = || vec![group(vec![0, 1], d(0, 0))];

        let a = module
            .sign(
                &seed,
                &request(tx_with(2, &[&w0, EMPTY_WITNESS, &[0x01]]), groups()),
            )
            .await
            .expect("a");
        let b = module
            .sign(
                &seed,
                &request(tx_with(2, &[&w0, EMPTY_WITNESS, &[0x02]]), groups()),
            )
            .await
            .expect("b");

        assert_ne!(a[0].witness, b[0].witness);
    }

    #[tokio::test]
    async fn each_group_signs_its_own_slot_with_its_own_key() {
        // A group whose witness slot is not index 0, alongside one whose
        // inputs are not contiguous. Three inputs: group A holds 0 and 2,
        // group B holds 1, and each must sign only its own lowest slot.
        let seed = [7u8; 64];
        let w = placeholder_witness(WitnessSize::Fixed(65));
        let tx = tx_with(3, &[&w, &w, EMPTY_WITNESS]);
        let req = request(
            tx.clone(),
            vec![group(vec![0, 2], d(0, 0)), group(vec![1], d(0, 1))],
        );

        let out = Secp256k1Lock.sign(&seed, &req).await.expect("signs");

        assert_eq!(out.len(), 2, "one signature per group");
        assert_eq!([out[0].index, out[1].index], [0, 1]);

        // Group A streams slot 2 after its own; group B streams nothing.
        let hash = tx_hash_of(&tx);
        for (out, digest, derivation) in [
            (&out[0], sighash_all(&hash, &w, &[EMPTY_WITNESS]), d(0, 0)),
            (&out[1], sighash_all(&hash, &w, &[]), d(0, 1)),
        ] {
            let pk = recover(&signature_in(&out.witness), &digest).expect("recovers");
            assert_eq!(
                blake160(&pk).to_vec(),
                Secp256k1Lock
                    .derive_lock_args(&seed, &derivation)
                    .expect("derives"),
                "group at slot {} signed the wrong digest or used the wrong key",
                out.index
            );
        }
    }

    #[tokio::test]
    async fn a_group_listed_out_of_order_still_signs_the_ascending_stream() {
        // `input_indices` is documented ascending but nothing enforces it —
        // sdk-schema's own tests build groups as `vec![7, 2, 5]`. Streaming
        // the group in the order given would produce a digest that differs
        // only once the transaction reaches a node.
        let module = Secp256k1Lock;
        let seed = [7u8; 64];
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let slots: [&[u8]; 3] = [&w0, &[0x11], &[0x22]];

        let ascending = module
            .sign(
                &seed,
                &request(tx_with(3, &slots), vec![group(vec![0, 1, 2], d(0, 0))]),
            )
            .await
            .expect("ascending");
        let shuffled = module
            .sign(
                &seed,
                &request(tx_with(3, &slots), vec![group(vec![2, 0, 1], d(0, 0))]),
            )
            .await
            .expect("shuffled");

        assert_eq!(ascending[0].witness, shuffled[0].witness);
    }

    #[tokio::test]
    async fn a_request_whose_witness_slots_are_short_is_refused() {
        // Rather than panicking on the index, or silently dropping the slot
        // and producing a digest that only fails at a node.
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let req = request(tx_with(2, &[&w0]), vec![group(vec![0, 1], d(0, 0))]);
        let err = Secp256k1Lock.sign(&[7u8; 64], &req).await.unwrap_err();
        assert_eq!(
            err,
            LockError::Signing("transaction has 2 inputs but only 1 witness slots".to_string())
        );
    }

    #[tokio::test]
    async fn a_group_naming_an_input_the_transaction_does_not_have_is_refused() {
        let w0 = placeholder_witness(WitnessSize::Fixed(65));
        let req = request(
            tx_with(2, &[&w0, EMPTY_WITNESS]),
            vec![group(vec![0, 5], d(0, 0))],
        );
        let err = Secp256k1Lock.sign(&[7u8; 64], &req).await.unwrap_err();
        assert_eq!(
            err,
            LockError::Signing(
                "script group names input 5, but the transaction has 2 inputs".to_string()
            )
        );
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

//! `Secp256k1Lock` through its public API.
//!
//! Moved here wholesale from `src/lock.rs`'s `mod tests`, which had grown to
//! 859 lines — ~285 of production and ~575 of tests — and was the only file
//! in the workspace over the 800-line ceiling. Nothing was rewritten in the
//! move: the same assertions, the same comments, the same fixtures. Two
//! tests stayed behind because they name private items; `src/lock.rs`'s
//! remaining `mod tests` says which and why.

use ckb_jsonrpc_types::DepType;
use ckb_types::packed::{
    Byte, Bytes, BytesOpt, BytesVec, CellInput, CellInputVec, OutPoint, RawTransaction,
    Transaction, WitnessArgs,
};
use ckb_types::prelude::*;
use lantern_sdk_schema::{
    Derivation, LockError, LockModule, LockType, Network, SeedKind, SigningGroup, SigningRequest,
    WitnessSize,
};
use lantern_signer_secp256k1::{
    HASH_TYPE_TYPE, SECP256K1_BLAKE160_CODE_HASH, Secp256k1Lock, SigningKey, blake160, public_key,
    recover, sighash_all,
};
use lantern_tx_builder::{EMPTY_WITNESS, placeholder_witness};

const TANK_SEED: &str = "1371018cfad5990f5e451bf586d59c3820a8671162d8700533549b0df61a63330e5cd5099a5d3938f833d51e4572104868bfac7cfe5b4063b1509a995652bc08";

/// A recoverable secp256k1 signature's length.
///
/// `lock.rs` keeps its own `SIGNATURE_LEN` private, so this is a local copy —
/// pinned against the module's public `witness_size()` by `identity` below,
/// which asserts `WitnessSize::Fixed(65)`. A copy that drifted would fail
/// there rather than quietly assert the wrong width here.
const SIGNATURE_LEN: usize = 65;

const fn d(change: u32, index: u32) -> Derivation {
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

fn tx_hash_of(tx: &Transaction) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(tx.calc_tx_hash().as_slice());
    out
}

const fn group(indices: Vec<usize>, derivation: Derivation) -> SigningGroup {
    SigningGroup {
        lock_hash: [0u8; 32],
        input_indices: indices,
        derivation,
    }
}

const fn request(tx: Transaction, groups: Vec<SigningGroup>) -> SigningRequest {
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
fn each_network_gets_its_own_secp256k1_dep_group() {
    // These out points are published chain data, not a derivation: the
    // genesis transaction that carries each chain's system script dep
    // group. They are also the single thing whose absence fails ONLY on
    // chain, as `ScriptNotFound` — so they are pinned against the values
    // ckb-sdk and lumos ship rather than against our own output.
    let m = Secp256k1Lock;
    let mainnet = m.cell_deps(Network::Mainnet);
    let testnet = m.cell_deps(Network::Testnet);
    assert_eq!(mainnet.len(), 1, "one dep group, not a list of code cells");
    assert_eq!(testnet.len(), 1);
    assert_eq!(
        hex::encode(mainnet[0].out_point.tx_hash.as_bytes()),
        "71a7ba8fc96349fea0ed3a5c47992e3b4084b031a42264a018e0072e8172e46c"
    );
    assert_eq!(
        hex::encode(testnet[0].out_point.tx_hash.as_bytes()),
        "f8de3bb47d055cdf460d93a2a6e1b05f7432f9777c8c474abf4eec1d4aeed4bb"
    );
    assert_ne!(
        mainnet[0].out_point.tx_hash, testnet[0].out_point.tx_hash,
        "one chain's dep group does not exist on the other; sharing the \
         constant would fail as ScriptNotFound and nowhere else"
    );
    for dep in [&mainnet[0], &testnet[0]] {
        assert_eq!(u32::from(dep.out_point.index), 0);
        assert_eq!(
            dep.dep_type,
            DepType::DepGroup,
            "the secp256k1 system script is reached through a dep group; \
             `code` would point at the group cell itself"
        );
    }
}

#[test]
fn lock_args_match_the_lumos_derived_key() {
    let seed = hex::decode(TANK_SEED).expect("hex");
    let args = Secp256k1Lock
        .derive_lock_args(&seed, &d(0, 0))
        .expect("derives");
    let key_bytes = hex::decode("848422863825f69e66dc7f48a3302459ec845395370c23578817456ad6b04b14")
        .expect("hex");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&key_bytes);
    let expected = blake160(&public_key(&SigningKey::from_bytes(arr).expect("valid")));
    assert_eq!(args, expected.to_vec());
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

/// The signature a module wrote into a witness slot's lock field.
///
/// Read through the molecule accessor rather than sliced at a fixed
/// offset: a placeholder carrying an `input_type` puts bytes after the
/// lock body, and `witness[20..]` would swallow them.
fn signature_in(witness: &[u8]) -> [u8; SIGNATURE_LEN] {
    let lock = WitnessArgs::from_slice(witness)
        .expect("WitnessArgs")
        .lock()
        .to_opt()
        .expect("a lock field")
        .raw_data();
    assert_eq!(lock.len(), SIGNATURE_LEN, "a 65-byte lock");
    let mut sig = [0u8; SIGNATURE_LEN];
    sig.copy_from_slice(&lock);
    sig
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

/// A serialised `WitnessArgs` with the given lock field, and optionally an
/// `input_type` alongside it.
fn witness_args(lock: &[u8], input_type: Option<&[u8]>) -> Vec<u8> {
    let field = |b: &[u8]| {
        BytesOpt::new_builder()
            .set(Some(
                Bytes::new_builder()
                    .set(b.iter().copied().map(Byte::new).collect())
                    .build(),
            ))
            .build()
    };
    let mut args = WitnessArgs::new_builder().lock(field(lock));
    if let Some(t) = input_type {
        args = args.input_type(field(t));
    }
    args.build().as_bytes().to_vec()
}

/// What the on-chain lock does before re-deriving the digest: zero the
/// broadcast witness's lock bytes IN PLACE, preserving every other field.
///
/// The script memsets the lock body; it does not rebuild the witness. A
/// version of this helper that dropped `input_type`/`output_type` would
/// make the assertions below vacuous — both sides lock-only by
/// construction — and would state the opposite of the property.
fn zero_lock(witness: &[u8]) -> Vec<u8> {
    let args = WitnessArgs::from_slice(witness).expect("WitnessArgs");
    let len = args.lock().to_opt().expect("a lock field").raw_data().len();
    let zeroed = Bytes::new_builder().set(vec![Byte::new(0); len]).build();
    args.as_builder()
        .lock(BytesOpt::new_builder().set(Some(zeroed)).build())
        .build()
        .as_bytes()
        .to_vec()
}

#[tokio::test]
async fn the_broadcast_witness_differs_from_the_signed_bytes_in_the_lock_alone() {
    // The property the whole scheme rests on. The lock script re-derives
    // this digest from the BROADCAST witness with its lock field zeroed,
    // so that must reproduce the bytes hashed here — byte for byte, every
    // field. A placeholder carrying an `input_type` is the case an
    // emitter that rebuilds from scratch silently gets wrong.
    let seed = [7u8; 64];
    assert_eq!(
        witness_args(&[0u8; SIGNATURE_LEN], None),
        placeholder_witness(WitnessSize::Fixed(SIGNATURE_LEN)),
        "this test's own witness builder must agree with the real one"
    );
    // A placeholder carrying a field beside the lock. Over a lock-only
    // placeholder this assertion cannot discriminate: rebuilding from
    // scratch happens to produce the same bytes.
    let slot = witness_args(&[0u8; SIGNATURE_LEN], Some(&[0xEE; 4]));

    let req = request(
        tx_with(2, &[&slot, EMPTY_WITNESS]),
        vec![group(vec![0, 1], d(0, 0))],
    );
    let out = Secp256k1Lock.sign(&seed, &req).await.expect("signs");

    assert_eq!(
        zero_lock(&out[0].witness),
        slot,
        "zeroing the broadcast witness's lock must reproduce the signed bytes"
    );
    let pk = recover(
        &signature_in(&out[0].witness),
        &sighash_all(&tx_hash_of(&req.tx), &slot, &[EMPTY_WITNESS]),
    )
    .expect("recovers");
    assert_eq!(
        blake160(&pk).to_vec(),
        Secp256k1Lock
            .derive_lock_args(&seed, &d(0, 0))
            .expect("derives")
    );
}

#[tokio::test]
async fn a_placeholder_field_beside_the_lock_is_carried_through() {
    // Same property, with a placeholder this module did not choose the
    // shape of. Rebuilding the witness from scratch would drop the
    // `input_type` that was hashed, and only say so as an on-chain -52.
    let slot = witness_args(&[0u8; SIGNATURE_LEN], Some(&[0xEE; 4]));
    let req = request(
        tx_with(2, &[&slot, EMPTY_WITNESS]),
        vec![group(vec![0, 1], d(0, 0))],
    );

    let out = Secp256k1Lock.sign(&[7u8; 64], &req).await.expect("signs");

    let broadcast = WitnessArgs::from_slice(&out[0].witness).expect("WitnessArgs");
    assert_eq!(
        broadcast
            .input_type()
            .to_opt()
            .expect("input_type survives")
            .raw_data()
            .to_vec(),
        vec![0xEEu8; 4]
    );
    assert_eq!(zero_lock(&out[0].witness), slot);
}

#[tokio::test]
async fn a_first_slot_that_is_not_a_witness_args_is_refused() {
    // A padding bug in the builder that leaves the group's first slot
    // empty would otherwise take the digest over ZERO BYTES, broadcast an
    // 85-byte witness, and produce a signature that is confidently wrong.
    let req = request(
        tx_with(2, &[EMPTY_WITNESS, EMPTY_WITNESS]),
        vec![group(vec![0, 1], d(0, 0))],
    );
    assert_eq!(
        Secp256k1Lock.sign(&[7u8; 64], &req).await.unwrap_err(),
        LockError::Signing("witness slot 0 is not a WitnessArgs".to_string())
    );
}

#[tokio::test]
async fn a_first_slot_that_is_not_this_modules_placeholder_is_refused() {
    // Both parse as a `WitnessArgs`, so rebuilding from the slot cannot
    // save either: a lock of the wrong length, and a lock the builder
    // never zeroed. In both, the on-chain zeroing yields different bytes
    // from the ones hashed.
    for slot in [
        placeholder_witness(WitnessSize::Fixed(100)),
        witness_args(&[0x01; SIGNATURE_LEN], None),
    ] {
        let req = request(
            tx_with(2, &[&slot, EMPTY_WITNESS]),
            vec![group(vec![0, 1], d(0, 0))],
        );
        assert_eq!(
            Secp256k1Lock.sign(&[7u8; 64], &req).await.unwrap_err(),
            LockError::Signing(
                "witness slot 0 is not this module's placeholder: its lock field must be \
                 65 zero bytes"
                    .to_string()
            )
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

//! One transfer, pinned byte for byte.
//!
//! Unit tests assert properties; this asserts the artifact. If a
//! dependency's serialiser changes behaviour, or an ordering rule drifts, a
//! property test may still pass while the bytes on the wire differ. That is
//! the failure this catches, and it is why the expected value is
//! transcribed from an independent encoding rather than from our own
//! output.
//!
//! # Provenance of `GOLDEN`
//!
//! `GOLDEN` was NOT produced by printing `plan.tx`'s bytes and pasting them
//! back — that would assert the code agrees with itself and pass against
//! any bug in the encoder, ordering, or field layout. It was produced by
//! `research/ckb-tx-construction/golden-encoder.py`, a hand-written Python
//! molecule encoder committed to this repository (run it with
//! `python3 research/ckb-tx-construction/golden-encoder.py`; its own header
//! carries the full derivation and the one bug it caught in itself along
//! the way), built from:
//!
//! - RFC 0008 (serialisation): the byte-level rules for `array` / `struct`
//!   / `fixvec` / `dynvec` / `table` / `option`, at
//!   `research/ckb-ecosystem-locks/raw/rfcs/rfcs/0008-serialization/0008-serialization.md`.
//! - `blockchain.mol` (the schema itself): field names, order, and which
//!   types are fixed vs. dynamic size, at
//!   `research/ckb-ecosystem-locks/raw/ckb-system-scripts/c/blockchain.mol`.
//! - `lumos/packages/base/src/blockchain.ts`: an independent (JavaScript)
//!   implementation, cross-checked for field order and the numeric
//!   encoding of `HashType`/`DepType` (`Type` = 1, `Code` = 0,
//!   `DepGroup` = 1), at
//!   `research/ckb-tx-construction/raw/lumos/packages/base/src/blockchain.ts`.
//!
//! None of those three sources is `ckb-types`, `ckb-jsonrpc-types`, or
//! anything under `crates/tx-builder` — the encoder shares no code with the
//! implementation it checks. If it ever disagrees with `GOLDEN` again, that
//! disagreement is the signal: re-derive from the spec, not from this
//! crate — see the script's own header for how that played out the first
//! time.
//!
//! The fixture: one 1000 CKB input, secp-shaped locks (20-byte args, hash
//! type `Type`), a 100 CKB payment, one cell dep, `DEFAULT_FEE_RATE`
//! (1000 — the pool minimum, at which `fee_for` rounds to exactly the
//! measured size with no division remainder to get wrong), and a
//! `Fixed(65)` witness placeholder. Every out point, lock arg, and hash
//! type is a fixed byte pattern, not a randomly seeded one, so the fixture
//! itself is reproducible by inspection.

use ckb_jsonrpc_types::{CellDep, DepType, JsonBytes, OutPoint, Script, ScriptHashType};
use ckb_types::H256;
use ckb_types::prelude::Entity;
use lantern_sdk_schema::{Derivation, InputContext, WitnessSize};
use lantern_tx_builder::{SHANNONS_PER_CKB, TransferRequest, build_transfer};

/// Duplicated from `build.rs`'s `#[cfg(test)]` helper of the same shape, as
/// `fee_properties.rs` already does — an integration test cannot reach a
/// helper defined inside a `#[cfg(test)]` module. Keep in step with the
/// other two copies.
fn script(fill: u8) -> Script {
    Script {
        code_hash: H256([fill; 32]),
        hash_type: ScriptHashType::Type,
        args: JsonBytes::from_vec(vec![fill; 20]),
    }
}

fn out_point(seed: u8) -> OutPoint {
    OutPoint {
        tx_hash: H256([seed; 32]),
        index: 0u32.into(),
    }
}

fn request() -> TransferRequest {
    let lock = script(0xaa);
    TransferRequest {
        candidates: vec![InputContext {
            out_point: out_point(0x11),
            capacity: 1000 * SHANNONS_PER_CKB,
            lock: lock.clone(),
            type_: None,
            data: Vec::new(),
        }],
        recipient: script(0xbb),
        amount: 100 * SHANNONS_PER_CKB,
        change_lock: lock,
        witness_size: WitnessSize::Fixed(65),
        derivation: Derivation {
            change: 1,
            index: 7,
        },
        cell_deps: vec![CellDep {
            out_point: out_point(0xff),
            dep_type: DepType::DepGroup,
        }],
        fee_rate: lantern_tx_builder::DEFAULT_FEE_RATE,
    }
}

#[test]
fn a_fixed_transfer_serialises_to_known_bytes() {
    let req = request();
    let plan = build_transfer(&req).expect("builds");
    let bytes = hex::encode(plan.tx.as_slice());

    assert_eq!(bytes, GOLDEN, "serialised transfer changed");
    assert_eq!(plan.size, plan.tx.as_slice().len() + 4);
    // The independent encoder computed fee and change too (see the module
    // doc): fee = 464 shannons (the measured size at rate 1000, which is
    // the pool minimum, so fee_for rounds to exactly the byte count), and
    // change = 1000 CKB - 100 CKB - 464 shannons.
    assert_eq!(plan.fee, 464);
    assert_eq!(
        plan.change,
        Some(1000 * SHANNONS_PER_CKB - 100 * SHANNONS_PER_CKB - 464)
    );
}

const GOLDEN: &str = "cc0100000c0000006b0100005f0100001c00000020000000490000004d0000007d0000004b0100000000000001000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff000000000100000000010000000000000000000000111111111111111111111111111111111111111111111111111111111111111100000000ce0000000c0000006d0000006100000010000000180000006100000000e40b540200000049000000100000003000000031000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb0114000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb6100000010000000180000006100000030026bf41400000049000000100000003000000031000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0114000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa140000000c00000010000000000000000000000061000000080000005500000055000000100000005500000055000000410000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

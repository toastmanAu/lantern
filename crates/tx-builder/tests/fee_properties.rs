//! The one invariant that must hold for every transfer this builder makes.
//!
//! Fee under-payment is the single most repeated failure in this project's
//! recorded history — across `JoyID`, CCC, and an sUDT builder. `fee_for`
//! rounds UP via `div_ceil`; rounding down instead is a one-shannon error
//! that only appears when `size * rate` is not an exact multiple of 1000.
//! Hand-picked example sizes can land on clean multiples and prove nothing
//! about direction — a randomised property is what this task exists to add
//! on top of Task 12's example tests.

use ckb_jsonrpc_types::{CellDep, DepType, JsonBytes, OutPoint, Script, ScriptHashType};
use ckb_types::H256;
use lantern_sdk_schema::{Derivation, InputContext, WitnessSize};
use lantern_tx_builder::{
    BuildError, SHANNONS_PER_CKB, TransferRequest, build_transfer, fee_for, measure,
};

/// A secp-shaped lock: 20-byte args, so `min_capacity` is 61 CKB.
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

/// Duplicated from `build.rs`'s `#[cfg(test)]` helper of the same shape,
/// rather than shared through the crate's public surface: that helper lives
/// inside the crate's unit tests, which an integration test under `tests/`
/// cannot reach without widening `tx-builder`'s public API just for tests.
/// The controller weighed that against duplication and chose duplication —
/// keep the two in step, since a drift between them would weaken this
/// test's fidelity to the builder's real call shape.
fn request(capacities_in_ckb: &[u64], amount: u64, fee_rate: u64) -> TransferRequest {
    let lock = script(0xaa);
    let candidates = capacities_in_ckb
        .iter()
        .enumerate()
        .map(|(i, &ckb)| InputContext {
            out_point: out_point(u8::try_from(i).expect("few candidates")),
            capacity: ckb * SHANNONS_PER_CKB,
            lock: lock.clone(),
            type_: None,
            data: Vec::new(),
        })
        .collect();
    TransferRequest {
        candidates,
        recipient: script(0xbb),
        amount,
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
        fee_rate,
    }
}

/// A deterministic pseudo-random generator, so a failure is reproducible
/// from its seed. A flaky property test that cannot be replayed is worse
/// than no property test.
struct Lcg(u64);

impl Lcg {
    const fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0 >> 16
    }

    const fn in_range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[test]
fn the_fee_never_under_pays_for_the_measured_size() {
    let mut rng = Lcg(0x1E_1E_1E_1E);
    let mut built = 0;

    for case in 0..500 {
        let wallet: Vec<u64> = (0..rng.in_range(1, 8))
            .map(|_| rng.in_range(62, 5_000))
            .collect();
        let amount_ckb = rng.in_range(61, 4_000);
        let rate = rng.in_range(1000, 3000);

        let req = request(&wallet, amount_ckb * SHANNONS_PER_CKB, rate);

        match build_transfer(&req) {
            Ok(plan) => {
                built += 1;
                assert!(
                    plan.fee >= fee_for(plan.size, req.fee_rate),
                    "case {case}: fee {} under-pays for {} bytes at rate {}",
                    plan.fee,
                    plan.size,
                    req.fee_rate
                );
                assert_eq!(
                    plan.size,
                    measure(&plan.tx),
                    "case {case}: reported size disagrees with the transaction"
                );
            }
            Err(
                BuildError::InsufficientFunds { .. }
                | BuildError::ChangeBelowFloor { .. }
                | BuildError::AmountBelowFloor { .. },
            ) => {}
            Err(other) => panic!("case {case}: unexpected {other:?}"),
        }
    }

    assert!(
        built > 100,
        "only {built} of 500 cases built a transaction — the generator is \
         producing mostly-unbuildable inputs and proving little"
    );
}

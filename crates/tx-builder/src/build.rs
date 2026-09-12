//! Assembling a transfer.
//!
//! Fee depends on size, size depends on input count, and input count
//! depends on fee. The loop below resolves that fixpoint, and terminates
//! because a `CellOutput`'s capacity is a fixed-width u64 — the change
//! VALUE does not affect serialised size, only whether a change output
//! exists does. So for a given input count and change-presence the size is
//! fully determined, each added input increases it by a known positive
//! amount, and every iteration consumes one candidate.
//!
//! That argument holds because size is measured rather than modelled. An
//! arithmetic estimate could drift from the serialiser and invalidate it
//! with no test noticing.

use ckb_jsonrpc_types::Script;
use ckb_types::packed::{
    Byte32Vec, Bytes, BytesVec, CellDepVec, CellInput, CellInputVec, CellOutput, CellOutputVec,
    RawTransaction, Script as PackedScript, Transaction, Uint32, Uint64,
};
use ckb_types::prelude::*;
use lantern_sdk_schema::{Derivation, InputContext, SigningGroup};

use crate::capacity::min_capacity;
use crate::error::BuildError;
use crate::fee::fee_for;
use crate::select::order_candidates;
use crate::size::{MAX_TX_SIZE, measure};
use crate::types::{TransferPlan, TransferRequest};
use crate::witness::{EMPTY_WITNESS, placeholder_witness};

/// Build an unsigned transfer.
///
/// # Errors
///
/// See [`BuildError`]. Every variant names the quantity that would resolve it.
pub fn build_transfer(req: &TransferRequest) -> Result<TransferPlan, BuildError> {
    let recipient_floor = min_capacity(&req.recipient, None, 0);
    if req.amount < recipient_floor {
        return Err(BuildError::AmountBelowFloor {
            amount: req.amount,
            floor: recipient_floor,
        });
    }
    if req.candidates.is_empty() {
        return Err(BuildError::NoSpendableCells);
    }
    let change_floor = min_capacity(&req.change_lock, None, 0);

    let order = order_candidates(&req.candidates);
    let mut taken: Vec<usize> = Vec::new();
    let mut available: u64 = 0;

    for &next in &order {
        taken.push(next);
        available = available.saturating_add(req.candidates[next].capacity);

        // Exact: no change output at all, so a smaller transaction and a
        // smaller fee. Check before the with-change shape, or a selection
        // that lands exactly is never recognised.
        let exact_tx = assemble(req, &taken, None);
        let exact_size = measure(&exact_tx);
        let exact_fee = fee_for(exact_size, req.fee_rate);
        if available == req.amount.saturating_add(exact_fee) {
            return finish(req, &taken, exact_tx, exact_fee, None, exact_size);
        }

        let change_tx = assemble(req, &taken, Some(0));
        let change_size = measure(&change_tx);
        let change_fee = fee_for(change_size, req.fee_rate);
        let needed = req
            .amount
            .saturating_add(change_fee)
            .saturating_add(change_floor);
        if available >= needed {
            let change = available - req.amount - change_fee;
            let tx = assemble(req, &taken, Some(change));
            return finish(req, &taken, tx, change_fee, Some(change), change_size);
        }
    }

    // Exhausted. Distinguish "cannot afford it" from "cannot make change".
    let tx = assemble(req, &taken, Some(0));
    let fee = fee_for(measure(&tx), req.fee_rate);
    let needed = req.amount.saturating_add(fee);
    if available < needed {
        return Err(BuildError::InsufficientFunds {
            needed,
            available,
            shortfall: needed - available,
        });
    }
    Err(BuildError::ChangeBelowFloor {
        change: available - needed,
        floor: change_floor,
    })
}

/// One cell output at `capacity` under `lock`, carrying no type script.
fn output(capacity: u64, lock: &Script) -> CellOutput {
    let capacity: Uint64 = capacity.pack();
    CellOutput::new_builder()
        .capacity(capacity)
        .lock(lock.clone())
        .build()
}

/// The packed transaction for a given selection and change shape.
///
/// Witness slots are padded to the input count, slot 0 carrying the lock
/// module's placeholder and the rest [`EMPTY_WITNESS`]. That padding is not
/// cosmetic: the sighash stream length-prefixes every slot, so a missing one
/// shifts every byte after it, and the secp256k1 module now refuses to sign a
/// request whose first slot is not a placeholder `WitnessArgs` with a zeroed
/// lock of its own signature's length.
fn assemble(req: &TransferRequest, taken: &[usize], change: Option<u64>) -> Transaction {
    let cell_deps: Vec<_> = req.cell_deps.iter().cloned().map(Into::into).collect();

    let inputs: Vec<CellInput> = taken
        .iter()
        .map(|&i| {
            CellInput::new_builder()
                .previous_output(req.candidates[i].out_point.clone())
                .since(Uint64::default())
                .build()
        })
        .collect();

    let mut outputs = vec![output(req.amount, &req.recipient)];
    outputs.extend(change.map(|value| output(value, &req.change_lock)));
    let outputs_data: Vec<Bytes> = outputs.iter().map(|_| Bytes::default()).collect();

    let placeholder = placeholder_witness(req.witness_size);
    let witnesses: Vec<Bytes> = (0..inputs.len())
        .map(|i| {
            if i == 0 {
                placeholder.as_slice().pack()
            } else {
                EMPTY_WITNESS.pack()
            }
        })
        .collect();

    let raw = RawTransaction::new_builder()
        .version(Uint32::default())
        .cell_deps(CellDepVec::new_builder().set(cell_deps).build())
        .header_deps(Byte32Vec::default())
        .inputs(CellInputVec::new_builder().set(inputs).build())
        .outputs(CellOutputVec::new_builder().set(outputs).build())
        .outputs_data(BytesVec::new_builder().set(outputs_data).build())
        .build();

    Transaction::new_builder()
        .raw(raw)
        .witnesses(BytesVec::new_builder().set(witnesses).build())
        .build()
}

/// Turn a resolved selection into a plan, or refuse it for being too large.
fn finish(
    req: &TransferRequest,
    taken: &[usize],
    tx: Transaction,
    fee: u64,
    change: Option<u64>,
    size: usize,
) -> Result<TransferPlan, BuildError> {
    if size > MAX_TX_SIZE {
        return Err(BuildError::TransactionTooLarge {
            size,
            limit: MAX_TX_SIZE,
        });
    }

    let inputs: Vec<InputContext> = taken.iter().map(|&i| req.candidates[i].clone()).collect();
    // Unreachable from `build_transfer`, which returns `NoSpendableCells`
    // before selection begins. Stated as a branch rather than an index so
    // that a future caller cannot turn it into a panic inside a builder.
    let Some(first) = inputs.first() else {
        return Err(BuildError::NoSpendableCells);
    };

    // One account, so one script group: every input shares this lock, and
    // the lock script therefore runs once over all of them.
    let mut lock_hash = [0u8; 32];
    lock_hash.copy_from_slice(
        &PackedScript::from(first.lock.clone())
            .calc_script_hash()
            .raw_data(),
    );

    let groups = vec![SigningGroup {
        lock_hash,
        input_indices: (0..inputs.len()).collect(),
        // The request carries no derivation, so the builder cannot know it.
        // `wallet-core` owns the account and fills this in before signing.
        derivation: Derivation {
            change: 0,
            index: 0,
        },
    }];

    Ok(TransferPlan {
        tx,
        inputs,
        groups,
        fee,
        change,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::build_transfer;
    use crate::capacity::{SHANNONS_PER_CKB, min_capacity};
    use crate::fee::DEFAULT_FEE_RATE;
    use crate::{BuildError, TransferRequest};
    use ckb_jsonrpc_types::{CellDep, DepType, JsonBytes, OutPoint, Script, ScriptHashType};
    use ckb_types::H256;
    use lantern_sdk_schema::{InputContext, WitnessSize};

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

    fn request(capacities_in_ckb: Vec<u64>, amount: u64) -> TransferRequest {
        let lock = script(0xaa);
        let candidates = capacities_in_ckb
            .into_iter()
            .enumerate()
            .map(|(i, ckb)| InputContext {
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
            cell_deps: vec![CellDep {
                out_point: out_point(0xff),
                dep_type: DepType::DepGroup,
            }],
            fee_rate: DEFAULT_FEE_RATE,
        }
    }

    #[test]
    fn an_amount_below_the_recipients_floor_is_refused_before_building() {
        // 30 CKB to a secp address. This exact mistake reached the chain
        // twice in a sibling project and came back as -302, after the
        // transaction had been built and broadcast.
        let req = request(vec![1000], 30 * SHANNONS_PER_CKB);
        assert!(matches!(
            build_transfer(&req),
            Err(BuildError::AmountBelowFloor { .. })
        ));
    }

    #[test]
    fn a_single_large_cell_covers_amount_fee_and_change() {
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.inputs.len(), 1);
        assert!(plan.change.is_some());
        let change = plan.change.expect("some");
        assert_eq!(
            change,
            1000 * SHANNONS_PER_CKB - 100 * SHANNONS_PER_CKB - plan.fee
        );
    }

    #[test]
    fn selection_grows_until_the_change_clears_the_floor() {
        // 170 CKB available across two cells, sending 100. One cell alone
        // leaves change below the 61 CKB floor, so the builder must take
        // the second rather than give up or burn it.
        //
        // The brief specified `vec![120, 30]` and described it as the same
        // scenario, but 150 - 100 = 50 CKB is below the floor with BOTH
        // cells taken, so that selection can only ever end in
        // `ChangeBelowFloor` and the test could not observe the growth it
        // names. The second cell is 50 rather than 30 so that the stated
        // intent is actually reachable; every assertion is the brief's.
        let req = request(vec![120, 50], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(
            plan.inputs.len(),
            2,
            "one cell alone cannot make viable change"
        );
        let floor = min_capacity(&req.change_lock, None, 0);
        assert!(plan.change.expect("some") >= floor);
    }

    #[test]
    fn a_wallet_that_cannot_make_viable_change_says_so_precisely() {
        // Enough to pay, not enough to leave a spendable remainder.
        let req = request(vec![120], 100 * SHANNONS_PER_CKB);
        match build_transfer(&req) {
            Err(BuildError::ChangeBelowFloor { change, floor }) => {
                assert!(change < floor);
            }
            other => panic!("expected ChangeBelowFloor, got {other:?}"),
        }
    }

    #[test]
    fn too_little_money_is_insufficient_funds_not_a_change_problem() {
        let req = request(vec![70], 100 * SHANNONS_PER_CKB);
        match build_transfer(&req) {
            Err(BuildError::InsufficientFunds { shortfall, .. }) => {
                assert!(shortfall > 0);
            }
            other => panic!("expected InsufficientFunds, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_wallet_is_its_own_error() {
        let req = request(vec![], 100 * SHANNONS_PER_CKB);
        assert!(matches!(
            build_transfer(&req),
            Err(BuildError::NoSpendableCells)
        ));
    }

    #[test]
    fn witness_slots_are_padded_to_the_input_count() {
        // At one input this assertion is trivially satisfied and proves
        // nothing. At three it is the guard against the -52 class of
        // failure, where a missing slot shifts the sighash stream.
        let req = request(vec![80, 80, 80], 150 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(plan.inputs.len() >= 2, "the point of this test");
        assert_eq!(
            plan.tx.witnesses().len(),
            plan.inputs.len(),
            "every input needs a witness slot, even an empty one"
        );
    }

    #[test]
    fn the_fee_covers_the_measured_size_at_the_requested_rate() {
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(
            plan.fee >= crate::fee::fee_for(plan.size, req.fee_rate),
            "fee {} under-pays for {} bytes",
            plan.fee,
            plan.size
        );
    }

    #[test]
    fn all_inputs_form_one_group_in_a_single_account_transfer() {
        // Two cells rather than one, so "all inputs" means more than one.
        // The brief specified `vec![80, 80]`; 160 - 100 = 60 CKB is one CKB
        // under the 61 CKB change floor, so that wallet cannot build at all
        // and the test observed nothing. 85 each clears it. Assertions are
        // the brief's, unchanged.
        let req = request(vec![85, 85], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].input_indices.len(), plan.inputs.len());
    }
}

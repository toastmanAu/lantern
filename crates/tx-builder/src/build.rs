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
use lantern_sdk_schema::{InputContext, SigningGroup};

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
    let Some(first) = req.candidates.first() else {
        return Err(BuildError::NoSpendableCells);
    };
    // One script group over every input is a PRECONDITION, not an observation:
    // `finish` reads the group's `lock_hash` off the first input and claims
    // all of them for it. `wallet-core` filters its candidates to one exact
    // lock, so today's only caller cannot violate this — but `build_transfer`
    // is public API, plan 1f adds callers, and the failure is a `-52` at a
    // node with nothing local to notice. Checked over every candidate rather
    // than over the selected subset, so the answer does not depend on how
    // much is being sent.
    if let Some((index, _)) = req
        .candidates
        .iter()
        .enumerate()
        .find(|(_, c)| c.lock != first.lock)
    {
        return Err(BuildError::MixedLocks { index });
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
        // Straight from the request. The builder has no way to derive this,
        // and a default would be a knowingly-wrong field inside a plan being
        // handed to a signer.
        derivation: req.derivation,
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
    use super::{assemble, build_transfer};
    use crate::capacity::{SHANNONS_PER_CKB, min_capacity};
    use crate::fee::DEFAULT_FEE_RATE;
    use crate::{BuildError, TransferRequest};
    use ckb_jsonrpc_types::{CellDep, DepType, JsonBytes, OutPoint, Script, ScriptHashType};
    use ckb_types::H256;
    use ckb_types::packed::{CellDep as PackedCellDep, Script as PackedScript, WitnessArgs};
    use ckb_types::prelude::*;
    use lantern_sdk_schema::{Derivation, InputContext, WitnessSize};

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
            // Deliberately not {0, 0}: a builder that ignored this field
            // would still pass every assertion if the fixture used the
            // default.
            derivation: Derivation {
                change: 1,
                index: 7,
            },
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
    fn candidates_under_two_different_locks_are_refused_before_anything_is_built() {
        // `finish` takes the group's `lock_hash` from the first input and
        // claims every input for it. A second lock among the candidates
        // therefore gets signed by the first lock's key — a `-52` on chain,
        // with nothing local to catch it. `wallet-core` filters to one exact
        // lock, so this guard protects the NEXT caller, not the current one.
        //
        // The second cell is large enough to be selected on its own, so a
        // build without the guard would happily reach `finish` rather than
        // stopping for an unrelated reason.
        let mut req = request(vec![1000, 1000], 100 * SHANNONS_PER_CKB);
        req.candidates[1].lock = script(0xcc);
        match build_transfer(&req) {
            // The index, not merely the variant: it must name the row that
            // differs, which is the only thing a caller can act on.
            Err(BuildError::MixedLocks { index }) => assert_eq!(index, 1),
            other => panic!("expected MixedLocks, got {other:?}"),
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
    #[test]
    fn the_group_slot_holds_a_zero_locked_witness_args_and_every_other_slot_is_empty() {
        // The count alone proves nothing about CONTENT, and content is what
        // the secp256k1 module checks before it will sign: it parses this
        // slot as a `WitnessArgs` and requires
        // `witness_with_lock(&parsed, &[0u8; 65]) == witnesses[first]`.
        // The lock re-derives the digest on chain from the BROADCAST witness
        // with its lock zeroed in place, so a slot that is empty, not a
        // `WitnessArgs`, or carries a lock of the wrong length or a
        // non-zeroed one makes the signed bytes and the broadcast bytes
        // differ — a -52 that no local check would see. Pinned here, on the
        // producing side, rather than only in the consumer that refuses it.
        let req = request(vec![85, 85, 85], 150 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(plan.inputs.len() >= 2, "the point of this test");

        let slots: Vec<Vec<u8>> = plan
            .tx
            .witnesses()
            .into_iter()
            .map(|w| w.raw_data().to_vec())
            .collect();

        let parsed = WitnessArgs::from_slice(&slots[0]).expect("slot 0 must be a WitnessArgs");
        let lock = parsed
            .lock()
            .to_opt()
            .expect("slot 0 must carry a lock field")
            .raw_data();
        assert_eq!(lock.len(), 65, "the lock must be the signature's length");
        assert!(lock.iter().all(|b| *b == 0), "the lock must be zeroed");

        for (i, slot) in slots.iter().enumerate().skip(1) {
            assert!(
                slot.is_empty(),
                "slot {i} is not the group's first and must be empty, got {} bytes",
                slot.len()
            );
        }
    }

    #[test]
    fn the_reported_size_is_the_size_of_the_transaction_returned() {
        // `plan.size` is measured on the change=0 shape, while `plan.tx`
        // carries the real change value — a different object. They agree
        // only because a `CellOutput`'s capacity is a fixed-width u64, which
        // is the same fact the fixpoint's termination rests on. If that ever
        // stopped holding, the reported size and the termination argument
        // would both be wrong and nothing else would notice.
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert!(plan.change.is_some(), "exercise the with-change branch");
        assert_eq!(plan.size, crate::size::measure(&plan.tx));
    }
    #[test]
    fn the_group_carries_the_requests_derivation() {
        // Its own test because its failure has nothing to do with grouping:
        // a wrong derivation signs with the wrong key, which recovers to a
        // different public key hash and fails on chain, and only for
        // accounts that are not the default. The fixture's {1, 7} is what
        // makes this observable — against a {0, 0} fixture a builder that
        // ignored the field entirely would still pass.
        let req = request(vec![1000], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.groups[0].derivation, req.derivation);
    }

    #[test]
    fn the_built_transaction_carries_the_deps_outputs_and_out_points_it_was_asked_for() {
        // Spec 7.2's fourth required test. Everything asserted here fails
        // ONLY on chain: missing cell deps as `ScriptNotFound`, a wrong
        // recipient lock as a payment that is gone. Before this test the
        // cell-deps line could be deleted outright with the whole suite
        // still green, which made this crate's module doc — "every decision
        // that can be wrong is a pure function reachable from a test with no
        // node" — untrue.
        let req = request(vec![85, 85, 85], 150 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        let raw = plan.tx.raw();

        let deps: Vec<PackedCellDep> = raw.cell_deps().into_iter().collect();
        let want_deps: Vec<PackedCellDep> = req
            .cell_deps
            .iter()
            .cloned()
            .map(PackedCellDep::from)
            .collect();
        assert_eq!(deps, want_deps, "the lock's cell deps must reach the chain");

        let outputs: Vec<_> = raw.outputs().into_iter().collect();
        assert_eq!(outputs.len(), 2, "recipient and change");
        assert_eq!(
            outputs[0].lock(),
            PackedScript::from(req.recipient.clone()),
            "output 0 must pay the recipient, not anyone else"
        );
        let paid: u64 = outputs[0].capacity().unpack();
        assert_eq!(paid, req.amount, "the recipient must receive the amount");
        assert_eq!(
            outputs[1].lock(),
            PackedScript::from(req.change_lock),
            "change must return to the change lock"
        );
        let returned: u64 = outputs[1].capacity().unpack();
        assert_eq!(returned, plan.change.expect("some"));

        let data: Vec<_> = raw.outputs_data().into_iter().collect();
        assert_eq!(
            data.len(),
            outputs.len(),
            "a transaction with fewer data entries than outputs is malformed"
        );
        assert!(data.iter().all(|d| d.raw_data().is_empty()));

        let spent: Vec<_> = raw
            .inputs()
            .into_iter()
            .map(|i| i.previous_output())
            .collect();
        let want_spent: Vec<_> = plan
            .inputs
            .iter()
            .map(|c| c.out_point.clone().into())
            .collect();
        assert_eq!(
            spent, want_spent,
            "the transaction must spend the cells the plan says it does"
        );
    }

    #[test]
    fn inputs_appear_in_selection_order_and_groups_index_positions_not_candidates() {
        // Every other fixture happens to select [0, 1, ..], so nothing
        // distinguishes a position in `plan.inputs` from an index into
        // `req.candidates`. Here the order is [1, 0]: if the group named
        // candidate indices it would read [1, 0], and the signer would
        // stream the witnesses in the wrong order for a digest that is only
        // observably wrong once a node sees it — a -52.
        let req = request(vec![50, 120], 100 * SHANNONS_PER_CKB);
        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.inputs.len(), 2);

        assert_eq!(
            plan.inputs[0].out_point, req.candidates[1].out_point,
            "largest first: candidate 1 is selected before candidate 0"
        );
        assert_eq!(plan.inputs[1].out_point, req.candidates[0].out_point);

        assert_eq!(
            plan.groups[0].input_indices,
            vec![0, 1],
            "input_indices are positions in plan.inputs, not candidate indices"
        );

        let spent: Vec<_> = plan
            .tx
            .raw()
            .inputs()
            .into_iter()
            .map(|i| i.previous_output())
            .collect();
        assert_eq!(
            spent,
            vec![
                req.candidates[1].out_point.clone().into(),
                req.candidates[0].out_point.clone().into(),
            ],
            "the transaction's inputs must follow the same order"
        );
    }

    #[test]
    fn a_selection_that_lands_exactly_produces_no_change_output() {
        // The no-change branch, otherwise unreachable from any fixture: it
        // fires whenever a cell happens to equal amount + fee, which is rare
        // but perfectly ordinary in production (a wallet re-spending a cell
        // that was itself change from a similar transfer).
        //
        // The capacity is SOLVED rather than guessed, and it has to be:
        // it depends on the serialised size of the very transaction the
        // capacity goes into. That is only well defined because a
        // CellOutput's capacity is a fixed-width u64 — the same property the
        // fixpoint's termination rests on — so the no-change shape's size is
        // identical for every capacity value, and one measurement settles it.
        let amount = 100 * SHANNONS_PER_CKB;
        let mut req = request(vec![1000], amount);
        let exact_size = crate::size::measure(&assemble(&req, &[0], None));
        let exact_fee = crate::fee::fee_for(exact_size, req.fee_rate);
        req.candidates[0].capacity = amount + exact_fee;

        let plan = build_transfer(&req).expect("builds");
        assert_eq!(plan.inputs.len(), 1);
        assert!(
            plan.change.is_none(),
            "an exact landing must not emit a change output, got {:?}",
            plan.change
        );
        assert_eq!(plan.fee, exact_fee);
        assert_eq!(plan.size, exact_size);
        assert_eq!(
            plan.tx.raw().outputs().len(),
            1,
            "recipient only — a zero-capacity change cell would be invalid on chain"
        );
        assert_eq!(plan.tx.raw().outputs_data().len(), 1);
    }
}

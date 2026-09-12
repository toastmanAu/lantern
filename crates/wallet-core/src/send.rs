//! The pieces [`crate::core::WalletCore::send`] is assembled from.
//!
//! Split out of `core.rs` purely for size: these are the resolution,
//! collection and splicing steps that `send` sequences, and every one of them
//! is a function of its arguments alone. `send` itself stays in `core.rs`,
//! where the vault, the registry and the backend live.

use ckb_types::packed::{Bytes, BytesVec, Transaction as PackedTransaction};
use ckb_types::prelude::*;
use lantern_account_registry::StoredAccount;
use lantern_chain_backend::{
    BackendError, CellQuery, ChainBackend, Cursor, H256, JsonBytes, Script, ScriptHashType,
};
use lantern_sdk_schema::{InputContext, LockModule, Network, SignedWitness, SigningRequest};

use crate::error::CoreError;

/// A `ScriptTemplate`'s hash-type byte as the chain types spell it.
///
/// The discriminants are deliberately not a dense range — `Data = 0`,
/// `Type = 1`, `Data1 = 2`, `Data2 = 4`, following `DataN = N << 1` because
/// the low bit encodes data-vs-type and the high bits the VM version. There is
/// no hash type 3. A catch-all `_ =>` arm would silently map every invalid
/// byte onto one real variant, producing a script that watches, and spends,
/// the wrong cells with nothing reporting an error.
const fn script_hash_type(byte: u8) -> Result<ScriptHashType, CoreError> {
    match byte {
        0 => Ok(ScriptHashType::Data),
        1 => Ok(ScriptHashType::Type),
        2 => Ok(ScriptHashType::Data1),
        4 => Ok(ScriptHashType::Data2),
        _ => Err(CoreError::Backend(BackendError::Unsupported(
            "unknown script hash type",
        ))),
    }
}

/// The lock script an account's cells sit under: its module's template
/// carrying the account's own args.
pub fn lock_script_for(
    account: &StoredAccount,
    module: &dyn LockModule,
) -> Result<Script, CoreError> {
    let template = module.script_template();
    Ok(Script {
        code_hash: H256(template.code_hash),
        hash_type: script_hash_type(template.hash_type)?,
        args: JsonBytes::from_vec(account.lock_args.clone()),
    })
}

/// The script a CKB2021 address names, refusing one from another chain.
pub fn decode_address(recipient: &str, network: Network) -> Result<Script, CoreError> {
    let (template, args) = lantern_account_registry::decode_full(network, recipient)?;
    Ok(Script {
        code_hash: H256(template.code_hash),
        hash_type: script_hash_type(template.hash_type)?,
        args: JsonBytes::from_vec(args),
    })
}

/// Every plain-capacity cell `lock` owns, paged until the scan is exhausted.
///
/// Two filters, both of which fail only on chain if they are missing:
///
/// * **Exactly this lock.** The indexer matches script args by PREFIX unless
///   told otherwise, so a lock whose args merely begin with this one's comes
///   back from the same query under a different script hash. The builder takes
///   its group's `lock_hash` from the first candidate and claims every input
///   for it, so one stray cell puts two script groups under one signature —
///   a `-52` with nothing local to catch it.
/// * **No type script and no data.** A transfer writes outputs carrying
///   neither, so spending a token cell as plain capacity consumes the tokens
///   and re-issues none. sUDT permits burning, so that is silent loss rather
///   than a rejection.
///
/// The data filter fails **closed**: `IndexerCell::output_data` is an
/// `Option`, and an absent field is not evidence of an empty cell — it means
/// the node did not send the data, which is indistinguishable from a cell
/// holding a token balance. Treating `None` as empty would accept exactly the
/// cells this filter exists to reject. The query asks for the data explicitly
/// ([`CellQuery::search_key`]), so `None` is already exceptional; excluding it
/// here means no caller depends on that request having been honoured.
///
/// The cursor is never persisted and never re-derived: an exhausted CKB scan
/// answers `last_cursor: "0x"`, and a later `get_cells` with `after: "0x"`
/// returns nothing forever. [`Cursor`] cannot represent that value, and this
/// simply stops when a page hands back no next cursor.
pub async fn collect_candidates(
    backend: &dyn ChainBackend,
    lock: &Script,
) -> Result<Vec<InputContext>, CoreError> {
    let query = CellQuery::lock(lock.clone());
    let mut cursor: Option<Cursor> = None;
    let mut candidates = Vec::new();

    loop {
        let page = backend.get_cells(&query, cursor.as_ref()).await?;
        let next = page.next().cloned();
        for cell in page.into_cells() {
            // No `unwrap_or_default()`: absent data is unknown data, not empty
            // data, and only a cell whose contents the node actually reported
            // can be called plain capacity.
            let Some(data) = cell.output_data.map(|d| d.as_bytes().to_vec()) else {
                continue;
            };
            if cell.output.lock != *lock || cell.output.type_.is_some() || !data.is_empty() {
                continue;
            }
            candidates.push(InputContext {
                out_point: cell.out_point,
                capacity: cell.output.capacity.into(),
                lock: cell.output.lock,
                type_: None,
                data,
            });
        }
        // A cursor that did not advance cannot yield new rows, so treating it
        // as exhausted is the difference between stopping and looping forever
        // against a node that repeats itself.
        if next.is_none() || next == cursor {
            return Ok(candidates);
        }
        cursor = next;
    }
}

/// Each witness slot's *unwrapped* bytes, in slot order.
fn witness_bytes(tx: &PackedTransaction) -> Vec<Vec<u8>> {
    tx.witnesses()
        .into_iter()
        .map(|w| w.raw_data().to_vec())
        .collect()
}

/// `tx` with `slots` as its witnesses and everything else untouched.
fn rebuild_with_witnesses(tx: PackedTransaction, slots: &[Vec<u8>]) -> PackedTransaction {
    let witnesses: Vec<Bytes> = slots.iter().map(|w| w.as_slice().pack()).collect();
    tx.as_builder()
        .witnesses(BytesVec::new_builder().set(witnesses).build())
        .build()
}

/// Splice a module's witnesses into the transaction, refusing any index the
/// module was not asked to sign.
///
/// The guard is the whole point: with third-party extension signers the
/// alternative is one module overwriting another lock's witness — a
/// transaction that still serialises, still broadcasts, and fails as someone
/// else's signature check.
pub fn apply_witnesses(
    tx: PackedTransaction,
    req: &SigningRequest,
    witnesses: Vec<SignedWitness>,
) -> Result<PackedTransaction, CoreError> {
    let owned = req.owned_indices();
    let mut slots = witness_bytes(&tx);
    for w in witnesses {
        if !owned.contains(&w.index) || w.index >= slots.len() {
            return Err(CoreError::WitnessOutOfRange { index: w.index });
        }
        slots[w.index] = w.witness;
    }
    Ok(rebuild_with_witnesses(tx, &slots))
}
#[cfg(test)]
mod tests {
    use ckb_types::packed::{
        Bytes, BytesVec, CellInput, CellInputVec, OutPoint, RawTransaction, Transaction,
    };
    use ckb_types::prelude::*;
    use lantern_sdk_schema::{Derivation, SignedWitness, SigningGroup, SigningRequest};

    use super::apply_witnesses;
    use crate::error::CoreError;

    /// A transaction with `count` inputs and `count` empty witness slots.
    fn tx_with_slots(count: u32) -> Transaction {
        let inputs: Vec<CellInput> = (0..count)
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
                    .set((0..count).map(|_| Bytes::default()).collect())
                    .build(),
            )
            .build()
    }

    /// One group owning input 0 of a four-input transaction — the shape a
    /// second lock's inputs would produce.
    fn one_group_of_four() -> SigningRequest {
        SigningRequest {
            tx: tx_with_slots(4),
            inputs: Vec::new(),
            groups: vec![SigningGroup {
                lock_hash: [7u8; 32],
                input_indices: vec![0],
                derivation: Derivation {
                    change: 0,
                    index: 0,
                },
            }],
        }
    }

    #[test]
    fn a_module_cannot_write_a_witness_it_does_not_own() {
        // With third-party extension signers, the alternative to this check is
        // one module silently overwriting another lock's witness.
        //
        // Index 3 is deliberately IN RANGE: the transaction has four slots, so
        // a build with the ownership check removed would splice it happily and
        // return `Ok`. A test whose rogue index was merely out of bounds would
        // "fail" on a panic whatever the guard did, and prove nothing.
        let req = one_group_of_four();
        let rogue = vec![SignedWitness {
            index: 3,
            witness: vec![0xFF],
        }];
        assert!(matches!(
            apply_witnesses(req.tx.clone(), &req, rogue),
            Err(CoreError::WitnessOutOfRange { index: 3 })
        ));
    }

    #[test]
    fn an_owned_witness_is_spliced_into_its_own_slot_and_no_other() {
        // The other half of the guard: refusing everything would also pass the
        // test above.
        let req = one_group_of_four();
        let signed = apply_witnesses(
            req.tx.clone(),
            &req,
            vec![SignedWitness {
                index: 0,
                witness: vec![0xAB, 0xCD],
            }],
        )
        .expect("an owned index is accepted");

        let slots: Vec<Vec<u8>> = super::witness_bytes(&signed);
        assert_eq!(slots.len(), 4, "the slot count must not change");
        assert_eq!(slots[0], vec![0xAB, 0xCD]);
        assert!(
            slots[1..].iter().all(Vec::is_empty),
            "only the group's own slot may be written"
        );
        assert_eq!(
            signed.raw().as_slice(),
            req.tx.raw().as_slice(),
            "splicing a witness must not disturb the signed-over raw part"
        );
    }
}

#!/usr/bin/env python3
"""Independent molecule encoder for the golden transfer in
`crates/tx-builder/tests/golden.rs`.

This script derives `GOLDEN` — the byte-for-byte expected serialisation of
one fixed `TransferRequest` — from the CKB molecule SPECIFICATION, not from
this repository's own implementation. It is the third oracle for Task 15
(`.superpowers/sdd/2026-09-12-plan-1e-tx-builder/task-15-brief.md`): unit
tests assert properties of `build_transfer`'s output using this crate's own
understanding of the format; this script encodes the same transaction from
scratch, in a different language, from documents nobody in this crate
wrote, so that a bug consistent enough to fool every unit test still shows
up as a byte mismatch here.

Sources used to write this (nothing else was consulted):

- RFC 0008 (serialisation) —
  `research/ckb-ecosystem-locks/raw/rfcs/rfcs/0008-serialization/0008-serialization.md`.
  The byte-level layout rules for `array` / `struct` / `fixvec` / `dynvec` /
  `table` / `option`, each with worked numeric examples this script's
  functions were checked against by hand before being trusted.
- `blockchain.mol` (the schema itself) —
  `research/ckb-ecosystem-locks/raw/ckb-system-scripts/c/blockchain.mol`.
  Field names, declaration order, and which type each field is — e.g.
  `cell_deps` is a `fixvec` because `CellDep` is a fixed-size `struct`,
  `outputs` is a `dynvec` because `CellOutput` is a `table`.
- `lumos/packages/base/src/blockchain.ts` —
  `research/ckb-tx-construction/raw/lumos/packages/base/src/blockchain.ts`.
  An independent JavaScript implementation, consulted only to cross-check
  field order and the numeric encoding of the two enums molecule leaves
  opaque as plain `byte`: `HashType` (`Type` = 1, `Data` = 0, `Data1` = 2,
  `Data2` = 4) and `DepType` (`Code` = 0, `DepGroup` = 1).

This script must NEVER be "fixed" by reading `ckb-types`, `ckb-jsonrpc-types`,
or anything under `crates/tx-builder`, and never by adjusting it until its
output matches `plan.tx`'s bytes. If it ever disagrees with `golden.rs`
again, that disagreement is the entire point of this file existing — go
back to RFC 0008 and `blockchain.mol` and work out which side is wrong from
the spec, the same way Task 15's original mismatch was resolved (see
below), not by tuning either side to match the other.

## The bug this script already caught once (BytesVec double-wrapping)

`witnesses` and `outputs_data` are both `BytesVec`, i.e. `vector BytesVec
<Bytes>` — the vector's ITEM TYPE is `Bytes`, itself a dynamic (`fixvec`)
type. RFC 0008's own worked `BytesVec` example is unambiguous: serialising
`[0x1234]` produces `0e000000 08000000 02000000 1234` — the item stored in
the vector body (`02000000 1234`) is itself a COMPLETE `Bytes` encoding (a
4-byte length prefix, then the content), not the raw bytes. So each
witness (or each output-data entry) is wrapped TWICE: once as an offset
target of the outer `dynvec`, and once more as its own length-prefixed
`Bytes` value. Every item needs both.

The first draft of this script's `mol_transaction` function wrapped
`outputs_data` items correctly but passed raw `WitnessArgs` content
straight into the outer `witnesses` dynvec, omitting the second, inner
`Bytes` wrap. That is exactly one missing 4-byte `u32` length prefix per
witness item — an off-by-four, and the same class of bug this project's
feedback log (`~/.claude/rules/ckb-transactions.feedback.md`) records under
2026-06-22 in a different language and a different vector type (a TS
molecule codegen tool omitting a fixvec's length prefix). The fix here was
one line: wrap each witness with `mol_bytes(...)` before it goes into the
outer vector, exactly as `outputs_data` already did. Confirmed against the
builder's actual (correct) output before either side was touched further —
see task-15-report.md for the full investigation.
"""


def u32le(n: int) -> bytes:
    return n.to_bytes(4, "little")


def u64le(n: int) -> bytes:
    return n.to_bytes(8, "little")


def mol_bytes(data: bytes) -> bytes:
    """`vector Bytes <byte>` — a fixvec of a fixed-size (1-byte) item."""
    return u32le(len(data)) + data


def mol_fixvec(items: list) -> bytes:
    """A vector whose inner item is fixed-size: 4-byte count + concatenation."""
    return u32le(len(items)) + b"".join(items)


def mol_dyn(items: list) -> bytes:
    """A `dynvec` or `table` body: full-size, then N offsets, then N items.

    Same layout serves both, per RFC 0008: 'table... can be considered as a
    dynvec but the length is fixed.' Every field gets an offset, even a
    fixed-size one (RFC 0008's `MixedType` example gives offsets to its
    `byte` and `Uint32` fields alongside the dynamic ones).
    """
    n = len(items)
    if n == 0:
        return u32le(4)
    offset = 4 + 4 * n
    offsets = []
    for it in items:
        offsets.append(offset)
        offset += len(it)
    full_size = offset
    return u32le(full_size) + b"".join(u32le(o) for o in offsets) + b"".join(items)


# HashType: Type=1, Data=0, Data1=2, Data2=4 (lumos blockchain.ts).
HASH_TYPE_TYPE = 1

# DepType: Code=0, DepGroup=1 (lumos blockchain.ts).
DEP_TYPE_CODE = 0
DEP_TYPE_DEP_GROUP = 1


def mol_script(code_hash: bytes, hash_type: int, args: bytes) -> bytes:
    assert len(code_hash) == 32
    field0 = code_hash
    field1 = bytes([hash_type])
    field2 = mol_bytes(args)
    return mol_dyn([field0, field1, field2])


def mol_script_opt(script_bytes) -> bytes:
    return b"" if script_bytes is None else script_bytes


def mol_out_point(tx_hash: bytes, index: int) -> bytes:
    assert len(tx_hash) == 32
    return tx_hash + u32le(index)


def mol_cell_input(since: int, out_point_bytes: bytes) -> bytes:
    return u64le(since) + out_point_bytes


def mol_cell_dep(out_point_bytes: bytes, dep_type: int) -> bytes:
    return out_point_bytes + bytes([dep_type])


def mol_cell_output(capacity: int, lock_bytes: bytes, type_bytes) -> bytes:
    field0 = u64le(capacity)
    field1 = lock_bytes
    field2 = mol_script_opt(type_bytes)
    return mol_dyn([field0, field1, field2])


def mol_witness_args(lock, input_type, output_type) -> bytes:
    f0 = b"" if lock is None else mol_bytes(lock)
    f1 = b"" if input_type is None else mol_bytes(input_type)
    f2 = b"" if output_type is None else mol_bytes(output_type)
    return mol_dyn([f0, f1, f2])


def mol_raw_transaction(version, cell_deps, header_deps, inputs, outputs, outputs_data) -> bytes:
    f_version = u32le(version)
    f_cell_deps = mol_fixvec(cell_deps)      # CellDep is a fixed-size struct -> fixvec
    f_header_deps = mol_fixvec(header_deps)  # Byte32 is fixed-size -> fixvec
    f_inputs = mol_fixvec(inputs)             # CellInput is a fixed-size struct -> fixvec
    f_outputs = mol_dyn(outputs)              # CellOutput is a table (dynamic) -> dynvec
    f_outputs_data = mol_dyn(outputs_data)    # Bytes is dynamic -> dynvec
    return mol_dyn([f_version, f_cell_deps, f_header_deps, f_inputs, f_outputs, f_outputs_data])


def mol_transaction(raw_bytes: bytes, witnesses: list) -> bytes:
    f_raw = raw_bytes
    # See the module doc's "BytesVec double-wrapping" section: each witness
    # is a `Bytes` VALUE inside the outer `BytesVec` dynvec, so it must be
    # wrapped with `mol_bytes` here, in addition to the dynvec's own offset
    # bookkeeping. Omitting this costs exactly 4 bytes per witness item.
    f_witnesses = mol_dyn([mol_bytes(w) for w in witnesses])
    return mol_dyn([f_raw, f_witnesses])


def _sanity_check_against_default_transaction() -> None:
    """`size.rs` pins `measure(&Transaction::default())` at 72 bytes. An
    empty transaction has no witnesses, so this case can't exercise the
    double-wrapping bug above — it only confirms the rest of the encoder
    (empty fixvec/dynvec framing) against the spec before the real fixture
    is built.
    """
    default_raw = mol_raw_transaction(0, [], [], [], [], [])
    default_tx = mol_transaction(default_raw, [])
    measured = len(default_tx) + 4  # molecule::NUMBER_SIZE
    assert measured == 72, f"sanity check failed: {measured}"
    print(f"[sanity] default Transaction measures to {measured} bytes (expect 72)")


def build_golden() -> dict:
    """Build the Task 15 fixture and return its derived values, including
    `golden_hex` — the expected content of `golden.rs`'s `GOLDEN` constant.
    """
    shannons_per_ckb = 100_000_000

    lock_code_hash = bytes([0xAA] * 32)
    lock_args = bytes([0xAA] * 20)
    lock_script = mol_script(lock_code_hash, HASH_TYPE_TYPE, lock_args)

    recipient_code_hash = bytes([0xBB] * 32)
    recipient_args = bytes([0xBB] * 20)
    recipient_script = mol_script(recipient_code_hash, HASH_TYPE_TYPE, recipient_args)

    candidate_tx_hash = bytes([0x11] * 32)
    candidate_out_point = mol_out_point(candidate_tx_hash, 0)
    candidate_capacity = 1000 * shannons_per_ckb

    amount = 100 * shannons_per_ckb

    dep_tx_hash = bytes([0xFF] * 32)
    dep_out_point = mol_out_point(dep_tx_hash, 0)
    cell_dep = mol_cell_dep(dep_out_point, DEP_TYPE_DEP_GROUP)

    cell_input = mol_cell_input(0, candidate_out_point)

    def build_raw(change_capacity):
        recipient_output = mol_cell_output(amount, recipient_script, None)
        change_output = mol_cell_output(change_capacity, lock_script, None)
        outputs = [recipient_output, change_output]
        outputs_data = [mol_bytes(b""), mol_bytes(b"")]
        return mol_raw_transaction(0, [cell_dep], [], [cell_input], outputs, outputs_data)

    # witness_size = Fixed(65): placeholder WitnessArgs with a 65-byte zeroed lock.
    witness0 = mol_witness_args(bytes(65), None, None)
    assert len(witness0) == 85, len(witness0)

    # Step 1: measure the change=0 shape to get the fee (capacity is a
    # fixed-width u64, so the change VALUE does not change this size).
    raw_for_size = build_raw(0)
    tx_for_size = mol_transaction(raw_for_size, [witness0])
    measured_size = len(tx_for_size) + 4  # molecule::NUMBER_SIZE

    # fee_rate == 1000 == the pool minimum, so fee_for(size, 1000) is
    # size*1000/1000 rounded up == size, exactly. No rounding ambiguity.
    fee = measured_size

    change = candidate_capacity - amount - fee
    assert change > 0

    raw_final = build_raw(change)
    tx_final = mol_transaction(raw_final, [witness0])
    final_size = len(tx_final) + 4

    return {
        "measured_size": measured_size,
        "fee": fee,
        "change": change,
        "final_tx_len": len(tx_final),
        "plan_size": final_size,
        "golden_hex": tx_final.hex(),
    }


if __name__ == "__main__":
    _sanity_check_against_default_transaction()
    result = build_golden()
    for key, value in result.items():
        print(f"{key} = {value}")

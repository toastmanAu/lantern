//! Lantern transaction builder.
//!
//! Pure: no I/O, no async, no dependency on `chain-backend`. Given candidate
//! cells and parameters it returns an unsigned transaction plan. Every
//! decision that can be wrong — capacity floors, the fee fixpoint, witness
//! sizing, the size ceiling — is a pure function reachable from a test with
//! no node.

#![forbid(unsafe_code)]

pub mod capacity;
pub mod fee;
pub mod size;
pub mod witness;

pub use capacity::{SHANNONS_PER_CKB, min_capacity, script_occupied_bytes};
pub use fee::{DEFAULT_FEE_RATE, fee_for};
pub use size::{MAX_TX_SIZE, measure};
pub use witness::{EMPTY_WITNESS, placeholder_witness};

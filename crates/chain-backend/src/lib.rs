#![forbid(unsafe_code)]

//! Lantern chain backend.
//!
//! One `dyn`-safe trait over four ways of reaching a CKB chain: a light
//! client Lantern supervises, someone else's light client, a local full node,
//! and a remote full node. Capabilities are data rather than assumptions, so
//! the UI can disable what a backend cannot do instead of failing at call
//! time.

pub mod cursor;
pub mod error;

pub use cursor::{CellPage, Cursor};
pub use error::BackendError;

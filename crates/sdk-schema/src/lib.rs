#![forbid(unsafe_code)]

//! Lantern SDK schemas.
//!
//! Single source of truth for types that cross the IPC boundary into the
//! TypeScript SDK, plus the `LockModule` contract every signer implements.
//! All wire types derive `specta::Type` so `tauri-specta` can export them.

pub mod backend;
pub mod error;
pub mod export;
pub mod lock;
pub mod types;

pub use backend::{BackendCapabilities, BackendKind, BackendProfile, BackendStatus};
pub use error::{LockError, SchemaError};
pub use export::typescript_bindings;
pub use lock::{LockModule, ScriptTemplate, SeedKind};
pub use types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};

//! Standalone TypeScript export of every schema type.
//!
//! Plan 1f replaces this with the `tauri-specta` builder, which also emits
//! command wrappers. Until then this function proves the Rust-first schema
//! pipeline from spec §9 produces the wire shapes we expect.

use specta_typescript::Typescript;

use crate::error::SchemaError;
use crate::types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};

/// Render every IPC type as TypeScript.
pub fn typescript_bindings() -> Result<String, SchemaError> {
    let conf = Typescript::default();
    let chunks = [
        specta_typescript::export::<Network>(&conf),
        specta_typescript::export::<LockType>(&conf),
        specta_typescript::export::<AccountCapabilities>(&conf),
        specta_typescript::export::<Derivation>(&conf),
        specta_typescript::export::<AccountRecord>(&conf),
    ];
    let mut out = String::new();
    for chunk in chunks {
        let rendered = chunk.map_err(|e| SchemaError::Export(e.to_string()))?;
        out.push_str(&rendered);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::typescript_bindings;

    #[test]
    fn bindings_use_camel_case_and_wire_literals() {
        let ts = typescript_bindings().expect("export succeeds");
        for needle in [
            "AccountRecord",
            "Derivation",
            "lockType",
            "extensionId",
            "publicMetadata",
            "canSign",
            "\"secp256k1_blake160\"",
            "\"mainnet\"",
            "\"testnet\"",
        ] {
            assert!(ts.contains(needle), "missing {needle} in:\n{ts}");
        }
        assert!(
            !ts.contains("lock_type"),
            "snake_case leaked into TS:\n{ts}"
        );
        assert!(!ts.contains("can_sign"), "snake_case leaked into TS:\n{ts}");
    }
}

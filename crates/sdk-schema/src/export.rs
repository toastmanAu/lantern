//! Standalone TypeScript export of every schema type.
//!
//! Plan 1f replaces this with the `tauri-specta` builder, which also emits
//! command wrappers. Until then this function proves the Rust-first schema
//! pipeline from spec §9 produces the wire shapes we expect.

use specta_typescript::{BigIntExportBehavior, Typescript};

use crate::backend::{BackendCapabilities, BackendKind, BackendProfile, BackendStatus};
use crate::error::SchemaError;
use crate::types::{AccountCapabilities, AccountRecord, Derivation, LockType, Network};

/// Render every IPC type as TypeScript.
pub fn typescript_bindings() -> Result<String, SchemaError> {
    // `BigIntExportBehavior` defaults to `Fail`, which aborts the export the
    // moment a `u64` appears. Block heights are safe as JS numbers; monetary
    // values would not be, and must use a string newtype instead.
    let conf = Typescript::default().bigint(BigIntExportBehavior::Number);
    let chunks = [
        specta_typescript::export::<Network>(&conf),
        specta_typescript::export::<LockType>(&conf),
        specta_typescript::export::<AccountCapabilities>(&conf),
        specta_typescript::export::<Derivation>(&conf),
        specta_typescript::export::<AccountRecord>(&conf),
        specta_typescript::export::<BackendKind>(&conf),
        specta_typescript::export::<BackendCapabilities>(&conf),
        specta_typescript::export::<BackendStatus>(&conf),
        specta_typescript::export::<BackendProfile>(&conf),
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

    #[test]
    fn backend_types_export_with_numeric_block_heights() {
        let ts = typescript_bindings().expect("export succeeds");
        for needle in [
            "BackendKind",
            "BackendCapabilities",
            "BackendStatus",
            "BackendProfile",
            "needsScriptRegistration",
            "indexerAvailable",
            "\"embedded_light\"",
            "\"remote_full\"",
        ] {
            assert!(ts.contains(needle), "missing {needle} in:\n{ts}");
        }
        // u64 block heights must render as `number`, not abort the export and
        // not become `bigint` (which JSON.parse would not produce).
        assert!(
            ts.contains("current: number"),
            "block height not numeric:\n{ts}"
        );
        assert!(
            !ts.contains("bigint"),
            "bigint leaked into the bindings:\n{ts}"
        );
        assert!(
            !ts.contains("needs_script_registration"),
            "snake_case leaked:\n{ts}"
        );
    }
}

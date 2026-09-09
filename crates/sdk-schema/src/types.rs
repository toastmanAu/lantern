//! IPC types. Every type here crosses the Tauri boundary, so each derives
//! `specta::Type` and uses camelCase on the wire.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which CKB network an address is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[specta(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// Bech32m human-readable prefix.
    pub const fn address_prefix(self) -> &'static str {
        match self {
            Self::Mainnet => "ckb",
            Self::Testnet => "ckt",
        }
    }
}

/// Lock script families Lantern knows how to sign for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
#[specta(rename_all = "snake_case")]
pub enum LockType {
    // `specta`'s `rename_all = "snake_case"` goes through the `inflector`
    // crate, which splits digit runs oddly (`Secp256k1Blake160` ->
    // `secp_25_6k_1_blake_160`) unlike serde's own snake_case conversion.
    // Override with an explicit rename so the wire value matches `slug()`.
    #[specta(rename = "secp256k1_blake160")]
    Secp256k1Blake160,
}

impl LockType {
    /// Stable identifier used in account ids and on the wire.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Secp256k1Blake160 => "secp256k1_blake160",
        }
    }
}

/// What an account can do. Public data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct AccountCapabilities {
    pub can_sign: bool,
    pub hardware: bool,
}

/// Position of an account under its lock module's key tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct Derivation {
    pub change: u32,
    pub index: u32,
}

/// Public projection of an account. Holds no secret material and is safe
/// to send to any frontend or extension with `accounts.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[specta(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub label: String,
    pub lock_type: LockType,
    pub extension_id: String,
    pub address: String,
    pub public_metadata: serde_json::Value,
    pub capabilities: AccountCapabilities,
}

#[cfg(test)]
mod tests {
    use super::{AccountCapabilities, AccountRecord, LockType, Network};

    #[test]
    fn network_prefixes() {
        assert_eq!(Network::Mainnet.address_prefix(), "ckb");
        assert_eq!(Network::Testnet.address_prefix(), "ckt");
    }

    #[test]
    fn lock_type_serialises_as_slug() {
        let json = serde_json::to_string(&LockType::Secp256k1Blake160).expect("serialises");
        assert_eq!(json, "\"secp256k1_blake160\"");
        assert_eq!(LockType::Secp256k1Blake160.slug(), "secp256k1_blake160");
    }

    #[test]
    fn account_record_round_trips_camel_case() {
        let record = AccountRecord {
            id: "secp256k1_blake160-00".into(),
            label: "Main".into(),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            address: "ckt1...".into(),
            public_metadata: serde_json::json!({ "lockArgs": "0x00" }),
            capabilities: AccountCapabilities {
                can_sign: true,
                hardware: false,
            },
        };
        let json = serde_json::to_string(&record).expect("serialises");
        assert!(
            json.contains("\"lockType\":\"secp256k1_blake160\""),
            "{json}"
        );
        assert!(json.contains("\"canSign\":true"), "{json}");
        let back: AccountRecord = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, record);
    }
}

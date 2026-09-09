//! Projection from the stored shape to the IPC `AccountRecord`. The address
//! is computed at read time so one stored record serves both networks.

use lantern_sdk_schema::{AccountCapabilities, AccountRecord, Network, ScriptTemplate};
use serde_json::{Map, Value};

use crate::address::encode_full;
use crate::error::RegistryError;
use crate::store::StoredAccount;

/// Build the IPC projection of a stored account for `network`.
pub fn to_record(
    account: &StoredAccount,
    network: Network,
    template: &ScriptTemplate,
    capabilities: AccountCapabilities,
) -> Result<AccountRecord, RegistryError> {
    let address = encode_full(network, template, &account.lock_args)?;
    let mut metadata = Map::new();
    metadata.insert(
        "lockArgs".to_string(),
        Value::String(format!("0x{}", hex::encode(&account.lock_args))),
    );
    if let Some(derivation) = account.derivation {
        let value = serde_json::to_value(derivation).map_err(|_| RegistryError::Corrupt)?;
        metadata.insert("derivation".to_string(), value);
    }
    Ok(AccountRecord {
        id: account.id.clone(),
        label: account.label.clone(),
        lock_type: account.lock_type,
        extension_id: account.extension_id.clone(),
        address,
        public_metadata: Value::Object(metadata),
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::{AccountCapabilities, Derivation, LockType, Network, ScriptTemplate};

    use super::to_record;
    use crate::store::{StoredAccount, account_id};

    #[test]
    fn projects_rfc21_args_to_address_and_camel_case_metadata() {
        let v = hex::decode("9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8")
            .expect("hex");
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(&v);
        let template = ScriptTemplate {
            code_hash,
            hash_type: 0x01,
        };
        let lock_args = hex::decode("b39bbc0b3673c7d36450bc14cfcdad2d559c6c64").expect("hex");
        let stored = StoredAccount {
            id: account_id(LockType::Secp256k1Blake160, &lock_args),
            label: "Main".into(),
            lock_type: LockType::Secp256k1Blake160,
            extension_id: "core.secp256k1".into(),
            lock_args,
            derivation: Some(Derivation {
                change: 0,
                index: 4,
            }),
            created_at: 1,
        };
        let caps = AccountCapabilities {
            can_sign: true,
            hardware: false,
        };
        let record = to_record(&stored, Network::Mainnet, &template, caps).expect("projects");
        assert_eq!(
            record.address,
            "ckb1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqdnnw7qkdnnclfkg59uzn8umtfd2kwxceqxwquc4"
        );
        assert_eq!(record.id, stored.id);
        assert_eq!(record.capabilities, caps);
        assert_eq!(
            record.public_metadata,
            serde_json::json!({
                "lockArgs": "0xb39bbc0b3673c7d36450bc14cfcdad2d559c6c64",
                "derivation": { "change": 0, "index": 4 }
            })
        );
    }
}

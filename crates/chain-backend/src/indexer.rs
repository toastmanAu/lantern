//! Client-side indexer request and response types.
//!
//! `ckb-jsonrpc-types` ships these with only the server-side serde half, so a
//! client cannot serialize its requests or deserialize its responses. These
//! mirror the wire format with both directions, as `ckb-sdk-rust` does.

use ckb_jsonrpc_types::{BlockNumber, CellOutput, JsonBytes, OutPoint, Script, Uint32};
use ckb_types::H256;
use serde::{Deserialize, Serialize};

/// Whether a search key matches a cell's lock or type script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    Lock,
    Type,
}

/// Result ordering by block number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Asc,
    Desc,
}

/// How the script args are matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Prefix,
    Exact,
    Partial,
}

/// Optional narrowing of a search. Every field is omitted when unset,
/// because a node rejects explicit nulls here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchKeyFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Script>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_data_len_range: Option<[Uint32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_capacity_range: Option<[ckb_jsonrpc_types::Uint64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_range: Option<[BlockNumber; 2]>,
}

/// The indexer's query key, shared by full nodes and light clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchKey {
    pub script: Script,
    pub script_type: ScriptType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_search_mode: Option<SearchMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<SearchKeyFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_data: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by_transaction: Option<bool>,
}

impl SearchKey {
    /// Search by lock script, the common case for a wallet.
    pub const fn lock(script: Script) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            script_search_mode: None,
            filter: None,
            with_data: None,
            group_by_transaction: None,
        }
    }
}

/// One cell as the indexer reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerCell {
    pub output: CellOutput,
    pub output_data: Option<JsonBytes>,
    pub out_point: OutPoint,
    pub block_number: BlockNumber,
    pub tx_index: Uint32,
}

/// A page of indexer results plus its raw continuation token.
///
/// `last_cursor` stays a plain `String` here so the sentinel survives
/// parsing; interpreting it is [`crate::cursor::CellPage`]'s job and only
/// its job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination<T> {
    pub objects: Vec<T>,
    pub last_cursor: String,
}

/// The indexer's own tip. `get_indexer_tip` returns `null` when the indexer
/// is disabled, which is how a full node's capability is probed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tip {
    pub block_hash: H256,
    pub block_number: BlockNumber,
}

/// A script the light client has been asked to watch, with how far it has synced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptStatus {
    pub script: Script,
    pub script_type: ScriptType,
    pub block_number: BlockNumber,
}

/// How `set_scripts` should treat the list it is given.
///
/// Lantern never sends `All`: it replaces the server's entire script list,
/// so two wallets pointed at one shared light client would erase each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetScriptsCommand {
    All,
    Partial,
    Delete,
}

/// A real `get_cells` response's single cell, reused by `indexer.rs` and
/// `cursor.rs` tests so both exercise the identical wire shape.
#[cfg(test)]
pub(crate) fn sample_cell() -> IndexerCell {
    serde_json::from_value(serde_json::json!({
        "block_number": "0x1554e00",
        "out_point": {
            "index": "0x0",
            "tx_hash": "0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"
        },
        "output": {
            "capacity": "0x1718c7e00",
            "lock": {
                "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                "hash_type": "type",
                "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
            },
            "type": null
        },
        "output_data": "0x",
        "tx_index": "0x1"
    }))
    .expect("sample cell")
}

#[cfg(test)]
mod tests {
    use super::{IndexerCell, Order, Pagination, ScriptType, SearchKey, Tip};

    fn secp_script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(serde_json::json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    #[test]
    fn search_key_serialises_to_the_wire_shape_a_node_accepts() {
        let key = SearchKey::lock(secp_script());
        let json = serde_json::to_value(&key).expect("serialises");
        assert_eq!(json["script_type"], "lock");
        assert_eq!(
            json["script"]["args"],
            "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        );
        // Optional fields must be omitted, not sent as null: a node rejects
        // `script_search_mode: null`.
        assert!(json.get("filter").is_none(), "{json}");
        assert!(json.get("script_search_mode").is_none(), "{json}");
        assert!(json.get("with_data").is_none(), "{json}");
    }

    #[test]
    fn order_and_script_type_use_the_wire_spelling() {
        assert_eq!(serde_json::to_value(Order::Asc).expect("ser"), "asc");
        assert_eq!(serde_json::to_value(Order::Desc).expect("ser"), "desc");
        assert_eq!(serde_json::to_value(ScriptType::Lock).expect("ser"), "lock");
        assert_eq!(serde_json::to_value(ScriptType::Type).expect("ser"), "type");
    }

    #[test]
    fn a_real_get_cells_response_deserialises() {
        // Shape taken from a live testnet get_cells response.
        let raw = serde_json::json!({
            "objects": [{
                "block_number": "0x1554e00",
                "out_point": {
                    "index": "0x0",
                    "tx_hash": "0x03e1abe59be2f5541d84590222048b4594318fa323e5ab0d377904cb84e624f4"
                },
                "output": {
                    "capacity": "0x1718c7e00",
                    "lock": {
                        "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                        "hash_type": "type",
                        "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
                    },
                    "type": null
                },
                "output_data": "0x",
                "tx_index": "0x1"
            }],
            "last_cursor": "0x40aabb"
        });
        let page: Pagination<IndexerCell> = serde_json::from_value(raw).expect("deserialises");
        assert_eq!(page.objects.len(), 1);
        assert_eq!(u64::from(page.objects[0].block_number), 22_367_744);
        assert_eq!(page.last_cursor, "0x40aabb");
        assert!(page.objects[0].output.type_.is_none());
    }

    #[test]
    fn an_exhausted_response_deserialises_with_the_sentinel_intact() {
        let raw = serde_json::json!({ "objects": [], "last_cursor": "0x" });
        let page: Pagination<IndexerCell> = serde_json::from_value(raw).expect("deserialises");
        assert!(page.objects.is_empty());
        assert_eq!(page.last_cursor, "0x", "the sentinel must survive parsing");
    }

    #[test]
    fn indexer_tip_is_optional_because_a_node_without_the_indexer_returns_null() {
        let present: Option<Tip> = serde_json::from_value(serde_json::json!({
            "block_hash": "0x8c94af53085ba511b1acba1fadd8d8215b45021f90fec7bf977687b6ee2103f1",
            "block_number": "0x1554ef4"
        }))
        .expect("deserialises");
        assert_eq!(u64::from(present.expect("some").block_number), 22_367_988);

        let absent: Option<Tip> = serde_json::from_value(serde_json::Value::Null).expect("null");
        assert!(absent.is_none(), "null means the indexer is off");
    }
}

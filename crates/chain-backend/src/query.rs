//! Wallet-facing query shapes, converted to indexer wire types at the edge.

use ckb_jsonrpc_types::{BlockNumber, Script, Uint32};

use crate::indexer::{Order, ScriptStatus, ScriptType, SearchKey, SearchMode};

/// A cell scan the wallet wants to run.
#[derive(Debug, Clone)]
pub struct CellQuery {
    pub script: Script,
    pub script_type: ScriptType,
    pub order: Order,
    pub limit: u32,
}

impl CellQuery {
    /// Scan by lock script, ascending, 100 cells per page.
    pub const fn lock(script: Script) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            order: Order::Asc,
            limit: 100,
        }
    }

    #[must_use]
    pub const fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    #[must_use]
    pub const fn with_order(mut self, order: Order) -> Self {
        self.order = order;
        self
    }

    /// The wire form of this query's key.
    ///
    /// `with_data` is sent explicitly rather than left to the node's default.
    /// `IndexerCell.output_data` is an `Option`, and a caller cannot tell
    /// "this cell holds no data" from "the node did not send any" — so a
    /// consumer deciding whether a cell is plain capacity would be relying on
    /// an unstated server-side default to be safe. Asking for the data makes
    /// the `None` case genuinely exceptional rather than routine.
    ///
    /// `script_search_mode` is sent as `Exact` for the same reason.
    /// Unset, a CKB indexer matches script args by PREFIX, so a query for one
    /// lock also returns cells under every lock whose args merely begin with
    /// it — a different script hash, and therefore a second script group.
    /// Every caller of this type writes a whole script and means it; none
    /// wants a prefix scan. Fixing it here rather than at one consumer is the
    /// point: a consumer-side filter leaves the next caller on the default.
    ///
    /// Both are real values, not `null`: the other optional fields stay
    /// omitted because a node rejects them as explicit nulls.
    pub fn search_key(&self) -> SearchKey {
        SearchKey {
            script: self.script.clone(),
            script_type: self.script_type,
            script_search_mode: Some(SearchMode::Exact),
            filter: None,
            with_data: Some(true),
            group_by_transaction: None,
        }
    }

    /// The limit as the node expects it: a hex-encoded `Uint32`.
    ///
    /// Not `const`: `Uint32::from(u32)` is a plain trait-provided `From`
    /// impl in `ckb-jsonrpc-types`, not a `const fn`.
    pub fn limit_param(&self) -> Uint32 {
        Uint32::from(self.limit)
    }
}

/// A script a light backend must watch, and the height to start from.
#[derive(Debug, Clone)]
pub struct WatchedScript {
    pub script: Script,
    pub script_type: ScriptType,
    /// Filters before this height are never downloaded, so this is the single
    /// biggest lever on first-sync time.
    pub from_block: u64,
}

impl WatchedScript {
    pub const fn lock(script: Script, from_block: u64) -> Self {
        Self {
            script,
            script_type: ScriptType::Lock,
            from_block,
        }
    }

    /// The wire form `set_scripts` takes.
    pub fn to_script_status(&self) -> ScriptStatus {
        ScriptStatus {
            script: self.script.clone(),
            script_type: self.script_type,
            block_number: BlockNumber::from(self.from_block),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CellQuery, WatchedScript};
    use crate::indexer::{Order, ScriptType};

    fn script() -> ckb_jsonrpc_types::Script {
        serde_json::from_value(serde_json::json!({
            "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
            "hash_type": "type",
            "args": "0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
        }))
        .expect("script")
    }

    #[test]
    fn a_lock_query_defaults_to_ascending_and_a_sane_limit() {
        let q = CellQuery::lock(script());
        assert_eq!(q.script_type, ScriptType::Lock);
        assert_eq!(q.order, Order::Asc);
        assert_eq!(q.limit, 100);
    }

    #[test]
    fn a_query_states_both_defaults_it_refuses_to_inherit_from_the_node() {
        // This is the shape that actually goes on the wire — `light.rs` and
        // `full.rs` both send `query.search_key()`.
        //
        // `with_data`: `IndexerCell.output_data` is an `Option`, so a consumer
        // that treats `None` as "no data" cannot distinguish a cell holding
        // nothing from one whose data the node simply did not send: a token
        // cell then reads as plain capacity. Asking explicitly is what makes
        // `None` exceptional.
        //
        // `script_search_mode`: unset, the indexer matches args by PREFIX, so
        // a scan for one lock also returns cells under any lock whose args
        // merely begin with it — a different script hash, and a second script
        // group inside one signature. This assertion previously required the
        // field to be ABSENT, which pinned the prefix default as if it were
        // the intent; it is changed deliberately.
        let json = serde_json::to_value(CellQuery::lock(script()).search_key()).expect("ser");
        assert_eq!(json["with_data"], true, "{json}");
        assert_eq!(json["script_search_mode"], "exact", "{json}");
        // ...and only those two: the rest stay omitted, because a node rejects
        // them as explicit nulls. `"exact"` is a real value, so it does not
        // fall foul of that rule.
        assert!(json.get("filter").is_none(), "{json}");
        assert!(json.get("group_by_transaction").is_none(), "{json}");
    }

    #[test]
    fn the_limit_is_sent_as_hex_because_the_node_expects_uint32() {
        let q = CellQuery::lock(script()).with_limit(16);
        assert_eq!(serde_json::to_value(q.limit_param()).expect("ser"), "0x10");
    }

    #[test]
    fn a_watched_script_carries_its_start_height() {
        let w = WatchedScript::lock(script(), 22_000_000);
        assert_eq!(w.from_block, 22_000_000);
        let json = serde_json::to_value(w.to_script_status()).expect("ser");
        assert_eq!(json["script_type"], "lock");
        assert_eq!(json["block_number"], "0x14fb180");
    }
}

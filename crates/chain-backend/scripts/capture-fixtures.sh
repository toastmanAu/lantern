#!/usr/bin/env bash
# Capture chain-backend test fixtures from a real CKB node.
#
# Fixtures are NEVER hand-written: a mock that invents an empty-page cursor is
# exactly how a silent paging bug shipped elsewhere. Re-run this and diff to
# confirm the committed fixtures still match reality.
#
#   ./scripts/capture-fixtures.sh [rpc-url]
set -euo pipefail

RPC="${1:-https://testnet.ckb.dev/}"
OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures"
mkdir -p "$OUT"

# A testnet lock that holds cells: the plan 1c sighash oracle's input.
FUNDED_ARGS="0x72f72b0cafd31de5072b10e84fc6c9d7d7596db7"
SECP_CODE_HASH="0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8"

call() { # name method params
  local name="$1" method="$2" params="$3"
  curl -sS -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params}" \
    | python3 -m json.tool > "$OUT/$name.json"
  echo "wrote $OUT/$name.json"
}

search_key() {
  printf '{"script":{"code_hash":"%s","hash_type":"type","args":"%s"},"script_type":"lock"}' \
    "$SECP_CODE_HASH" "$1"
}

call tip_header      get_tip_header   '[]'
call indexer_tip     get_indexer_tip  '[]'
call local_node_info local_node_info  '[]'
call cells_page      get_cells        "[$(search_key "$FUNDED_ARGS"),\"asc\",\"0x1\",null]"

# The important one: page to exhaustion so the empty page and its terminal
# cursor are captured verbatim rather than imagined.
cursor=$(python3 -c "import json;print(json.load(open('$OUT/cells_page.json'))['result']['last_cursor'])")
call cells_exhausted get_cells "[$(search_key "$FUNDED_ARGS"),\"asc\",\"0x40\",\"$cursor\"]"

python3 - "$OUT/cells_exhausted.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))["result"]
assert r["objects"] == [], "expected an exhausted page; widen the limit or pick a smaller lock"
assert r["last_cursor"] == "0x", f"expected the 0x sentinel, got {r['last_cursor']!r}"
print("verified: exhausted page returns objects=[] last_cursor='0x'")
PY

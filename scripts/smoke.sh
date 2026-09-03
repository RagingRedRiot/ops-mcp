#!/usr/bin/env bash
#
# smoke.sh — drive ops-mcp over stdio with real JSON-RPC and check the replies.
#
# No MCP client required. A stdio MCP server is just a process that reads
# newline-delimited JSON-RPC on stdin and writes it on stdout, so a pipe is a
# perfectly legitimate client. Needs jq; nothing else.
#
# Usage: scripts/smoke.sh [path-to-binary]     (default: target/debug/ops-mcp)

set -euo pipefail

BIN="${1:-target/debug/ops-mcp}"
[ -x "$BIN" ] || { echo "no executable at '$BIN' — run 'cargo build' first" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "smoke.sh needs jq" >&2; exit 1; }

# The handshake is mandatory: initialize, then the initialized notification,
# before any tool call. Ids let us match replies below; the notification has none.
OUT=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke.sh","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system_info","arguments":{"target":"local"}}}' \
  '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"system_info","arguments":{"target":"nas"}}}' \
  | timeout 10 "$BIN")

reply() { printf '%s' "$OUT" | jq -c "select(.id==$1)"; }

fail=0
check() { # check <label> <actual> <expected>
    if [ "$2" = "$3" ]; then
        printf '  ok    %s\n' "$1"
    else
        printf '  FAIL  %s\n        got:  %s\n        want: %s\n' "$1" "$2" "$3"
        fail=1
    fi
}

echo "smoke: $BIN"

check "server identifies as ops-mcp" \
    "$(reply 1 | jq -r '.result.serverInfo.name')" "ops-mcp"

check "advertises the system_info tool" \
    "$(reply 2 | jq -r '[.result.tools[].name] | join(",")')" "system_info"

check "target is a required parameter" \
    "$(reply 2 | jq -r '.result.tools[0].inputSchema.required | join(",")')" "target"

check "local target echoes itself back" \
    "$(reply 3 | jq -r '.result.structuredContent.target')" "local"

check "local target reports a hostname" \
    "$(reply 3 | jq -r '.result.structuredContent.hostname | length > 0')" "true"

check "unknown target is rejected as invalid_params" \
    "$(reply 4 | jq -r '.error.code')" "-32602"

echo
if [ "$fail" -eq 0 ]; then
    echo "all checks passed"
    printf '%s\n' "$(reply 3 | jq -c '.result.structuredContent')"
else
    echo "FAILURES — full transcript:" >&2
    printf '%s\n' "$OUT" | jq -c . >&2
    exit 1
fi

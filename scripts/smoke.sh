#!/usr/bin/env bash
#
# smoke.sh — drive stethoscope-mcp over stdio with real JSON-RPC and check the replies.
#
# No MCP client required. A stdio MCP server is just a process that reads
# newline-delimited JSON-RPC on stdin and writes it on stdout, so a pipe is a
# perfectly legitimate client. Needs jq; nothing else.
#
# Usage: scripts/smoke.sh [path-to-binary]     (default: target/debug/stethoscope-mcp)

set -euo pipefail

BIN="${1:-target/debug/stethoscope-mcp}"
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
  '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"system_health","arguments":{"target":"local"}}}' \
  '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"container_list","arguments":{"target":"local"}}}' \
  '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"container_health","arguments":{"target":"local","container":"definitely-not-a-container"}}}' \
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

check "server identifies as stethoscope-mcp" \
    "$(reply 1 | jq -r '.result.serverInfo.name')" "stethoscope-mcp"

check "advertises all four tools" \
    "$(reply 2 | jq -r '[.result.tools[].name] | sort | join(",")')" \
    "container_health,container_list,system_health,system_info"

# Every tool takes a target (decision 15), so assert it of all of them rather
# than of whichever happens to be first. Tools acting on one container require
# a container ID as well.
check "target is required on every tool" \
    "$(reply 2 | jq -r '[.result.tools[] | .inputSchema.required | index("target") != null] | all')" "true"

check "local target echoes itself back" \
    "$(reply 3 | jq -r '.result.structuredContent.target')" "local"

check "local target reports a hostname" \
    "$(reply 3 | jq -r '.result.structuredContent.hostname | length > 0')" "true"

check "unknown target is rejected as invalid_params" \
    "$(reply 4 | jq -r '.error.code')" "-32602"

check "system_health reports a positive cpu count" \
    "$(reply 5 | jq -r '.result.structuredContent.cpu.count > 0')" "true"

# meminfo reports KiB under a kB label; a machine with at least 256 MiB fails
# this if the conversion in meminfo_bytes is ever dropped (decision 18, rule 1).
check "memory is reported in bytes, not meminfo's kB" \
    "$(reply 5 | jq -r '.result.structuredContent.memory.total_bytes > 268435456')" "true"

# Sourced from the target's own clock (btime + uptime), so this also catches
# the arithmetic being wrong rather than merely the field being present.
check "collected_at is a fresh RFC 3339 timestamp" \
    "$(reply 5 | jq -r --argjson now "$(date +%s)" \
        '(.result.structuredContent.collected_at | fromdateiso8601) as $t
         | (($t - $now) | length) < 120')" "true"

check "load is accompanied by its denominator" \
    "$(reply 5 | jq -r '.result.structuredContent.cpu | has("load_1m") and has("count")')" "true"

# Decision 18, rule 4: collecting is this server's job, judging is not.
check "no verdict fields anywhere in the payload" \
    "$(reply 5 | jq -r '[.result.structuredContent | .. | objects | keys_unsorted[]]
                        | map(select(. == "status" or . == "health" or . == "severity"))
                        | length')" "0"

# Containers are discovered from the cgroup tree, so a host with none is a
# perfectly good answer — assert the shape, not that anything was found.
check "container_list returns a count matching its list" \
    "$(reply 6 | jq -r '.result.structuredContent
                        | .count == (.containers | length)')" "true"

check "container_list reports no names or images (decision 24)" \
    "$(reply 6 | jq -r '[.result.structuredContent.containers[]? | keys_unsorted[]]
                        | map(select(. == "name" or . == "image")) | length')" "0"

# An ID that does not exist is a malformed request, not an unreachable machine.
check "unknown container is rejected as invalid_params" \
    "$(reply 7 | jq -r '.error.code')" "-32602"

echo
if [ "$fail" -eq 0 ]; then
    echo "all checks passed"
    printf '%s\n' "$(reply 3 | jq -c '.result.structuredContent')"
    printf '%s\n' "$(reply 5 | jq -c '.result.structuredContent')"
else
    echo "FAILURES — full transcript:" >&2
    printf '%s\n' "$OUT" | jq -c . >&2
    exit 1
fi

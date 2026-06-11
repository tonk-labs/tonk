#!/usr/bin/env bash
# Trend over the last N scored runs (default 10).
set -euo pipefail
ROOT="${ROOT:?}"
N="${1:-10}"
IDX="$ROOT/bench/runs/index.jsonl"
[ -f "$IDX" ] || { echo "no runs yet" >&2; exit 1; }
echo "run | scenario | outcome | friction | failed | turns | wall(s) | tokens-out"
echo "--- | -------- | ------- | -------- | ------ | ----- | ------- | ----------"
tail -n "$N" "$IDX" | jq -r \
  '"\(.run) | \(.scenario) | \(.outcome) | \(.friction_count) | \(.failed_tool_results) | \(.num_turns) | \(.wall_seconds) | \(.tokens_out)"'

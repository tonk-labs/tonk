#!/usr/bin/env bash
# Trend over the last N scored runs (default 10).
set -euo pipefail
ROOT="${ROOT:?}"
N="${1:-10}"
IDX="$ROOT/bench/runs/index.jsonl"
[ -f "$IDX" ] || { echo "no runs yet" >&2; exit 1; }
echo "run | variant | scenario | outcome | first tonk/live/data/write | tonk failed/orient | friction | turns | wall(s) | tokens-out"
echo "--- | ------- | -------- | ------- | -------------------------- | ------------------ | -------- | ----- | ------- | ----------"
tail -n "$N" "$IDX" | jq -r \
  '"\(.run) | \(.variant // \"-\") | \(.scenario) | \(.outcome) | \(.first_tonk // \"-\")/\(.first_read // \"-\")/\(.first_data_read // \"-\")/\(.first_write // \"-\") | \(.failed_tonk_calls // \"-\")/\(.orientation_calls // \"-\") | \(.friction_count) | \(.num_turns) | \(.wall_seconds) | \(.tokens_out)"'

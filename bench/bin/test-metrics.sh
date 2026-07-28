#!/usr/bin/env bash
# Fast regression check for the trajectory classifier. No agent or stack.
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tonk-metrics-test.XXXXXX")"
trap 'rm -rf "$RUN_DIR"' EXIT

cp "$ROOT/bench/testdata/codex-first-use-episode.jsonl" "$RUN_DIR/episode.jsonl"
export RUN_DIR
"$ROOT/bench/bin/metrics.sh" >/dev/null

jq -e '
  .journey.cmds_before_first_success == 0
  and .journey.cmds_before_first_read == 0
  and .journey.cmds_before_first_data_read == 1
  and .journey.cmds_before_first_write == 3
  and .journey.tonk_calls == 5
  and .journey.failed_tonk_calls == 0
  and .journey.orientation_calls == 1
  and .journey.class_counts == {"orient": 1, "other": 1, "read": 2, "write": 2}
' "$RUN_DIR/metrics.json" >/dev/null

echo "metrics: trajectory classifier passed" >&2

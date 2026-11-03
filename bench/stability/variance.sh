#!/usr/bin/env bash
# Run the stability scenarios N times on one build and print, per scenario
# and metric, the values each run saw: the spread a tolerance has to cover.
#
# Usage: bench/stability/variance.sh [N] [--dist DIR]
# Writes bench/trace/runs/variance-<stamp>/variance.json and each run's results.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
N="${1:-8}"
shift || true
OUT="$ROOT/bench/trace/runs/variance-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
for i in $(seq 1 "$N"); do
  echo "variance: run $i of $N" >&2
  TRACE_DRIVER="$ROOT/bench/stability/stability.mjs" "$ROOT/bench/trace/run.sh" "$@" > "$OUT/run-$i.log" 2>&1
  dir="$(sed -n 's/^trace: run dir //p' "$OUT/run-$i.log" | head -1)"
  [ -f "$dir/stability.json" ] && cp "$dir/stability.json" "$OUT/run-$i.json" || echo "variance: run $i wrote no results" >&2
done

# Per scenario and metric: every run's value, plus min and max. Metrics
# that stayed 0 in every run are left out.
jq -s '
  [ .[] | .[] | {scenario, values: (.values + {settled: (if (.observed.settled_ms|type)=="number" then 1 else 0 end)})} ]
  | group_by(.scenario)
  | map({
      scenario: .[0].scenario,
      runs: length,
      metrics: ( [ .[].values | to_entries[] | select(.key|test("total_ms")|not) ]
        | group_by(.key)
        | map({key: .[0].key, value: ([.[].value] | {min: min, max: max, all: .})})
        | map(select(.value.max != 0 or .key == "settled"))
        | from_entries )
    })' "$OUT"/run-*.json > "$OUT/variance.json"
echo "variance: $OUT/variance.json" >&2
jq -r '.[] | .scenario as $s | .metrics | to_entries[] | "\($s)\t\(.key)\t\(.value.min)..\(.value.max)\t\(.value.all|map(tostring)|join(","))"' "$OUT/variance.json" 

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
output="$(
  python3 "$ROOT/bench/bin/compare.py" \
    first-use baseline treatment \
    --index "$ROOT/bench/testdata/experiment-index.jsonl"
)"

grep -q '^PASS statistically_significant$' <<<"$output"
grep -q '^PASS practical_effect$' <<<"$output"
grep -q '^decision: GRADUATE$' <<<"$output"

echo "compare: significance gate passed" >&2

#!/usr/bin/env bash
# Pixel-diff each screenshot against the scenario baseline (when one
# exists). Reported, never fatal — a diff may be the improvement.
# Writes visual-diff.json and diff images under shots/diff/.
#
# Scientific notation: compare -metric AE can emit values like "4e+06 (1)".
# We extract the first token and normalize via awk printf "%.0f" so large
# AE counts are never silently zeroed.
#
# Dimension mismatch: IM7 compare pads/crops rather than erroring, so we
# detect mismatched dimensions by comparing identify output before running
# compare. When sizes differ the entry is omitted from visual-diff.json
# (not a meaningful pixel-diff).
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO_NAME="${SCENARIO_NAME:?}"
BASE="$ROOT/bench/baselines/$SCENARIO_NAME"
[ -d "$BASE" ] || { echo "visual-diff: no baseline for $SCENARIO_NAME, skipping" >&2; exit 0; }
mkdir -p "$RUN_DIR/shots/diff"

results="[]"
for png in "$RUN_DIR/shots"/*.png; do
  [ -e "$png" ] || continue
  name="$(basename "$png")"
  ref="$BASE/$name"
  [ -f "$ref" ] || continue

  # Skip incomparable pairs: IM7 silently pads on dimension mismatch, which
  # would produce a meaningless AE count. Compare dimensions first.
  cur_dims=$(identify -format '%wx%h' "$png" 2>/dev/null || true)
  ref_dims=$(identify -format '%wx%h' "$ref" 2>/dev/null || true)
  if [ "$cur_dims" != "$ref_dims" ]; then
    echo "visual-diff: $name — dimension mismatch ($ref_dims vs $cur_dims), skipping" >&2
    continue
  fi

  pixels=$(identify -format '%w %h' "$png" | awk '{print $1 * $2}')

  # compare exits 1 on any difference; capture stderr (where AE goes) with 2>&1.
  # Extract the first whitespace-delimited token (the raw AE count, possibly in
  # scientific notation like "4e+06") and normalize to an integer via awk.
  ae_raw=$(compare -metric AE -fuzz 2% "$ref" "$png" "$RUN_DIR/shots/diff/$name" 2>&1 || true)
  ae=$(echo "$ae_raw" | awk '{printf "%.0f", $1}')
  # If awk produced nothing (e.g. empty output from compare), treat as 0.
  [ -n "$ae" ] || ae=0

  pct=$(awk -v a="$ae" -v p="$pixels" 'BEGIN { printf "%.2f", (p > 0 ? 100 * a / p : 0) }')
  results=$(printf '%s' "$results" | jq --arg s "$name" --arg p "$pct" '. + [{shot: $s, diff_pct: ($p | tonumber)}]')
done
printf '%s\n' "$results" | jq . > "$RUN_DIR/visual-diff.json"
jq . "$RUN_DIR/visual-diff.json" >&2

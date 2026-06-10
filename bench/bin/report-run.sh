#!/usr/bin/env bash
# Render report.md for one run and append a summary line to
# runs/index.jsonl. Tolerates scripted runs (no scores.json).
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO_NAME="${SCENARIO_NAME:-unknown}"

{
  echo "# Bench run: $SCENARIO_NAME — $(basename "$RUN_DIR")"
  echo
  if [ -f "$RUN_DIR/scores.json" ]; then
    echo "## Scores"
    echo
    jq -r '"- outcome: \(.judge.outcome)/10",
           "- wall: \(.wall_seconds)s, turns: \(.num_turns), tool calls: \(.tool_calls)",
           "- failed tool results: \(.failed_tool_results), repeated commands: \(.repeated_commands | length)",
           "- tokens out: \(.tokens.output)"' "$RUN_DIR/scores.json"
    echo
    echo "## Friction"
    echo
    # Use || true inside the group so pipefail doesn't fire on empty arrays
    jq -r '.judge.friction[] | "- **\(.what)**\n  - evidence: \(.evidence)\n  - suggested fix: \(.suggested_fix)"' \
      "$RUN_DIR/scores.json" 2>/dev/null || true
    # If friction array was empty jq above prints nothing; emit the placeholder
    friction_count="$(jq '.judge.friction | length' "$RUN_DIR/scores.json")"
    [ "$friction_count" -gt 0 ] || echo "(none)"
    echo
    echo "## Judge notes"
    echo
    jq -r '.judge.notes // "(none)"' "$RUN_DIR/scores.json"
    echo
  else
    echo "(scripted run — no episode scores)"
    echo
  fi
  echo "## Screenshots"
  echo
  shots_found=0
  if [ -d "$RUN_DIR/shots" ]; then
    for png in "$RUN_DIR/shots"/*.png; do
      [ -e "$png" ] || continue
      echo "- ![$(basename "$png")](shots/$(basename "$png"))"
      shots_found=1
    done
    if [ -f "$RUN_DIR/shots/MISSING" ]; then
      echo
      echo "Missing captures:"
      sed 's/^/- /' "$RUN_DIR/shots/MISSING"
    fi
  fi
  [ "$shots_found" -eq 1 ] || echo "(no screenshots)"
  if [ -f "$RUN_DIR/visual-diff.json" ]; then
    echo
    echo "## Visual diff vs baseline"
    echo
    jq -r '.[] | "- \(.shot): \(.diff_pct)% pixels differ"' "$RUN_DIR/visual-diff.json"
  fi
} > "$RUN_DIR/report.md"

if [ -f "$RUN_DIR/scores.json" ]; then
  jq -c '{run, scenario, outcome: .judge.outcome, wall_seconds, num_turns,
          tool_calls, failed_tool_results, tokens_out: .tokens.output,
          friction_count: (.judge.friction | length)}' \
    "$RUN_DIR/scores.json" >> "$ROOT/bench/runs/index.jsonl"
fi
echo "report: $RUN_DIR/report.md" >&2

#!/usr/bin/env bash
# Judge the run: a headless claude reads the rubric, the screenshots,
# and a condensed transcript, and returns strict JSON. One retry on
# invalid output. Mechanical numbers come from metrics.sh, never from
# the judge. Writes judge.json and scores.json.
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
SCENARIO_NAME="${SCENARIO_NAME:-$(basename "$SCENARIO")}"
JUDGE_TIMEOUT="${JUDGE_TIMEOUT:-600}"

# Condensed transcript: agent text, commands run, and errors — enough
# for friction analysis without the full stream.
if [ -s "$RUN_DIR/episode.jsonl" ]; then
  jq -r '
    select(.type == "assistant") | .message.content[]? |
    if .type == "text" then "AGENT: \(.text)"
    elif .type == "tool_use" and .name == "Bash" then "RAN: \(.input.command)"
    elif .type == "tool_use" then "TOOL \(.name): \(.input | tostring | .[0:200])"
    else empty end
  ' "$RUN_DIR/episode.jsonl" > "$RUN_DIR/episode-summary.txt"
  jq -r '
    select(.type == "user") | .message.content[]? |
    select(.type == "tool_result" and .is_error == true) |
    "ERROR: \(.content | tostring | .[0:500])"
  ' "$RUN_DIR/episode.jsonl" >> "$RUN_DIR/episode-summary.txt"
else
  # Missing or empty transcript — judge scores screenshots only.
  : > "$RUN_DIR/episode-summary.txt"
fi

shots_list=$(ls "$RUN_DIR/shots"/*.png 2>/dev/null || true)

prompt() {
  cat <<EOF
You are judging one benchmark run of an agent that used the slide CLI
to build something rendered by the tonk web UI.

Rubric:
$(cat "$SCENARIO/rubric.md")

Read each screenshot listed below with the Read tool, then read the
condensed transcript. Screenshots are the ground truth for outcome;
the transcript is the ground truth for friction.

Screenshots:
$shots_list
$([ -f "$RUN_DIR/shots/MISSING" ] && { echo "Missing (failed to capture):"; cat "$RUN_DIR/shots/MISSING"; } || true)
$([ -f "$RUN_DIR/shots/reference.png" ] && echo "reference.png is the original artifact the agent was converting; compare fidelity against it." || true)

Transcript: $RUN_DIR/episode-summary.txt
Mechanical metrics (context only, do not recompute): $RUN_DIR/metrics.json

Respond with ONLY a JSON object, no markdown fences, matching:
{
  "outcome": <0-10>,
  "friction": [{"what": "...", "evidence": "<transcript excerpt>", "suggested_fix": "..."}],
  "notes": "<one paragraph>"
}
EOF
}

# Auth: default to the claude CLI's logged-in (OAuth) session by
# stripping ANTHROPIC_API_KEY; BENCH_USE_API_KEY=1 opts into the API
# key, resolving an op:// reference via `op read`.
KEY_ENV=(-u ANTHROPIC_API_KEY)
if [ -n "${BENCH_USE_API_KEY:-}" ]; then
  RESOLVED_KEY="${ANTHROPIC_API_KEY:-}"
  if [[ "$RESOLVED_KEY" == op://* ]]; then
    RESOLVED_KEY="$(op read "$RESOLVED_KEY")"
  fi
  KEY_ENV=(ANTHROPIC_API_KEY="$RESOLVED_KEY")
fi

run_judge() {
  ( cd "$RUN_DIR" && \
    env "${KEY_ENV[@]}" \
    timeout -k 30 "$JUDGE_TIMEOUT" claude -p "$(prompt)" \
      --allowedTools "Read" \
      --output-format json \
  ) \
    | jq -r '.result' \
    | sed -e 's/^```json//' -e 's/^```//' -e 's/```$//'
}

valid() { jq -e 'has("outcome") and (.outcome | type == "number") and has("friction")' >/dev/null 2>&1; }

# `|| true`: a claude/timeout failure must fall through to the retry,
# not kill the script via set -e before validation runs.
out="$(run_judge)" || true
if ! printf '%s' "$out" | valid; then
  echo "judge: invalid JSON, retrying" >&2
  out="$(run_judge)" || true
  printf '%s' "$out" | valid || { echo "judge: invalid JSON twice" >&2; printf '%s\n' "$out" > "$RUN_DIR/judge-raw.txt"; exit 1; }
fi
printf '%s\n' "$out" | jq . > "$RUN_DIR/judge.json"

jq -s '.[0] * {judge: .[1]}' "$RUN_DIR/metrics.json" "$RUN_DIR/judge.json" \
  | jq --arg scenario "$SCENARIO_NAME" --arg run "$(basename "$RUN_DIR")" \
       '{scenario: $scenario, run: $run} + .' \
  > "$RUN_DIR/scores.json"
jq . "$RUN_DIR/scores.json" >&2

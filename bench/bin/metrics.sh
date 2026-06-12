#!/usr/bin/env bash
# Compute everything computable without a judge from the episode
# transcript: wall clock, tokens, turns, tool calls, failed bash
# invocations, repeated identical commands. Writes metrics.json.
set -euo pipefail

RUN_DIR="${RUN_DIR:?}"
T="$RUN_DIR/episode.jsonl"

# Wall clock — degrade gracefully if stamps are missing or empty.
start_file="$RUN_DIR/episode-start"
end_file="$RUN_DIR/episode-end"
wall=null
if [ -s "$start_file" ] && [ -s "$end_file" ]; then
  start_ts=$(cat "$start_file")
  end_ts=$(cat "$end_file")
  if [[ "$start_ts" =~ ^[0-9]+$ ]] && [[ "$end_ts" =~ ^[0-9]+$ ]]; then
    wall=$(( end_ts - start_ts ))
  fi
fi

# Exit code — degrade to null if file missing or malformed.
exit_code=null
exit_file="$RUN_DIR/episode-exit.json"
if [ -f "$exit_file" ]; then
  exit_code=$(jq -r '.episode_exit // empty' "$exit_file" 2>/dev/null || true)
  [[ "$exit_code" =~ ^[0-9]+$ ]] || exit_code=null
fi

# If there is no transcript (empty or missing), emit a zeroed metrics.json.
if [ ! -s "$T" ]; then
  jq -n \
    --argjson wall "$wall" \
    --argjson exit_code "$exit_code" '
  {
    wall_seconds:         $wall,
    episode_exit:         $exit_code,
    num_turns:            null,
    duration_ms:          null,
    tokens: {
      input:              null,
      output:             null,
      cache_read:         null
    },
    tool_calls:           0,
    bash_calls:           0,
    failed_tool_results:  0,
    repeated_commands:    []
  }' > "$RUN_DIR/metrics.json"
  jq . "$RUN_DIR/metrics.json" >&2
  exit 0
fi

jq -s \
  --argjson wall "$wall" \
  --argjson exit_code "$exit_code" '
  (map(select(.type == "result")) | first) as $result |
  ([.[] | select(.type == "assistant") | .message.content[]? | select(.type == "tool_use")]) as $tools |
  ([.[] | select(.type == "user") | .message.content[]? | select(.type == "tool_result" and .is_error == true)]) as $errors |
  ([$tools[] | select(.name == "Bash") | .input.command]) as $cmds |
  {
    wall_seconds:         $wall,
    episode_exit:         $exit_code,
    num_turns:            ($result.num_turns // null),
    duration_ms:          ($result.duration_ms // null),
    tokens: {
      input:              ($result.usage.input_tokens // null),
      output:             ($result.usage.output_tokens // null),
      cache_read:         ($result.usage.cache_read_input_tokens // null)
    },
    tool_calls:           ($tools | length),
    bash_calls:           ($cmds | length),
    failed_tool_results:  ($errors | length),
    repeated_commands:    ($cmds | group_by(.) | map(select(length > 1) | {command: .[0], times: length}))
  }' "$T" > "$RUN_DIR/metrics.json"

jq . "$RUN_DIR/metrics.json" >&2

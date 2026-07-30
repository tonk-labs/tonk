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
    repeated_commands:    [],
    journey: {
      cmds_before_join:            null,
      cmds_before_first_eval:      null,
      cmds_before_first_success:   null,
      cmds_before_first_read:      null,
      cmds_before_first_data_read: null,
      cmds_before_first_write:     null,
      first_success:               null,
      first_read:                  null,
      first_data_read:             null,
      first_write:                 null,
      tonk_calls:                  0,
      failed_tonk_calls:           0,
      orientation_calls:           0,
      dry_runs:                    0,
      class_counts:                {},
      doc_fetches:                 0,
      ask_user_calls:              0
    }
  }' > "$RUN_DIR/metrics.json"
  jq . "$RUN_DIR/metrics.json" >&2
  exit 0
fi

first_type="$(head -1 "$T" | jq -r '.type // empty')"

JOURNEY_DEF='
  # Shell episodes wrap commands in several ways (`zsh -lc`, full binary
  # paths, env assignments, or `npx --yes @tonk/cli`). Match an executable in
  # command position: a docs search containing the word "tonk" is not a call.
  def is_tonk:
    test("(^|[;&][[:space:]]*|[|][|]?[[:space:]]+|-[A-Za-z]*c[A-Za-z]*[[:space:]]+[\u0027\"]?)(env[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]+[[:space:]]+)*(([^[:space:]\u0027\";&|]+/)?tonk|npx([[:space:]]+--yes)?[[:space:]]+@tonk/cli)([[:space:]\u0027\";&|]|$)");
  def has_subcommand($pattern):
    test(
      "(tonk|@tonk/cli)"
      + "([[:space:]]+--spot[[:space:]]+[^[:space:]\u0027\"]+)?"
      + "[[:space:]]+(" + $pattern + ")([[:space:]\u0027\"]|$)"
    );
  def is_bare_tonk:
    is_tonk and test("(tonk|@tonk/cli)([[:space:]]+--spot[[:space:]]+[^[:space:]]+)?[^A-Za-z0-9_.-]*$");
  def is_agents_set:
    is_tonk and has_subcommand("agents[[:space:]]+set");
  def is_agents_read:
    is_tonk and has_subcommand("agents") and (is_agents_set | not);
  def requests_help:
    is_tonk and has_subcommand(
      "--help|help"
      + "|[^[:space:]]+[[:space:]]+(-h|--help)"
      + "|[^[:space:]]+[[:space:]]+[^[:space:]]+[[:space:]]+(-h|--help)"
    );
  def is_orientation:
    is_bare_tonk
    or requests_help
    or is_agents_read
    or (is_tonk and has_subcommand("context|guide|schema|status|concept[[:space:]]+ls|view[[:space:]]+ls"));
  def is_live_read:
    is_tonk and (
      (
        has_subcommand("context|schema|query|render|status|concept[[:space:]]+ls|view[[:space:]]+ls")
        and (requests_help | not)
      )
      or is_agents_read
      or is_bare_tonk
    );
  def is_direct_data_read:
    is_tonk and has_subcommand("query|render") and (requests_help | not);
  def is_eval:
    is_tonk and has_subcommand("eval");
  def is_explicit_content_write:
    is_tonk
    and has_subcommand("concept[[:space:]]+add|view[[:space:]]+add|agents[[:space:]]+set|assert|retract|home|import|join")
    and (test("--help|--dry-run") | not);
  def revision_changed:
    (
      (
        try (
          capture("(?s)revision-before:[[:space:]]+[\u0027\"]?(?<before>#[^\u0027\"\\n]+)[\u0027\"]?.*revision-after:[[:space:]]+[\u0027\"]?(?<after>#[^\u0027\"\\n]+)")
          | .before != .after
        ) catch false
      ) // false
    )
    or test("\"claims\"[[:space:]]*:[[:space:]]*[1-9][0-9]*");
  def row_is_content_write:
    (.command | is_explicit_content_write)
    or (
      (.command | is_eval)
      and ((.command | test("--dry-run")) | not)
      and (.output | revision_changed)
    );
  def row_is_data_read:
    (.command | is_direct_data_read)
    or (
      (.command | is_eval)
      and ((.output | revision_changed) | not)
    );
  def row_is_live_read:
    (.command | is_live_read) or row_is_data_read;
  def row_class:
    if (.command | test("--dry-run")) then "probe"
    elif (.command | is_orientation) then "orient"
    elif row_is_content_write then "write"
    elif row_is_data_read then "read"
    else "other"
    end;
  def first_matching($rows; predicate):
    ($rows | to_entries | map(select(.value.ok and (.value | predicate))) | first);
  def journey($rows):
    (first_matching($rows; (.command | is_tonk))) as $first_success |
    (first_matching($rows; row_is_live_read)) as $first_read |
    (first_matching($rows; row_is_data_read)) as $first_data_read |
    (first_matching($rows; row_is_content_write)) as $first_write |
    ($rows | map(.command)) as $cmds |
    ($cmds | to_entries) as $e |
    ($rows | map(row_class)) as $classes |
    {
      cmds_before_join:            (($e | map(select(.value | has_subcommand("join"))) | first | .key) // null),
      cmds_before_first_eval:      (($e | map(select(.value | has_subcommand("eval"))) | first | .key) // null),
      cmds_before_first_success:   ($first_success.key // null),
      cmds_before_first_read:      ($first_read.key // null),
      cmds_before_first_data_read: ($first_data_read.key // null),
      cmds_before_first_write:     ($first_write.key // null),
      first_success:               ($first_success.value.command // null),
      first_read:                  ($first_read.value.command // null),
      first_data_read:             ($first_data_read.value.command // null),
      first_write:                 ($first_write.value.command // null),
      tonk_calls:                  ([$rows[] | select(.command | is_tonk)] | length),
      failed_tonk_calls:           ([$rows[] | select((.command | is_tonk) and (.ok | not))] | length),
      orientation_calls:           ([$rows[] | select(.command | is_orientation)] | length),
      dry_runs:                    ([$cmds[] | select(test("--dry-run"))] | length),
      class_counts:                ($classes | group_by(.) | map({key: .[0], value: length}) | from_entries),
      doc_fetches:                 ([$cmds[] | select(test("tonk +guide|llms(-full)?\\.txt"))] | length),
      ask_user_calls:              ([$cmds[] | select(test("(^|[ /;&|])ask-user( |$|\")"))] | length)
    };
'

if [ "$first_type" = "thread.started" ]; then
  jq -s \
    --argjson wall "$wall" \
    --argjson exit_code "$exit_code" "$JOURNEY_DEF"'
    ([.[] | select(.type == "item.completed") | .item
      | select(.type == "command_execution" or .type == "file_change")]) as $tools |
    ([$tools[] | select(.type == "command_execution")]) as $execs |
    ([$execs[] | {
      command,
      ok: (.status == "completed" and ((.exit_code // 0) == 0)),
      output: (.aggregated_output // "")
    }]) as $rows |
    ([$rows[].command]) as $cmds |
    ([.[] | select(.type == "turn.completed") | .usage] | map(select(. != null))) as $usages |
    {
      wall_seconds:         $wall,
      episode_exit:         $exit_code,
      num_turns:            ([.[] | select(.type == "turn.completed")] | length),
      duration_ms:          null,
      tokens: {
        input:              (if ($usages|length) > 0 then ($usages | map(.input_tokens // 0) | add) else null end),
        output:             (if ($usages|length) > 0 then ($usages | map(.output_tokens // 0) | add) else null end),
        cache_read:         (if ($usages|length) > 0 then ($usages | map(.cached_input_tokens // 0) | add) else null end)
      },
      tool_calls:           ($tools | length),
      bash_calls:           ($execs | length),
      failed_tool_results:  ([$execs[] | select(.status == "failed" or ((.exit_code // 0) != 0))] | length),
      repeated_commands:    ($cmds | group_by(.) | map(select(length > 1) | {command: .[0], times: length})),
      journey:              journey($rows)
    }' "$T" > "$RUN_DIR/metrics.json"
else
  jq -s \
    --argjson wall "$wall" \
    --argjson exit_code "$exit_code" "$JOURNEY_DEF"'
    (map(select(.type == "result")) | first) as $result |
    ([.[] | select(.type == "assistant") | .message.content[]? | select(.type == "tool_use")]) as $tools |
    ([.[] | select(.type == "user") | .message.content[]? | select(.type == "tool_result")]) as $results |
    ([$results[] | select(.is_error == true)]) as $errors |
    ([$errors[].tool_use_id]) as $error_ids |
    ([$tools[] | select(.name == "Bash") | . as $tool | {
      command: $tool.input.command,
      ok: (($tool.id as $id | $error_ids | index($id)) == null),
      output: (
        [$results[] | select(.tool_use_id == $tool.id) | .content]
        | first // ""
        | if type == "string" then . else tostring end
      )
    }]) as $rows |
    ([$rows[].command]) as $cmds |
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
      repeated_commands:    ($cmds | group_by(.) | map(select(length > 1) | {command: .[0], times: length})),
      journey:              journey($rows)
    }' "$T" > "$RUN_DIR/metrics.json"
fi

jq . "$RUN_DIR/metrics.json" >&2

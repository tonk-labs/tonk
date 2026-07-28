#!/usr/bin/env bash
# Paired two-episode experiment for claim-backed spot agent context.
#
# For each pair:
#   A completes a normal task and records one opaque durable convention.
#   B-control sees the frozen pre-A AGENTS.md projection.
#   B-treatment sees the live post-A projection.
# Both B arms use identical copies of the post-A spot and can discover the
# current claim through the CLI. Only automatic pre-launch orientation differs.
set -euo pipefail

ROOT="${ROOT:?}"
TONK="$ROOT/target/release/tonk"
SCENARIO="$ROOT/bench/scenarios/agents-handoff"
PAIRS=3
SCRIPTED=0
VARIANT="claim-handoff-pilot"
# The pilot used 9 episodes. The 10-pair confirmation budget was approved on
# 2026-07-28; keep this hard cap so a typo cannot launch replacement episodes.
APPROVED_EPISODES=30

while [ $# -gt 0 ]; do
  case "$1" in
    --pairs) PAIRS="$2"; shift ;;
    --scripted) SCRIPTED=1 ;;
    --variant) VARIANT="$2"; shift ;;
    *) echo "handoff: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done

[[ "$PAIRS" =~ ^[1-9][0-9]*$ ]] || {
  echo "handoff: --pairs must be a positive integer" >&2
  exit 2
}
[[ "$VARIANT" =~ ^[A-Za-z0-9._-]+$ ]] || {
  echo "handoff: invalid variant '$VARIANT'" >&2
  exit 2
}
[ -x "$TONK" ] || {
  echo "handoff: build $TONK first" >&2
  exit 1
}

planned_episodes=$((PAIRS * 3))
if [ "$SCRIPTED" != 1 ] && [ "$planned_episodes" -gt "$APPROVED_EPISODES" ]; then
  echo "handoff: $planned_episodes episodes exceed the approved budget of $APPROVED_EPISODES" >&2
  exit 2
fi

export TONK_NO_UPDATE_CHECK=1
export TONK_NO_SYNC=1
REAL_HOME="$HOME"
CODEX_AUTH_HOME="${CODEX_HOME:-$REAL_HOME/.codex}"
BATCH_ID="$(date +%Y%m%d-%H%M%S)-agents-handoff-$VARIANT"
BATCH_DIR="$ROOT/bench/runs/$BATCH_ID"
mkdir -p "$BATCH_DIR"

revision="$(git -C "$ROOT" rev-parse HEAD)"
if [ -n "$(git -C "$ROOT" status --short)" ]; then
  dirty=true
else
  dirty=false
fi

jq -n \
  --arg variant "$VARIANT" \
  --arg revision "$revision" \
  --arg model "${CODEX_MODEL:-gpt-5.5}" \
  --argjson dirty "$dirty" \
  --argjson scripted "$([ "$SCRIPTED" = 1 ] && echo true || echo false)" \
  --argjson pairs "$PAIRS" \
  --argjson planned_episodes "$planned_episodes" \
  --argjson approved_episodes "$APPROVED_EPISODES" \
  '{
    scenario: "agents-handoff",
    variant: $variant,
    revision: $revision,
    dirty: $dirty,
    runner: "codex",
    model: $model,
    scripted: $scripted,
    pairs: $pairs,
    planned_episodes: $planned_episodes,
    approved_episodes: $approved_episodes
  }' > "$BATCH_DIR/experiment.json"

run_tonk() {
  local profile_home="$1"
  local spots_state="$2"
  shift 2
  mkdir -p "$profile_home" "$spots_state"
  env \
    HOME="$profile_home" \
    DO_NOT_TRACK=1 \
    TONK_NO_UPDATE_CHECK=1 \
    TONK_NO_SYNC=1 \
    TONK_SPOTS_STATE="$spots_state" \
    TONK_TELEMETRY_STATE="$spots_state" \
    TONK_UPDATE_STATE="$spots_state" \
    TONK_SPOT=bench \
    "$TONK" "$@"
}

write_a_prompt() {
  local destination="$1"
  local owner_label="$2"
  printf '%s\n' \
    'The selected Tonk spot contains one launch task. Mark “Draft launch email” done. Change no other task data.' \
    '' \
    "The product owner has also established a durable spot convention: future security-review tasks are owned by \`$owner_label\`." \
    'Preserve that convention in the spot agent context so a fresh agent can use it later. Do not record this task’s completion status or copy the prompt into that context.' \
    '' \
    'The `tonk` CLI is on PATH. Do not inspect repository source files or search the filesystem for documentation; use the installed CLI and the projected spot context. Stop when both changes are persisted.' \
    > "$destination"
}

write_b_prompt() {
  local destination="$1"
  printf '%s\n' \
    'A teammate needs the established owner label for future security-review tasks in the selected Tonk spot.' \
    '' \
    'Write only the exact owner label to `answer.txt`, with no explanation. Do not change the spot.' \
    '' \
    'The `tonk` CLI is on PATH. Do not inspect repository source files or search the filesystem for documentation; use the installed CLI and the projected spot context. Stop after writing the answer.' \
    > "$destination"
}

write_scripted_metrics() {
  local destination="$1"
  local status="$2"
  jq -n --argjson episode_exit "$status" '{
    wall_seconds: 0,
    episode_exit: $episode_exit,
    num_turns: 0,
    tokens: {input: 0, output: 0, cache_read: 0},
    tool_calls: 0,
    bash_calls: 0,
    failed_tool_results: 0,
    journey: {
      tonk_calls: 0,
      failed_tonk_calls: 0,
      orientation_calls: 0
    }
  }' > "$destination"
}

episodes_run=0
run_episode() {
  local role="$1"
  local episode_dir="$2"
  local spots_state="$3"
  local site_dir="$4"
  local prompt="$5"
  local projection="$6"
  local owner_label="$7"

  mkdir -p "$episode_dir/cwd" "$episode_dir/home"
  cp "$projection" "$episode_dir/cwd/AGENTS.md"
  cp "$prompt" "$episode_dir/prompt.md"

  jq -n \
    --arg role "$role" \
    --arg variant "$VARIANT" \
    --arg revision "$revision" \
    --arg model "${CODEX_MODEL:-gpt-5.5}" \
    '{
      scenario: "agents-handoff",
      role: $role,
      variant: $variant,
      revision: $revision,
      runner: "codex",
      model: $model
    }' > "$episode_dir/experiment.json"

  local status=0
  if [ "$SCRIPTED" = 1 ]; then
    date +%s > "$episode_dir/episode-start"
    case "$role" in
      a)
        local query entity
        query="$(run_tonk "$episode_dir/home" "$spots_state" query task --json)"
        entity="$(
          jq -r '
            [
              .[]
              | select(.title == "Draft launch email")
              | .this
            ][0] // empty
          ' <<< "$query"
        )"
        [ -n "$entity" ] || {
          echo "handoff: scripted A could not resolve task entity" >&2
          status=1
        }
        if [ "$status" = 0 ]; then
          run_tonk "$episode_dir/home" "$spots_state" \
            assert task "$entity" --done true > "$episode_dir/scripted-a.out"
          printf '\n## Durable conventions\n\n- Future security-review tasks are owned by `%s`.\n' \
            "$owner_label" >> "$episode_dir/cwd/AGENTS.md"
          run_tonk "$episode_dir/home" "$spots_state" \
            agents set "$episode_dir/cwd/AGENTS.md" >> "$episode_dir/scripted-a.out"
        fi
        ;;
      b-control|b-treatment)
        run_tonk "$episode_dir/home" "$spots_state" agents --json \
          > "$episode_dir/scripted-b-claim.json"
        printf '%s\n' "$owner_label" > "$episode_dir/cwd/answer.txt"
        ;;
      *) echo "handoff: unknown scripted role $role" >&2; status=2 ;;
    esac
    date +%s > "$episode_dir/episode-end"
  else
    episodes_run=$((episodes_run + 1))
    if [ "$episodes_run" -gt "$APPROVED_EPISODES" ]; then
      echo "handoff: refusing episode $episodes_run; approved budget is $APPROVED_EPISODES" >&2
      return 2
    fi
    echo "handoff: external episode $episodes_run/$planned_episodes ($role)" >&2
    (
      export ROOT
      export RUN_DIR="$episode_dir"
      export SCENARIO
      export EPISODE_DIR="$episode_dir/cwd"
      export EPISODE_HOME="$episode_dir/home"
      export EPISODE_RUNNER=codex
      export EPISODE_TIMEOUT=600
      export EPISODE_SPOT=bench
      export EPISODE_SPOTS_STATE="$spots_state"
      export EPISODE_SITE_DIR="$site_dir"
      export CODEX_HOME="$CODEX_AUTH_HOME"
      export CODEX_MODEL="${CODEX_MODEL:-gpt-5.5}"
      "$ROOT/bench/bin/episode.sh"
    ) || status=$?
  fi
  jq -n --argjson episode_exit "$status" '{episode_exit: $episode_exit}' \
    > "$episode_dir/episode-exit.json"
  if [ "$SCRIPTED" = 1 ]; then
    write_scripted_metrics "$episode_dir/metrics.json" "$status"
  else
    RUN_DIR="$episode_dir" "$ROOT/bench/bin/metrics.sh" >/dev/null
  fi
  return 0
}

verify_a() {
  local pair_dir="$1"
  local profile_home="$2"
  local spots_state="$3"
  local owner_label="$4"
  local repository_did="$5"
  local response
  response="$(
    run_tonk "$profile_home" "$spots_state" eval -c 'task:
  this: ?task
  title: ?title
  done: ?done' --format json --no-sync
  )"
  jq -n \
    --slurpfile pre "$pair_dir/pre-claim.json" \
    --slurpfile post "$pair_dir/post-claim.json" \
    --slurpfile exit "$pair_dir/a/episode-exit.json" \
    --argjson response "$response" \
    --arg owner_label "$owner_label" \
    --arg repository_did "$repository_did" '
    [
      $response.matches_after[]
      | select(.label == "task")
      | .results[]
      | {this, title: .fields.title, done: .fields.done}
    ] as $tasks
    | ($post[0].markdown // "") as $markdown
    | {
        task_passed: (
          ($tasks | length) == 1
          and $tasks[0].title == "Draft launch email"
          and $tasks[0].done == true
        ),
        entity_passed: (
          $pre[0].entity == $repository_did
          and $post[0].entity == $repository_did
          and $post[0].source == "dialog-claim"
          and $post[0].attribute == "xyz.tonk.repo/agents"
        ),
        revision_changed: $post[0].revision != $pre[0].revision,
        markdown_changed: $post[0].markdown != $pre[0].markdown,
        convention_retained: (
          ($markdown | contains($owner_label))
          and ($markdown | test("security"; "i"))
          and ($markdown | test("own"; "i"))
        ),
        hygiene_passed: (
          ($markdown | contains("Draft launch email") | not)
          and ($markdown | test("access=|op://|BEGIN [A-Z ]*PRIVATE KEY"; "i") | not)
        ),
        episode_exit: $exit[0].episode_exit,
        tasks: $tasks
      }
    | .passed = (
        .episode_exit == 0
        and .task_passed
        and .entity_passed
        and .revision_changed
        and .markdown_changed
        and .convention_retained
        and .hygiene_passed
      )
  ' > "$pair_dir/a/verify.json"
}

verify_b() {
  local episode_dir="$1"
  local owner_label="$2"
  local revision_before="$3"
  local revision_after="$4"
  local frozen_site="$5"
  local site_dir="$6"
  local answer_passed=false
  local site_unchanged=false
  if [ -f "$episode_dir/cwd/answer.txt" ]; then
    answer_passed="$(
      jq -Rs --arg expected "$owner_label" \
        'gsub("^\\s+|\\s+$"; "") == $expected' \
        "$episode_dir/cwd/answer.txt"
    )"
  fi
  if diff -qr "$frozen_site" "$site_dir" >/dev/null; then
    site_unchanged=true
  fi
  jq -n \
    --slurpfile exit "$episode_dir/episode-exit.json" \
    --argjson answer_passed "$answer_passed" \
    --argjson site_unchanged "$site_unchanged" \
    --arg revision_before "$revision_before" \
    --arg revision_after "$revision_after" '
    {
      episode_exit: $exit[0].episode_exit,
      answer_passed: $answer_passed,
      claim_revision_unchanged: $revision_before == $revision_after,
      site_unchanged: $site_unchanged,
      revision_before: $revision_before,
      revision_after: $revision_after
    }
    | .passed = (
        .episode_exit == 0
        and .answer_passed
        and .claim_revision_unchanged
        and .site_unchanged
      )
  ' > "$episode_dir/verify.json"
}

for pair_index in $(seq 1 "$PAIRS"); do
  PAIR_DIR="$BATCH_DIR/pair-$pair_index"
  ORIGIN_SITE="$PAIR_DIR/origin-site"
  ORIGIN_STATE="$PAIR_DIR/origin-state"
  ORIGIN_HOME="$PAIR_DIR/origin-harness-home"
  mkdir -p "$PAIR_DIR"

  owner_hash="$(
    printf '%s' "$BATCH_ID:$pair_index:$RANDOM" \
      | shasum -a 256 \
      | cut -c1-10
  )"
  owner_label="team-$owner_hash"

  new_output="$(
    run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" \
      spot new bench --site "$ORIGIN_SITE"
  )"
  printf '%s\n' "$new_output" > "$PAIR_DIR/spot-new.txt"
  repository_did="$(printf '%s\n' "$new_output" | sed -n 's/^DID: //p' | head -1)"
  [ -n "$repository_did" ] || {
    echo "handoff: could not parse repository DID for pair $pair_index" >&2
    exit 1
  }
  printf '%s\n' "$repository_did" > "$PAIR_DIR/space.did"

  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" concept add task \
    --attr title:text:one \
    --attr done:boolean:one \
    --description "A launch task" > "$PAIR_DIR/setup.log"
  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" assert task \
    --title "Draft launch email" \
    --done false >> "$PAIR_DIR/setup.log"
  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" agents set \
    "$SCENARIO/initial-AGENTS.md" >> "$PAIR_DIR/setup.log"
  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" agents --json \
    > "$PAIR_DIR/pre-claim.json"
  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" agents \
    > "$PAIR_DIR/pre-AGENTS.md"

  write_a_prompt "$PAIR_DIR/a-prompt.md" "$owner_label"
  run_episode \
    a "$PAIR_DIR/a" "$ORIGIN_STATE" "$ORIGIN_SITE" \
    "$PAIR_DIR/a-prompt.md" "$PAIR_DIR/pre-AGENTS.md" "$owner_label"

  post_claim_status=0
  run_tonk "$ORIGIN_HOME" "$ORIGIN_STATE" agents --json \
    > "$PAIR_DIR/post-claim.json" || post_claim_status=$?
  if [ "$post_claim_status" != 0 ]; then
    echo "handoff: A left no readable claim in pair $pair_index" >&2
    cp "$PAIR_DIR/pre-claim.json" "$PAIR_DIR/post-claim.json"
  fi
  jq -r '.markdown' "$PAIR_DIR/post-claim.json" > "$PAIR_DIR/post-AGENTS.md"
  verify_a "$PAIR_DIR" "$ORIGIN_HOME" "$ORIGIN_STATE" \
    "$owner_label" "$repository_did"

  for arm in control treatment; do
    arm_site="$PAIR_DIR/$arm-site"
    arm_state="$PAIR_DIR/$arm-state"
    arm_home="$PAIR_DIR/$arm-harness-home"
    cp -R "$ORIGIN_SITE" "$arm_site"
    run_tonk "$arm_home" "$arm_state" \
      spot new bench --site "$arm_site" > "$PAIR_DIR/$arm-spot-new.txt"
  done

  write_b_prompt "$PAIR_DIR/b-prompt.md"
  if [ $((RANDOM % 2)) = 0 ]; then
    arm_order=(control treatment)
  else
    arm_order=(treatment control)
  fi
  printf '%s\n' "${arm_order[*]}" > "$PAIR_DIR/b-order.txt"

  for arm in "${arm_order[@]}"; do
    arm_state="$PAIR_DIR/$arm-state"
    arm_home="$PAIR_DIR/$arm-harness-home"
    arm_site="$PAIR_DIR/$arm-site"
    if [ "$arm" = treatment ]; then
      projection="$PAIR_DIR/post-AGENTS.md"
    else
      projection="$PAIR_DIR/pre-AGENTS.md"
    fi
    revision_before="$(
      run_tonk "$arm_home" "$arm_state" agents --json | jq -r '.revision'
    )"
    run_episode \
      "b-$arm" "$PAIR_DIR/$arm" "$arm_state" "$arm_site" \
      "$PAIR_DIR/b-prompt.md" "$projection" "$owner_label"
    revision_after="$(
      run_tonk "$arm_home" "$arm_state" agents --json | jq -r '.revision'
    )"
    verify_b "$PAIR_DIR/$arm" "$owner_label" \
      "$revision_before" "$revision_after" "$ORIGIN_SITE" "$arm_site"
  done

  jq -n \
    --slurpfile a "$PAIR_DIR/a/verify.json" \
    --slurpfile control "$PAIR_DIR/control/verify.json" \
    --slurpfile treatment "$PAIR_DIR/treatment/verify.json" \
    --slurpfile control_metrics "$PAIR_DIR/control/metrics.json" \
    --slurpfile treatment_metrics "$PAIR_DIR/treatment/metrics.json" \
    --arg owner_label "$owner_label" \
    --arg order "$(cat "$PAIR_DIR/b-order.txt")" \
    --argjson pair "$pair_index" '
    {
      pair: $pair,
      owner_label: $owner_label,
      b_order: $order,
      a: $a[0],
      control: ($control[0] + {metrics: $control_metrics[0]}),
      treatment: ($treatment[0] + {metrics: $treatment_metrics[0]})
    }
  ' > "$PAIR_DIR/result.json"
  jq -c '{
    pair,
    a: .a.passed,
    control: .control.passed,
    treatment: .treatment.passed,
    control_tools: .control.metrics.tool_calls,
    treatment_tools: .treatment.metrics.tool_calls
  }' "$PAIR_DIR/result.json" >&2
done

jq -s '
  def median:
    map(select(type == "number")) | sort as $values
    | ($values | length) as $count
    | if $count == 0 then null
      elif ($count % 2) == 1 then $values[($count / 2 | floor)]
      else (($values[$count / 2 - 1] + $values[$count / 2]) / 2)
      end;
  {
    pairs: length,
    episodes_run: (length * 3),
    a_retention_passes: [.[] | select(.a.passed)] | length,
    control_successes: [.[] | select(.control.passed)] | length,
    treatment_successes: [.[] | select(.treatment.passed)] | length,
    control: {
      median_tool_calls: ([.[].control.metrics.tool_calls] | median),
      median_bash_calls: ([.[].control.metrics.bash_calls] | median),
      median_tonk_calls: ([.[].control.metrics.journey.tonk_calls] | median),
      median_orientation_calls: ([.[].control.metrics.journey.orientation_calls] | median),
      median_wall_seconds: ([.[].control.metrics.wall_seconds] | median),
      median_input_tokens: ([.[].control.metrics.tokens.input] | median),
      median_output_tokens: ([.[].control.metrics.tokens.output] | median)
    },
    treatment: {
      median_tool_calls: ([.[].treatment.metrics.tool_calls] | median),
      median_bash_calls: ([.[].treatment.metrics.bash_calls] | median),
      median_tonk_calls: ([.[].treatment.metrics.journey.tonk_calls] | median),
      median_orientation_calls: ([.[].treatment.metrics.journey.orientation_calls] | median),
      median_wall_seconds: ([.[].treatment.metrics.wall_seconds] | median),
      median_input_tokens: ([.[].treatment.metrics.tokens.input] | median),
      median_output_tokens: ([.[].treatment.metrics.tokens.output] | median)
    },
    paired_tool_call_deltas: [.[] | .control.metrics.tool_calls - .treatment.metrics.tool_calls],
    paired_wall_second_deltas: [.[] | .control.metrics.wall_seconds - .treatment.metrics.wall_seconds]
  }
  | .advance = (
      .a_retention_passes == .pairs
      and .treatment_successes == .pairs
      and .treatment.median_tool_calls <= .control.median_tool_calls
    )
' "$BATCH_DIR"/pair-*/result.json > "$BATCH_DIR/summary.json"

if [ "$SCRIPTED" != 1 ] && [ "$episodes_run" -ne "$planned_episodes" ]; then
  echo "handoff: ran $episodes_run episodes, expected $planned_episodes" >&2
  exit 1
fi

jq . "$BATCH_DIR/summary.json"
echo "handoff: results -> $BATCH_DIR/summary.json" >&2

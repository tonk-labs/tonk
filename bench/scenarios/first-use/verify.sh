#!/usr/bin/env bash
# Verify the exact final data state independently of screenshots or a judge.
set -euo pipefail

TONK="${TONK:?}"
response="$("$TONK" query task --json)"

jq -n \
  --argjson response "$response" \
  --slurpfile baseline "${RUN_DIR:?}/baseline-tasks.json" '
  $baseline[0] as $before
  | [$before[] | select(.title == "Draft launch email" and .done == false)] as $targets
  | ($before | map(if .this == $targets[0].this then .done = true else . end)) as $expected
  | {
      available: true,
      passed: (
        ($targets | length) == 1
        and ($response | sort_by(.this)) == ($expected | sort_by(.this))
      ),
      task_count: ($response | length),
      distinct_entities: ($response | map(.this) | unique | length),
      tasks: $response
    }
  '

#!/usr/bin/env bash
# Verify the exact final data state independently of screenshots or a judge.
set -euo pipefail

TONK="${TONK:?}"
response="$(
  "$TONK" eval -c 'task:
  this: ?task
  title: ?title
  done: ?done' --format json --no-sync
)"

jq -n \
  --argjson response "$response" '
  [
    $response.matches_after[]
    | select(.label == "task")
    | .results[]
    | {
        this,
        title: .fields.title,
        done: .fields.done
      }
  ] as $tasks
  | {
      available: true,
      passed: (
        ($tasks | length) == 2
        and ($tasks | map(.this) | unique | length) == 2
        and ($tasks | map(select(.title == "Draft launch email" and .done == true)) | length) == 1
        and ($tasks | map(select(.title == "Book venue" and .done == false)) | length) == 1
      ),
      task_count: ($tasks | length),
      distinct_entities: ($tasks | map(.this) | unique | length),
      tasks: $tasks
    }
  '

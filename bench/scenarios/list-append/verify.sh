#!/usr/bin/env bash
set -euo pipefail

TONK="${TONK:?}"
projected="$($TONK project todo/add-form --spot "$TONK_SPOT" --fixture "$SCENARIO/fixture.yaml" --json)"
browser="$(cat "$RUN_DIR/browser.json")"
"$TONK" pull --spot "$TONK_SPOT" >/dev/null
durable="$($TONK eval -c 'todo/item:
  this: ?todo
  title: ?title
  list: ?list' --spot "$TONK_SPOT" --format json --no-sync)"

jq -n \
  --argjson projected "$projected" \
  --argjson browser "$browser" \
  --argjson durable "$durable" '
  "Buy milk from the benchmark" as $expected
  | [
      $durable.matches_after[]
      | select(.label == "todo/item")
      | .results[]
      | select(.fields.title == $expected and .fields.list == "id:todo-list")
    ] as $todos
  | ($projected.request.claims[0]) as $claim
  | {
      available: true,
      passed: (
        $browser.passed == true
        and ($todos | length) == 1
        and $claim.op == "invoke"
        and $claim.command == "id:todo/add"
        and $claim.arguments.title == $expected
      ),
      durable_matches: ($todos | length),
      browser: $browser,
      projection: $projected
    }
  '

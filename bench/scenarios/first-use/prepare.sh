#!/usr/bin/env bash
# Seed a tiny, rendered task list using only public CLI commands. The benchmark
# prompt intentionally does not name any Tonk subcommand.
set -euo pipefail

ROOT="${ROOT:?}"
TONK="${TONK:-$ROOT/target/release/tonk}"

"$TONK" concept add task \
  --field title:text:one \
  --field done:boolean:one \
  --description "A launch task"

"$TONK" eval -c '
task!: &launch-email
  title: "Draft launch email"
  done: false

task!: &book-venue
  title: "Book venue"
  done: false
'

# Optional stress fixture: many unrelated tasks and an already-completed task
# sharing the requested title. The intended target remains the unfinished one.
extra_tasks="${BENCH_DISTRACTOR_TASKS:-0}"
duplicate_title="${BENCH_DUPLICATE_TITLE:-0}"
[[ "$extra_tasks" =~ ^[0-9]+$ ]] && [ "$extra_tasks" -le 10000 ] || {
  echo "prepare: BENCH_DISTRACTOR_TASKS must be an integer from 0 to 10000" >&2
  exit 2
}
[[ "$duplicate_title" = 0 || "$duplicate_title" = 1 ]] || {
  echo "prepare: BENCH_DUPLICATE_TITLE must be 0 or 1" >&2
  exit 2
}
fixture="$RUN_DIR/extra-tasks.notation"
{
  for ((i=0; i<extra_tasks; i++)); do
    printf 'task!: &distractor-%s\n  title: "Unrelated task %s"\n  done: false\n\n' "$i" "$i"
  done
  if [ "$duplicate_title" = 1 ]; then
    printf 'task!: &completed-launch-email\n  title: "Draft launch email"\n  done: true\n'
  fi
} > "$fixture"
if [ -s "$fixture" ]; then
  "$TONK" eval "$fixture"
fi
"$TONK" query task --json > "$RUN_DIR/baseline-tasks.json"

"$TONK" view add task \
  --home \
  --template '<div class="task"><b>{title}</b><span>done: {done}</span></div>'

if [ "${BENCH_SPACE_AGENTS:-0}" = 1 ]; then
  "$TONK" space agents set "$SCENARIO/space-AGENTS.md"
  "$TONK" space agents get --json > "$RUN_DIR/agents-claim.json"
  "$TONK" space agents get > "$RUN_DIR/site/AGENTS.md"
  claim_entity="$(jq -r '.rows[0].entity' "$RUN_DIR/agents-claim.json")"
  space_entity="$(cat "$RUN_DIR/space.did")"
  if [ "$claim_entity" != "$space_entity" ]; then
    echo "prepare: AGENTS.md claim maps $claim_entity, expected $space_entity" >&2
    exit 1
  fi
  if ! cmp -s "$SCENARIO/space-AGENTS.md" "$RUN_DIR/site/AGENTS.md"; then
    echo "prepare: projected AGENTS.md differs from the space claim fixture" >&2
    exit 1
  fi
  echo "prepare: asserted and projected trusted space AGENTS.md claim" >&2
fi

echo "prepare: seeded first-use task list" >&2

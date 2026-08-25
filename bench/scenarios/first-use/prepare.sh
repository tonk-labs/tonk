#!/usr/bin/env bash
# Seed a tiny, rendered task list using only public CLI commands. The benchmark
# prompt intentionally does not name any Tonk subcommand.
set -euo pipefail

ROOT="${ROOT:?}"
TONK="$ROOT/target/release/tonk"

"$TONK" concept add task \
  --attr title:text:one \
  --attr done:boolean:one \
  --description "A launch task"

"$TONK" eval -c '
task!: &launch-email
  title: "Draft launch email"
  done: false

task!: &book-venue
  title: "Book venue"
  done: false
'

"$TONK" view add task \
  --name tasks \
  --template '<div class="task"><b>{title}</b><span>done: {done}</span></div>'

"$TONK" home task

if [ "${BENCH_SPACE_AGENTS:-0}" = 1 ]; then
  "$TONK" agents set "$SCENARIO/space-AGENTS.md"
  "$TONK" agents --json > "$RUN_DIR/agents-claim.json"
  "$TONK" agents > "$RUN_DIR/site/AGENTS.md"
  claim_entity="$(jq -r '.entity' "$RUN_DIR/agents-claim.json")"
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

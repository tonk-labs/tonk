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

if [ "${BENCH_SPOT_AGENTS:-0}" = 1 ]; then
  cp "$SCENARIO/spot-AGENTS.md" "$RUN_DIR/site/AGENTS.md"
  echo "prepare: installed trusted spot AGENTS.md fixture" >&2
fi

echo "prepare: seeded first-use task list" >&2

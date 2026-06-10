#!/usr/bin/env bash
# Orchestrate one benchmark run:
#   stack up -> site setup -> episode (or scripted) -> browser join
#   -> checkpoint screenshots -> metrics -> judge -> report
set -euo pipefail

ROOT="${ROOT:?}"
SCENARIO_NAME="${1:?usage: run.sh <scenario> [--scripted] [--runs N]}"; shift
SCRIPTED=0; RUNS=1
while [ $# -gt 0 ]; do
  case "$1" in
    --scripted) SCRIPTED=1 ;;
    --runs) RUNS="$2"; shift ;;
    *) echo "unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done

SCENARIO="$ROOT/bench/scenarios/$SCENARIO_NAME"
[ -d "$SCENARIO" ] || { echo "no scenario $SCENARIO_NAME" >&2; exit 2; }

for i in $(seq 1 "$RUNS"); do
  RUN_DIR="$ROOT/bench/runs/$(date +%Y%m%d-%H%M%S)-$SCENARIO_NAME"
  mkdir -p "$RUN_DIR"
  export RUN_DIR SCENARIO SCENARIO_NAME
  export BENCH_PORT="${BENCH_PORT:-8787}"
  export BENCH_URL="http://127.0.0.1:$BENCH_PORT"
  export SPACE_NAME="bench"
  echo "run: $RUN_DIR" >&2

  cleanup() {
    "$ROOT/bench/bin/browser.sh" stop 2>/dev/null || true
    "$ROOT/bench/bin/stack.sh" stop 2>/dev/null || true
  }
  trap cleanup EXIT

  "$ROOT/bench/bin/stack.sh" start
  "$ROOT/bench/bin/site.sh" setup

  episode_status=0
  if [ "$SCRIPTED" = 1 ]; then
    ( cd "$RUN_DIR/site" && SLIDE="$ROOT/target/release/slide" bash "$SCENARIO/scripted.sh" ) \
      || episode_status=$?
  else
    "$ROOT/bench/bin/episode.sh" || episode_status=$?
  fi
  echo "{\"episode_exit\": $episode_status}" > "$RUN_DIR/episode-exit.json"

  "$ROOT/bench/bin/browser.sh" start
  "$ROOT/bench/bin/bridge.sh"
  "$ROOT/bench/bin/shots.sh"

  if [ "$SCRIPTED" != 1 ]; then
    "$ROOT/bench/bin/metrics.sh"
    "$ROOT/bench/bin/judge.sh"
  fi
  "$ROOT/bench/bin/report-run.sh"

  cleanup
  trap - EXIT
  echo "run: done -> $RUN_DIR/report.md" >&2
done

#!/usr/bin/env bash
# Run the episode agent: a fresh headless claude in the site dir with
# the scenario task. It gets slide on PATH and nothing else from this
# repo — its struggles with the CLI are the benchmark signal.
#
# Env: ROOT, RUN_DIR, SCENARIO
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
SITE="$RUN_DIR/site"
EPISODE_TIMEOUT="${EPISODE_TIMEOUT:-1200}"   # seconds

# Guard: task file must exist before we do anything else.
[ -f "$SCENARIO/task.md" ] || { echo "episode: missing $SCENARIO/task.md" >&2; exit 1; }

# Fixtures are the episode's working material (e.g. artifact.html).
if [ -d "$SCENARIO/fixtures" ]; then
  cp -R "$SCENARIO/fixtures/." "$SITE/"
fi

date +%s > "$RUN_DIR/episode-start"

# Write episode-end on any exit so an interrupted run still leaves a pair.
trap 'date +%s > "$RUN_DIR/episode-end"' EXIT

# Auth: default to the claude CLI's logged-in (OAuth) session by
# stripping ANTHROPIC_API_KEY from the child env. Set
# BENCH_USE_API_KEY=1 to use the API key instead; an op:// reference
# is resolved via `op read` (headless claude can't reach the op-agent
# the way the interactive shell does).
KEY_ENV=(-u ANTHROPIC_API_KEY)
if [ -n "${BENCH_USE_API_KEY:-}" ]; then
  RESOLVED_KEY="${ANTHROPIC_API_KEY:-}"
  if [[ "$RESOLVED_KEY" == op://* ]]; then
    RESOLVED_KEY="$(op read "$RESOLVED_KEY")"
  fi
  KEY_ENV=(ANTHROPIC_API_KEY="$RESOLVED_KEY")
fi

set +e
( cd "$SITE" && \
  env "${KEY_ENV[@]}" \
  PATH="$ROOT/target/release:$PATH" \
  timeout -k 30 "$EPISODE_TIMEOUT" claude -p "$(cat "$SCENARIO/task.md")" \
    --output-format stream-json --verbose \
    --allowedTools "Bash(slide:*),Read,Write,Edit,Glob,Grep" \
) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
status=$?
set -e
echo "episode: exit $status" >&2
exit "$status"

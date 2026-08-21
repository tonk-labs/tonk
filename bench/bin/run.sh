#!/usr/bin/env bash
# Orchestrate one benchmark run:
#   stack up -> site setup -> episode (or scripted) -> browser join
#   -> checkpoint screenshots -> metrics -> judge -> report
set -euo pipefail

ROOT="${ROOT:?}"

# This harness drives a real `tonk` binary (site.sh, shots.sh, and
# scenario scripts all call $ROOT/target/release/tonk directly, from
# this process's own real HOME). A release check would hit github.com
# on every run, stamp the developer's real update.json, and put a nag
# on stderr that metrics.sh greps as agent friction — so opt the whole
# harness out. episode.sh separately opts out the agent's own tonk
# calls, since those run under EPISODE_HOME rather than inheriting this.
export TONK_NO_UPDATE_CHECK=1

# Minting an invite shortens the link with a live PUT to its own
# origin. Against the local stack that is the bench worker, which is
# fine, but a scenario that mints without a resolved remote would PUT
# to production. Nothing here reads a short link, so skip the round
# trip entirely.
export TONK_NO_SHORTEN=1

SCENARIO_NAME="${1:?usage: run.sh <scenario> [--scripted] [--runs N] [--variant NAME] [--space-agents]}"; shift
SCRIPTED=0
RUNS=1
SPACE_AGENTS=0
BENCH_VARIANT="${BENCH_VARIANT:-unlabelled}"
while [ $# -gt 0 ]; do
  case "$1" in
    --scripted) SCRIPTED=1 ;;
    --runs) RUNS="$2"; shift ;;
    --variant) BENCH_VARIANT="$2"; shift ;;
    --space-agents) SPACE_AGENTS=1 ;;
    *) echo "unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done
export BENCH_SPACE_AGENTS="$SPACE_AGENTS"
if [ "$SPACE_AGENTS" = 1 ]; then
  SPACE_AGENTS_JSON=true
  SPACE_AGENTS_SOURCE=dialog-claim
else
  SPACE_AGENTS_JSON=false
  SPACE_AGENTS_SOURCE=none
fi

if [[ ! "$BENCH_VARIANT" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "invalid variant '$BENCH_VARIANT': use letters, digits, dot, underscore, or hyphen" >&2
  exit 2
fi

SCENARIO="$ROOT/bench/scenarios/$SCENARIO_NAME"
[ -d "$SCENARIO" ] || { echo "no scenario $SCENARIO_NAME" >&2; exit 2; }

cleanup() {
  "$ROOT/bench/bin/browser.sh" stop 2>/dev/null || true
  "$ROOT/bench/bin/stack.sh" stop 2>/dev/null || true
}

for i in $(seq 1 "$RUNS"); do
  RUN_DIR="$ROOT/bench/runs/$(date +%Y%m%d-%H%M%S)-${i}-$SCENARIO_NAME-$BENCH_VARIANT"
  unset EPISODE_DIR EPISODE_BIN EPISODE_HOME EPISODE_PATH_SANDBOX EPISODE_RUNNER \
        EPISODE_SANDBOX EPISODE_SPACE EPISODE_SPACES_STATE
  mkdir -p "$RUN_DIR"
  export RUN_DIR SCENARIO SCENARIO_NAME
  export BENCH_PORT="${BENCH_PORT:-8787}"
  export BENCH_URL="http://127.0.0.1:$BENCH_PORT"
  echo "run: $RUN_DIR" >&2

  # The CLI is space-based: it resolves a space by --space, then TONK_SPACE,
  # then the `tonk space use` selection, and never consults the cwd. So a
  # `cd` into the site directory buys nothing — without these two the
  # harness would silently drive whatever space the developer happens to
  # have selected globally. TONK_SPACES_STATE keeps the registry (and
  # the canonical spaces/ root beneath it) inside the run directory, so
  # concurrent runs and the developer's own spaces never see each other.
  export TONK_SPACES_STATE="$RUN_DIR/spaces-state"
  export TONK_SPACE=bench
  mkdir -p "$TONK_SPACES_STATE"

  trap cleanup EXIT

  "$ROOT/bench/bin/stack.sh" start
  "$ROOT/bench/bin/site.sh" setup

  # The space is addressed by the repository's DID (auto-join returns it
  # as repository.name); site.sh setup stashed it in space.did. Both the
  # bridge pull and the checkpoint shots must use this, not a name.
  if [ -s "$RUN_DIR/space.did" ]; then
    SPACE_NAME="$(cat "$RUN_DIR/space.did")"
  else
    echo "run: no space.did from site setup; falling back to SPACE_NAME=${SPACE_NAME:-bench}" >&2
    SPACE_NAME="${SPACE_NAME:-bench}"
  fi
  export SPACE_NAME
  echo "run: space addressed as $SPACE_NAME" >&2

  # Scenario hooks: scenario.env exports episode knobs (runner, dir,
  # sandbox); prepare.sh does scenario-specific setup (seed data, mint
  # an invite, build a registry or a prompt). Both are optional.
  if [ -f "$SCENARIO/scenario.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCENARIO/scenario.env"
    set +a
  fi
  revision="$(git -C "$ROOT" rev-parse HEAD)"
  if [ -n "$(git -C "$ROOT" status --short)" ]; then
    dirty=true
  else
    dirty=false
  fi
  case "${EPISODE_RUNNER:-claude}" in
    codex) effective_model="${CODEX_MODEL:-gpt-5.5}" ;;
    claude) effective_model="claude-cli-default" ;;
    *) effective_model="unknown" ;;
  esac
  jq -n \
    --arg variant "$BENCH_VARIANT" \
    --arg scenario "$SCENARIO_NAME" \
    --arg revision "$revision" \
    --arg runner "${EPISODE_RUNNER:-claude}" \
    --arg model "$effective_model" \
    --arg space_agents_source "$SPACE_AGENTS_SOURCE" \
    --argjson space_agents "$SPACE_AGENTS_JSON" \
    --argjson dirty "$dirty" \
    '{
      variant: $variant,
      scenario: $scenario,
      revision: $revision,
      dirty: $dirty,
      runner: $runner,
      model: $model,
      space_agents: $space_agents,
      space_agents_source: $space_agents_source
    }' > "$RUN_DIR/experiment.json"
  if [ -x "$SCENARIO/prepare.sh" ]; then
    "$SCENARIO/prepare.sh"
  fi

  EPISODE_DIR="${EPISODE_DIR:-$RUN_DIR/site}"
  export EPISODE_DIR

  episode_status=0
  if [ "$SCRIPTED" = 1 ]; then
    ( cd "$EPISODE_DIR" && TONK="$ROOT/target/release/tonk" bash "$SCENARIO/scripted.sh" ) \
      || episode_status=$?
  else
    "$ROOT/bench/bin/episode.sh" || episode_status=$?
  fi
  echo "{\"episode_exit\": $episode_status}" > "$RUN_DIR/episode-exit.json"

  verifier_status=0
  if [ -x "$SCENARIO/verify.sh" ]; then
    TONK="$ROOT/target/release/tonk" "$SCENARIO/verify.sh" > "$RUN_DIR/verify.json" \
      || verifier_status=$?
  else
    jq -n '{available: false, passed: null}' > "$RUN_DIR/verify.json"
  fi
  echo "{\"verifier_exit\": $verifier_status}" > "$RUN_DIR/verifier-exit.json"

  "$ROOT/bench/bin/browser.sh" start
  bridge_status=0
  "$ROOT/bench/bin/bridge.sh" || bridge_status=$?
  echo "{\"bridge_exit\": $bridge_status}" > "$RUN_DIR/bridge-exit.json"
  "$ROOT/bench/bin/shots.sh"

  if [ "$SCRIPTED" != 1 ]; then
    "$ROOT/bench/bin/metrics.sh"
    judge_status=0
    "$ROOT/bench/bin/judge.sh" || judge_status=$?
  fi
  "$ROOT/bench/bin/visual-diff.sh"
  "$ROOT/bench/bin/report-run.sh"

  cleanup
  trap - EXIT
  echo "run: done -> $RUN_DIR/report.md" >&2
done

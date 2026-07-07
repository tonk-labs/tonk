#!/usr/bin/env bash
# Run the episode agent: a fresh headless agent in the episode dir with
# the scenario task. It gets tonk on PATH (unless the scenario sandboxes
# it away) and nothing else from this repo — its struggles with the CLI
# are the benchmark signal.
#
# Env: ROOT, RUN_DIR, SCENARIO
# Optional (from scenario.env): EPISODE_DIR, EPISODE_BIN,
#   EPISODE_PATH_SANDBOX, EPISODE_RUNNER, EPISODE_SANDBOX, CODEX_MODEL
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
EPISODE_DIR="${EPISODE_DIR:-$RUN_DIR/site}"
EPISODE_TIMEOUT="${EPISODE_TIMEOUT:-1200}"   # seconds
EPISODE_RUNNER="${EPISODE_RUNNER:-claude}"

# The prompt is the generated $RUN_DIR/prompt.md when a prepare hook
# built one (cold-onboard renders the live core.yaml invite copy);
# otherwise the scenario's static task.md.
if [ -f "$RUN_DIR/prompt.md" ]; then
  PROMPT_FILE="$RUN_DIR/prompt.md"
else
  PROMPT_FILE="$SCENARIO/task.md"
fi
[ -f "$PROMPT_FILE" ] || { echo "episode: missing prompt ($PROMPT_FILE)" >&2; exit 1; }

mkdir -p "$EPISODE_DIR"

# Fixtures are the episode's working material (e.g. artifact.html).
if [ -d "$SCENARIO/fixtures" ]; then
  cp -R "$SCENARIO/fixtures/." "$EPISODE_DIR/"
fi

# Episode PATH. Default: tonk release binary available. Sandbox mode:
# tonk deliberately absent — the episode must discover the install
# path itself (npx against the run's local registry). Codex commands
# run through a login shell that re-sources system paths, so the
# sandbox holds only if tonk isn't globally installed: guard it.
EPISODE_PATH="$PATH"
if [ "${EPISODE_PATH_SANDBOX:-0}" = 1 ]; then
  if command -v tonk >/dev/null 2>&1; then
    echo "episode: EPISODE_PATH_SANDBOX=1 but 'tonk' is globally installed at $(command -v tonk); the cold-start scenario would be invalid. Uninstall it or drop the sandbox." >&2
    exit 1
  fi
else
  EPISODE_PATH="$ROOT/target/release:$EPISODE_PATH"
fi
if [ -n "${EPISODE_BIN:-}" ]; then
  EPISODE_PATH="$EPISODE_BIN:$EPISODE_PATH"
fi

date +%s > "$RUN_DIR/episode-start"

# Write episode-end on any exit so an interrupted run still leaves a pair.
trap 'date +%s > "$RUN_DIR/episode-end"' EXIT

# Auth: default to the claude CLI's logged-in (OAuth) session by
# stripping ANTHROPIC_API_KEY from the child env. Set
# BENCH_USE_API_KEY=1 to use the API key instead; an op:// reference
# is resolved via `op read` (headless claude can't reach the op-agent
# the way the interactive shell does). Codex episodes authenticate via
# ~/.codex (run `codex login` once); the strip is harmless there.
KEY_ENV=(-u ANTHROPIC_API_KEY)
if [ -n "${BENCH_USE_API_KEY:-}" ]; then
  RESOLVED_KEY="${ANTHROPIC_API_KEY:-}"
  if [[ "$RESOLVED_KEY" == op://* ]]; then
    RESOLVED_KEY="$(op read "$RESOLVED_KEY")"
  fi
  KEY_ENV=(ANTHROPIC_API_KEY="$RESOLVED_KEY")
fi

run_claude() {
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" \
    PATH="$EPISODE_PATH" \
    timeout -k 30 "$EPISODE_TIMEOUT" claude -p "$(cat "$PROMPT_FILE")" \
      --output-format stream-json --verbose \
      --allowedTools "Bash,Read,Write,Edit,Glob,Grep" \
  ) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
}

run_codex() {
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" \
    PATH="$EPISODE_PATH" \
    timeout -k 30 "$EPISODE_TIMEOUT" codex exec --json \
      -m "${CODEX_MODEL:-gpt-5.5}" \
      --skip-git-repo-check --ephemeral \
      -s "${EPISODE_SANDBOX:-workspace-write}" \
      -c sandbox_workspace_write.network_access=true \
      --add-dir "$RUN_DIR" \
      - < "$PROMPT_FILE" \
  ) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
}

set +e
case "$EPISODE_RUNNER" in
  claude) run_claude ;;
  codex)  run_codex ;;
  *) echo "episode: unknown EPISODE_RUNNER '$EPISODE_RUNNER'" >&2; exit 2 ;;
esac
status=$?
set -e
echo "episode: exit $status (runner: $EPISODE_RUNNER)" >&2
exit "$status"

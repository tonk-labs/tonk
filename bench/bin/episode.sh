#!/usr/bin/env bash
# Run the episode agent: a fresh headless agent in the episode dir with
# the scenario task. It gets tonk on PATH (unless the scenario sandboxes
# it away) and nothing else from this repo — its struggles with the CLI
# are the benchmark signal.
#
# Env: ROOT, RUN_DIR, SCENARIO, TONK_SPACES_STATE, TONK_SPACE
# Optional (from scenario.env): EPISODE_DIR, EPISODE_BIN,
#   EPISODE_PATH_SANDBOX, EPISODE_RUNNER, EPISODE_SANDBOX, CODEX_MODEL,
#   EPISODE_HOME, EPISODE_SPACE, EPISODE_SPACES_STATE, EPISODE_SITE_DIR
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
EPISODE_DIR="${EPISODE_DIR:-$RUN_DIR/site}"
EPISODE_TIMEOUT="${EPISODE_TIMEOUT:-1200}"   # seconds
EPISODE_RUNNER="${EPISODE_RUNNER:-claude}"
REAL_HOME="$HOME"

# Which space the agent's own `tonk` calls resolve to, and which
# registry they resolve against. The CLI never consults the cwd, so
# without these the agent would drive whichever space the developer has
# selected globally — succeeding against the wrong repo, silently.
#
# Both default to the harness's, which is what a scenario handing the
# agent a ready-made site wants. A scenario where the agent stands up
# its own space (cold-onboard joins from an invite) overrides both: an
# empty EPISODE_SPACE (the CLI reads an empty TONK_SPACE as unset) plus
# its own registry directory, so before the join `tonk` honestly
# reports no spaces rather than silently resolving the origin site the
# agent is supposed to be joining. `tonk join` selects what it
# registers, so afterwards the agent's own space governs.
#
# `-` rather than `:-` so a deliberately-empty override wins.
EPISODE_SPACE="${EPISODE_SPACE-${TONK_SPACE:?}}"
EPISODE_SPACES_STATE="${EPISODE_SPACES_STATE-${TONK_SPACES_STATE:?}}"
mkdir -p "$EPISODE_SPACES_STATE"

# EPISODE_HOME isolates the episode's HOME (keeps it from touching the
# real ~/.claude, ~/.codex, ~/.cargo state or shell history). Unset by
# default: behavior is then byte-identical to before this var existed.
if [ -n "${EPISODE_HOME:-}" ]; then
  mkdir -p "$EPISODE_HOME"
fi

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
if [ "${EPISODE_PATH_SANDBOX:-0}" = 1 ]; then
  # Minimal system PATH — the inherited harness PATH carries user
  # toolchain dirs incl. a globally installed tonk (~/.cargo/bin).
  # nix-darwin's zshenv preserves an inherited PATH verbatim when
  # __NIX_DARWIN_SET_ENVIRONMENT_DONE is set, which the harness env
  # guarantees, so what we construct here is what the episode sees.
  EPISODE_PATH="/usr/bin:/bin:/usr/sbin:/sbin"
else
  EPISODE_PATH="$ROOT/target/release:$PATH"
fi
if [ -n "${EPISODE_BIN:-}" ]; then
  EPISODE_PATH="$EPISODE_BIN:$EPISODE_PATH"
fi

# Sandbox guard: tonk must be genuinely unreachable via the path the
# episode will actually see. Under EPISODE_HOME, codex runs commands
# through a login shell (zsh -lc) with HOME=$EPISODE_HOME, which
# re-sources system paths (e.g. ~/.zprofile can pull in ~/.cargo/bin) —
# so check reachability that way rather than via `command -v` in this
# shell's real $HOME. Without EPISODE_HOME, the direct check still
# applies, since that's the HOME the episode will actually run under.
if [ "${EPISODE_PATH_SANDBOX:-0}" = 1 ]; then
  if [ -n "${EPISODE_HOME:-}" ]; then
    if env HOME="$EPISODE_HOME" PATH="$EPISODE_PATH" zsh -lc 'command -v tonk' >/dev/null 2>&1; then
      echo "episode: EPISODE_PATH_SANDBOX=1 but 'tonk' is reachable under EPISODE_HOME=$EPISODE_HOME via the login shell (e.g. pulled in by ~/.zprofile); the cold-start scenario would be invalid." >&2
      exit 1
    fi
  else
    if command -v tonk >/dev/null 2>&1; then
      echo "episode: EPISODE_PATH_SANDBOX=1 but 'tonk' is globally installed at $(command -v tonk); the cold-start scenario would be invalid. Uninstall it or drop the sandbox." >&2
      exit 1
    fi
  fi
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

# env applies PATH="$EPISODE_PATH" before exec'ing the command, so
# `timeout` and the runner binary must be pre-resolved via the harness
# PATH — the minimal sandbox PATH doesn't contain them.
TIMEOUT_BIN="$(command -v timeout)"

# run.sh opts the harness process itself out of tonk's release check,
# but that export doesn't reach here: the agent's own `tonk` calls run
# under EPISODE_HOME (a fresh HOME each episode), so without this the
# check would fire on every episode's first `tonk` command — live
# network traffic in an otherwise-hermetic harness, plus a nag on
# stderr that metrics.sh greps as agent friction. Set explicitly in
# both run_claude and run_codex below rather than relying on inheritance.

run_claude() {
  local CLAUDE_BIN
  CLAUDE_BIN="$(command -v claude)"
  local HOME_ENV=()
  if [ -n "${EPISODE_HOME:-}" ]; then
    HOME_ENV=(
      HOME="$EPISODE_HOME"
      CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$REAL_HOME/.claude}"
    )
  fi
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" ${HOME_ENV[@]:+"${HOME_ENV[@]}"} \
    PATH="$EPISODE_PATH" \
    TONK_NO_UPDATE_CHECK=1 \
    TONK_SPACES_STATE="$EPISODE_SPACES_STATE" \
    TONK_SPACE="$EPISODE_SPACE" \
    "$TIMEOUT_BIN" -k 30 "$EPISODE_TIMEOUT" "$CLAUDE_BIN" -p "$(cat "$PROMPT_FILE")" \
      --output-format stream-json --verbose \
      --allowedTools "Bash,Read,Write,Edit,Glob,Grep" \
  ) > "$RUN_DIR/episode.jsonl" 2> "$RUN_DIR/episode.stderr"
}

run_codex() {
  local CODEX_BIN
  CODEX_BIN="$(command -v codex)"
  local HOME_ENV=()
  if [ -n "${EPISODE_HOME:-}" ]; then
    HOME_ENV=(
      HOME="$EPISODE_HOME"
      CODEX_HOME="${CODEX_HOME:-$REAL_HOME/.codex}"
    )
  fi
  # tonk keeps its profile at "$HOME/Library/Application Support/dialog"
  # (Directory::Profile); workspace-write denies unlisted paths by
  # default, so grant it explicitly. Under EPISODE_HOME this is the
  # episode's profile dir, not the real one.
  local EP_PROFILE_DIR="${EPISODE_HOME:-$REAL_HOME}/Library/Application Support/dialog"
  mkdir -p "$EP_PROFILE_DIR"
  local SITE_DIR_ARGS=()
  if [ -n "${EPISODE_SITE_DIR:-}" ]; then
    SITE_DIR_ARGS=(--add-dir "$EPISODE_SITE_DIR")
  fi
  ( cd "$EPISODE_DIR" && \
    env "${KEY_ENV[@]}" ${HOME_ENV[@]:+"${HOME_ENV[@]}"} \
    PATH="$EPISODE_PATH" \
    TONK_NO_UPDATE_CHECK=1 \
    TONK_SPACES_STATE="$EPISODE_SPACES_STATE" \
    TONK_SPACE="$EPISODE_SPACE" \
    "$TIMEOUT_BIN" -k 30 "$EPISODE_TIMEOUT" "$CODEX_BIN" exec --json \
      -m "${CODEX_MODEL:-gpt-5.5}" \
      --skip-git-repo-check --ephemeral \
      -s "${EPISODE_SANDBOX:-workspace-write}" \
      -c sandbox_workspace_write.network_access=true \
      --add-dir "$RUN_DIR" \
      --add-dir "$EPISODE_SPACES_STATE" \
      ${SITE_DIR_ARGS[@]:+"${SITE_DIR_ARGS[@]}"} \
      --add-dir "$EP_PROFILE_DIR" \
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

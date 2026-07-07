#!/usr/bin/env bash
# Build the run's npm registry, mint a real invite from the origin
# site, and render the live core.yaml agent prompt around it. Also
# stand up the episode's blank HOME and a minimal bin dir carrying
# node/npm/npx: EPISODE_HOME denies the episode the user's shell rc
# (which is what normally puts node/homebrew on PATH), and
# EPISODE_PATH_SANDBOX constructs a minimal PATH, so node/npm/npx must
# be supplied explicitly or `npx tonk` cannot run at all.
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"

"$ROOT/bench/bin/registry.sh" build

INVITE_URL="$("$ROOT/bench/bin/site.sh" invite | tr -d '[:space:]')"
printf '%s' "$INVITE_URL" > "$RUN_DIR/invite.url"

"$ROOT/bench/bin/prompt.sh" --invite-url "$INVITE_URL" --name bench \
  > "$RUN_DIR/prompt.md"

mkdir -p "$RUN_DIR/agent"
mkdir -p "$RUN_DIR/home"

mkdir -p "$RUN_DIR/bin"
for tool in node npm npx; do
  tool_path="$(command -v "$tool" 2>/dev/null || true)"
  [ -n "$tool_path" ] || { echo "prepare: host '$tool' not found on PATH — cannot build EPISODE_BIN" >&2; exit 1; }
  ln -sf "$tool_path" "$RUN_DIR/bin/$tool"
done

echo "prepare: prompt rendered ($(wc -l < "$RUN_DIR/prompt.md") lines)" >&2

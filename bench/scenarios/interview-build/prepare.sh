#!/usr/bin/env bash
# Install the ask-user bridge onto the episode PATH. Also pins an
# absolute-path symlink to the claude CLI alongside it: ask-user spawns
# claude for the persona, and the episode's PATH (assembled by
# episode.sh from EPISODE_BIN plus the base PATH) is not guaranteed to
# carry the host's claude once inside codex's sandboxed exec — resolving
# it here, on the host before the sandbox exists, and shipping a
# symlink is more robust than trusting `command -v claude` at ask-user's
# call time.
# Env: ROOT, RUN_DIR
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"
mkdir -p "$RUN_DIR/bin"
cp "$ROOT/bench/bin/ask-user.sh" "$RUN_DIR/bin/ask-user"
chmod +x "$RUN_DIR/bin/ask-user"

CLAUDE_BIN="$(command -v claude)"
ln -sf "$CLAUDE_BIN" "$RUN_DIR/bin/claude"

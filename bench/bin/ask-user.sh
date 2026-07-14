#!/usr/bin/env bash
# The simulated user: forwards one question to a headless claude
# holding the scenario persona plus the conversation so far, appends
# the exchange to $RUN_DIR/interview.log, prints the reply.
# Installed on the episode PATH as `ask-user` by prepare.sh.
#
# Env: RUN_DIR, PERSONA_FILE
set -euo pipefail
RUN_DIR="${RUN_DIR:?}"; PERSONA_FILE="${PERSONA_FILE:?}"
Q="$*"
[ -n "$Q" ] || { echo "usage: ask-user <question for the user>" >&2; exit 2; }
LOG="$RUN_DIR/interview.log"
touch "$LOG"

# Resolve claude by absolute path rather than trusting the caller's
# PATH: this script runs inside the episode, whose PATH is assembled
# by episode.sh and is not guaranteed to carry the real HOME's claude
# install. prepare.sh pins a symlink at $RUN_DIR/bin/claude next to
# this script (same dir, so it's first on PATH once installed); the
# `command -v` fallback covers standalone/manual invocation where no
# such symlink exists.
CLAUDE_BIN="$(command -v claude || echo claude)"

prompt() {
  cat <<EOF
You are role-playing one specific end user in a product test. Stay in
character. Never mention being an AI, never help with technical
details, never write notation or commands.

Your character:
$(cat "$PERSONA_FILE")

The conversation so far (may be empty):
$(cat "$LOG")

The assistant now asks you:
$Q

Reply in character, in plain text, 1-3 sentences. If the question is
vague or open-ended, answer vaguely, as your character would. Only
reveal a hidden preference when a concrete question surfaces it.
EOF
}

reply="$(env -u ANTHROPIC_API_KEY timeout -k 15 120 "$CLAUDE_BIN" -p "$(prompt)" 2>>"$RUN_DIR/ask-user.stderr")"
printf 'AGENT: %s\nUSER: %s\n' "$Q" "$reply" >> "$LOG"
printf '%s\n' "$reply"

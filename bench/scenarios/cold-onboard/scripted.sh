#!/usr/bin/env bash
# Known-good cold-onboard sequence: join via npx against the hermetic
# registry from the empty agent dir, then push one renderable note.
# Runs inside $EPISODE_DIR ($RUN_DIR/agent). No claude/codex spend.
set -euo pipefail
RUN_DIR="${RUN_DIR:?}"

INVITE_URL="$(cat "$RUN_DIR/invite.url")"

npx --yes tonk join "$INVITE_URL"
npx --yes tonk status

npx --yes tonk eval -c '
attribute!: &note-title
  description: "The note title"
  the: bench.note/title
  as: text
  cardinality: one

concept!: &note
  description: "A note"
  with:
    title: note-title

note!:
  title: "Cold onboarding worked"
'
npx --yes tonk push || true
npx --yes tonk status

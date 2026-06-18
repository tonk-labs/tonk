#!/usr/bin/env bash
# Create the per-run slide site and wire it to the local access
# service: init, remote add, set-upstream. The episode agent then
# works inside $RUN_DIR/site with everything pre-connected.
#
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="${RUN_DIR:?}"
BENCH_URL="${BENCH_URL:?}"
SITE="$RUN_DIR/site"
SLIDE="$ROOT/target/release/slide"

setup() {
  mkdir -p "$SITE"
  cd "$SITE"
  # `slide init` prints "DID: did:key:..." — the repository's subject
  # DID. This is the identity the tonk-ui addresses a space by (the
  # join flow returns it as repository.name); the harness must use it
  # everywhere instead of a chosen name. Stash it for run.sh to export
  # as SPACE_NAME.
  init_out="$("$SLIDE" init)"
  printf '%s\n' "$init_out"
  did="$(printf '%s\n' "$init_out" | sed -n 's/^DID: //p' | head -1)"
  if [ -n "$did" ]; then
    printf '%s' "$did" > "$RUN_DIR/space.did"
  else
    echo "site: could not parse repository DID from 'slide init' output" >&2
    exit 1
  fi
  "$SLIDE" remote add origin "$BENCH_URL/ucan/"
  "$SLIDE" remote set-upstream origin
  "$SLIDE" status
}

# Mint an invite launcher URL for the browser side; prints the URL.
invite() {
  if [ ! -d "$SITE/.tonk" ]; then
    echo "site: no site at $SITE (run setup first)" >&2
    exit 1
  fi
  cd "$SITE"
  "$SLIDE" invite --remote origin
}

case "${1:-}" in
  setup) setup ;;
  invite) invite ;;
  *) echo "usage: site.sh {setup|invite}" >&2; exit 2 ;;
esac

#!/usr/bin/env bash
# Create the per-run tonk site and wire it to the local access
# service: init, remote add, set-upstream. The episode agent then
# works inside $RUN_DIR/site with everything pre-connected.
#
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="${RUN_DIR:?}"
BENCH_URL="${BENCH_URL:?}"
SITE="$RUN_DIR/site"
TONK="$ROOT/target/release/tonk"

setup() {
  mkdir -p "$SITE"
  cd "$SITE"
  # `tonk init` prints "DID: did:key:..." — the repository's subject
  # DID. This is the identity the tonk-ui addresses a space by (the
  # join flow returns it as repository.name); the harness must use it
  # everywhere instead of a chosen name. Stash it for run.sh to export
  # as SPACE_NAME.
  init_out="$("$TONK" init)"
  printf '%s\n' "$init_out"
  did="$(printf '%s\n' "$init_out" | sed -n 's/^DID: //p' | head -1)"
  if [ -n "$did" ]; then
    printf '%s' "$did" > "$RUN_DIR/space.did"
  else
    echo "site: could not parse repository DID from 'tonk init' output" >&2
    exit 1
  fi
  "$TONK" remote add origin "$BENCH_URL/ucan/"
  "$TONK" remote set-upstream origin
  # Publish the init-seeded stdlib to the remote now, so a joiner (or an
  # inspector) sees current state even before an invite is minted. `tonk
  # init` commits the seed before the upstream exists, so nothing has
  # pushed it yet.
  "$TONK" push
  "$TONK" status
}

# Mint an invite launcher URL for the browser side; prints the URL.
invite() {
  if [ ! -d "$SITE/.tonk" ]; then
    echo "site: no site at $SITE (run setup first)" >&2
    exit 1
  fi
  cd "$SITE"
  "$TONK" invite --remote origin
}

case "${1:-}" in
  setup) setup ;;
  invite) invite ;;
  *) echo "usage: site.sh {setup|invite}" >&2; exit 2 ;;
esac

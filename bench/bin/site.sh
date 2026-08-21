#!/usr/bin/env bash
# Register the per-run tonk space and wire it to the local access
# service: space new, remote add, set-upstream. The episode agent then
# works inside $RUN_DIR/site with everything pre-connected.
#
# The space is named by TONK_SPACE (exported by run.sh) and its site
# lives at $RUN_DIR/site, adopted via `--site` so the run directory
# stays self-contained rather than the CLI's canonical spaces/ root.
# Every `tonk` call here resolves through TONK_SPACE — the CLI never
# consults the cwd, so no `cd` is involved or useful.
#
# Env: ROOT, RUN_DIR, BENCH_URL, TONK_SPACE
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="${RUN_DIR:?}"
BENCH_URL="${BENCH_URL:?}"
SPACE="${TONK_SPACE:?}"
SITE="$RUN_DIR/site"
TONK="$ROOT/target/release/tonk"

setup() {
  # `tonk space new` prints "DID: did:key:..." — the repository's subject
  # DID. This is the identity the tonk-ui addresses a space by (the
  # join flow returns it as repository.name); the harness must use it
  # everywhere instead of a chosen name. Stash it for run.sh to export
  # as SPACE_NAME.
  new_out="$("$TONK" space new "$SPACE" --site "$SITE")"
  printf '%s\n' "$new_out"
  did="$(printf '%s\n' "$new_out" | sed -n 's/^DID: //p' | head -1)"
  if [ -n "$did" ]; then
    printf '%s' "$did" > "$RUN_DIR/space.did"
  else
    echo "site: could not parse repository DID from 'tonk space new' output" >&2
    exit 1
  fi
  "$TONK" remote add origin "$BENCH_URL/ucan/"
  "$TONK" remote set-upstream origin
  # Publish the seeded stdlib to the remote now, so a joiner (or an
  # inspector) sees current state even before an invite is minted.
  # `tonk space new` commits the seed before the upstream exists, so
  # nothing has pushed it yet.
  "$TONK" push
  "$TONK" status
}

# Mint an invite launcher URL for the browser side; prints the URL.
invite() {
  if [ ! -d "$SITE" ]; then
    echo "site: no site at $SITE (run setup first)" >&2
    exit 1
  fi
  "$TONK" invite --remote origin
}

case "${1:-}" in
  setup) setup ;;
  invite) invite ;;
  *) echo "usage: site.sh {setup|invite}" >&2; exit 2 ;;
esac

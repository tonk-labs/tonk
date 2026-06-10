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
  "$SLIDE" init
  "$SLIDE" remote add origin "$BENCH_URL/ucan/"
  "$SLIDE" remote set-upstream origin
  "$SLIDE" status
}

# Mint an invite launcher URL for the browser side; prints the URL.
invite() {
  cd "$SITE"
  "$SLIDE" invite --remote origin
}

case "${1:-}" in
  setup) setup ;;
  invite) invite ;;
  *) echo "usage: site.sh {setup|invite}" >&2; exit 2 ;;
esac

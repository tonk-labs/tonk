#!/usr/bin/env bash
# Seed the returning-user spot: a working habit tracker with views.
# Env: ROOT, RUN_DIR, SCENARIO
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
TONK="$ROOT/target/release/tonk"
cd "$RUN_DIR/site"
"$TONK" eval "$SCENARIO/seed.notation"
echo "prepare: seeded habit tracker" >&2

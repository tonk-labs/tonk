#!/usr/bin/env bash
# Seed the returning-user space: a working habit tracker with views.
# Resolves through TONK_SPACE (exported by run.sh) — the CLI never
# consults the cwd, so there is nothing to cd into.
# Env: ROOT, RUN_DIR, SCENARIO, TONK_SPACE
set -euo pipefail
ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; SCENARIO="${SCENARIO:?}"
: "${TONK_SPACE:?}"
TONK="$ROOT/target/release/tonk"
"$TONK" eval "$SCENARIO/seed.notation"
echo "prepare: seeded habit tracker" >&2

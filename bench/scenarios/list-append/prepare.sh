#!/usr/bin/env bash
# The harness has already created and selected a disposable blank spot. Keep
# preparation deliberately empty so the cold task receives no application code.
set -euo pipefail

ROOT="${ROOT:?}"
TONK="$ROOT/target/release/tonk"
"$TONK" status >/dev/null
echo "prepare: blank disposable spot is ready" >&2

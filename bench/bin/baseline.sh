#!/usr/bin/env bash
# Promote a run's screenshots to the scenario baseline.
set -euo pipefail
ROOT="${ROOT:?}"
SCENARIO_NAME="${1:?usage: baseline <scenario> <run-dir>}"
SRC="${2:?usage: baseline <scenario> <run-dir>}"
DEST="$ROOT/bench/baselines/$SCENARIO_NAME"
mkdir -p "$DEST"
cp "$SRC"/shots/*.png "$DEST/"
echo "baseline: $(ls "$DEST" | wc -l | tr -d ' ') screenshots -> $DEST" >&2

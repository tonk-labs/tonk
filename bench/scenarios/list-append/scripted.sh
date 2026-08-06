#!/usr/bin/env bash
# Known-good nominal command/projection reference path.
#
# The notation is not inlined here: it is extracted from `tonk guide
# events`, so a scripted pass proves the shipped guide's own example
# evaluates and produces a working app.
set -euo pipefail

TONK="${TONK:?}"
BENCH_BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../bin" && pwd)"

"$BENCH_BIN/guide-example.sh" | "$TONK" eval -

"$TONK" home todo/list

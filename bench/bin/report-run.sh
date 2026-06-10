#!/usr/bin/env bash
set -euo pipefail
RUN_DIR="${RUN_DIR:?}"
{
  echo "# Bench run: $SCENARIO_NAME"
  echo
  echo "Screenshots:"
  ls "$RUN_DIR/shots"/*.png 2>/dev/null | sed 's/^/- /' || true
  if [ -f "$RUN_DIR/shots/MISSING" ]; then
    echo
    echo "Missing:"
    sed 's/^/- /' "$RUN_DIR/shots/MISSING"
  fi
} > "$RUN_DIR/report.md"
true

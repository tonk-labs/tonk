#!/usr/bin/env bash
# Capture the scenario's checkpoint screenshots. Each line of the
# scenario's `checkpoints` file is a path suffix under
# /space/$SPACE_NAME/ (or "home" for the shell root). Blank lines and
# #-comments skipped. A failed screenshot is recorded as missing, not
# fatal — the judge sees what there is.
#
# Env: ROOT, RUN_DIR, BENCH_URL, SPACE_NAME, SCENARIO (scenario dir)
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"
SPACE_NAME="${SPACE_NAME:-bench}"; SCENARIO="${SCENARIO:?}"
B="$ROOT/bench/bin/browser.sh"
mkdir -p "$RUN_DIR/shots"

n=0
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue ;; esac
  n=$((n + 1))
  if [ "$line" = "home" ]; then
    url="$BENCH_URL/"
    name="$(printf '%02d-home' "$n")"
  else
    url="$BENCH_URL/space/$SPACE_NAME/$line"
    name="$(printf '%02d-%s' "$n" "$(printf '%s' "$line" | tr '/?=&' '----')")"
  fi
  echo "shots: $url" >&2
  if "$B" goto "$url" && "$B" wait-render && "$B" shot "$RUN_DIR/shots/$name.png"; then
    :
  else
    echo "$name $url" >> "$RUN_DIR/shots/MISSING"
  fi
done < "$SCENARIO/checkpoints"

# Reference render of the original artifact, when the scenario has one.
if [ -f "$SCENARIO/fixtures/artifact.html" ]; then
  # wait-doc: wait only for document.readyState complete — no tonk-host.
  wait_doc() {
    local timeout=10
    for _ in $(seq 1 $((timeout * 2))); do
      out="$("$B" eval "document.readyState === 'complete'" 2>&1 || true)"
      [ "$out" = "true" ] && return 0
      sleep 0.5
    done
    echo "shots: timed out waiting for doc ready" >&2
    return 1
  }
  if "$B" goto "file://$SCENARIO/fixtures/artifact.html" \
    && wait_doc \
    && "$B" shot "$RUN_DIR/shots/reference.png"; then
    :
  else
    echo "reference file://$SCENARIO/fixtures/artifact.html" >> "$RUN_DIR/shots/MISSING"
  fi
fi

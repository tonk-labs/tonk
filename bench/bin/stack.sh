#!/usr/bin/env bash
# Boot/teardown the hermetic local stack: trunk-built tonk-ui assets
# served by the access-service worker under `wrangler dev` with
# miniflare's local R2, persisted into the run directory.
#
# Env: ROOT (repo root), RUN_DIR (per-run artifact dir),
#      BENCH_PORT (default 8787), BENCH_URL (http://127.0.0.1:$BENCH_PORT)
set -euo pipefail

ROOT="${ROOT:?set ROOT to the repo root}"
RUN_DIR="${RUN_DIR:?set RUN_DIR}"
BENCH_PORT="${BENCH_PORT:-8787}"
BENCH_URL="${BENCH_URL:-http://127.0.0.1:$BENCH_PORT}"

ensure_shim() {
  if [ ! -e "$ROOT/result/tonk-access-service/worker/shim.mjs" ]; then
    echo "stack: building access-service shim (nix build .#tonk-cloudflare-artifacts)..." >&2
    (cd "$ROOT" && nix build .#tonk-cloudflare-artifacts)
  fi
}

ensure_ui() {
  echo "stack: trunk build tonk-ui..." >&2
  (cd "$ROOT/rust/tonk-ui" && trunk build)
}

start() {
  ensure_shim
  ensure_ui
  mkdir -p "$RUN_DIR"
  WRANGLER_SEND_METRICS=false wrangler dev \
    --config "$ROOT/bench/wrangler.bench.toml" \
    --port "$BENCH_PORT" --ip 127.0.0.1 \
    --persist-to "$RUN_DIR/wrangler-state" \
    > "$RUN_DIR/wrangler.log" 2>&1 &
  echo $! > "$RUN_DIR/wrangler.pid"
  for _ in $(seq 1 60); do
    if curl -fso /dev/null "$BENCH_URL/"; then
      echo "stack: up at $BENCH_URL" >&2
      return 0
    fi
    sleep 1
  done
  echo "stack: failed to come up; tail of wrangler.log:" >&2
  tail -20 "$RUN_DIR/wrangler.log" >&2
  exit 1
}

stop() {
  if [ -f "$RUN_DIR/wrangler.pid" ]; then
    kill "$(cat "$RUN_DIR/wrangler.pid")" 2>/dev/null || true
    rm -f "$RUN_DIR/wrangler.pid"
  fi
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *) echo "usage: stack.sh {start|stop}" >&2; exit 2 ;;
esac

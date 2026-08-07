#!/usr/bin/env bash
# Boot/teardown the hermetic local stack: trunk-built tonk-ui assets
# served by Caddy with /ucan/* proxied to the native access service
# (tonk-access-local, in-process S3) and /revocations proxied to the
# native account service's in-memory immutable-artifact relay.
#
# Env: ROOT (repo root), RUN_DIR (per-run artifact dir),
#      BENCH_PORT (default 8787), BENCH_URL (http://127.0.0.1:$BENCH_PORT)
set -euo pipefail

ROOT="${ROOT:?set ROOT to the repo root}"
RUN_DIR="${RUN_DIR:?set RUN_DIR}"
BENCH_PORT="${BENCH_PORT:-8787}"
BENCH_URL="${BENCH_URL:-http://127.0.0.1:$BENCH_PORT}"

ensure_ui() {
  echo "stack: trunk build tonk-ui..." >&2
  # The devshell exports the conventional NO_COLOR=1. Current Trunk maps that
  # env var onto clap's boolean `--no-color`, which accepts true/false rather
  # than 1/0. Normalize it for this subprocess.
  (cd "$ROOT/rust/tonk-ui" && NO_COLOR=true trunk build)
}

ensure_access_bin() {
  echo "stack: cargo build tonk-access-local..." >&2
  (cd "$ROOT" && cargo build --release -p tonk-access-service --features helpers --bin tonk-access-local)
}

ensure_account_bin() {
  echo "stack: cargo build tonk-account-local..." >&2
  (cd "$ROOT" && cargo build --release -p tonk-account-service --features helpers --bin tonk-account-local)
}

ensure_tonk() {
  echo "stack: cargo build tonk-cli..." >&2
  (cd "$ROOT" && cargo build --release -p tonk-cli)
}

start() {
  if [ -f "$RUN_DIR/access.pid" ] && kill -0 "$(cat "$RUN_DIR/access.pid")" 2>/dev/null \
  && [ -f "$RUN_DIR/caddy.pid" ] && kill -0 "$(cat "$RUN_DIR/caddy.pid")" 2>/dev/null; then
    echo "stack: already running (access pid $(cat "$RUN_DIR/access.pid"))" >&2
    return 0
  fi
  stop

  ensure_ui
  ensure_access_bin
  ensure_account_bin
  ensure_tonk
  mkdir -p "$RUN_DIR"

  # --- launch access service ---
  set -m
  "$ROOT/target/release/tonk-access-local" > "$RUN_DIR/access.log" 2>&1 &
  ACCESS_PID=$!
  set +m
  echo "$ACCESS_PID" > "$RUN_DIR/access.pid"

  # poll for ACCESS_SERVICE_URL= line (timeout 15s)
  ACCESS_URL=""
  for _ in $(seq 1 30); do
    if ACCESS_URL=$(grep -m1 '^ACCESS_SERVICE_URL=' "$RUN_DIR/access.log" 2>/dev/null | cut -d= -f2-); then
      [ -n "$ACCESS_URL" ] && break
    fi
    sleep 0.5
  done

  if [ -z "$ACCESS_URL" ]; then
    echo "stack: access service did not print URL; tail of access.log:" >&2
    tail -20 "$RUN_DIR/access.log" >&2
    exit 1
  fi

  echo "$ACCESS_URL" > "$RUN_DIR/access.url"
  ACCESS_PORT="${ACCESS_URL##*:}"
  echo "stack: access service up at $ACCESS_URL" >&2

  # --- launch immutable revocation relay ---
  set -m
  "$ROOT/target/release/tonk-account-local" > "$RUN_DIR/account.log" 2>&1 &
  ACCOUNT_PID=$!
  set +m
  echo "$ACCOUNT_PID" > "$RUN_DIR/account.pid"

  ACCOUNT_URL=""
  for _ in $(seq 1 30); do
    if ACCOUNT_URL=$(grep -m1 '^ACCOUNT_SERVICE_URL=' "$RUN_DIR/account.log" 2>/dev/null | cut -d= -f2-); then
      [ -n "$ACCOUNT_URL" ] && break
    fi
    sleep 0.5
  done

  if [ -z "$ACCOUNT_URL" ]; then
    echo "stack: account service did not print URL; tail of account.log:" >&2
    tail -20 "$RUN_DIR/account.log" >&2
    exit 1
  fi

  echo "$ACCOUNT_URL" > "$RUN_DIR/account.url"
  ACCOUNT_PORT="${ACCOUNT_URL##*:}"
  echo "stack: revocation relay up at $ACCOUNT_URL/revocations" >&2

  # --- write Caddyfile ---
  cat > "$RUN_DIR/Caddyfile" <<EOF
{
  auto_https off
  admin off
}
:$BENCH_PORT {
  handle /registry/tonk {
    root * "$RUN_DIR/registry"
    rewrite * /tonk.json
    header Content-Type application/json
    file_server
  }
  handle /registry/* {
    uri strip_prefix /registry
    root * "$RUN_DIR/registry"
    file_server
  }
  handle /ucan/* {
    reverse_proxy 127.0.0.1:$ACCESS_PORT
  }
  handle /revocations {
    reverse_proxy 127.0.0.1:$ACCOUNT_PORT
  }
  handle {
    root * "$ROOT/rust/tonk-ui/dist"
    try_files {path} /index.html
    file_server
  }
}
EOF

  # --- launch caddy ---
  set -m
  caddy run --config "$RUN_DIR/Caddyfile" --adapter caddyfile > "$RUN_DIR/caddy.log" 2>&1 &
  CADDY_PID=$!
  set +m
  echo "$CADDY_PID" > "$RUN_DIR/caddy.pid"

  # health check: poll until $BENCH_URL/ responds (timeout 60s)
  for _ in $(seq 1 60); do
    if curl -fso /dev/null "$BENCH_URL/"; then
      echo "stack: up at $BENCH_URL" >&2
      return 0
    fi
    sleep 1
  done

  echo "stack: failed to come up; logs:" >&2
  echo "--- access.log ---" >&2
  tail -20 "$RUN_DIR/access.log" >&2
  echo "--- caddy.log ---" >&2
  tail -20 "$RUN_DIR/caddy.log" >&2
  exit 1
}

stop() {
  for svc in caddy access account; do
    local pidfile="$RUN_DIR/$svc.pid"
    if [ -f "$pidfile" ]; then
      local pid
      pid="$(cat "$pidfile")"
      kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
      for _ in $(seq 1 20); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5
      done
      rm -f "$pidfile"
    fi
  done
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  *) echo "usage: stack.sh {start|stop}" >&2; exit 2 ;;
esac

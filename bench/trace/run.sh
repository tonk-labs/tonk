#!/usr/bin/env bash
# Trace the interface through the cases that make it reload or stall.
#
#   1. start the access service, the logging front proxy (bench/perf's
#      tonk-perf proxy, which serves the trunk dist), and a gate in front
#      of it that the trace switches between open, slow, offline and stalled
#   2. seed two spaces from a CLI "other device" and mint an invite to each
#   3. run trace.mjs: join both in a fresh browser, then reload, switch,
#      restart the worker, write remotely, and repeat under bad networks
#
# Output in bench/trace/runs/<stamp>/: events.jsonl (every DOM and network
# event, per frame), summary.json (per scenario counts), screenshots, and
# the proxy's requests.jsonl.
#
# Usage: bench/trace/run.sh [--dist DIR] [--notes N]
# Expects release builds of tonk-access-local, tonk and tonk-perf, and a
# trunk dist (default rust/tonk-ui/dist).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST="$ROOT/rust/tonk-ui/dist"
NOTES=200
while [ $# -gt 0 ]; do
  case "$1" in
    --dist) DIST="$2"; shift 2 ;;
    --notes) NOTES="$2"; shift 2 ;;
    *) echo "unknown arg $1" >&2; exit 2 ;;
  esac
done
[ -f "$DIST/index.html" ] || { echo "trace: no dist at $DIST" >&2; exit 1; }

RUN_DIR="$ROOT/bench/trace/runs/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$RUN_DIR"
echo "trace: run dir $RUN_DIR" >&2

PROXY_PORT="${TRACE_PROXY_PORT:-8798}"
GATE_PORT="${TRACE_GATE_PORT:-8799}"
BASE="http://127.0.0.1:$GATE_PORT"
TONK="$ROOT/target/release/tonk"
PERF="$ROOT/target/release/tonk-perf"
for bin in "$ROOT/target/release/tonk-access-local" "$TONK" "$PERF"; do
  [ -x "$bin" ] || { echo "trace: missing $bin" >&2; exit 1; }
done

cleanup() {
  [ -n "${TRACE_KEEP:-}" ] && return 0
  for pid in gate proxy provisioner access; do
    [ -f "$RUN_DIR/$pid.pid" ] && kill "$(cat "$RUN_DIR/$pid.pid")" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

"$ROOT/target/release/tonk-access-local" > "$RUN_DIR/access.log" 2>&1 &
echo $! > "$RUN_DIR/access.pid"
ACCESS_URL=""
for _ in $(seq 1 60); do
  ACCESS_URL="$(sed -n 's/^ACCESS_SERVICE_URL=//p' "$RUN_DIR/access.log" | head -1)"
  [ -n "$ACCESS_URL" ] && break
  sleep 0.5
done
[ -n "$ACCESS_URL" ] || { echo "trace: access service never came up" >&2; exit 1; }

# Provision every subject the run mints, as bench/perf does.
(
  tail -0 -f "$RUN_DIR/access.log" | while IFS= read -r line; do
    case "$line" in
      *"is not provisioned"*)
        subj="$(printf '%s' "$line" | grep -o 'did:key:[A-Za-z0-9]*' | head -1)"
        [ -n "$subj" ] && curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
          -H 'Content-Type: application/json' -d "{\"subject\":\"$subj\"}" || true
        ;;
    esac
  done
) &
echo $! > "$RUN_DIR/provisioner.pid"

"$PERF" proxy --listen "$PROXY_PORT" --ucan "$ACCESS_URL" --root "$DIST" \
  --latency-ms 0 --bandwidth-kbps 0 --log "$RUN_DIR/requests.jsonl" > "$RUN_DIR/proxy.log" 2>&1 &
echo $! > "$RUN_DIR/proxy.pid"
node "$ROOT/bench/trace/gate.mjs" "$GATE_PORT" "http://127.0.0.1:$PROXY_PORT" > "$RUN_DIR/gate.log" 2>&1 &
echo $! > "$RUN_DIR/gate.pid"
for _ in $(seq 1 20); do
  grep -q PROXY_READY "$RUN_DIR/proxy.log" 2>/dev/null && grep -q GATE_READY "$RUN_DIR/gate.log" && break
  sleep 0.5
done
curl -fso /dev/null "$BASE/" || { echo "trace: gate not serving" >&2; exit 1; }

# The CLI is a second device with no account: a device-rooted space is
# enough to seed, push and invite. Keep its state inside the run.
export TONK_SPACES_STATE="$RUN_DIR/spaces-state"
export XDG_DATA_HOME="$RUN_DIR/cli-data"
export TONK_UNSAFE_ALLOW_DEVICE_ROOT=1 TONK_NO_UPDATE_CHECK=1 DO_NOT_TRACK=1
seed_space() { # seed_space <name> -> prints DID
  local name="$1" out did
  out="$("$TONK" space new "$name" --site "$RUN_DIR/site-$name")"
  did="$(printf '%s\n' "$out" | sed -n 's/^DID: //p' | head -1)"
  curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
    -H 'Content-Type: application/json' -d "{\"subject\":\"$did\"}"
  TONK_SPACE="$name" "$TONK" remote add origin "$ACCESS_URL/ucan/" >&2
  TONK_SPACE="$name" "$TONK" remote set-upstream origin >&2
  TONK_SPACE="$name" TONK="$TONK" PERF_NOTES="$NOTES" "$ROOT/bench/perf/seed.sh" >&2
  TONK_SPACE="$name" "$TONK" push >&2 || { echo "trace: push of $name failed" >&2; exit 1; }
  printf '%s' "$did"
}
invite() { # invite <name> -> invite URL on the gate's origin
  local url
  url="$(TONK_SPACE="$1" "$TONK" invite --remote origin | tr -d '[:space:]')"
  printf '%s%s' "$BASE" "$(printf '%s' "$url" | sed -E 's#^https?://[^/]+##')"
}
DID_A="$(seed_space trace-a)" || exit 1
DID_B="$(seed_space trace-b)" || exit 1
echo "trace: seeded $DID_A and $DID_B" >&2

jq -n \
  --arg out "$RUN_DIR" --arg base "$BASE" --arg gate "$BASE" \
  --arg invite "$(invite trace-a)" --arg inviteB "$(invite trace-b)" \
  --arg a "$DID_A" --arg b "$DID_B" --arg tonk "$TONK" \
  --arg state "$TONK_SPACES_STATE" \
  '{out:$out, base:$base, gate:$gate, invite:$invite, inviteB:$inviteB,
    spaces:[$a,$b], tonk:$tonk,
    cliEnv:{TONK_SPACES_STATE:$state, TONK_SPACE:"trace-a",
      XDG_DATA_HOME:($out+"/cli-data"), TONK_UNSAFE_ALLOW_DEVICE_ROOT:"1",
      TONK_NO_UPDATE_CHECK:"1", DO_NOT_TRACK:"1"}}' > "$RUN_DIR/config.json"

# TRACE_DRIVER picks what runs against the stack: the trace (default) or
# bench/stability/stability.mjs, which checks the invariants.
DRIVER="${TRACE_DRIVER:-$ROOT/bench/trace/trace.mjs}"
node "$DRIVER" "$RUN_DIR/config.json" | tee "$RUN_DIR/summary.txt"
echo "trace: artifacts in $RUN_DIR" >&2

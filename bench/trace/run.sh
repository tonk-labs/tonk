#!/usr/bin/env bash
# Trace the interface through the cases that make it reload or stall.
#
#   1. start the access service, the logging front proxy (bench/perf's
#      tonk-perf proxy, which serves the trunk dist), and shaped links
#      (`tonk-perf link`) in front of both: `page` before the proxy and
#      `ucan` before the access service, which the browser syncs through.
#      The trace switches them between open, slow, offline and stalled
#      through the links' control server
#   2. seed two spaces from a CLI "other device" and mint an invite to each
#   3. run trace.mjs: join both in a fresh browser, then reload, switch,
#      restart the worker, write remotely, and repeat under bad networks
#
# Output in bench/trace/runs/<stamp>/: events.jsonl (every DOM and network
# event, per frame), summary.json (per scenario counts), screenshots, and
# the proxy's requests.jsonl, and links.jsonl: every HTTP exchange that
# crossed a link, with its kind, bytes and timing as the browser saw them.
#
# TRACE_PROFILE names the network the links start as (default lan; see
# `tonk-perf profiles`), and TRACE_SHAPE adjusts it (`kbps=400&jitter_ms=50`);
# a scenario's `open` returns to it.
#
# Usage: bench/trace/run.sh [--dist DIR] [--notes N]
# Expects release builds of tonk-access-local, tonk and tonk-perf (or their
# paths in TRACE_ACCESS_LOCAL, TRACE_TONK and TRACE_PERF), and a
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
UCAN_PORT="${TRACE_UCAN_PORT:-8800}"
CONTROL_PORT="${TRACE_CONTROL_PORT:-8801}"
PROFILE="${TRACE_PROFILE:-lan}"
BASE="http://127.0.0.1:$GATE_PORT"
CONTROL="http://127.0.0.1:$CONTROL_PORT"
# The remote the browser syncs with: the access service, through its link.
REMOTE="http://127.0.0.1:$UCAN_PORT"
TONK="${TRACE_TONK:-$ROOT/target/release/tonk}"
PERF="${TRACE_PERF:-$ROOT/target/release/tonk-perf}"
ACCESS_LOCAL="${TRACE_ACCESS_LOCAL:-$ROOT/target/release/tonk-access-local}"
for bin in "$ACCESS_LOCAL" "$TONK" "$PERF"; do
  [ -x "$bin" ] || { echo "trace: missing $bin" >&2; exit 1; }
done

cleanup() {
  # What crossed each link, and what was still waiting on an answer when
  # the run ended: a request in `inflight.json` was never answered.
  if [ -n "${CONTROL:-}" ]; then
    curl -fso "$RUN_DIR/links-stats.json" "$CONTROL/stats" || true
    curl -fso "$RUN_DIR/inflight.json" "$CONTROL/inflight" || true
  fi
  [ -n "${TRACE_KEEP:-}" ] && return 0
  for pid in links proxy provisioner access; do
    [ -f "$RUN_DIR/$pid.pid" ] && kill "$(cat "$RUN_DIR/$pid.pid")" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

# The browser must reach the service only through its link: the object
# permits name the link (ACCESS_PUBLIC_ENDPOINT), and so does the service's
# DID document (ACCESS_PUBLIC_ORIGIN), which is where the browser finds the
# socket live updates arrive on. A document naming another host is not
# taken, and the browser falls back to polling.
ACCESS_PUBLIC_ENDPOINT="$REMOTE" ACCESS_PUBLIC_ORIGIN="$REMOTE" "$ACCESS_LOCAL" > "$RUN_DIR/access.log" 2>&1 &
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
for _ in $(seq 1 20); do
  grep -q PROXY_READY "$RUN_DIR/proxy.log" 2>/dev/null && break
  sleep 0.5
done
"$PERF" link --profile "$PROFILE" --control "$CONTROL_PORT" \
  --link "page=$GATE_PORT:127.0.0.1:$PROXY_PORT" \
  --link "ucan=$UCAN_PORT:$ACCESS_URL" \
  --log "$RUN_DIR/links.jsonl" > "$RUN_DIR/links.log" 2>&1 &
echo $! > "$RUN_DIR/links.pid"
for _ in $(seq 1 20); do
  grep -q LINK_CONTROL "$RUN_DIR/links.log" 2>/dev/null && break
  sleep 0.5
done
curl -fso /dev/null "$BASE/" || { echo "trace: links not serving" >&2; exit 1; }
# TRACE_SHAPE adjusts the profile, as `/profile` takes it, such as
# `idle_cut_ms=10000` for a NAT that forgets a connection quiet for 10 s.
if [ -n "${TRACE_SHAPE:-}" ]; then
  curl -fsS "$CONTROL/profile?$TRACE_SHAPE" >&2 || { echo "trace: bad TRACE_SHAPE" >&2; exit 1; }
fi
echo "trace: network $PROFILE${TRACE_SHAPE:+ ($TRACE_SHAPE)}, control $CONTROL" >&2

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
  # The CLI is another device on another network: it pushes to the
  # service directly, so a stalled link never holds up a remote write.
  # Its invites name the link, the address the browser syncs through.
  TONK_SPACE="$name" "$TONK" remote add origin "$ACCESS_URL/ucan/" >&2
  TONK_SPACE="$name" "$TONK" remote add browser "$REMOTE/ucan/" >&2
  TONK_SPACE="$name" "$TONK" remote set-upstream origin >&2
  TONK_SPACE="$name" TONK="$TONK" PERF_NOTES="$NOTES" "$ROOT/bench/perf/seed.sh" >&2
  TONK_SPACE="$name" "$TONK" push >&2 || { echo "trace: push of $name failed" >&2; exit 1; }
  printf '%s' "$did"
}
invite() { # invite <name> -> invite URL on the page link's origin
  local url
  url="$(TONK_SPACE="$1" "$TONK" invite --remote browser | tr -d '[:space:]')"
  printf '%s%s' "$BASE" "$(printf '%s' "$url" | sed -E 's#^https?://[^/]+##')"
}
DID_A="$(seed_space trace-a)" || exit 1
DID_B="$(seed_space trace-b)" || exit 1
echo "trace: seeded $DID_A and $DID_B" >&2

jq -n \
  --arg out "$RUN_DIR" --arg base "$BASE" --arg gate "$CONTROL" \
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

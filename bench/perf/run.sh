#!/usr/bin/env bash
# Cold-load replication perf run, end to end:
#   1. build tonk-access-local (with redeem-object logging), the tonk
#      CLI, and the tonk-perf proxy/analyzer
#   2. start access service + the logging/shaping front proxy (serves
#      the trunk dist; caddy/nix not needed)
#   3. seed a space with content, push, mint an invite
#   4. cold headless Chrome joins via the invite and lands on the
#      space route; phase timestamps recorded
#   5. reload the space page (warm client)
#   6. `tonk-perf analyze` digests the proxy log + redeem log per phase
#
# Usage: bench/perf/run.sh [--latency-ms N] [--bandwidth-kbps N] [--notes N]
# The trunk dist at rust/tonk-ui/dist is used as-is; rebuild it first if
# it is stale (trunk serves/builds into it).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LATENCY_MS=0
BANDWIDTH_KBPS=0
NOTES=200
while [ $# -gt 0 ]; do
  case "$1" in
    --latency-ms) LATENCY_MS="$2"; shift 2 ;;
    --bandwidth-kbps) BANDWIDTH_KBPS="$2"; shift 2 ;;
    --notes) NOTES="$2"; shift 2 ;;
    *) echo "unknown arg $1" >&2; exit 2 ;;
  esac
done

RUN_DIR="$ROOT/bench/perf/runs/$(date +%Y%m%d-%H%M%S)-lat${LATENCY_MS}-n${NOTES}"
mkdir -p "$RUN_DIR"
echo "perf: run dir $RUN_DIR" >&2

PROXY_PORT="${PERF_PROXY_PORT:-8798}"
BENCH_URL="http://127.0.0.1:$PROXY_PORT"
TONK="$ROOT/target/release/tonk"
PERF="$ROOT/target/release/tonk-perf"

now_ms() { "$PERF" now; }

phase() { # phase <name> <t0> <t1>
  "$PERF" phase "$RUN_DIR" "$1" "$2" "$3"
}

cleanup() {
  # PERF_KEEP=1 leaves the stack and browser running for interactive
  # debugging; kill the pids in $RUN_DIR by hand when done.
  [ -n "${PERF_KEEP:-}" ] && { echo "perf: keeping stack alive (PERF_KEEP)" >&2; return 0; }
  "$ROOT/bench/bin/browser.sh" stop 2>/dev/null || true
  [ -f "$RUN_DIR/provisioner.pid" ] && kill "$(cat "$RUN_DIR/provisioner.pid")" 2>/dev/null || true
  [ -f "$RUN_DIR/proxy.pid" ] && kill "$(cat "$RUN_DIR/proxy.pid")" 2>/dev/null || true
  [ -f "$RUN_DIR/access.pid" ] && kill "$(cat "$RUN_DIR/access.pid")" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- 1. build ---
if [ -z "${PERF_SKIP_BUILD:-}" ]; then
  echo "perf: building tonk-access-local + tonk + tonk-perf..." >&2
  (cd "$ROOT" && cargo build --release -p tonk-access-service --features helpers --bin tonk-access-local 2>&1 | tail -2 >&2)
  (cd "$ROOT" && cargo build --release -p tonk-cli 2>&1 | tail -2 >&2)
  (cd "$ROOT" && cargo build --release -p tonk-perf 2>&1 | tail -2 >&2)
fi
[ -x "$ROOT/target/release/tonk-access-local" ] || { echo "perf: no tonk-access-local binary" >&2; exit 1; }
[ -x "$TONK" ] || { echo "perf: no tonk binary" >&2; exit 1; }
[ -x "$PERF" ] || { echo "perf: no tonk-perf binary" >&2; exit 1; }

[ -f "$ROOT/rust/tonk-ui/dist/index.html" ] || { echo "perf: no dist; run trunk build first" >&2; exit 1; }

# --- 2. stack ---
"$ROOT/target/release/tonk-access-local" > "$RUN_DIR/access.log" 2>&1 &
echo $! > "$RUN_DIR/access.pid"
ACCESS_URL=""
for _ in $(seq 1 60); do
  ACCESS_URL="$(sed -n 's/^ACCESS_SERVICE_URL=//p' "$RUN_DIR/access.log" | head -1)"
  [ -n "$ACCESS_URL" ] && break
  sleep 0.5
done
[ -n "$ACCESS_URL" ] || { echo "perf: access service never came up" >&2; tail -5 "$RUN_DIR/access.log" >&2; exit 1; }
echo "perf: access service at $ACCESS_URL" >&2

# The provisioning gate would refuse every subject this run mints (the
# space, the browser's account). /_test/provision is the sanctioned
# shortcut; subjects appear mid-run, so watch the access log for
# refusals and provision reactively.
(
  tail -0 -f "$RUN_DIR/access.log" | while IFS= read -r line; do
    case "$line" in
      *"is not provisioned"*)
        subj="$(printf '%s' "$line" | grep -o 'did:key:[A-Za-z0-9]*' | head -1)"
        [ -n "$subj" ] && curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
          -H 'Content-Type: application/json' -d "{\"subject\":\"$subj\"}" \
          && echo "provisioned $subj" >> "$RUN_DIR/provisioner.log"
        ;;
    esac
  done
) &
echo $! > "$RUN_DIR/provisioner.pid"

"$PERF" proxy \
  --listen "$PROXY_PORT" --ucan "$ACCESS_URL" \
  --root "$ROOT/rust/tonk-ui/dist" \
  --latency-ms "$LATENCY_MS" --bandwidth-kbps "$BANDWIDTH_KBPS" \
  --log "$RUN_DIR/requests.jsonl" > "$RUN_DIR/proxy.log" 2>&1 &
echo $! > "$RUN_DIR/proxy.pid"
for _ in $(seq 1 20); do
  grep -q PROXY_READY "$RUN_DIR/proxy.log" 2>/dev/null && break
  sleep 0.5
done
curl -fso /dev/null "$BENCH_URL/" || { echo "perf: proxy not serving" >&2; exit 1; }
echo "perf: front proxy at $BENCH_URL (latency ${LATENCY_MS}ms)" >&2

# --- 3. seed ---
export TONK_SPACES_STATE="$RUN_DIR/spaces-state"
export TONK_SPACE=perf
t0="$(now_ms)"
new_out="$("$TONK" space new perf --site "$RUN_DIR/site")"
did="$(printf '%s\n' "$new_out" | sed -n 's/^DID: //p' | head -1)"
[ -n "$did" ] || { echo "perf: no DID from space new" >&2; exit 1; }
printf '%s' "$did" > "$RUN_DIR/space.did"
curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
  -H 'Content-Type: application/json' -d "{\"subject\":\"$did\"}"
# The CLI talks straight to the access service (unshaped): the run
# measures the browser, and the seed would otherwise crawl at 3G too.
"$TONK" remote add origin "$ACCESS_URL/ucan/"
"$TONK" remote set-upstream origin
TONK="$TONK" PERF_NOTES="$NOTES" "$ROOT/bench/perf/seed.sh"
"$TONK" push
phase seed "$t0" "$(now_ms)"
echo "perf: seeded space $did with $NOTES notes" >&2

INVITE_URL="$("$TONK" invite --remote origin | tr -d '[:space:]')"
PATH_QF="$(printf '%s' "$INVITE_URL" | sed -E 's#^https?://[^/]+##')"
INVITE_URL="$BENCH_URL$PATH_QF"
echo "perf: invite $INVITE_URL" >&2

# --- 4. cold join ---
export RUN_DIR
export CHROMEDRIVER="${CHROMEDRIVER:-$HOME/.local/bin/chromedriver}"
B="$ROOT/bench/bin/browser.sh"
"$B" start
t0="$(now_ms)"
"$B" goto "$INVITE_URL"
"$B" wait-render
"$B" wait-sw
"$B" wait-render
for _ in $(seq 1 120); do
  loc="$("$B" eval "window.location.pathname")"
  loc="${loc#\"}"; loc="${loc%\"}"
  case "$loc" in */join*) sleep 1 ;; *) break ;; esac
done
case "$loc" in
  */join*) echo "perf: join did not complete" >&2; "$B" shot "$RUN_DIR/join-stuck.png" || true; exit 1 ;;
esac
echo "perf: joined, landed on $loc" >&2
"$B" wait-render
"$B" wait-sw
# Confirm the pull so "join done" means data arrived, not just the route.
for i in $(seq 1 240); do
  pull_raw="$("$B" eval-async "(function(done){fetch('/api/repository/$did/branch/main/sync/pull', {method:'POST'}).then(function(r){return r.text().then(function(t){done(r.status+':'+t.slice(0,120));});}).catch(function(e){done('err:'+String(e));});})(arguments[0])" 2>/dev/null || true)"
  case "$pull_raw" in
    \"200:*) break ;;
    *) sleep 0.5 ;;
  esac
done
phase join "$t0" "$(now_ms)"
"$B" shot "$RUN_DIR/join-done.png" || true
echo "perf: join phase complete" >&2

# --- 4b. the narrow query: directory route over the seeded concept ---
# /space/<did>/note is the {*model} directory route; rendering it is
# the "list all notes" query, which must walk the note tree cold.
# Quiesce on the redeem log: sync is done when block permits stop.
t0="$(now_ms)"
"$B" goto "$BENCH_URL/space/$did/note"
"$B" wait-render
prev=-1
for _ in $(seq 1 60); do
  cur="$(grep -c 'get/archive/block' "$RUN_DIR/access.log" || true)"
  [ "$cur" = "$prev" ] && break
  prev="$cur"
  sleep 3
done
phase query "$t0" "$(now_ms)"
"$B" shot "$RUN_DIR/query-done.png" || true
echo "perf: query phase complete ($cur block permits total)" >&2

# --- 5. warm reload ---
sleep 3
t0="$(now_ms)"
"$B" goto "$BENCH_URL/space/$did"
"$B" wait-render
sleep 5
phase reload "$t0" "$(now_ms)"
"$B" shot "$RUN_DIR/reload-done.png" || true

# --- 6. report ---
"$B" stop
sleep 1
{
  "$PERF" analyze "$RUN_DIR" --phase join
  echo
  "$PERF" analyze "$RUN_DIR" --phase query
  echo
  "$PERF" analyze "$RUN_DIR" --phase reload
} | tee "$RUN_DIR/report.txt"
echo "perf: artifacts in $RUN_DIR" >&2

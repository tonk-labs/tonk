#!/usr/bin/env bash
# Why does a narrow query need so many blocks?
#
# Seeds a space (configurable data size and commit granularity), then
# drives a COLD native CLI client through join -> status -> cold query
# -> warm query -> no-op pull, slicing the access service's redeem log
# per step. The per-step block sets and their overlaps attribute every
# remote block fetch to the operation that needed it, and comparing runs
# with different --batches counts isolates how much of the cold cost is
# commit history rather than live tree.
#
# Usage: bench/perf/why-blocks.sh [--notes N] [--batches B]
# Needs target/release/{tonk,tonk-access-local} (PERF_SKIP_BUILD to skip).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NOTES=500
BATCHES=10
while [ $# -gt 0 ]; do
  case "$1" in
    --notes) NOTES="$2"; shift 2 ;;
    --batches) BATCHES="$2"; shift 2 ;;
    *) echo "unknown arg $1" >&2; exit 2 ;;
  esac
done

RUN_DIR="$ROOT/bench/perf/runs/$(date +%Y%m%d-%H%M%S)-why-n${NOTES}-b${BATCHES}"
mkdir -p "$RUN_DIR/steps"
TONK="$ROOT/target/release/tonk"
LOG="$RUN_DIR/access.log"

if [ -z "${PERF_SKIP_BUILD:-}" ]; then
  (cd "$ROOT" && cargo build --release -p tonk-access-service --features helpers --bin tonk-access-local 2>&1 | tail -1 >&2)
  (cd "$ROOT" && cargo build --release -p tonk-cli 2>&1 | tail -1 >&2)
fi

cleanup() {
  [ -f "$RUN_DIR/provisioner.pid" ] && kill "$(cat "$RUN_DIR/provisioner.pid")" 2>/dev/null || true
  [ -f "$RUN_DIR/access.pid" ] && kill "$(cat "$RUN_DIR/access.pid")" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- access service + reactive provisioning (mirrors run.sh) ---
"$ROOT/target/release/tonk-access-local" > "$LOG" 2>&1 &
echo $! > "$RUN_DIR/access.pid"
ACCESS_URL=""
for _ in $(seq 1 60); do
  ACCESS_URL="$(sed -n 's/^ACCESS_SERVICE_URL=//p' "$LOG" | head -1)"
  [ -n "$ACCESS_URL" ] && break
  sleep 0.5
done
[ -n "$ACCESS_URL" ] || { echo "why-blocks: access service never came up" >&2; exit 1; }
(
  tail -0 -f "$LOG" | while IFS= read -r line; do
    case "$line" in
      *"is not provisioned"*)
        subj="$(printf '%s' "$line" | grep -o 'did:key:[A-Za-z0-9]*' | head -1)"
        [ -n "$subj" ] && curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
          -H 'Content-Type: application/json' -d "{\"subject\":\"$subj\"}"
        ;;
    esac
  done
) &
echo $! > "$RUN_DIR/provisioner.pid"

# --- step slicing: everything the log gained since the last mark ---
step_begin() { wc -l < "$LOG" > "$RUN_DIR/.offset"; }
step_end() { # step_end <name>
  local offset
  offset="$(cat "$RUN_DIR/.offset")"
  tail -n "+$((offset + 1))" "$LOG" > "$RUN_DIR/steps/$1.log"
}
blocks_of() { # object hashes of GET block permits in a step log
  sed -n 's/.*command=\/use\/get\/archive\/block .*object=GET .*\/index\///p' "$RUN_DIR/steps/$1.log"
}

# --- seed: NOTES notes across BATCHES commits, one push ---
export TONK_SPACES_STATE="$RUN_DIR/seeder"
export TONK_SPACE=perf
new_out="$("$TONK" space new perf --site "$RUN_DIR/site")"
did="$(printf '%s\n' "$new_out" | sed -n 's/^DID: //p' | head -1)"
[ -n "$did" ] || { echo "why-blocks: no DID" >&2; exit 1; }
curl -fso /dev/null -X POST "$ACCESS_URL/_test/provision" \
  -H 'Content-Type: application/json' -d "{\"subject\":\"$did\"}"
"$TONK" remote add origin "$ACCESS_URL/ucan/" >/dev/null
"$TONK" remote set-upstream origin >/dev/null
"$TONK" concept add note --description "A perf note" \
  --field title:text:one --field body:text:one >/dev/null

per_batch=$(( (NOTES + BATCHES - 1) / BATCHES ))
batch_file="$(mktemp)"
i=1
while [ "$i" -le "$NOTES" ]; do
  : > "$batch_file"
  end=$((i + per_batch - 1))
  [ "$end" -gt "$NOTES" ] && end="$NOTES"
  while [ "$i" -le "$end" ]; do
    printf 'note!: &note-%s\n  title: "Note %s"\n  body: "Body of perf note %s with enough text to be non-trivial."\n' "$i" "$i" "$i" >> "$batch_file"
    i=$((i + 1))
  done
  "$TONK" eval "$batch_file" >/dev/null
done
rm -f "$batch_file"

step_begin
"$TONK" push >/dev/null
step_end push
INVITE_URL="$("$TONK" invite --remote origin | head -1 | tr -d '[:space:]')"

# --- cold client steps ---
export TONK_SPACES_STATE="$RUN_DIR/joiner"
unset TONK_SPACE

step_begin
"$TONK" join --name perf-joined "$INVITE_URL" > "$RUN_DIR/join.out" 2>&1 || {
  echo "why-blocks: join failed:" >&2; tail -5 "$RUN_DIR/join.out" >&2; exit 1; }
step_end join
export TONK_SPACE=perf-joined

step_begin
"$TONK" status > "$RUN_DIR/status.out" 2>&1 || true
step_end status

step_begin
"$TONK" show note > "$RUN_DIR/show.out" 2>&1 || true
step_end show

step_begin
"$TONK" query note > "$RUN_DIR/query-cold.out" 2>&1 || true
step_end query-cold

step_begin
"$TONK" query note > /dev/null 2>&1 || true
step_end query-warm

step_begin
"$TONK" pull > /dev/null 2>&1 || true
step_end pull-noop

# --- report ---
{
  echo "== why-blocks: $NOTES notes in $BATCHES commits =="
  # The CLI auto-pushes each eval once an upstream is set, so the
  # seeder's uploads are spread across the whole log, not the final
  # explicit push. The union is the space's full remote block set.
  pushed="$(grep -c 'command=/use/put/archive/block' "$RUN_DIR/access.log" || true)"
  sed -n 's/.*command=\/use\/put\/archive\/block .*object=PUT .*\/index\///p' "$RUN_DIR/access.log" \
    | sort -u > "$RUN_DIR/pushed-blocks"
  echo "   seeder uploaded: $pushed block puts, $(wc -l < "$RUN_DIR/pushed-blocks" | tr -d ' ') distinct"
  echo
  printf '   %-12s %8s %10s %9s %4s\n' step redeems blockGETs distinct dup
  for step in join status show query-cold query-warm pull-noop; do
    total="$(grep -c ACCESS_UCAN "$RUN_DIR/steps/$step.log" || true)"
    gets="$(blocks_of "$step" | wc -l | tr -d ' ')"
    distinct="$(blocks_of "$step" | sort -u | wc -l | tr -d ' ')"
    printf '   %-12s %8s %10s %9s %4s\n' "$step" "$total" "$gets" "$distinct" "$((gets - distinct))"
  done
  echo
  echo "   overlaps (blocks a later step re-fetched that an earlier step already had):"
  for pair in "join query-cold" "show query-cold" "join show"; do
    a="${pair% *}"; b="${pair#* }"
    shared="$(comm -12 <(blocks_of "$a" | sort -u) <(blocks_of "$b" | sort -u) | wc -l | tr -d ' ')"
    echo "     $a ∩ $b: $shared"
  done
  echo
  echo "   cold-client blocks that the seeder uploaded (sanity: should be all):"
  for step in join show query-cold; do
    in_push="$(comm -12 <(blocks_of "$step" | sort -u) "$RUN_DIR/pushed-blocks" | wc -l | tr -d ' ')"
    echo "     $step: $in_push of $(blocks_of "$step" | sort -u | wc -l | tr -d ' ')"
  done
} | tee "$RUN_DIR/report.txt"
echo "why-blocks: artifacts in $RUN_DIR" >&2

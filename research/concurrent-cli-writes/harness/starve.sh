#!/usr/bin/env bash
# One big eval (K notes in one document) against S small writers that keep
# committing until the big one is done. Policy:
#   retry  - a writer whose eval exits 4 runs it again at once (no backoff),
#            the big one up to 64 attempts: optimistic, document re-run
#   queue  - every eval runs under a blocking flock: serialized
# Usage: TONK=... starve.sh <policy> <K> <S> <label> <out-dir>
# Writes <out>/<label>/{big.tsv (attempt, code, ms), small.tsv, summary}
set -euo pipefail
TONK="${TONK:?}"; POLICY="$1"; K="$2"; S="$3"; LABEL="$4"; OUT="$5"
DIR="$OUT/$LABEL"; rm -rf "$DIR"; mkdir -p "$DIR"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home" DO_NOT_TRACK=1 TONK_NO_UPDATE_CHECK=1 TONK_NO_SYNC=1 \
  TONK_SPACES_STATE="$WORK/state" TONK_TELEMETRY_STATE="$WORK/state" \
  TONK_UPDATE_STATE="$WORK/state" TONK_SPACE=bench
mkdir -p "$HOME" "$TONK_SPACES_STATE"
"$TONK" space new bench --site "$WORK/site" > /dev/null 2>&1
"$TONK" concept add note --description "A note" --field title:text:one > /dev/null 2>&1

BIG="$WORK/big.tonk"; : > "$BIG"
for i in $(seq 1 "$K"); do printf 'note!: &b%s\n  title: "big %s"\n' "$i" "$i" >> "$BIG"; done

run() { # run <file-or--c> <doc>
  if [ "$POLICY" = queue ]; then flock "$WORK/queue.lock" "$TONK" eval "$@"
  else "$TONK" eval "$@"; fi
}

DONE="$WORK/done"
for s in $(seq 1 "$S"); do
  (
    n=0
    while [ ! -e "$DONE" ]; do
      n=$((n + 1)); t=$(date +%s%N)
      set +e; run -c "note!: &s${s}x$n
  title: \"small $s $n\"" > /dev/null 2>&1; code=$?; set -e
      printf '%s\t%s\t%s\t%s\n' "$s" "$n" "$code" $(( ($(date +%s%N) - t) / 1000000 )) >> "$DIR/small-$s.tsv"
    done
  ) &
done
sleep 0.5
start=$(date +%s%N)
for a in $(seq 1 64); do
  t=$(date +%s%N)
  set +e; run "$BIG" > /dev/null 2> "$DIR/big-$a.err"; code=$?; set -e
  printf '%s\t%s\t%s\n' "$a" "$code" $(( ($(date +%s%N) - t) / 1000000 )) >> "$DIR/big.tsv"
  [ "$code" = 0 ] && break
done
total=$(( ($(date +%s%N) - start) / 1000000 ))
touch "$DONE"; wait
cat "$DIR"/small-*.tsv > "$DIR/small.tsv" 2>/dev/null || : > "$DIR/small.tsv"; rm -f "$DIR"/small-*.tsv
attempts=$(wc -l < "$DIR/big.tsv"); last=$(tail -1 "$DIR/big.tsv" | cut -f2)
small_ok=$(awk '$3==0' "$DIR/small.tsv" | wc -l); small_fail=$(awk '$3!=0' "$DIR/small.tsv" | wc -l)
printf 'policy=%s K=%s S=%s big_attempts=%s big_final_code=%s big_total_ms=%s small_ok=%s small_failed=%s\n' \
  "$POLICY" "$K" "$S" "$attempts" "$last" "$total" "$small_ok" "$small_fail" | tee "$DIR/summary"

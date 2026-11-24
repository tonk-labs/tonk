#!/usr/bin/env bash
# N concurrent `tonk eval` processes on one replica, no upstream.
# Each asserts one distinct note; afterwards the final head is read to see
# which reported successes landed.
#
# Usage: TONK=path/to/tonk run.sh <N> <run-label> <out-dir>
# Writes <out-dir>/<label>-n<N>/{w<i>.{out,err,code,ms}, final.json, summary.tsv}
set -euo pipefail

TONK="${TONK:?TONK must be set to the tonk binary}"
N="$1"; LABEL="$2"; OUT="$3"
DIR="$OUT/$LABEL-n$N"
rm -rf "$DIR"; mkdir -p "$DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

export HOME="$WORK/home" DO_NOT_TRACK=1 TONK_NO_UPDATE_CHECK=1 TONK_NO_SYNC=1 \
  TONK_SPACES_STATE="$WORK/state" TONK_TELEMETRY_STATE="$WORK/state" \
  TONK_UPDATE_STATE="$WORK/state" TONK_SPACE=bench
mkdir -p "$HOME" "$TONK_SPACES_STATE"

"$TONK" space new bench --site "$WORK/site" > "$DIR/setup.txt" 2>&1
"$TONK" concept add note --description "A note" --field title:text:one >> "$DIR/setup.txt" 2>&1

# Start every writer at once: each waits on a fifo that is opened only
# after all have been forked.
GO="$WORK/go"; mkfifo "$GO"
for i in $(seq 1 "$N"); do
  (
    read -r _ < "$GO" || true
    start=$(date +%s%N)
    set +e
    "$TONK" eval -c "note!: &n$i
  title: \"writer $i\"" > "$DIR/w$i.out" 2> "$DIR/w$i.err"
    echo $? > "$DIR/w$i.code"
    echo $(( ($(date +%s%N) - start) / 1000000 )) > "$DIR/w$i.ms"
  ) &
done
sleep 0.2
# One line per reader releases them all together.
for i in $(seq 1 "$N"); do echo go; done > "$GO"
wait

"$TONK" eval -c 'note:
  this: ?n
  title: ?title' > "$DIR/final.json" 2>&1 || true

for i in $(seq 1 "$N"); do
  code=$(cat "$DIR/w$i.code")
  if grep -q "\"writer $i\"" "$DIR/final.json"; then landed=yes; else landed=no; fi
  kind=ok
  if [ "$code" != 0 ]; then
    if grep -qi "version mismatch" "$DIR/w$i.err"; then kind=version-mismatch
    elif grep -qi "lock" "$DIR/w$i.err"; then kind=lock
    else kind=other; fi
  fi
  printf '%s\t%s\t%s\t%s\t%s\n' "$i" "$code" "$kind" "$landed" "$(cat "$DIR/w$i.ms")"
done > "$DIR/summary.tsv"

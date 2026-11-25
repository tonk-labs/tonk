#!/usr/bin/env bash
# Option G: every writer on a branch of its own (TONK_BRANCH), a reconciler
# (TONK_RECONCILE) merging them into main while they write. Needs the
# experiment build (harness/eval-policies.patch).
#
#   branches.sh burst <N> <label> <out>   N writers at once, one note each
#   branches.sh starve <K> <S> <label> <out>
#                                         one eval of K notes against S
#                                         writers committing in a loop
# Each writes <out>/<label>/summary plus per-writer rows; afterwards main is
# read and every write that reported success must be in it.
set -euo pipefail
TONK="${TONK:?}"; MODE="$1"; shift
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home" DO_NOT_TRACK=1 TONK_NO_UPDATE_CHECK=1 TONK_NO_SYNC=1 \
  TONK_SPACES_STATE="$WORK/state" TONK_TELEMETRY_STATE="$WORK/state" \
  TONK_UPDATE_STATE="$WORK/state" TONK_SPACE=bench
mkdir -p "$HOME" "$TONK_SPACES_STATE"
"$TONK" space new bench --site "$WORK/site" > /dev/null 2>&1
"$TONK" concept add note --description "A note" --field title:text:one > /dev/null 2>&1

ms() { echo $(( ($(date +%s%N) - $1) / 1000000 )); }
reconcile() { # names...
  TONK_RECONCILE="$(IFS=,; echo "$*")" TONK_RECONCILE_STOP="$WORK/stop" \
    "$TONK" eval -c x 2> "$DIR/reconcile.log" > /dev/null &
  RECONCILER=$!
}
finish() {
  local t; t=$(date +%s%N); touch "$WORK/stop"; wait "$RECONCILER" || true
  echo "drain_ms=$(ms "$t")" >> "$DIR/summary"
  "$TONK" eval -c 'note:
  this: ?n
  title: ?title' > "$DIR/main.txt" 2>&1 || true
}

if [ "$MODE" = burst ]; then
  N="$1"; DIR="$3/$2"; rm -rf "$DIR"; mkdir -p "$DIR"; : > "$DIR/summary"
  names=(); for i in $(seq 1 "$N"); do names+=("w$i"); done
  reconcile "${names[@]}"
  GO="$WORK/go"; mkfifo "$GO"
  for i in $(seq 1 "$N"); do
    ( read -r _ < "$GO" || true; t=$(date +%s%N); set +e
      TONK_BRANCH="w$i" "$TONK" eval -c "note!: &n$i
  title: \"writer $i\"" > /dev/null 2> "$DIR/w$i.err"; c=$?
      printf '%s\t%s\t%s\n' "$i" "$c" "$(ms "$t")" > "$DIR/w$i.tsv" ) &
    WRITERS+=($!)
  done
  sleep 0.2; for i in $(seq 1 "$N"); do echo go; done > "$GO"
  wait "${WRITERS[@]}"
  finish
  cat "$DIR"/w*.tsv > "$DIR/writers.tsv"
  missing=0; while IFS=$'\t' read -r i c t; do
    if [ "$c" = 0 ] && ! grep -q "\"writer $i\"" "$DIR/main.txt"; then missing=$((missing+1)); fi
  done < "$DIR/writers.tsv"
  printf 'failed=%s missing_from_main=%s\n' "$(awk '$2!=0' "$DIR/writers.tsv" | wc -l)" "$missing" >> "$DIR/summary"
else
  K="$1"; S="$2"; DIR="$4/$3"; rm -rf "$DIR"; mkdir -p "$DIR"; : > "$DIR/summary"
  BIG="$WORK/big.tonk"; : > "$BIG"
  for i in $(seq 1 "$K"); do printf 'note!: &b%s\n  title: "big %s"\n' "$i" "$i" >> "$BIG"; done
  names=(big); for s in $(seq 1 "$S"); do names+=("s$s"); done
  reconcile "${names[@]}"
  DONE="$WORK/done"
  for s in $(seq 1 "$S"); do
    ( n=0; while [ ! -e "$DONE" ]; do n=$((n+1)); t=$(date +%s%N); set +e
        TONK_BRANCH="s$s" "$TONK" eval -c "note!: &s${s}x$n
  title: \"small $s $n\"" > /dev/null 2>&1; c=$?; set -e
        printf '%s\t%s\t%s\t%s\n' "$s" "$n" "$c" "$(ms "$t")" >> "$DIR/small-$s.tsv"; done ) &
    WRITERS+=($!)
  done
  sleep 0.5; t=$(date +%s%N); set +e
  TONK_BRANCH=big "$TONK" eval "$BIG" > /dev/null 2> "$DIR/big.err"; c=$?; set -e
  echo "big_code=$c big_ms=$(ms "$t")" >> "$DIR/summary"
  touch "$DONE"; wait "${WRITERS[@]}"
  finish
  cat "$DIR"/small-*.tsv > "$DIR/small.tsv"
  big_in_main=$(grep -c '"big ' "$DIR/main.txt" || true)
  small_ok=$(awk '$3==0' "$DIR/small.tsv" | wc -l)
  small_in_main=$(grep -c '"small ' "$DIR/main.txt" || true)
  printf 'small_ok=%s small_failed=%s small_in_main=%s big_in_main=%s of %s\n' \
    "$small_ok" "$(awk '$3!=0' "$DIR/small.tsv" | wc -l)" "$small_in_main" "$big_in_main" "$K" >> "$DIR/summary"
fi
cat "$DIR/summary"

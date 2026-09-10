#!/usr/bin/env bash
# Seed the perf space with enough content that a cold join has real
# blocks to replicate: a note concept, PERF_NOTES instances (default
# 200), a view, and the space home pointed at it.
#
# Env: TONK (binary), PERF_NOTES
set -euo pipefail

TONK="${TONK:?TONK must be set to the tonk binary path}"
NOTES="${PERF_NOTES:-200}"

# The authoring verb, not raw eval: it also writes the Name claim the
# name-addressed directory route (/space/<did>/note) resolves by.
"$TONK" concept add note --description "A perf note" \
  --field title:text:one --field body:text:one

# Assert the instances in batches of 50 so a single eval body stays
# reasonable while the seed still lands in a handful of transactions.
batch_file="$(mktemp)"
trap 'rm -f "$batch_file"' EXIT
i=1
while [ "$i" -le "$NOTES" ]; do
  : > "$batch_file"
  end=$((i + 49))
  [ "$end" -gt "$NOTES" ] && end="$NOTES"
  while [ "$i" -le "$end" ]; do
    cat >> "$batch_file" <<EOF
note!: &note-$i
  title: "Note $i"
  body: "Body of perf note $i. Enough text to make the block payloads non-trivial rather than a couple of bytes each."
EOF
    i=$((i + 1))
  done
  "$TONK" eval "$batch_file"
done

# A directory view over the notes, auto-surfaced as the space home when
# the home is blank — so the joiner's first render issues the narrow
# query that walks the note tree.
"$TONK" view add note --template '<article><h3>{title}</h3><p>{body}</p></article>' \
  || echo "seed: view add failed (continuing; home stays default)" >&2

echo "seed: $NOTES notes asserted" >&2

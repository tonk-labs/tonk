#!/usr/bin/env bash
# Plumbing check: canned book-club build (no interview). Set
# BENCH_SCRIPTED_INTERVIEW=1 to also exercise one ask-user round trip
# (spends one small claude call).
set -euo pipefail
TONK="${TONK:?}"

if [ -n "${BENCH_SCRIPTED_INTERVIEW:-}" ]; then
  RUN_DIR="${RUN_DIR:?}"
  # Scripted runs execute in run.sh's own shell (unlike the episode,
  # whose PATH is assembled by episode.sh), so $RUN_DIR/bin is never
  # added to PATH here — call the installed bridge by absolute path.
  "$RUN_DIR/bin/ask-user" "Quick check: do you want to track who attends each meeting?"
  cat "$RUN_DIR/interview.log"
fi

"$TONK" eval -c '
attribute!: &meeting-date
  description: "Meeting date (YYYY-MM-DD)"
  the: bench.meeting/date
  as: text
  cardinality: one

attribute!: &meeting-book
  description: "Book discussed"
  the: bench.meeting/book
  as: text
  cardinality: one

concept!: &meeting
  description: "One book club meeting"
  with:
    date: meeting-date
    book: meeting-book

concept!: &view
  this: tonk:view
  description: "A display template for rendering an entity"
  with:
    model:
      description: "Concept this view renders"
      the: xyz.tonk.view/model
      cardinality: one
      as: entity
    display:
      description: "HTML template for the view"
      the: xyz.tonk.view/display
      cardinality: one
      as: text

meeting!:
  date: "2026-07-01"
  book: "The Overstory"

view!: &meetings
  model: meeting
  display: |
    <div class="meeting">{date} — {book}</div>
'
"$TONK" status

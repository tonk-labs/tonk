#!/usr/bin/env bash
# Known-good tonk sequence for the smoke scenario.
# Runs inside $RUN_DIR/site with TONK set to the release binary.
# Declares the note concept (title, body), asserts two instances,
# and declares a tonk:view-compatible view named `notes`.
set -euo pipefail

TONK="${TONK:?TONK must be set to the tonk binary path}"

# Step 1: declare schema and assert instances in one eval.
"$TONK" eval -c '
attribute!: &note-title
  description: "The note title"
  the: bench.note/title
  as: text
  cardinality: one

attribute!: &note-body
  description: "The note body"
  the: bench.note/body
  as: text
  cardinality: one

concept!: &note
  description: "A note"
  with:
    title: note-title
    body: note-body

note!: &hello
  title: "Hello"
  body: "First note from the bench harness."

note!: &world
  title: "World"
  body: "Second note."
'

# Step 2: declare the tonk:view concept and assert a notes view.
# The view concept must be pinned to tonk:view so tonk-display can
# resolve it. The display template uses {title} and {body} field
# interpolations from the note concept shape.
"$TONK" eval -c '
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

view!: &notes
  model: note
  display: !text/html |
    <article>
      <h2>{title}</h2>
      <p>{body}</p>
    </article>
'

"$TONK" status

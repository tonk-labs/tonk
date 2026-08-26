# Notebook — an Observable-style notebook over dialog queries

Status: **design, for review**. Nothing implemented yet.

A notebook is a persisted, ordered list of cells. Each cell holds a
dialog-notation source; a code cell's result renders directly beneath
it. The notebook lives in the DB, so it reloads.

## What already exists

Most of the machine is built. The design's job is mostly to *not*
rebuild it.

- **`<tonk-code>`** — the cell editor. Dispatches `change` (per
  keystroke), `run` (Shift+Enter), `diagnostics` (on a fresh LSP
  frame, carrying `errorCount`), and `tonk-code-connect` /
  `tonk-code-disconnect` for LSP wiring. `.value` round-trips the
  buffer.
- **`<tonk-diagnostics-provider>`** — owns the LSP client, keyed by
  each editor's `source` URI. The sealed guest has no app-wide one, so
  a notebook mounts its own (the inspector already does this).
- **`tonk-inspector`** — a working notebook-of-cells: per-cell editor +
  result slot, auto-evaluate on a clean diagnostics frame, explicit
  commit on submit, and `render.rs`, an engine-free `EvaluateResponse`
  → HTML renderer. Reusable nearly whole.
- **`<tonk-sheet-binder>`** (`tonk-workspace`) — the ordered-container
  pattern: sort children in the component, apply CSS `order`.
- **`prose.yaml`** — the model template shape: attribute → concept →
  command → rule → view → seed, with the editor's `change` feeding a
  command that a rule persists, and a versioned envelope so the
  element drops its own echo.

### The one that changes the design

**`<tonk-prose>` already embeds `<tonk-code>`.** Every fenced code
block in a prose document mounts a real `<tonk-code>` as a ProseMirror
node view, with prefix/suffix-diff sync in both directions, boundary
escapes, and `stopEvent`/`ignoreMutation` guards
(`tonk-prose/src-js/editor/code-block.ts:304`). The sealed guest
injects prose *after* code precisely so this upgrade works
(`tonk-portal/src/bridge.rs:646`).

So the prose/code pairing is not something to build. It is something
to decide whether to use.

## Prior art: `~/Projects/replicator`

Irakli's earlier Observable-style notebook (JS cells, IPFS-distributed,
CodeMirror only — **no ProseMirror**, so the prose/code pairing here is
genuinely new ground). Its contribution is the cell and execution model,
and it answers three of the open questions outright.

**Cells are a projection of one document, not stored entities.**
`Data.init` calls `Cell.tokenize(input)` to split a single source file
into cells, splitting at labeled expressions
(`CELL_PATTERN = /(^[A-Za-z_]\w*\s*\:.*$)/gm`). Serialization is the
inverse: `textInput` joins each cell's `input` with `\n\n`
(`Notebook/Data.js:239`). The notebook round-trips as plain source, and
a cell id is just `${url}#${n}`.

**Outputs are never persisted.** A cell is `{id, input, output}`, and
only `input` survives serialization. Output is in-memory, recomputed on
load.

**Execution is a sequential cascade, not a dependency graph.** On load,
execute the first cell (`Notebook.js:59`); when a cell finishes,
`onCellChanged` executes `idByOffset(1, id, state)` — the *next* one
(`:68-71`). No graph, no topological sort.

**Cells share state through the global object.** Each cell compiles to
an ES module exporting `[value, bindings]`; the bindings are installed
with `Object.defineProperties(window, bindings)`
(`Cell/Effect.js:60`). A downstream cell simply reads what an upstream
one defined. This is what makes the naive cascade sufficient: order in
the document *is* the dependency order.

The lesson worth carrying: a notebook does not need a dataflow graph to
feel like a notebook. Document order plus a re-run cascade gets most of
the way, and it is dramatically less machinery.

## Two shapes

### A. Cells as entities (the wiki/board shape)

The notebook is a list of cell entities. Each cell is `kind: prose |
code`, owns its source, and renders its own editor. The notebook view
iterates cells; a binder orders them.

- Cells are first-class: addressable, individually re-runnable,
  reorderable, and each can carry its own output and metadata.
- Matches every existing ordered collection in the library (wiki
  blocks, board cards, sheets).
- The prose/code pairing is *sibling* cells, not nested. Typing a code
  fence inside a prose cell would give you a second, nested code
  editor with different semantics — a real confusion to design around.

### B. One prose document, code fences as cells

The notebook is a single markdown document. Every ```dialog fence is a
cell, already an embedded `<tonk-code>`.

- The seamless interaction is free and already built. Writing prose
  around code is exactly the Observable feel, with no cell-boundary
  friction.
- The persisted artifact is one markdown string — diffable, portable,
  readable outside tonk.
- But a fence is not an entity. It has no stable id, no place for an
  output, and no per-cell facts. `languageOf` reads only the *first*
  word of the info string and discards the rest, so ```dialog id=abc
  parses as language `dialog` with `id=abc` preserved in `node.attrs.params`
  but unused — an identity channel exists, though nothing reads it today.
- Ordering is the document's own order. No position machinery at all.
- Outputs would have to live outside the document, keyed by cell id,
  or be recomputed and never stored.

**Recommendation: B, revised in light of replicator.**

My first instinct was A (cells as entities), on the reasoning that
entities buy identity, ordering, and a place for outputs. Replicator
shows that reasoning was weaker than it looked:

- Outputs don't need a home — they aren't persisted at all.
- Ordering doesn't need a key — document order *is* the order, and it
  is also the dependency order.
- Identity doesn't need an entity — `${url}#${n}` was enough, and here
  a fence's `params` can carry a stable id if one is wanted.

That removes most of A's advantages, and B's remaining advantage is
large: the prose/code pairing is already built and needs no
cell-boundary design at all. A notebook becomes one `<tonk-prose>`
document where every ```dialog fence is already a live `<tonk-code>`.

A is still the better answer if cells must be *separately* addressable
from outside the notebook — queried, linked to, or reordered by
something other than the author editing the document. That is worth
deciding explicitly rather than by default (see Open questions).

What tips it: shape B makes this change small. One concept, one
attribute, and a component that finds fences and mounts results. Shape
A is a new ordered collection with commands, rules, projections, and a
binder — most of a wiki.

## Ordering

**Under shape B this section is moot** — document order is the order,
and there is nothing to sort. It is kept because it applies the moment
shape A is chosen, and because the `dialog/position` findings are worth
recording regardless.

Three options, in descending order of how much they'd cost.

**The renderer will not order cells for us.** `select_rows`
(`tonk-template/src/fold.rs:36`) preserves query order, but the DOM
reconciler keys rows in a `BTreeMap` on entity DID
(`tonk-display/src/render.rs:423`), so rows always mount in DID order.
The workspace binder says so outright: *"The view's `{sheet}`
iteration is CID-keyed (not author-controllable), so the binder is
what makes the order deterministic."* Whatever we choose, a binder
sorts and applies CSS `order`.

### 1. `as: text` order key (what the library does today)

Every ordered thing in the library — wiki blocks, board cards, sheets,
tiles — uses a lexicographic text field plus a JS `between(a, b)`
midpoint helper, duplicated in `board.yaml:481` and `wiki.yaml:685`.

Works today, zero new machinery. Two known weaknesses: the helper has
**no bias**, so two replicas inserting different cells at the same slot
derive the *same* key and collide (papered over by an id tiebreak), and
repeated middle inserts grow keys linearly.

### 2. Same, with a biased `between`

Swap the shared helper for a biased variant (jitter drawn from the
member's reference). Buys convergence for a small, contained change.
Does not buy single-scan retrieval or log growth.

### 3. `dialog/position` — the real thing

Dialog has a landed fractional-index primitive:
`dialog_artifacts::position` with `Position`, `Bias::derive`, and
`insert(bias, range)` where the range syntax *is* the insertion
(`insert(&bias, first..second)`). It is deterministic, so two replicas
inserting the same entity between the same neighbors converge. Two
formulas are registered in dialog: `dialog/position` and
`dialog/position-parts`.

The design note (`dialog notes/ordered-relations.md`) cites Irakli's
own Observable notebook as its basis.

Two things block using it from tonk notation:

1. **The analyzer doesn't know the formula.** `build_registry()`
   (`tonk-analyzer/src/analyzer/formula.rs:103`) lists 17 formulas and
   omits `dialog/position`. The module doc says both tables must match,
   so this is drift, and the fix is small and self-contained.
2. **The encoding has no schema surface.** A position lives in the
   attribute *predicate* (`[list  todo.item/<position>  member]`), not
   in a field, so retrieval is one prefix scan. tonk-schema has no
   `Directory`/`Sequence` field type and no attribute-prefix selector;
   dialog deferred concept-field aggregation as "the realize-layer
   follow-up". A concept `with:` cannot express this today.

**Recommendation: start with 1, treat 3 as the follow-up.** A notebook
is a coarse, single-author list where collision risk is low and cell
counts are small. Shipping on the pattern every other collection
already uses keeps this change about notebooks. Adopting positions is
a real project (analyzer registry + a schema surface for ordered
relations) that pays off across wiki, board, and sheets at once — it
deserves its own branch, not a rider on this one.

## Sketch: `notebook.yaml` (shape B)

A notebook is a prose document that happens to contain ```dialog
fences. The model is `prose.yaml` with a different view — which is a
sign the shape is right.

```yaml
attribute!: &notebook/content
  description: |
    The notebook's content as the <tonk-prose> `content` envelope
    (versioned markdown). ```dialog fences are the cells.
  the: xyz.tonk.notebook/content
  as: text

attribute!: &notebook/title
  the: xyz.tonk.notebook/title
  as: text

concept!: &notebook
  this: tonk:notebook
  description: A notebook — prose with live dialog query cells.
  with:
    title:
      the: xyz.tonk.notebook/title
      cardinality: one
      as: text
    content:
      the: xyz.tonk.notebook/content
      cardinality: one
      as: text

# The edit command + rule are `prose/edit` verbatim, retargeted at
# `xyz.tonk.notebook/content`.

view!:
  this: id:notebook/view
  model: notebook
  display: |
    <tonk-notebook
      onchange=notebook/edit
      data-subject={this}
      with={dom.host/with}
    >{content}</tonk-notebook>
```

`<tonk-notebook>` is the one new element: it wraps `<tonk-prose>`,
mounts a `<tonk-diagnostics-provider>` (the sealed guest has none), and
for each ```dialog fence attaches the inspector's evaluate + result
rendering beneath the embedded `<tonk-code>`. It is the inspector's
`NotebookCell` logic re-hosted onto fences the prose editor already
creates.

### If shape A is chosen instead

Following `prose.yaml`'s shape, cells as entities. Illustrative.

```yaml
attribute!: &notebook/title
  the: xyz.tonk.notebook/title
  as: text

# A cell's back-reference to its notebook (the wiki/block shape).
attribute!: &cell/notebook
  the: xyz.tonk.notebook.cell/notebook
  as: entity

attribute!: &cell/order
  description: Lexicographic key ordering the cell in its notebook
  the: xyz.tonk.notebook.cell/order
  as: text

attribute!: &cell/kind
  description: Cell kind — "prose" or "code"
  the: xyz.tonk.notebook.cell/kind
  as: text

attribute!: &cell/source
  description: |
    The cell's source. For a code cell, dialog notation. For a prose
    cell, the <tonk-prose> versioned content envelope.
  the: xyz.tonk.notebook.cell/source
  as: text

concept!: &notebook
  this: tonk:notebook
  with:
    title: {the: xyz.tonk.notebook/title, cardinality: one, as: text}

concept!: &notebook/cell
  this: tonk:notebook/cell
  with:
    notebook: {the: xyz.tonk.notebook.cell/notebook, cardinality: one, as: entity}
    order:    {the: xyz.tonk.notebook.cell/order,    cardinality: one, as: text}
    kind:     {the: xyz.tonk.notebook.cell/kind,     cardinality: one, as: text}
    source:   {the: xyz.tonk.notebook.cell/source,   cardinality: one, as: text}

# Single-field projections, so an edit upserts one fact rather than
# re-binding every field (the wiki/board pattern).
concept!: &cell/source-only
  with:
    source: {the: xyz.tonk.notebook.cell/source, cardinality: one, as: text}

concept!: &cell/order-only
  with:
    order: {the: xyz.tonk.notebook.cell/order, cardinality: one, as: text}
```

Commands and rules mirror `prose/edit`: a command reads the new source
off the editor's `change` event plus the cell's identity from
`data-subject`, and a rule writes it back onto the cell.

## Output

**Derived, not stored** — and replicator agrees: its cell is
`{id, input, output}`, but only `input` is serialized. Output is
recomputed on load.

A code cell auto-evaluates on a clean diagnostics frame (the inspector
already does exactly this) and renders through the existing
`render_result`. Nothing about the result is persisted.

Storing outputs would make a notebook readable without running it, and
shareable as a finished document. The cost is results that silently go
stale against a branch that has moved, plus a second write path per
cell. If notebooks later need to read as published documents, that is a
good reason to revisit — as an explicit "snapshot", not a default.

## Open questions

**1. Shape A or B — do cells need to be addressable from outside?**
The one question that changes everything downstream. If something other
than the author editing the document needs to query, link to, or
reorder an individual cell, cells must be entities (A). If not, B is
much less machinery. Replicator never needed it.

**2. Does a code cell commit, or dry-run?** The inspector separates
auto-evaluate (dry run) from explicit submit (`transact: true`). A cell
that re-runs on load must not re-commit mutations on every load.
Suggest: dry-run on load and on edit, commit only on explicit run. This
decides whether a notebook is a *document* or a *script*, and it is
sharper here than in replicator, whose JS cells had no transaction
boundary to worry about.

**3. Re-run cascade: is document order enough?** Replicator's answer is
yes — execute the first cell on load, and each cell's completion
triggers the next. That works because JS cells share state through
`window` bindings, so document order *is* dependency order.

Dialog cells have no equivalent shared binding: each `/evaluate` is
independent, against the branch. So the cascade buys less. Two live
options: re-run everything below an edited cell (cheap, predictable,
possibly wasteful), or re-run only the edited cell (cheapest, but a
mutation in cell 3 leaves cell 5's stale result on screen). Worth
deciding once the commit question above is settled — they interact.

**4. Per-cell LSP `source` URIs.** The inspector uses
`tonk-buffer:///{repo}/{branch}/scratch-{n}` with a session counter.
Fences want something stable across reloads: either an index
(`.../cell-{n}`, which shifts when a fence is inserted above) or an id
carried in the fence `params`. The LSP treats the suffix as opaque
(`parse_repo_branch` splits three ways and ignores the tail), so either
works.

**5. Prose debounces at 400 ms; code fires per keystroke.** The two
editors have different change cadences (`CHANGE_DEBOUNCE_MS = 400` vs.
CodeMirror's `docChanged`). Inside a prose document the fence edits
arrive through prose's debounced `change`, which is probably what we
want for persistence — but auto-evaluate is currently driven by the
code editor's own `diagnostics` event. Worth confirming those two
paths don't fight.

# `<tonk-layout>` — a magazine-style tiling layout web component

## Context

We have two live web components today: `<tonk-concept>` (renders *many*
matched entities into an author-supplied template) and `<tonk-display>`
(renders *one* entity using a `view` template stored on the branch).
Both are vanilla custom elements written in Rust via the
`custom-elements` crate, compiled to wasm by Trunk, and registered in
`rust/tonk-ui/src/bin/ui.rs`.

This plan adds a third element: `<tonk-layout>`, a tiling layout
modelled on a **magazine of justified rows**. It arranges tiles into a
vertically page-scrolling stack of rows. Each row is a horizontal run
of columns justified to fill the available width; each column is a
vertical stack of one or more tiles. Every tile mounts a
`<tonk-display>` pointed at a branch entity. The layout itself (rows,
columns, tiles, width presets, focus) is persisted to the dialog
database as **normalized entities**, so a reload (or another device)
reconstructs the exact board.

It ships as a new `tonk-layout` crate.

## Target usage

```html
<tonk-layout space="home" branch="main" board="default"></tonk-layout>
```

No children. The element subscribes to its layout entities on the
branch, builds the row stack, and mounts a `<tonk-display>` inside each
tile. An empty board renders an empty page with an "add tile"
affordance.

## Layout model

```
   page (scrolls vertically) — a board
   ┌───────────────────────────────────────────────┐
   │  row 0                                         │
   │ ┌─────────┐  ┌──────────┐  ┌────────────────┐  │
   │ │ tile A  │  │  tile C  │  │    tile E      │  │
   │ ├─────────┤  └──────────┘  ├────────────────┤  │
   │ │ tile B  │                │    tile F      │  │
   │ └─────────┘                └────────────────┘  │
   │  row 1                                         │
   │ ┌────────────────────┐  ┌─────────────────┐    │
   │ │ tile G             │  │  tile H         │    │
   │ └────────────────────┘  └─────────────────┘    │
   │ …                                              │  ↓ page scrolls
   └───────────────────────────────────────────────┘
```

- **Board** — a named, vertically page-scrolling stack of rows. There
  is exactly ONE scroller: the page itself. No element inside the
  board has its own scrollbar (no scroller-within-a-scroller). Tiles
  are content-height, rows grow to their tallest content, the board
  grows down, the page scrolls. One branch can hold several boards
  (`default`, `scratch`, …); the `board` attribute selects one.
  (Earlier drafts called this a "workspace" — renamed because a board
  is a content stack, not a fixed desktop.)
- **Row** — a horizontal run of **columns**, justified to fill the
  available width. A row is a real entity (see "Data model"). Rows do
  not share a height: each row is as tall as its tallest column, the
  magazine layout.
- **Column** — an ordered vertical stack of one or more **tiles**,
  plus a **width preset** (XS, S, M, L, XL). All tiles in a column
  share the column width.
- **Tile** — one cell. Its width is its column's, and its **height is
  content-driven**, as tall as its `<tonk-display>` needs. It carries
  a content descriptor (`entity` / `view` / `model`). There is no
  stored tile height.
- **Focus** — exactly one tile is focused. Focus is a board-level
  pointer used by keyboard navigation and interaction.

### Width presets and the base column

The unit of width is a fixed, device-independent **base column** of
roughly 380px (a comfortable reading-column width, about a phone's
portrait width). The stylesheet exposes it as a CSS custom property,
`--tonk-layout-base`. It does not change with viewport size and there
is no zoom control.

Presets are integer multiples of the base:

- **XS** = 1 base
- **S** = 2 base
- **M** = 3 base
- **L** = 4 base
- **XL** = fill the current row

The base column is a constant. "How many tiles fit across" is just
the viewport width divided by the base:

- On a phone (~390px viewport) one base fits, so you see one tile
  across, never more, whatever a tile's preset says.
- On a laptop ~3 to 4 bases fit.
- On a big display ~7 bases fit. `XL` means "fill the current row" so
  a tile never sprawls past the row even on the widest screen.

This is responsive WITHOUT any zoom control and WITHOUT
viewport-fraction sizing. The base column is the constant; the layout
adapts because "how many fit" is a division, and because a row that
runs out of width wraps to the next row.

### Justified rows — no gaps

A row fills the available width with no horizontal gap: the columns in
a row FLEX so the row's right edge is flush with the board's. A
column's preset is its **target** width; the actual rendered width is
adjusted proportionally per row so the row is justified. This is the
Google+/Vjeux justified-row (bento) behavior: targets set the
proportions, the row stretches or compresses them to fit.

When a column would not fit on the current row at (something near) its
target width, it starts a new row. Rows do NOT share a height. Each
row is as tall as its tallest column, so the board reads as a magazine
page of uneven rows rather than a uniform grid.

The page (the board) is the only scroller. As rows accumulate the
board grows taller and the page scrolls vertically. Nothing inside a
row or column scrolls independently.

## Data model — normalized layout entities

Four concepts on the branch describe a board's layout: `board`, `row`,
`column`, `tile`. They are declared once per repository
(asserted-notation), exactly like the `view` concept that
`<tonk-display>` depends on.

This is a four-level tree: **board → row → column → tile**. The `row`
level is new. Earlier drafts of this element had only board → column →
tile; a row is now a real entity with its own identity and `order`, so
that wrapping is explicit and persisted rather than recomputed from
column widths on every render.

```yaml
concept!: &board
  description: A named, vertically scrolling stack of rows
  with:
    name:
      description: Board name (selects which board to render)
      the: xyz.tonk.layout/board-name
      as: text
    focus:
      description: Currently focused tile
      the: xyz.tonk.layout/board-focus
      as: entity
      cardinality: one

concept!: &row
  description: A horizontal run of columns within a board
  with:
    board:
      the: xyz.tonk.layout/row-board
      as: entity
    order:
      description: Position of the row in the board (float, sortable)
      the: xyz.tonk.layout/row-order
      as: float

concept!: &column
  description: A vertical stack of tiles within a row
  with:
    board:
      description: Board the column belongs to (denormalized from the
        parent row, so all columns for a board come back in one query)
      the: xyz.tonk.layout/column-board
      as: entity
    row:
      the: xyz.tonk.layout/column-row
      as: entity
    order:
      description: Position of the column within its row (float, sortable)
      the: xyz.tonk.layout/column-order
      as: float
    width:
      description: Width preset — XS / S / M / L / XL
      the: xyz.tonk.layout/column-width
      as: text

concept!: &tile
  description: One cell; mounts a <tonk-display>
  with:
    board:
      description: Board the tile belongs to (denormalized from the
        parent column, so all tiles for a board come back in one
        query)
      the: xyz.tonk.layout/tile-board
      as: entity
    column:
      the: xyz.tonk.layout/tile-column
      as: entity
    order:
      description: Vertical position within the column (float, sortable)
      the: xyz.tonk.layout/tile-order
      as: float
    entity:
      description: Entity the tile's <tonk-display> renders
      the: xyz.tonk.layout/tile-entity
      as: entity
    view:
      the: xyz.tonk.layout/tile-view
      as: text
    model:
      the: xyz.tonk.layout/tile-model
      as: text
```

There is **no `tile.height`** — a tile's height is content-driven, as
tall as its `<tonk-display>` renders. Only column `width` is stored,
as one of the five presets. The vertical axis flows.

The width preset is stored as text (`XS`/`S`/`M`/`L`/`XL`). A small
integer would work too; text is chosen so a stored board is readable
without a decoder ring. The base-column pixel size never appears in
the data: it is a stylesheet constant, not stored state.

Notes on the model:

- **`order` is a float, not an integer.** Inserting a row between two
  others sets `order` to the midpoint of its neighbours: no
  renumbering, no write amplification. Same for column `order` within
  a row and tile `order` within a column. (Fractional indexing.)
- **References point upward** (`tile.column`, `column.row`,
  `row.board`) so a child can be inserted or removed with a single
  assertion and the parent never needs rewriting. `column.board` and
  `tile.board` are **denormalized** copies of the board reference:
  they let the columns and tiles subscriptions each filter by board in
  a single query, instead of one subscription per row or per column.
  A tile moved between columns of the same board leaves `tile.board`
  unchanged; only a move across boards rewrites it (alongside
  `tile.column`). The same holds for `column.board` when a column
  moves between rows of one board.
- **`board.focus`** is a cardinality-one pointer to a tile entity;
  re-asserting it retracts the previous value automatically (the
  git-ref pattern from the notation guide).
- A tile that loses its content (entity retracted) is still a valid
  empty tile until explicitly removed. A row or column with no
  children is likewise valid until removed.
- Entities get **stable identities** via explicit `this:` mappings so
  edits target the same entity instead of spawning content-addressed
  duplicates.

### Why normalized rather than a JSON blob

Per-attribute merge: two devices dragging different columns, or
changing different width presets, commit disjoint claims and merge
cleanly on sync. A single JSON-blob entity would re-hash on every edit
and lose one side's change. The cost is more query and assemble code,
accepted here.

## Reading layout state — subscriptions

`<tonk-layout>` reads **four** things from the content branch, all as
live SSE subscriptions against the worker's `/query` route (the same
machinery as `<tonk-concept>`: a `POST` with `Accept:
text/event-stream`, frames are `Vec<Conclusion>` JSON, cancellation
via `AbortController`).

1. **Board subscription** — the `board` row whose `name` equals the
   `board` attribute. Frame carries `focus`.
2. **Rows subscription** — all `row` rows whose `board` equals the
   resolved board entity.
3. **Columns subscription** — all `column` rows whose `board` equals
   the resolved board entity (the denormalized `column.board` field
   lets this be one query instead of one per row). Frame carries each
   column's `row`, `order`, and `width`.
4. **Tiles subscription** — all `tile` rows whose `board` equals the
   resolved board entity (the denormalized `tile.board` field lets
   this be one query instead of one per column).

A reusable **reconciler** (see "Rendering") folds the latest frame of
each subscription into an in-memory `Board` tree (board → row → column
→ tile), sorts rows, columns, and tiles by `order`, and patches the
DOM in place. It preserves each tile's `<tonk-display>` node identity
so a layout change never tears down and remounts a healthy tile.

Because every layout write goes through `/evaluate` (which re-polls
subscriptions on commit), the layout is **reactive across tabs and
devices** for free: move a column on one screen, it moves on the
other.

## Writing layout state — `/evaluate` + debounce

There is no write-debounce primitive in the codebase; we build one.

- **Discrete actions** (open tile, close tile, move row or column,
  cycle a column's width preset, focus change) write **immediately**:
  one `POST /evaluate` with an asserted-notation document. These are
  cheap and infrequent.
- **Continuous actions** (a pointer drag that reorders rows or
  columns) update the DOM optimistically on every pointer event but
  **debounce the write**: coalesce into a single `/evaluate` flushed
  ~200 ms after the input goes idle (or on `pointerup`). Implemented
  with `setTimeout` + cancellation, mirroring the `AbortController`
  pattern already in `tonk-concept`. The in-flight optimistic state is
  the source of truth for the DOM until the write lands; the
  subscription frame that follows is idempotent against it.
- **Batching:** a single user action that touches several entities
  (moving a tile between columns rewrites `tile.column` + `tile.order`;
  a tile drop that empties a column and removes it rewrites several
  rows) goes in **one** `/evaluate` document = one dialog transaction
  = atomic.

A small `writer` module owns: building the notation document for each
mutation, the debounce timer, and the optimistic-state bookkeeping.

## Element shape

```html
<tonk-layout
    [board="<name>"]
    [space="<space>"]
    [branch="<branch>"]>
</tonk-layout>
```

| Attribute | Required | Default | Meaning |
|---|---|---|---|
| `board` | no | `"default"` | Which named board to render. |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

All attributes are observed; changing any aborts whatever
subscriptions are open, clears the board, and restarts — the same
teardown/restart discipline as `<tonk-concept>` / `<tonk-display>`.

## Interaction (keyboard + pointer)

Keyboard (focus must be within the host; the element listens on its
own root):

| Key | Action |
|---|---|
| `←` / `→` | Move focus to previous / next column in the focused row. |
| `↑` / `↓` | Move focus up / down within the focused column. |
| `Ctrl+←/→` | Move the focused column left / right within its row. |
| `Ctrl+↑/↓` | Move the focused tile up / down within its column. |
| `R` | Cycle the focused column through width presets (XS → S → M → L → XL → XS). |
| `Q` | Close the focused tile. |
| `Enter` | Open a new tile (prompt for entity/view — see below). |

The `R` preset cycle walks the five presets XS / S / M / L / XL. The
stored `width` is the preset name itself; there is no pixel value to
round, the value *is* the preset.

There is no tile-height control: a tile's height is its content's
height, not a stored or resizable field.

Pointer:

- **Focus** — click a tile to focus it.
- **Width preset** — a small control on each column's chrome (a
  `<wa-button>` or segmented control) cycles or picks the preset.
  Tiles have no resize handle (height is content-driven).
- **Reorder** — drag a column within or across rows; drag a tile
  within or across columns. The DOM updates optimistically on every
  pointer move; the write is debounced and flushed on `pointerup`
  (see "Writing layout state").
- **Open a tile** — a ghost `+` affordance at the end of a row (and at
  the bottom of a column) opens the open-tile prompt with the new
  tile's row/column position pre-filled.

**Opening a tile** needs an entity to display. v1: a `<wa-dialog>`
(Web Awesome) with inputs for `entity`, `model`, `view`. A richer
picker (browse branch entities) is a follow-up. Authors can also seed
tiles by asserting `tile` rows directly.

Focus is purely a layout pointer here: the page is the scroller, so
focusing a tile that is off-screen uses `scrollIntoView` on the tile
node to bring it into the viewport.

## Tile content — `<tonk-display>` per tile

Each tile's body is a single `<tonk-display>` created with
`document.createElement` and configured by `set_attribute` from the
tile row's `entity` / `view` / `model` fields, plus the layout's own
`space` / `branch`. `<tonk-display>` then owns its entity + view
subscriptions and its own `data-state`. The layout element never
touches a tile's inner DOM — it only manages geometry, focus, and the
descriptor.

When a tile row's content fields change, the layout element calls
`set_attribute` on the existing `<tonk-display>` (which already
restarts its flows on attribute change) rather than remounting it.

## Rendering

The host gets a non-shadow light DOM tree the layout owns entirely:

```
<tonk-layout data-state="ready">
  <div class="tonk-layout-board">              <!-- vertical flow; the page scrolls -->
    <div class="tonk-layout-row" data-id=…>     <!-- flex row, justified, wraps to next row -->
      <div class="tonk-layout-column" data-id=… data-width="M">
        <div class="tonk-layout-tile" data-focused>
          <tonk-display entity=… view=… />
        </div>
        …
      </div>
      …
    </div>
    …
  </div>
</tonk-layout>
```

- **One base-column custom property drives column sizing.** The
  stylesheet defines `--tonk-layout-base` (≈380px). A column's preset
  goes on the column node as a `data-width` attribute (`XS`…`XL`); the
  stylesheet maps each preset to a `flex-basis` of the preset's
  base-multiple (`calc(N * var(--tonk-layout-base))`) and a
  `flex-grow` so the row justifies. `XL` is a column that takes the
  whole row (`flex-basis: 100%`). There is no per-element width or
  height written by script: presets are declarative, justification is
  flexbox.
- **Rows are justified, no horizontal gap.** Each `tonk-layout-row` is
  a flex row whose columns flex from their preset target to a flush
  right edge (the Google+/Vjeux justified behavior). A column whose
  target does not fit wraps to a new row. Rows do not share a height;
  each row is as tall as its tallest column.
- **The board is the only scroller.** The `tonk-layout-board` is a
  plain vertical flow with no `overflow` of its own; it grows as rows
  are added and the page scrolls. No element inside has a scrollbar:
  tiles are content-height, rows hug their tallest column, the board
  hugs its rows.
- The **reconciler** keys rows, columns, and tiles by entity URI. On
  each merged frame it: (a) removes DOM nodes whose entity vanished,
  (b) inserts nodes for new entities, (c) reorders rows, columns, and
  tiles by `order`, (d) updates `data-width` / `data-focused` /
  descriptor on survivors in place. Node identity of healthy tiles is
  preserved, so `<tonk-display>` subscriptions are never needlessly
  dropped.
- `data-state` on the host: `loading` → `ready` → `empty` (no rows) →
  `error`, the same convention as `<tonk-display>`, for CSS hooks.

Web Awesome usage: `<wa-dialog>` for the open-tile prompt,
`<wa-icon>` / `<wa-button>` for row / column / tile chrome (close,
width preset), `<wa-spinner>` while the first frame loads,
`<wa-callout variant="danger">` on error. All `<wa-*>` auto-register
via the loader already in `index.html`.

## DOM state signalling & events

| `data-state` | Meaning |
|---|---|
| `loading` | Subscriptions opening, no frame yet. |
| `ready` | Board rendered. |
| `empty` | Board has zero rows. |
| `error` | Query / network failure. |

Custom events (bubbling + composed, for diagnostics and host
integration):

| Event | When | Detail |
|---|---|---|
| `tonk-layout:connected` | Subscriptions opened | `{ board }` |
| `tonk-layout:layout` | Board reconciled | `{ rows, columns, tiles }` |
| `tonk-layout:focus` | Focused tile changed | `{ tile }` |
| `tonk-layout:error` | Failure | `{ kind, message }` |

## Crate layout

New `tonk-layout` crate, `crate-type = ["cdylib", "rlib"]`, Cargo.toml
mirroring `tonk-display`'s (the `custom-elements`, `web-sys`,
`wasm-bindgen*` set; depend on `tonk-schema` for wire types and on
`tonk-concept` for the `open_sse` helper — or move `open_sse` into a
shared spot if a cleaner home appears).

```
rust/tonk-layout/
  Cargo.toml
  SPEC.md                # author-facing element spec (parallel to tonk-concept/SPEC.md)
  src/lib.rs             # pub fn register() — wasm32-gated
  src/element.rs         # CustomElement impl: lifecycle, attribute observation
  src/model.rs           # Board / Row / Column / Tile structs; sort + reconcile-into-tree
  src/resolve.rs         # query builders: board / rows / columns / tiles. native-testable
  src/reconcile.rs       # frames → Board tree; in-place DOM patch (wasm32)
  src/writer.rs          # mutation → notation document; /evaluate POST; debounce timer
  src/interact.rs        # keyboard + pointer handlers → mutations (wasm32)
  src/state.rs           # data-state reflection helper
  src/error.rs
```

Workspace wiring:

- Add `rust/tonk-layout` to the workspace `members` in `Cargo.toml`.
- Call `tonk_layout::register()` in `rust/tonk-ui/src/bin/ui.rs`,
  alongside `tonk_concept::register()` / `tonk_display::register()`.
- Add `../tonk-layout` to `Trunk.toml`'s watch list so dev rebuilds
  pick it up.
- A standalone `tonk-layout.rs` bin + `index.html` Trunk artifact is
  **only** needed if the element must run inside the worker's
  sandboxed iframe (separate `customElements` registry). Deferred
  until a use case appears.

### `tonk-ui` route

Expose a `tonk-ui` route that mounts the element via the
imperative-slot pattern used for `<tonk-display>` in
`tonk-ui/src/components/`:

- `/space/{space}/branch/{branch}/board/{board}` — a stored board.
  The route passes `board` (plus `space` / `branch`) down to the
  element and lets it subscribe.

The route is a plain stored-board route. There is no URL-fragment
board encoding: the board's structure lives entirely in entities, and
the URL only names which board to render. This mirrors how the
`/display` route resolves `:subject` before mounting `<tonk-display>`:
routing logic stays in the route, the element stays a pure
structure-and-events component.

## Implementation order

A skeleton crate already exists, but against an *older* model: it is a
read-only board → column → tile layout with a niri-ish horizontal
strip and grid-cell column widths. The existing crate has a skeleton,
`model.rs` (a `Layout`/`Column`/`Tile` fold), `resolve.rs` (the
queries), `element.rs` (read-path subscriptions), `reconcile.rs` (the
DOM reconciler), a stylesheet section in `tonk-ui/styles.css`, and a
route component. All of it needs reworking for the new model: the
`row` level, width presets, the justified magazine layout, and the
single page scroller. The order below is framed as a **migration** of
what exists, then the still-unbuilt interaction layer.

1. **Rename `workspace`/strip framing → board** across the schema
   concepts, `model.rs`, `resolve.rs`, `reconcile.rs`, `element.rs`,
   the route, and `SPEC.md` / README. Drop the horizontal-strip
   vocabulary.
2. **Add the `row` concept and `row` level** — declare the `row`
   concept, add a `Row` struct to `model.rs` between `Board` and
   `Column`, add `column.row` and re-point the tree fold to board →
   row → column → tile. Add the rows subscription in `element.rs` and
   its query builder in `resolve.rs`.
3. **Switch column width to presets** — replace grid-cell `width`
   sizing with the five-preset model (XS…XL). `width` becomes a text
   preset on the `column` concept and the `Column` struct; reconcile
   writes `data-width` on the column node.
4. **Drop any stored `tile.height`** — if the old `tile` concept or
   `Tile` struct carries a height, remove it from the concept, the
   struct, and the resolve / reconcile paths. Tiles are
   content-height.
5. **Justified-row stylesheet** — rework the `tonk-layout` section of
   `tonk-ui/styles.css`: `--tonk-layout-base`, the per-preset
   `flex-basis`/`flex-grow` mapping, `XL` as a full-row column, rows
   as wrapping justified flex containers, the board as a plain
   vertical flow with no inner scroller.
6. **Reconciler for the four-level tree** — update `reconcile.rs` to
   diff board → row → column → tile, key each level by entity URI,
   reorder by `order`, preserve `<tonk-display>` node identity.
7. **`writer.rs` discrete mutations** — notation-document builders +
   `/evaluate` POST for open / close tile, move row, move column, move
   tile, cycle width preset, set focus.
8. **`interact.rs` focus nav** — keyboard focus / move bindings wired
   to `writer` mutations; click-to-focus; `scrollIntoView` on the
   focused tile.
9. **Width-preset cycling** — the `R` keybinding and the column-chrome
   control, wired to a `writer` mutation.
10. **Drag reorder + debounce** — pointer-drag column and tile
    reordering (within and across rows / columns), optimistic DOM
    update, debounced flush.
11. **Open-tile `<wa-dialog>` + ghost `+`** — the entity/model/view
    prompt and the open-here affordances at row end and column bottom.
12. **`tonk-ui` route** — the `/board/{board}` route, mounting the
    element via the imperative-slot pattern with the `board`
    attribute.

Steps 1 to 6 migrate the existing read-only crate onto the new
magazine model; 7 to 11 add the interaction layer; 12 exposes the
stored-board route.

## Tests

Native (no DOM, `#[dialog_common::test]`, `it_<verb_phrase>` naming):

- `resolve`: board query constrains `name`; rows query constrains
  `board`; columns query constrains `board` (denormalized); tiles
  query constrains `board` (denormalized).
- `model`: frames fold into a correctly **sorted** board → row →
  column → tile tree; fractional `order` places an inserted row
  between neighbours, and likewise for columns and tiles; an orphan
  column (row missing) or orphan tile (column missing) is dropped or
  parked predictably.
- `writer`: open / close / move / cycle-preset / focus produce the
  expected notation document; a multi-entity move (tile between
  columns) emits one document.
- debounce: rapid drag-reorder events coalesce into a single flush.

WASM (real DOM, `wasm_bindgen_test` via the same macro):

- a multi-row, mixed-stack frame renders the expected justified board.
- a layout frame that moves a column reorders the DOM **without**
  remounting an unaffected tile's `<tonk-display>`.
- a tile whose entity vanishes is removed; a new entity is inserted.
- `data-state` goes `loading` → `ready` → `empty` correctly.
- focusing a tile sets `data-focused` and scrolls it into view.
- cycling a column's preset updates `data-width` and, after the
  debounce where applicable, posts exactly one `/evaluate`.
- a column whose target width does not fit wraps to a new row; rows do
  not share a height.

## Open questions

1. **Board bootstrapping.** If the `board` attribute names a board
   with no `board` row yet, does `<tonk-layout>` create it on first
   interaction, or require it pre-asserted? Recommend **lazy-create on
   first tile open** so a fresh branch just works. Opening the first
   tile also lazily creates the first `row` and `column`.
2. **Concurrent move conflicts.** Float `order` makes most concurrent
   edits merge, but two devices inserting a row (or column, or tile)
   at the *same* gap can collide on `order`. Acceptable for v1 (visual
   reorder, no data loss); a tie-break by entity URI can be added
   later.
3. **Tile content beyond `<tonk-display>`.** v1 locks tiles to a
   single-entity `<tonk-display>`. If a tile later needs a
   `<tonk-concept>` list or other element, the `tile` concept grows a
   `kind` + descriptor — flagged, not designed here.
4. **Base-column size and wrap threshold.** The base column is ≈380px;
   the exact value, and how close to its target a column must come
   before it wraps to a new row instead of compressing, want tuning
   against real content and real viewports.
5. **Empty rows and columns.** A row or column with no children is a
   valid entity. Whether the layout auto-prunes an emptied row /
   column, or keeps it as a drop target, is a v1 interaction decision
   (recommend: prune on the write that empties it, since the ghost `+`
   already provides the add affordance).
6. **`open_sse` ownership.** Currently lives in `tonk-concept`.
   Depending on `tonk-concept` just for it is acceptable short-term; a
   `tonk-rt`/`tonk-template` shared crate is the cleaner home if more
   plumbing gets shared (the `tonk-display` plan already noted this
   extraction).

## Appendix: explored directions, parked

The current plan is the magazine of justified rows described above.
Before settling there, three richer models were explored. They are
recorded here as parked ideas, NOT part of the current plan, so the
reasoning is not lost and someone could pick one up later.

### 1. Niri-style horizontal strip + canvas zoom

The board was a single infinite horizontal scrollable strip of columns
(modelled on [niri](https://github.com/YaLTeR/niri)) rather than a
vertical stack of rows. Column widths were stored in abstract **canvas
units**, and a per-board **zoom** mapped units to pixels: zoom was "how
many units span the viewport", so resize and zoom rescaled every
column together without reflow. Zoom itself was stored per-profile on
the **meta branch**, keyed by `(profile, board)`, so it synced across
one person's devices without being shared with other viewers (a
`board-zoom` concept with a `units` field). Parked because the
magazine model is responsive without any zoom control: a fixed base
column plus "how many fit is a division" plus row wrapping covers the
same form-factor range with no zoom UI, no meta-branch concept, and no
units-to-pixels math.

### 2. Fork-tree / spatial-trails model

Tiles would be opened *from* other tiles (open-right, open-below), and
a tile's position would be derived from an **origin + direction tree**
rather than stored coordinates. The board would render that tree as a
set of stacked root-to-leaf paths (flattened "trails"), with
shared-prefix dedup so common ancestors are not drawn twice, per-row
independent horizontal pan, and gap cells acting as open-here
affordances. The inspiration was the Browser.html "lossless web
navigation" spatial model and its "trails" follow-up. Parked for two
reasons: packing a 2D fork-tree onto a grid has no gap-free,
collision-free general solution, and the flattened-paths rendering
(per-row pan plus shared-prefix dedup) is a large amount of novel UI
that would need validation before it is worth building.

### 3. URL-expressible boards

Alongside stored boards, a board's whole structure could be encoded in
the URL **fragment** for ad-hoc, shareable arrangements: a `|`/`,`
grammar of columns and tile refs, where a tile ref is a name (resolved
against the branch `Name` index) or a URI used verbatim, with optional
`view` / `model` selectors and a `!` focus marker. Editing such a
board would rewrite the fragment instead of writing entities, and a
"save as named board" action would promote it to a stored board.
Parked as a possible later addition once the stored-board model is
solid; it is additive and does not conflict with the magazine model,
so it can be layered on without rework.

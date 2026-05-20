# `<tonk-layout>` — a niri-style tiling window manager web component

## Context

We have two live web components today: `<tonk-concept>` (renders *many*
matched entities into an author-supplied template) and `<tonk-display>`
(renders *one* entity using a `view` template stored on the branch).
Both are vanilla custom elements written in Rust via the
`custom-elements` crate, compiled to wasm by Trunk, and registered in
`rust/tonk-ui/src/bin/ui.rs`.

This plan adds a third element: `<tonk-layout>`, a tiling window manager
modelled on [niri](https://github.com/YaLTeR/niri). It arranges tiles
on an **infinite horizontal scrollable strip of columns**; each column
is a **vertical stack of tiles**. Every tile mounts a `<tonk-display>`
pointed at a branch entity. The layout itself — columns, tiles,
column widths, focus — is persisted to the dialog database as
**normalized entities**, so a reload (or another device) reconstructs
the exact board. (Scroll position is derived from focus, not stored;
zoom is stored separately, per profile, on the meta branch — see
below.)

It ships as a new `tonk-layout` crate.

## Target usage

```html
<tonk-layout space="home" branch="main" board="default"></tonk-layout>
```

No children. The element subscribes to its layout entities on the
branch, builds the strip, and mounts a `<tonk-display>` inside each
tile. An empty board renders an empty strip with an "add column"
affordance.

## Layout model

```
            viewport (scrolls horizontally) — a board
   ┌───────────────────────────────────────────────┐
   │  column 0     column 1        column 2         │ ...→ infinite
   │ ┌─────────┐  ┌──────────┐  ┌──────────────┐   │
   │ │ tile A  │  │  tile C  │  │   tile E     │   │
   │ ├─────────┤  └──────────┘  ├──────────────┤   │
   │ │ tile B  │                │   tile F     │   │
   │ └─────────┘                └──────────────┘   │
   └───────────────────────────────────────────────┘
```

- **Board** — a named, zoomable canvas of columns laid end to end.
  Scrolls horizontally; total width is unbounded. One branch can
  hold several boards (`default`, `scratch`, …); the `board`
  attribute selects one. (Earlier drafts called this a
  "workspace" — renamed because a board is a zoomable canvas, not
  a fixed desktop.)
- **Column** — an ordered list of **tiles** stacked vertically,
  plus a `width` in canvas units. All tiles in a column share the
  column width.
- **Tile** — one cell. Its width is its column's; its **height is
  content-driven** — a tile is as tall as its `<tonk-display>`
  needs. It carries a content descriptor (`entity` / `view` /
  `model`).
- **Focus** — exactly one tile is focused. Focus drives scroll:
  the board slides so the focused column is fully on screen.

### Sizing — canvas units and zoom

This is the core of the model, and it is **not** niri's
resolution-relative fractions nor absolute pixels. It is a
**zoomable canvas** (the Figma / tldraw model), which is both
grid-aligned and responsive.

A column's `width` is an integer count of **canvas units**. A unit
has no fixed pixel size on its own. What gives it a size is the
board's **zoom**: zoom is "how many units span the viewport".

```
rendered px per unit  =  (viewport width − chrome) / units-per-viewport
column rendered width =  width-units × px-per-unit
```

- The stored `width` (a unit count) is **stable** — it never
  changes on resize or zoom.
- **Resize** changes the viewport, so px-per-unit rescales; every
  column rescales together, nothing reflows or clamps.
- **Zoom out** raises units-per-viewport, so px-per-unit shrinks:
  a 2-unit column that filled half the screen now fills a third,
  and a column that overflowed now fits. **Zoom in** is the
  reverse.

The unit is a **quarter** of the default viewport: at the default
zoom, 4 units span the screen, so a column can be a quarter, half,
three-quarters, or full width (1/2/3/4 units), and wider. Resize
presets and the UI speak in those fractions (¼, ½, ¾, full); the
stored value is the integer unit count.

Why this model:

- **Responsive.** "Half" is half the screen on a phone and on a
  desktop — the distinction between column sizes survives the form
  factor, which absolute pixels do not. Default units-per-viewport
  can differ per form factor (e.g. fewer units across on a phone)
  so a board is sensible on any device without per-device data.
- **Grid-aligned.** The unit *is* the grid. The graph-paper dot
  background is drawn in **sub-cells** (a unit subdivided, e.g.
  ×4), so the grid stays visible and meaningful at any zoom —
  "grids within the grid". Zoom steps are whole units; drag-resize
  and scroll snap to sub-cells.
- **Stable.** Pan and zoom never mutate stored sizes, so two
  devices viewing the same board at different zooms still agree on
  the layout.

### Zoom state — per profile, on the meta branch

Zoom is **not** part of a board's layout (that would make every
viewer share one zoom). It is stored the way branch state is: as a
per-profile fact on a **meta branch**, keyed by `(profile, board)`.

Consequently zoom **syncs across one profile's own devices** (open
the board on your phone, it has the zoom you left on your laptop)
but is **not shared with other people** and is not part of the
board's content. This is deliberately "the same trick as
branches": a meta-branch fact is per-profile and device-syncing.
(A value that must stay on a single device would need browser
`localStorage`; that is explicitly *not* what is wanted here.)

The element reads its zoom from this fact and writes it back when
the user zooms, debounced like every other write.

The keyboard width presets are computed from the *current*
viewport and recomputed on resize, so the "full" preset always
fills the visible width.

## Data model — normalized tile entities

Three concepts on the branch describe a board's layout. They are
declared once per repository (asserted-notation), exactly like the
`view` concept that `<tonk-display>` depends on. A fourth concept,
`board-zoom`, lives on the **meta branch** (see below).

```yaml
concept!: &board
  description: A named zoomable canvas of columns
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

concept!: &column
  description: A vertical stack of tiles within a board
  with:
    board:
      the: xyz.tonk.layout/column-board
      as: entity
    order:
      description: Position of the column in the board (float, sortable)
      the: xyz.tonk.layout/column-order
      as: float
    width:
      description: Column width in canvas units (zoom maps units to px)
      the: xyz.tonk.layout/column-width
      as: unsigned-integer

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

There is **no `tile.height`** — a tile's height is content-driven,
as tall as its `<tonk-display>` renders (a Pinterest-style column).
Only column `width` is stored; the vertical axis flows.

The per-profile zoom is a separate concept on the meta branch:

```yaml
concept!: &board-zoom
  description: A profile's zoom level for one board (units per viewport)
  with:
    board:
      description: Name of the board this zoom applies to
      the: xyz.tonk.layout/zoom-board
      as: text
    units:
      description: Canvas units spanning the viewport at this zoom
      the: xyz.tonk.layout/zoom-units
      as: unsigned-integer
```

Notes on the model:

- **`order` is a float, not an integer.** Inserting a column between
  two others sets `order` to the midpoint of its neighbours — no
  renumbering, no write amplification. Same for tile `order` within a
  column. (Fractional indexing.)
- **References point upward** (`tile.column`, `column.board`) so a
  child can be inserted/removed with a single assertion and the parent
  never needs rewriting. `tile.board` is a **denormalized** copy of
  `column.board`: it lets the tiles subscription filter by board in a
  single query, instead of one subscription per column or fetching
  every tile on the branch. A tile moved between columns of the same
  board leaves `tile.board` unchanged; only a move across boards
  rewrites it (alongside `tile.column`).
- **`board.focus`** is a cardinality-one pointer to a tile entity;
  re-asserting it retracts the previous value automatically (the
  git-ref pattern from the notation guide).
- **`board-zoom` lives on the meta branch**, keyed by board name and
  scoped to the profile. It is per-profile and syncs across that
  profile's devices, but is not part of the board's shared layout —
  see "Zoom state" above. The element resolves the matching
  `board-zoom` row for the current board (creating one at a default
  units-per-viewport if absent) and re-asserts `units` when the user
  zooms.
- A tile that loses its content (entity retracted) is still a valid
  empty tile until explicitly removed.
- Entities get **stable identities** via explicit `this:` mappings so
  edits target the same entity instead of spawning content-addressed
  duplicates.

### Why normalized rather than a JSON blob

Per-attribute merge: two devices dragging different columns, or
resizing different tiles, commit disjoint claims and merge cleanly on
sync. A single JSON-blob entity would re-hash on every edit and lose
one side's change. The cost is more query/assemble code, accepted here.

## Boards in the URL

A board can come from one of two places, and `<tonk-layout>` reads
its structure from whichever the URL provides:

- **Stored board.** The route names a persisted board (e.g.
  `/space/home/branch/main/layout/default`). The element loads that
  board's `board` / `column` / `tile` entities and behaves exactly
  as the "Data model" section describes: durable, shared between
  people, reactive across devices, per-attribute merge.
- **Ad-hoc board.** The URL carries a fragment that encodes the
  columns and tiles directly. The element builds the board from the
  fragment instead, with no `board` / `column` / `tile` entities
  backing it. This is for arrangements not worth saving as a named
  board: a quick side-by-side, a link you paste to a colleague.

These are additive, not alternatives: named boards stay the durable
default, the URL form is an extra. The two never mix in one board.

### Fragment grammar

The board structure rides in the URL **fragment** (the part after
`#`). The fragment is a list of **columns** separated by `|`; each
column is a list of **tile refs** separated by `,`:

```
#<tileRef>,<tileRef>|<tileRef>|<tileRef>,<tileRef>
```

A **tile ref** identifies the entity a tile's `<tonk-display>`
renders. It can be one of two forms, exactly as the `/display/:subject`
route already accepts (see `rust/tonk-ui/src/components/display.rs`):

- a **name** (anything with no `:`), resolved against the branch's
  `Name` index — the same `id:<name>` lookup `resolve_name` does;
- a **URI** (anything containing a `:`, e.g. `did:key:…` or
  `concept:…`), used verbatim with no lookup.

A tile ref may carry the optional `view` and `model` selectors as a
query-ish suffix on the ref, reusing the `/display` route's
parameter names: `<ref>?view=<name>&model=<concept>`. The `?` and `&`
sit inside one tile ref and never collide with the `,` / `|`
separators. Refs are percent-encoded so a literal `|`, `,`, `?`, or
`#` inside a name or URI survives.

Column **width is not in the fragment.** An ad-hoc board's columns
all take the default width; the fragment stays short and readable,
and per-column widths are exactly the kind of fine-tuning that
warrants saving the board as a named one. (If a real need for
fragment widths appears, a compact `~<units>` suffix on the first
tile ref of a column is the reserved spot, but v1 omits it.)

Focus rides in the fragment too: a `!` prefix on one tile ref marks
the focused tile (e.g. `|!colB-tile|`). At most one ref carries it;
if none does, the first tile is focused. Keeping focus in the
fragment means a pasted URL reopens at the same column, and a focus
change is just another fragment rewrite.

Examples:

```
# two columns, one tile each, names resolved on the branch
/space/home/branch/main/layout#inbox|calendar

# three columns; column A stacks two tiles; calendar tile focused
/space/home/branch/main/layout#note-a,note-b|!calendar|tasks

# a URI tile ref with a view selector, alongside a named tile
/space/home/branch/main/layout#did:key:z6Mk…abc?view=card|inbox
```

### Ad-hoc board identity — focus and zoom

An ad-hoc board has no stored `board` row, so the two pieces of
state a `board` row would carry need another home:

- **Focus** rides in the fragment (the `!` prefix above). It is
  part of the structure the URL already round-trips, so no separate
  store is needed.
- **Zoom is ephemeral for ad-hoc boards.** The meta-branch
  `board-zoom` concept is keyed by `(profile, board-name)`, and an
  ad-hoc board has no name. Rather than invent a synthetic key (a
  hash of the normalized fragment, which would accumulate orphan
  `board-zoom` rows for every throwaway arrangement), an ad-hoc
  board simply starts each load at the default units-per-viewport
  for the form factor and is zoomable in-session without persisting.
  A board whose zoom is worth keeping is a board worth saving as a
  named board. This keeps the meta branch free of churn from
  one-off URLs.

### Editing an ad-hoc board

Rearranging an ad-hoc board — moving a column, adding or closing a
tile, changing focus — writes **no entities**. Each structural edit
**rewrites the URL fragment** instead, and the round-trip below
rebuilds the board from the new fragment. The ad-hoc board lives
entirely in the address bar.

An ad-hoc board becomes durable only through an explicit **"save as
named board"** action. That action takes the current in-memory
structure and writes the `board` / `column` / `tile` entities for
it (one `/evaluate` document, see "Writing layout state"), then
navigates to the stored-board route for the new name. From that
point on it is an ordinary stored board: edits write entities, zoom
gets a real `(profile, name)` key.

### Two-way binding — the route owns the URL

`<tonk-layout>` must (a) react to URL changes by rebuilding the
board and (b) update the URL when the layout changes. Rather than
have the element listen on `hashchange` / the History API itself,
the **`tonk-ui` route component owns the URL**, consistent with how
the `/display` route resolves `:subject` before mounting
`<tonk-display>`:

- **URL → element.** The route parses the path and fragment, decides
  stored-vs-ad-hoc, and passes a **structure descriptor** down to
  the element as an attribute (for an ad-hoc board) or passes the
  board name (for a stored board). When the fragment changes the
  route re-derives the descriptor and updates the attribute; the
  element's normal attribute-changed teardown/restart rebuilds the
  board.
- **element → URL.** When the layout changes (focus moved, column
  reordered) the element dispatches a `tonk-layout:layout` /
  `tonk-layout:focus` event upward (it already does, see "DOM state
  signalling & events"). The route listens for those events and,
  for an ad-hoc board, re-encodes the structure into the fragment
  and writes it via the History API; for a stored board it writes
  only the board name and focus, since the rest is in entities.

Keeping `hashchange` / History handling in the route mirrors the
existing precedent and keeps the element free of routing concerns:
the element speaks structure descriptors and events, the route
speaks URLs.

## Reading layout state — subscriptions

The element's structure has **two input paths**, decided by the
route (see "Boards in the URL"):

- **Stored board** — the element is given a board name and reads its
  layout from entity subscriptions, as detailed below.
- **Ad-hoc board** — the element is given a structure descriptor
  (parsed from the URL fragment) and builds the strip from it
  directly, with no `board` / `column` / `tile` subscriptions. It
  still subscribes each tile's `<tonk-display>` to its entity, and
  it still resolves name-form tile refs through the branch's `Name`
  index. Zoom is ephemeral, so the meta-branch zoom subscription is
  skipped too.

The rest of this section describes the **stored-board** path.

`<tonk-layout>` reads **four** things. Three describe the shared
layout and come from the **content branch**; the fourth is the
per-profile zoom and comes from the **meta branch**. All four are
live SSE subscriptions against the worker's `/query` route (same
machinery as `<tonk-concept>`: a `POST` with `Accept:
text/event-stream`, frames are `Vec<Conclusion>` JSON, cancellation
via `AbortController`).

Content branch (the shared layout):

1. **Board subscription** — the `board` row whose `name` equals the
   `board` attribute. Frame carries `focus`.
2. **Columns subscription** — all `column` rows whose `board` equals
   the resolved board entity.
3. **Tiles subscription** — all `tile` rows whose `board` equals the
   resolved board entity (the denormalized `tile.board` field lets
   this be one query instead of one per column).

Meta branch (per-profile, device-syncing):

4. **Board-zoom subscription** — the `board-zoom` row for the
   current board, queried against the **meta branch** rather than
   the content branch. The query constrains `zoom-board` to the
   `board` attribute value. If no row comes back, the element
   creates one at a default units-per-viewport (see "Open
   questions" for the per-form-factor defaults) and uses that until
   the write lands. The frame carries `units`, which feeds the
   px-per-unit computation.

A reusable **reconciler** (see "Rendering") folds the latest frame of
each subscription into an in-memory `Layout` tree, sorts columns and
tiles by `order`, derives px-per-unit from the zoom frame, and
patches the DOM in place — preserving each tile's `<tonk-display>`
node identity so a layout change never tears down and remounts a
healthy tile. A zoom-only frame just recomputes px-per-unit and
rescales; it touches no tile content.

Because every layout write goes through `/evaluate` (which re-polls
subscriptions on commit), the WM is **reactive across tabs and
devices** for free: move a column on one screen, it moves on the
other. The same holds for zoom, but scoped to one profile: zoom on
the laptop and the phone (same profile) follows, while another
person's view is untouched.

## Writing layout state — `/evaluate` + debounce

There is no write-debounce primitive in the codebase; we build one.

- **Discrete actions** (open tile, close tile, move column left/right,
  focus change) write **immediately**: one `POST /evaluate` with an
  asserted-notation document. These are cheap and infrequent. These
  writes target the **content branch**.
- **Continuous actions** (drag-resize a column, zoom the board) update
  the DOM optimistically on every pointer / key / wheel event but
  **debounce the write**: coalesce into a single `/evaluate` flushed
  ~200 ms after the input goes idle (or on `pointerup`). Implemented
  with `setTimeout` + cancellation, mirroring the `AbortController`
  pattern already in `tonk-concept`. The in-flight optimistic state is
  the source of truth for the DOM until the write lands; the
  subscription frame that follows is idempotent against it.
- **Zoom writes** re-assert `board-zoom.units` for the current board
  and, unlike every other write, go to the **meta branch**, not the
  content branch. They are debounced like a drag-resize: a flurry of
  zoom steps coalesces into one `/evaluate` against the meta branch.
  Because `board-zoom` is cardinality-one per `(profile, board)`,
  re-asserting `units` retracts the prior value. If the element had
  to lazily create the row (no frame yet), the first zoom write is
  also its first assertion.
- **Batching:** a single user action that touches several entities
  (e.g. removing a column re-spaces nothing thanks to float `order`,
  but moving a tile between columns rewrites `tile.column` +
  `tile.order`) goes in **one** `/evaluate` document = one dialog
  transaction = atomic.

A small `writer` module owns: building the notation document for each
mutation, the debounce timer, and the optimistic-state bookkeeping.
It builds documents for both branches; the zoom document is the only
one routed to the meta branch.

**Ad-hoc boards write no entities.** Everything above applies to a
stored board. For an ad-hoc board (built from a URL fragment) a
structural edit — move a column, open or close a tile, change focus
— produces **no `/evaluate` document at all**: the element dispatches
its layout event, and the route re-encodes the structure into the
URL fragment (see "Boards in the URL"). The only entity write an
ad-hoc board ever triggers is the explicit **"save as named board"**
action, which builds one `/evaluate` document asserting the `board`
/ `column` / `tile` rows for the current structure (the same
notation builders, fed the in-memory `Layout` instead of a single
mutation) and then hands off to the stored-board route.

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
| `board` | no | `"default"` | Which named board to render (stored-board path). |
| `structure` | no | — | A fragment-encoded structure descriptor (ad-hoc-board path). |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

`board` and `structure` are the **two input paths** (see "Boards in
the URL" and "Reading layout state"). They are mutually exclusive:

- with `board` set, the element loads a stored board's entities and
  the meta-branch zoom;
- with `structure` set, the element builds an ad-hoc board from the
  descriptor, runs no `board` / `column` / `tile` subscriptions, and
  uses ephemeral zoom.

`structure` carries the same column / tile / focus encoding as the
URL fragment; the route parses the fragment and passes it down here,
so the element never touches `hashchange` or the History API itself.

All attributes are observed; changing any aborts whatever
subscriptions are open, clears the strip, and restarts — same
teardown/restart discipline as `<tonk-concept>` / `<tonk-display>`.
Changing `board` re-targets the board-zoom subscription on the meta
branch; changing `structure` rebuilds the ad-hoc strip.

## Interaction (niri keybindings + pointer)

Keyboard (focus must be within the host; the element listens on its
own root):

| Key | Action |
|---|---|
| `←` / `→` | Move focus to previous / next column. |
| `↑` / `↓` | Move focus up / down within the focused column. |
| `Ctrl+←/→` | Move the focused column left / right in the strip. |
| `Ctrl+↑/↓` | Move the focused tile up / down within its column. |
| `R` | Cycle the focused column through preset widths (¼ ½ ¾ full = 1/2/3/4 units). |
| `Q` | Close the focused tile. |
| `Enter` | Open a new tile (prompt for entity/view — see below). |
| `Ctrl+` / `Ctrl-` | Zoom the board out / in (units-per-viewport ±1). |

The `R` preset cycle walks the four canvas-unit widths ¼/½/¾/full,
which are 1/2/3/4 units at the default zoom (4 units span the
screen). The stored `width` is the integer unit count itself; there
is no viewport-relative rounding, the value *is* the unit count.

Zoom changes `board-zoom.units` (units per viewport) in whole-unit
steps: `Ctrl+` raises it (zoom out, columns shrink), `Ctrl-` lowers
it (zoom in, columns grow), clamped to the min/max bounds (see "Open
questions"). There is no tile-height cycle: a tile's height is its
content's height, not a stored or resizable field.

Pointer:

- **Focus** — click a tile to focus it.
- **Resize** — a drag handle sits on each column's trailing edge.
  Tiles have no resize handle (height is content-driven). Dragging
  snaps to **sub-cells** (a unit subdivided ×4): the handle follows
  the pointer but the committed `width` is the nearest whole unit
  count. The DOM updates optimistically on every pointer move; the
  write to the content branch is debounced and flushed on
  `pointerup` (see *Writing layout state*).
- **Zoom** — wheel with a modifier (e.g. Ctrl+wheel, the platform
  pinch-zoom gesture) zooms the board in / out, changing
  units-per-viewport. Like keyboard zoom it steps in whole units
  and writes `board-zoom.units` debounced to the meta branch.
- **Scroll** — horizontal wheel / trackpad (no modifier) scrolls the
  strip. The strip carries CSS scroll-snap with a one-sub-cell
  stride, so a flick settles with content aligned to the dot grid.
  The snap is *proximity*, not *mandatory* — a deliberate scroll can
  rest anywhere, it just nudges to the nearest sub-cell line when it
  would otherwise stop between them.

**Opening a tile** needs an entity to display. v1: a `<wa-dialog>`
(Web Awesome) with inputs for `entity`, `model`, `view`. A richer
picker (browse branch entities) is a follow-up. Authors can also seed
tiles by asserting `tile` rows directly.

Focus changes scroll the strip so the focused column is fully
visible: `scrollIntoView` on the focused column with
`scroll-behavior: smooth`. Because the strip snaps to the sub-cell
grid, the slide settles grid-aligned without extra work. Scroll
follows focus at whatever the current zoom is — px-per-unit is read
fresh, so a focused column off-screen at one zoom is still brought
fully into view at another.

## Tile content — `<tonk-display>` per tile

Each tile's body is a single `<tonk-display>` created with
`document.createElement` and configured by `set_attribute` from the
tile row's `entity` / `view` / `model` fields, plus the WM's own
`space` / `branch`. `<tonk-display>` then owns its entity + view
subscriptions and its own `data-state`. The WM never touches a tile's
inner DOM — it only manages geometry, focus, and the descriptor.

When a tile row's content fields change, the WM calls
`set_attribute` on the existing `<tonk-display>` (which already
restarts its flows on attribute change) rather than remounting it.

## Rendering

The host gets a non-shadow light DOM tree the WM owns entirely:

```
<tonk-layout data-state="ready">
  <div class="tonk-layout-strip">             <!-- scroll container, grid snap -->
    <div class="tonk-layout-rail">            <!-- flex row of columns -->
      <div class="tonk-layout-column" data-id=…>
        <div class="tonk-layout-tile" data-focused>
          <tonk-display entity=… view=… />
        </div>
        …
      </div>
      …
    </div>
  </div>
</tonk-layout>
```

- **One zoom-derived custom property drives all sizing.** The
  reconciler computes px-per-unit from the zoom frame and the
  current viewport — `(viewport width − chrome) / units-per-viewport`
  — and writes it as a single `--tonk-layout-unit` CSS custom
  property on the host (or the strip). Each column's `width-units`
  goes on the column node as `--tonk-layout-width` (the integer unit
  count); the stylesheet sizes the column `calc(var(--tonk-layout-width)
  * var(--tonk-layout-unit))`. There is **no viewport `min()` cap**:
  a column wider than the viewport simply overflows and the rail
  scrolls. Tiles have **no width or height property** — width is
  inherited from the column, height is content-driven (the tile box
  hugs its `<tonk-display>`), so columns are Pinterest-style vertical
  stacks.
- A resize observer on the host recomputes `--tonk-layout-unit` when
  the viewport changes; a zoom frame recomputes it from the new
  `units`. Either way the whole strip rescales with one property
  write and nothing reflows.
- The strip is a plain scrolling `<div>` (a `<wa-scroller>` was tried
  but does not propagate height to its slotted child). It carries
  `scroll-snap-type: x proximity` with a one-sub-cell snap stride
  (the unit subdivided ×4) so scrolling settles aligned to the
  dotted background. The dot grid is drawn in sub-cells so it stays
  meaningful at any zoom.
- The **reconciler** keys columns by entity URI and tiles by entity
  URI. On each merged frame it: (a) removes DOM nodes whose entity
  vanished, (b) inserts nodes for new entities, (c) reorders by
  `order`, (d) updates `width`/`focused`/descriptor on survivors in
  place, (e) on a zoom frame, recomputes `--tonk-layout-unit`. Node
  identity of healthy tiles is preserved, so `<tonk-display>`
  subscriptions are never needlessly dropped.
- `data-state` on the host: `loading` → `ready` → `empty` (no
  columns) → `error`, same convention as `<tonk-display>`, for CSS
  hooks.

Web Awesome usage: `<wa-dialog>` for the open-tile prompt,
`<wa-icon>` / `<wa-button>` for column/tile chrome (close, resize),
`<wa-spinner>` while the first frame loads, `<wa-callout
variant="danger">` on error. All `<wa-*>` auto-register via the loader
already in `index.html`.

## DOM state signalling & events

| `data-state` | Meaning |
|---|---|
| `loading` | Subscriptions opening, no frame yet. |
| `ready` | Strip rendered. |
| `empty` | Board has zero columns. |
| `error` | Query / network failure. |

Custom events (bubbling + composed, for diagnostics and host
integration):

| Event | When | Detail |
|---|---|---|
| `tonk-layout:connected` | Subscriptions opened | `{ board }` |
| `tonk-layout:layout` | Strip reconciled | `{ columns, tiles }` |
| `tonk-layout:focus` | Focused tile changed | `{ tile }` |
| `tonk-layout:zoom` | Zoom (units-per-viewport) changed | `{ units }` |
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
  src/model.rs           # Layout / Column / Tile structs; sort + reconcile-into-tree
  src/resolve.rs         # query builders: board / columns / tiles (content branch),
                         #   board-zoom (meta branch); zoom math (units → px-per-unit).
                         #   native-testable
  src/reconcile.rs       # frames → Layout; in-place DOM patch; --tonk-layout-unit (wasm32)
  src/writer.rs          # mutation → notation document; /evaluate POST; debounce timer;
                         #   zoom writes routed to the meta branch
  src/interact.rs        # keyboard + pointer + zoom handlers → mutations (wasm32)
  src/state.rs           # data-state reflection helper
  src/error.rs
```

The **zoom** concern is small enough not to need its own module: the
units → px-per-unit math lives in `resolve.rs` (alongside the
board-zoom query builder, native-testable), the CSS-property write
lives in `reconcile.rs`, and the zoom keybindings / wheel handler
live in `interact.rs` feeding `writer.rs`. If it grows it can split
out into `zoom.rs`.

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
`tonk-ui/src/components/`. The route handles **both** board paths
and owns the URL <-> layout two-way binding (see "Boards in the
URL"):

- `/space/{space}/branch/{branch}/layout/{board}` — a **stored
  board**. The route passes `board` (plus `space` / `branch`) down
  to the element and lets it subscribe.
- `/space/{space}/branch/{branch}/layout` with a `#…` fragment — an
  **ad-hoc board**. The route parses the fragment into a structure
  descriptor and passes it down as the `structure` attribute.

The route is the only component that touches the URL. It:

1. parses the path and fragment, decides stored-vs-ad-hoc, and
   passes either `board` or `structure` to the element;
2. listens on `hashchange` (and route-param changes) and re-derives
   the attribute when the URL changes, so the element rebuilds via
   its normal attribute-changed path;
3. listens for the element's `tonk-layout:layout` / `tonk-layout:focus`
   events and writes the URL back via the History API — the whole
   structure into the fragment for an ad-hoc board, just the board
   name and focus for a stored board.

This mirrors how the `/display` route resolves `:subject` before
mounting `<tonk-display>`: routing logic stays in the route, the
element stays a pure structure-and-events component.

## Implementation order

The skeleton crate, model, resolve, read path, and reconciler are
**already built**, but against the *old* model: `workspace` instead
of `board`, column `width` as grid cells with a viewport `min()`
cap, and a stored `tile.height`. So the order below is framed as a
**migration** of what exists, then the still-unbuilt interaction
layer.

1. **Rename `workspace` → `board`** across the schema concepts
   (`board-*`, `column-board`, `tile-board`), `model.rs`,
   `resolve.rs`, `reconcile.rs`, `element.rs` (the `board`
   attribute), the `tonk-ui` route, and `SPEC.md` / README. Pure
   rename, no behaviour change.
2. **Drop `tile.height`** — remove the field from the `tile`
   concept, from the `Tile` struct in `model.rs`, and from the
   resolve / reconcile paths. Tiles become content-height.
3. **Switch column width to canvas units** — replace the "grid
   cells with `min()` viewport cap" sizing with "canvas units ×
   zoom-derived px-per-unit". `width` stays an integer but is now a
   unit count; reconcile drops the `min()` and writes only
   `--tonk-layout-width`.
4. **Board-zoom meta-branch query** — add the `board-zoom` concept,
   its query builder in `resolve.rs` (against the meta branch), and
   the units → px-per-unit math. Wire the fourth subscription into
   `element.rs`, with lazy default if no row exists.
5. **Apply zoom in the reconciler** — compute px-per-unit from the
   zoom frame + viewport and write `--tonk-layout-unit`; add the
   resize observer that recomputes it.
6. **Zoom interaction + writes** — `Ctrl+`/`Ctrl-` and modified
   wheel in `interact.rs`, debounced `board-zoom.units` re-assertion
   in `writer.rs` routed to the meta branch.
7. **`writer.rs` discrete mutations** — notation-document builders +
   `/evaluate` POST for open/close tile, move column/tile, set
   focus (content branch).
8. **`interact.rs` focus nav** — keyboard focus / move bindings
   wired to `writer` mutations; click-to-focus.
9. **Drag-resize + debounce** — pointer-drag column resize snapping
   to sub-cells, with optimistic DOM update and the debounced flush.
10. **Open-tile `<wa-dialog>`** — the entity/model/view prompt.
11. **Scroll-follows-focus** — `scrollIntoView` on focus change.
12. **`tonk-ui` route — stored boards** — the
    `/layout/{board}` route, mounting the element via the
    imperative-slot pattern with the `board` attribute.
13. **Fragment grammar + parser** — the column / tile / focus
    encoding and a parser that turns a fragment into a structure
    descriptor (and back). Native-testable, no DOM.
14. **Ad-hoc input path in the element** — the `structure`
    attribute: build the strip from a descriptor, skip the
    `board` / `column` / `tile` subscriptions, ephemeral zoom.
15. **Route URL <-> layout binding** — the route parses path +
    fragment, decides stored-vs-ad-hoc, listens on `hashchange`,
    and writes the URL back from the element's layout / focus
    events.
16. **"Save as named board"** — the action that turns the current
    ad-hoc structure into `board` / `column` / `tile` entities and
    navigates to the stored-board route.

Steps 1–5 migrate the existing read-only strip onto the new
canvas-unit + zoom model; 6–11 add interaction; 12 exposes the
stored-board route; 13–16 add URL-expressible boards on top of the
finished board and interaction layers (they depend on both).

## Tests

Native (no DOM, `#[dialog_common::test]`, `it_<verb_phrase>` naming):

- `resolve`: board query constrains `name`; columns query constrains
  `board`; tiles query constrains `board` (denormalized); board-zoom
  query constrains `zoom-board` and targets the meta branch.
- `resolve` zoom math: a given units-per-viewport and viewport width
  yield the expected px-per-unit; a column's unit count times
  px-per-unit yields the expected rendered width.
- `model`: frames fold into a correctly **sorted** strip; fractional
  `order` places an inserted column between neighbours; an orphan tile
  (column missing) is dropped or parked predictably.
- `writer`: open/close/move/resize produce the expected notation
  document; a multi-entity move emits one document; a zoom write
  re-asserts `board-zoom.units` and is addressed to the meta branch.
- debounce: rapid resize **and** rapid zoom events each coalesce into
  a single flush.

WASM (real DOM, `wasm_bindgen_test` via the same macro):

- a three-column / mixed-stack frame renders the expected strip.
- a layout frame that moves a column reorders the DOM **without**
  remounting an unaffected tile's `<tonk-display>`.
- a tile whose entity vanishes is removed; a new entity is inserted.
- `data-state` goes `loading` → `ready` → `empty` correctly.
- focusing a tile sets `data-focused` and scrolls it into view.
- resizing a column updates `--tonk-layout-width` and, after the
  debounce, posts exactly one `/evaluate`.
- a zoom frame (or `Ctrl+`/`Ctrl-`) updates `--tonk-layout-unit` on
  the host, rescaling every column, with no tile remount.
- a viewport resize recomputes `--tonk-layout-unit` while leaving
  stored unit counts unchanged.

## Open questions

1. **Board bootstrapping.** If the `board` attribute names a board
   with no `board` row yet, does `<tonk-layout>` create it on first
   interaction, or require it pre-asserted? Recommend **lazy-create
   on first tile open** so a fresh branch just works.
2. **Concurrent move conflicts.** Float `order` makes most concurrent
   edits merge, but two devices inserting a column at the *same* gap
   can collide on `order`. Acceptable for v1 (visual reorder, no data
   loss); a tie-break by entity URI can be added later.
3. **Tile content beyond `<tonk-display>`.** v1 locks tiles to a
   single-entity `<tonk-display>`. If a tile later needs a
   `<tonk-concept>` list or other element, the `tile` concept grows a
   `kind` + descriptor — flagged, not designed here.
4. **Scroll position persistence.** Niri derives scroll from focus; we
   do too, so scroll is *not* a stored field. Revisit if free-scroll
   (focus-independent) is wanted.
5. **`open_sse` ownership.** Currently lives in `tonk-concept`.
   Depending on `tonk-concept` just for it is acceptable short-term;
   a `tonk-rt`/`tonk-template` shared crate is the cleaner home if
   more plumbing gets shared (the `tonk-display` plan already noted
   this extraction).
6. **Default units-per-viewport per form factor.** The unit is a
   quarter of the *default* viewport, so the desktop default is 4
   units across. A phone wants fewer (a 2-unit column should still
   be a usable half-screen), so the lazy-created `board-zoom.units`
   should depend on form factor — 4 on desktop, perhaps 2 on a
   phone. Exact thresholds and the detection (viewport width
   breakpoint vs. pointer type) are unresolved.
7. **Zoom min / max bounds.** `Ctrl+`/`Ctrl-` and modified wheel must
   clamp `units` to a sensible range — a floor (e.g. 1 unit across,
   a single column fills the screen) and a ceiling (so px-per-unit
   does not collapse to an unreadable size). Provisional 1..16;
   confirm against real content.
8. **Focus-scroll under zoom.** Scroll-follows-focus brings the
   focused column fully into view, but at a low zoom (many units
   per viewport) a wide column may exceed the viewport entirely.
   Decide whether `scrollIntoView` aligns the column's leading edge,
   or whether deep zoom-out should itself be bounded so the focused
   column always fits.
9. **Fragment length limits.** A large ad-hoc board with long URI
   tile refs can push the fragment past comfortable URL lengths.
   v1 accepts it (browsers tolerate long fragments); revisit if a
   compaction step or a "this is large, save it" nudge is wanted.
10. **Stored board with a fragment.** If a URL names a stored board
    *and* carries a fragment, the fragment currently has no role
    (the stored board's `focus` lives in its `board` row). A future
    use could let the fragment override focus or zoom on a stored
    board for a one-off shared link; left out of v1, where the two
    paths are strictly disjoint.

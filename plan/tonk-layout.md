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
pointed at a branch entity (in v1; the `kind` field leaves room for
other tile bodies later). The layout itself — columns, tiles, sizes,
focus, scroll position — is persisted to the dialog database as
**normalized entities**, so a reload (or another device) reconstructs
the exact workspace.

It ships as a new `tonk-layout` crate.

## Target usage

```html
<tonk-layout space="home" branch="main" workspace="default"></tonk-layout>
```

No children. The element subscribes to its layout entities on the
branch, builds the strip, and mounts a `<tonk-display>` inside each
tile. An empty workspace renders an empty strip with an "add column"
affordance.

## Layout model (niri semantics)

```
            viewport (scrolls horizontally)
   ┌───────────────────────────────────────────────┐
   │  column 0     column 1        column 2        │ ...→ infinite
   │ ┌─────────┐  ┌──────────┐  ┌──────────────┐   │
   │ │ tile A  │  │  tile C  │  │   tile E     │   │
   │ ├─────────┤  └──────────┘  ├──────────────┤   │
   │ │ tile B  │                │   tile F     │   │
   │ └─────────┘                └──────────────┘   │
   └───────────────────────────────────────────────┘
```

- **Strip** — an ordered list of **columns**. Scrolls horizontally;
  width is unbounded.
- **Column** — an ordered list of **tiles** stacked vertically, plus a
  `width` (a fraction of the viewport, e.g. `0.5`). All tiles in a
  column share the column width.
- **Tile** — one cell. Carries a `height` (fraction of column height
  when sharing with sibling tiles), a `kind` (the content type — v1
  recognises `display`), and a content descriptor used to mount the
  tile body.
- **Focus** — exactly one tile is focused. Focus drives scroll: niri
  keeps the focused column in view (centred / nearest-edge).
- **Workspace** — a named strip. One branch can hold several
  (`default`, `scratch`, …); the `workspace` attribute selects one.

Sizing is **relative**, never pixel-absolute, so the same layout
restores correctly at any viewport size. Niri's preset column widths
(⅓, ½, ⅔, full) are the width values we cycle through.

## Data model — normalized entities

Three concepts on the branch. They are declared once per repository
(asserted-notation), exactly like the `view` concept that
`<tonk-display>` depends on.

```yaml
concept!: &workspace
  description: A named niri-style strip
  with:
    name:
      description: Workspace name (selects which strip to render)
      the: xyz.tonk.layout/workspace-name
      as: text
    focus:
      description: Currently focused tile
      the: xyz.tonk.layout/workspace-focus
      as: entity
      cardinality: one

concept!: &column
  description: A vertical stack of tiles within a workspace
  with:
    workspace:
      the: xyz.tonk.layout/column-workspace
      as: entity
    order:
      description: Lexicographic ordering key within the strip
      the: xyz.tonk.layout/column-order
      as: text
    width:
      description: Column width as a fraction of the viewport (0..1)
      the: xyz.tonk.layout/column-width
      as: float

concept!: &tile
  description: One cell; mounts a presentation body chosen by `kind`
  with:
    column:
      the: xyz.tonk.layout/tile-column
      as: entity
    order:
      description: Lexicographic ordering key within the column
      the: xyz.tonk.layout/tile-order
      as: text
    height:
      description: Tile height as a fraction of the column (0..1)
      the: xyz.tonk.layout/tile-height
      as: float
    kind:
      description: Content kind; v1 recognises "display"
      the: xyz.tonk.layout/tile-kind
      as: text
    entity:
      description: For kind=display — the entity the tile renders
      the: xyz.tonk.layout/tile-entity
      as: entity
    view:
      description: For kind=display — the view name
      the: xyz.tonk.layout/tile-view
      as: text
    model:
      description: For kind=display — the model name
      the: xyz.tonk.layout/tile-model
      as: text
```

Notes on the model:

- **`order` is a lexicographic text key**, not a number. Inserting an
  entity between two neighbours picks a key that sorts strictly
  between them; with strings there's no precision floor, so
  subdivision works indefinitely. LexoRank-style: a fixed printable
  ASCII alphabet, midpoint of two keys is found by character-wise
  bisection. Implemented in a tiny in-crate module — no external
  crate needed.
- **References point upward** (`tile.column`, `column.workspace`) so a
  child can be inserted/removed with a single assertion and the parent
  never needs rewriting.
- **`workspace.focus`** is a cardinality-one pointer to a tile entity;
  re-asserting it retracts the previous value automatically (the
  git-ref pattern from the notation guide).
- A tile that loses its content (entity retracted) is still a valid
  empty tile until explicitly removed.
- **`kind` defaults to `display`** when reading; v1 only renders that
  value. Future kinds (a `<tonk-concept>` list, static HTML, an
  inspector) are additive: new kind value + new mount path, no
  schema migration of existing tiles.
- Entities get **stable identities** via client-minted ULIDs (see
  next section) so edits target the same entity instead of spawning
  a new one each time the body content changes.

### Stable identity

Every workspace/column/tile entity is created with a client-minted
ULID embedded as the `this:` URI:

```yaml
column!:
  this: id:01HMX...
  workspace: id:01HMW...
  order: "n"
  width: 0.5
```

The `id:<ulid>` form is a direct URI literal; the analyser does *not*
content-address the body, so subsequent edits to the same ULID target
the same entity. Without this, dialog's default behaviour
(`Entity::of(&body_digest)`) would compute a fresh entity every time
a field changed — a column re-asserted with a new `order` would
become a new entity, orphaning the old.

Two devices independently creating "the first column" mint distinct
ULIDs — distinct entities, no spurious merge. That is the right
behaviour for v1; cross-device deduplication of "should-be-the-same"
entities requires consensus and is out of scope.

Workspaces additionally get a `name!` binding so the `workspace`
attribute can be a logical name rather than a ULID URI:

```yaml
workspace!:
  this: id:01HMW...
  name: "default"

name!:
  this:   id:default-workspace
  entity: id:01HMW...
```

The element resolves `workspace="default"` by looking up
`id:default-workspace` through dialog's name table — same pattern as
`<tonk-display>`'s `view="basic"` resolution.

### Why normalized rather than a JSON blob

Per-attribute merge: two devices dragging different columns, or
resizing different tiles, commit disjoint claims and merge cleanly on
sync. A single JSON-blob entity would re-hash on every edit and lose
one side's change. The cost is more query/assemble code, accepted here.

## Reading layout state — subscriptions

`<tonk-layout>` opens **three** live SSE subscriptions against the
worker's `/query` route (same machinery as `<tonk-concept>`: a `POST`
with `Accept: text/event-stream`, frames are `Vec<Conclusion>` JSON,
cancellation via `AbortController`):

1. **Workspace subscription** — the `workspace` row whose `name`
   equals the `workspace` attribute. Frame carries `focus`.
2. **Columns subscription** — all `column` rows whose `workspace`
   equals the resolved workspace entity.
3. **Tiles subscription** — all `tile` rows whose `column` is one of
   the workspace's columns.

A reusable **reconciler** (see "Rendering") folds the latest frame of
each subscription into an in-memory `Layout` tree, sorts columns and
tiles by `order` (lex compare), and patches the DOM in place —
preserving each tile's `<tonk-display>` node identity so a layout
change never tears down and remounts a healthy tile.

Because every layout write goes through `/evaluate` (which re-polls
subscriptions on commit), the WM is **reactive across tabs and
devices** for free: move a column on one screen, it moves on the
other.

## Writing layout state — direct writes

There is no local optimistic-state machine and no pending-override
bookkeeping. Mutations build an asserted-notation document, POST it
to `/evaluate`, and rely on the resulting subscription frame to
update the DOM.

- **Discrete actions** (open tile, close tile, move column/tile,
  focus change) write immediately: one POST per action.
- **Continuous actions** (drag-resize a column or tile) update the
  DOM directly on each `pointermove` (inline CSS), then **debounce
  the write**: coalesce into a single `/evaluate` flushed ~200 ms
  after the pointer goes idle (or on `pointerup`). Debouncing here
  is about avoiding ~60 writes per second, not about optimistic
  correctness. Implemented with `setTimeout` + cancellation,
  mirroring the `AbortController` pattern already in `tonk-concept`.
- **Batching:** a single user action that touches several entities
  (e.g. moving a tile between columns rewrites `tile.column` +
  `tile.order`) goes in **one** `/evaluate` document = one dialog
  transaction = atomic.

**Concurrent-writer flicker.** If another tab/device commits to the
same column while the user is dragging it, the arriving frame will
patch the DOM to the remote value for one frame before the next
`pointermove` patches it back. We accept this for v1; revisit if
anyone reports the artifact in practice. A future pending-override
map can address it without protocol changes.

A small `writer` module owns: building the notation document for each
mutation and the debounce timer.

## Element shape

```html
<tonk-layout
    [workspace="<name>"]
    [space="<space>"]
    [branch="<branch>"]>
</tonk-layout>
```

| Attribute | Required | Default | Meaning |
|---|---|---|---|
| `workspace` | no | `"default"` | Which named strip to render. |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

All observed; changing any aborts the three subscriptions, clears the
strip, and restarts — same teardown/restart discipline as
`<tonk-concept>` / `<tonk-display>`.

## Interaction (niri keybindings + pointer)

Keyboard (focus must be within the host; the element listens on its
own root):

| Key | Action |
|---|---|
| `←` / `→` | Move focus to previous / next column. |
| `↑` / `↓` | Move focus up / down within the focused column. |
| `Ctrl+←/→` | Move the focused column left / right in the strip. |
| `Ctrl+↑/↓` | Move the focused tile up / down within its column. |
| `R` | Cycle the focused column through preset widths (⅓ ½ ⅔ 1). |
| `Q` | Close the focused tile. |
| `Enter` | Open a new tile (prompt for entity/view — see below). |

Pointer: click a tile to focus it; drag the gap between columns or
tiles to resize; horizontal wheel / trackpad scrolls the strip.

**Opening a tile** needs an entity to display. v1: a `<wa-dialog>`
(Web Awesome) with inputs for `entity`, `model`, `view` — these
build a `tile` row with `kind: display`. A richer picker (browse
branch entities) is a follow-up. Authors can also seed tiles by
asserting `tile` rows directly.

Focus changes scroll the strip so the focused column is fully visible
(niri's "centre / nearest edge" behaviour) — a CSS
`scroll-behavior: smooth` container plus `scrollIntoView` on the
focused column suffices for v1.

## Tile content — `kind=display` mounts `<tonk-display>`

Each tile's body is chosen by the `kind` field.

**`kind: "display"` (v1):** a `<tonk-display>` is created with
`document.createElement` and configured by `set_attribute` from the
tile row's `entity` / `view` / `model` fields, plus the WM's own
`space` / `branch`. `<tonk-display>` then owns its entity + view
subscriptions and its own `data-state`. The WM never touches a tile's
inner DOM — it only manages geometry, focus, and the descriptor.

When a `display` tile row's content fields change, the WM calls
`set_attribute` on the existing `<tonk-display>` (which already
restarts its flows on attribute change) rather than remounting it.

**Unknown `kind`:** rendered as a placeholder with
`data-state="error"`. Adding a new kind later is additive — a new
mount routine in `reconcile.rs`, no change to existing tiles.

## Rendering

The host gets a non-shadow light DOM tree the WM owns entirely:

```
<tonk-layout data-state="ready">
  <div class="niri-strip">                <!-- scroll container -->
    <div class="niri-column" data-order=…> <!-- flex column, width: Nfr -->
      <div class="niri-tile" data-focused>  <!-- height: Nfr -->
        <tonk-display entity=… view=… />
      </div>
      …
    </div>
    …
  </div>
</tonk-layout>
```

- Column width and tile height come from the `width` / `height`
  fractions, written as inline `flex` / CSS custom properties so the
  browser does the pixel math; the layout stays resolution-independent.
- The **reconciler** keys columns by entity URI and tiles by entity
  URI. On each merged frame it: (a) removes DOM nodes whose entity
  vanished, (b) inserts nodes for new entities, (c) reorders by
  `order`, (d) updates `width`/`height`/`focused`/descriptor on
  survivors in place. Node identity of healthy tiles is preserved, so
  `<tonk-display>` subscriptions are never needlessly dropped.
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
| `empty` | Workspace has zero columns. |
| `error` | Query / network failure. |

Custom events (bubbling + composed, for diagnostics and host
integration):

| Event | When | Detail |
|---|---|---|
| `tonk-layout:connected` | Subscriptions opened | `{ workspace }` |
| `tonk-layout:layout` | Strip reconciled | `{ columns, tiles }` |
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
  src/model.rs           # Layout / Column / Tile structs; sort + reconcile-into-tree
  src/order.rs           # LexoRank-style ASCII midpoint / between
  src/ulid.rs            # tiny ULID generator (wasm32-friendly)
  src/resolve.rs         # query builders: workspace / columns / tiles  (native-testable)
  src/reconcile.rs       # frames → Layout; in-place DOM patch (wasm32)
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

Optionally expose a `tonk-ui` route
(`/space/{space}/branch/{branch}/layout/{workspace}`) that mounts the
element via the imperative-slot pattern used for `<tonk-display>` in
`tonk-ui/src/components/`.

## Implementation order

1. **Skeleton crate** — `tonk-layout` with `register()`, the
   `CustomElement` impl observing `workspace`/`space`/`branch`,
   reflecting `data-state="loading"`. Registered in `ui.rs`. Renders
   an empty strip.
2. **`order.rs` + `ulid.rs`** — LexoRank midpoint/between and ULID
   minting. Pure logic, native-tested.
3. **`model.rs`** — `Layout` / `Column` / `Tile`; folding three frame
   sets into a sorted tree (lex sort by `order`). Native-tested.
4. **`resolve.rs`** — the three query builders. Native-tested.
5. **Read path** — open the three subscriptions, wire frames through
   `model.rs`, set `data-state`. Strip renders read-only from
   branch data; no interaction yet.
6. **`reconcile.rs`** — in-place DOM patch with node-identity
   preservation; mount a `<tonk-display>` per tile.
7. **`writer.rs`** — notation-document builders + `/evaluate` POST for
   discrete mutations (open/close tile, move column/tile, set focus).
8. **`interact.rs`** — keyboard bindings wired to `writer` mutations;
   click-to-focus.
9. **Resize + debounce** — pointer-drag resize with direct DOM
   update and the debounced flush.
10. **Open-tile `<wa-dialog>`** — the entity/model/view prompt;
    builds a `tile` row with `kind: display`.
11. **Scroll-follows-focus** — `scrollIntoView` on focus change.
12. **`tonk-ui` route** (optional) — mount via the imperative-slot
    pattern.

Steps 1–6 land a read-only, branch-driven niri strip; 7–11 make it
interactive; the rest is polish.

## Tests

Native (no DOM, `#[dialog_common::test]`, `it_<verb_phrase>` naming):

- `order`: `between(a, b)` returns a key strictly between `a` and
  `b` for adjacent keys; repeated `between` of one neighbour
  doesn't blow up; bisecting a long shared prefix terminates.
- `resolve`: workspace query constrains `name`; columns query
  constrains `workspace`; tiles query constrains `column`.
- `model`: frames fold into a correctly **sorted** strip;
  `between`-inserting a column between neighbours places it in the
  right slot under lex compare; an orphan tile (column missing) is
  dropped or parked predictably.
- `writer`: open/close/move/resize produce the expected notation
  document; a multi-entity move emits one document; every
  newly-minted entity carries a `this: id:<ulid>` URI.
- debounce: rapid resize events coalesce into a single flush.

WASM (real DOM, `wasm_bindgen_test` via the same macro):

- a three-column / mixed-stack frame renders the expected strip.
- a layout frame that moves a column reorders the DOM **without**
  remounting an unaffected tile's `<tonk-display>`.
- a tile whose entity vanishes is removed; a new entity is inserted.
- `data-state` goes `loading` → `ready` → `empty` correctly.
- focusing a tile sets `data-focused` and scrolls it into view.
- resizing a column updates inline sizing and, after the debounce,
  posts exactly one `/evaluate`.

## Open questions

1. **Workspace bootstrapping.** If the `workspace` attribute names a
   workspace with no entity yet, does `<tonk-layout>` create it
   on first interaction, or require it pre-asserted? Recommend
   **lazy-create on first tile open** so a fresh branch just works —
   the open-tile transaction mints the workspace ULID, the
   `name!` binding, and the first column + tile in one
   `/evaluate` document. The risk: a typo in `workspace=` silently
   creates a new workspace rather than failing loudly. Mitigation
   ideas welcome.
2. **Scroll position persistence.** Niri derives scroll from focus;
   we do too, so scroll is *not* a stored field. Revisit if
   free-scroll (focus-independent) is wanted.
3. **`open_sse` ownership.** Currently lives in `tonk-concept`.
   Depending on `tonk-concept` just for it is acceptable short-term;
   a `tonk-rt`/`tonk-template` shared crate is the cleaner home if
   more plumbing gets shared (the `tonk-display` plan already noted
   this extraction).
4. **Future tile kinds.** `kind` is open; v1 only knows `display`.
   Likely future values: `concept` (mount a `<tonk-concept>` list
   over a query), `html` (static markup), `inspector` (debug view of
   the tile's entity). Each is additive — defer until a concrete
   user appears.

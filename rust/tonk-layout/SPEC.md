# `<tonk-layout>` — niri-style tiling workspace

A custom element that renders a **strip of columns**, each a vertical
stack of tiles, modelled on the [niri](https://github.com/YaLTeR/niri)
tiling window manager. Each tile mounts a `<tonk-display>` pointed at
a branch entity. The strip — columns, tiles, sizes, focus — is
persisted as **normalized entities** on the branch, so a reload on
the same device or any other one reconstructs the exact workspace.

The element is registered by `tonk-layout`'s `register()` (the shell
does this at startup; pages don't have to). Once registered, drop the
tag anywhere in the document.

## Shape

```html
<tonk-layout [workspace="<name>"]
             [space="<space>"]
             [branch="<branch>"]>
</tonk-layout>
```

No children — the element owns the entire subtree, building it from
the branch.

| Attribute | Required | Default | Meaning |
|---|---|---|---|
| `workspace` | no | `"default"` | Logical name of the strip to render. Resolves through dialog's name table to a workspace entity. |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

Changing any of these after the element is connected aborts the
current subscriptions, clears the strip, and restarts against the new
target — same teardown/restart discipline as `<tonk-concept>` /
`<tonk-display>`.

## Layout model

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
- **Focus** — exactly one tile is focused at a time. Focus drives
  scroll: the focused column is brought fully into view (centred /
  nearest-edge).
- **Workspace** — a named strip. One branch can hold several
  (`default`, `scratch`, …); the `workspace` attribute selects one.

Sizing is **relative**, never pixel-absolute, so the same layout
restores correctly at any viewport size. The preset column widths
(⅓, ½, ⅔, full) are the values the `R` key cycles through.

## Data model

The element reads and writes three concepts on the branch. They are
declared once per repository — drop them into a dialog-yaml document
under `concept!:` the same way `<tonk-display>` depends on the
`view` concept:

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

### Ordering keys

`column.order` and `tile.order` are **lexicographic text keys**, not
numbers. Inserting between two neighbours picks a key that sorts
strictly between them; with strings there's no precision floor, so
subdivision works indefinitely. The element uses a fixed printable
ASCII alphabet and finds the midpoint of two keys by character-wise
bisection — the same approach as LexoRank. Authors writing
notation by hand can use plain ASCII letters: `"a"`, `"b"`, `"c"`,
or `"a"`, `"n"`, `"z"` for a coarser split.

### Stable identity

Every workspace / column / tile entity is created with a
client-minted ULID embedded as the `this:` URI:

```yaml
column!:
  this: id:01HMX...
  workspace: id:01HMW...
  order: "n"
  width: 0.5
```

The `id:<ulid>` form is a direct URI literal — the analyser does
*not* content-address the body, so subsequent edits to the same
ULID target the same entity. Without this, dialog's default
behaviour (`Entity::of(&body_digest)`) would compute a fresh
entity every time a field changed, so a column re-asserted with a
new `order` would orphan its tiles.

Two devices independently creating "the first column" mint distinct
ULIDs — distinct entities, no spurious merge. That is the right
behaviour for v1; cross-device deduplication of "should-be-the-same"
entities requires consensus and is out of scope.

### Workspace name resolution

The `workspace` attribute is matched against the workspace concept's
`name` field. The element subscribes to the workspace concept with
`name = "<attribute value>"` pinned as a constant, picks the first
matching row, and uses its `this` URI as the parent reference for
column / tile queries. Same pattern as `<tonk-display>`'s
`view="basic"` resolution — concept-field filter, not a name-table
lookup.

```yaml
workspace!:
  this: id:01HMW...
  name: "default"
```

A workspace's ULID is internal — authors and other tools address
the workspace by `name`, which is human-meaningful and stable
across re-asserts of the workspace entity.

### Seeding a workspace by hand

A complete two-tile workspace, asserted directly:

```yaml
workspace!:
  this: id:01HMW000000000000000000000
  name: "default"

column!:
  this:      id:01HMC000000000000000000000
  workspace: id:01HMW000000000000000000000
  order:     "n"
  width:     0.5

tile!:
  this:    id:01HMT000000000000000000000
  column:  id:01HMC000000000000000000000
  order:   "n"
  height:  1.0
  kind:    "display"
  entity:  id:01HENT00000000000000000000
  model:   "person"
  view:    "card"
```

Drop that into an `/evaluate` request and `<tonk-layout
workspace="default">` will render a single column of one tile.

### Why normalized rather than a JSON blob

Per-attribute merge: two devices dragging different columns, or
resizing different tiles, commit disjoint claims and merge cleanly on
sync. A single JSON-blob workspace entity would re-hash on every
edit and lose one side's change.

## Bootstrapping an empty workspace

If the `workspace` attribute names a workspace with no entity yet,
the element renders an empty strip (state `empty`) with an
"add column" affordance. The first open-tile action **lazy-mints**
the workspace entity, its `name!` binding, and the first column /
tile in a single `/evaluate` document — atomic.

This means a fresh branch + `<tonk-layout>` "just works" with no
ceremony. The trade-off: a typo in `workspace="defualt"` silently
creates a new empty workspace rather than failing loudly. Pre-asserting
a workspace (above) sidesteps the risk.

## Tile content

Each tile's body is chosen by the `kind` field.

### `kind: "display"` (v1)

The element mounts a `<tonk-display>` configured from the tile row's
`entity` / `model` / `view` fields, plus the WM's own `space` /
`branch`:

```html
<tonk-display entity="<tile.entity>"
              model="<tile.model>"
              view="<tile.view>"
              space="<host.space>"
              branch="<host.branch>" />
```

`<tonk-display>` then owns its own subscriptions and `data-state`.
The WM never touches a tile's inner DOM — it only manages geometry,
focus, and the descriptor.

When a `display` tile row's content fields change, the element calls
`set_attribute` on the existing `<tonk-display>` (which already
restarts its flows on attribute change) rather than remounting it.
A layout change that reorders columns or tiles does **not** remount
healthy `<tonk-display>` instances either — node identity is
preserved across reconciliations.

### Unknown `kind`

A tile with a `kind` value the element does not recognise is rendered
as a placeholder with `data-state="error"`. Adding a new kind later
is purely additive (new mount path, no schema migration).

## Interaction

### Keyboard

Focus must be within the host; the element listens on its own root.

| Key | Action |
|---|---|
| `←` / `→` | Move focus to previous / next column. |
| `↑` / `↓` | Move focus up / down within the focused column. |
| `Ctrl+←/→` | Move the focused column left / right in the strip. |
| `Ctrl+↑/↓` | Move the focused tile up / down within its column. |
| `R` | Cycle the focused column through preset widths (⅓ ½ ⅔ 1). |
| `Q` | Close the focused tile. |
| `Enter` | Open a new tile — see "Opening a tile". |

### Pointer

- **Click** a tile to focus it.
- **Drag** the gap between columns or tiles to resize. The DOM
  updates inline on each `pointermove`; the persisted `width` /
  `height` is debounced (~200 ms after the pointer goes idle, or on
  `pointerup`).
- **Horizontal wheel / trackpad** scrolls the strip.

### Opening a tile

`Enter` (or the empty-strip affordance) opens a `<wa-dialog>` with
inputs for `entity`, `model`, and `view`. Submitting builds a
`tile!` row with `kind: display` and posts it. A richer
branch-aware picker is a follow-up; authors can also seed tiles by
asserting `tile` rows directly.

### Focus follows scroll

Focus changes scroll the strip so the focused column is fully
visible (niri's "centre / nearest edge" behaviour). Scroll position
is **not persisted** — it is derived from focus, so a reload simply
scrolls to the saved focus tile.

## Persistence behaviour

There is no local-only state. Every mutation — open a tile, move a
column, resize, focus — is written as an assertion document to
`/evaluate` and reaches the element back through its subscriptions.

- **Discrete actions** write immediately, one POST per action.
- **Continuous actions** (drag-resize) update inline CSS on each
  `pointermove`, then coalesce into a single debounced `/evaluate`
  ~200 ms after the pointer goes idle (or on `pointerup`).
- **Multi-entity actions** (moving a tile between columns rewrites
  both `tile.column` and `tile.order`) go in one `/evaluate`
  document — one dialog transaction — atomically.

Because every write goes through `/evaluate`, the workspace is
**reactive across tabs and devices** for free: move a column on one
screen, it moves on the other.

**Concurrent-writer flicker.** If a second tab commits to the same
column while a drag is in progress, the arriving frame patches the
DOM to the remote value for one frame before the next `pointermove`
patches it back. v1 accepts this; a pending-override map can
address it later without protocol changes.

## Rendered DOM

The host gets a non-shadow light DOM tree the element owns entirely:

```
<tonk-layout data-state="ready">
  <div class="niri-strip">                       <!-- scroll container -->
    <div class="niri-column" data-order="...">   <!-- flex column, width: Nfr -->
      <div class="niri-tile" data-focused>       <!-- height: Nfr -->
        <tonk-display entity="..." view="..." />
      </div>
      ...
    </div>
    ...
  </div>
</tonk-layout>
```

Column width and tile height come from the persisted fractions,
written as inline `flex` / CSS custom properties so the browser
does the pixel math; the layout stays resolution-independent.

Web Awesome elements (`<wa-dialog>`, `<wa-button>`, `<wa-icon>`,
`<wa-spinner>`, `<wa-callout>`) provide the chrome — they auto-register
via the loader already in the page.

## DOM state and events

| `data-state` | Meaning |
|---|---|
| `loading` | Subscriptions opening, no frame yet. |
| `ready` | Strip rendered. |
| `empty` | Workspace has zero columns (or doesn't exist yet — see "Bootstrapping"). |
| `error` | Query / network failure. |

Custom events (all bubble and are composed):

| Event | When | Detail |
|---|---|---|
| `tonk-layout:connected` | Subscriptions opened | `{ workspace }` |
| `tonk-layout:layout` | Strip reconciled | `{ columns, tiles }` |
| `tonk-layout:focus` | Focused tile changed | `{ tile }` |
| `tonk-layout:error` | Failure | `{ kind, message }` |

## Known limitations

- **Typo-creates-empty-workspace.** Pre-assert workspaces if loud-fail
  matters for your deployment.
- **Concurrent-drag flicker.** A remote commit during a local drag
  flashes the remote value for one frame.
- **Single tile kind.** v1 recognises `kind: "display"` only.
  `concept` (a `<tonk-concept>` list), `html` (static markup), and
  `inspector` (debug view) are likely future values; each is
  additive.
- **Cross-device "first column" duplicates.** Two offline devices
  each creating "the first column" on the same workspace mint two
  distinct columns — there is no consensus pass.
- **No free-scroll.** Scroll follows focus; there is no
  focus-independent scroll-position storage.

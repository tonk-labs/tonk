# `<tonk-layout>`

A tiling window manager as a custom element. It arranges tiles on
an infinite horizontal scrollable strip of columns — each column a
vertical stack of tiles — and persists the whole layout (columns,
tiles, sizes, focus) to the branch as normalized entities. Every
tile mounts a [`<tonk-display>`](../tonk-display) pointed at a
branch entity.

The element is registered by the Tonk UI shell at startup
(`tonk_layout::register()` in `rust/tonk-ui/src/bin/ui.rs`). Once
registered, drop the tag anywhere:

```html
<tonk-layout workspace="default" space="home" branch="main"></tonk-layout>
```

| Attribute   | Default     | Meaning                                  |
|-------------|-------------|------------------------------------------|
| `workspace` | `"default"` | Which named strip to render.             |
| `space`     | `"home"`    | Repository space the queries run against.|
| `branch`    | `"main"`    | Branch the queries run against.          |

All three are observed: changing any one tears down the
subscriptions and rebuilds the strip.

See [`/plan/tonk-layout.md`](../../plan/tonk-layout.md) for the
full design rationale.

## Concepts the branch must declare

`<tonk-layout>` reads and writes three concepts. They must exist
on the branch before the element can render or be edited. Declare
them once per repository, in asserted notation (paste into a
`<tonk-code>` cell or `POST` to the `/evaluate` route). The
attribute URIs below are fixed — the element's queries hard-code
them.

```yaml
# ── workspace ────────────────────────────────────────────────
attribute!: &workspace-name
  description: Workspace name (selects which strip to render)
  the:         xyz.tonk.layout/workspace-name
  as:          text
  cardinality: one

attribute!: &workspace-focus
  description: Currently focused tile
  the:         xyz.tonk.layout/workspace-focus
  as:          entity
  cardinality: one

concept!: &workspace
  description: A named strip
  with:
    name:  workspace-name
    focus: workspace-focus

# ── column ───────────────────────────────────────────────────
attribute!: &column-workspace
  description: Workspace the column belongs to
  the:         xyz.tonk.layout/column-workspace
  as:          entity
  cardinality: one

attribute!: &column-order
  description: Position of the column in the strip (sortable)
  the:         xyz.tonk.layout/column-order
  as:          float
  cardinality: one

attribute!: &column-width
  description: Column width in major grid units (1 unit = 64px)
  the:         xyz.tonk.layout/column-width
  as:          unsigned-integer
  cardinality: one

concept!: &column
  description: A vertical stack of tiles
  with:
    workspace: column-workspace
    order:     column-order
    width:     column-width

# ── tile ─────────────────────────────────────────────────────
attribute!: &tile-workspace
  description: Workspace the tile belongs to (denormalized from
    the column, so one query returns all of a workspace's tiles)
  the:         xyz.tonk.layout/tile-workspace
  as:          entity
  cardinality: one

attribute!: &tile-column
  description: Column the tile belongs to
  the:         xyz.tonk.layout/tile-column
  as:          entity
  cardinality: one

attribute!: &tile-order
  description: Vertical position within the column (sortable)
  the:         xyz.tonk.layout/tile-order
  as:          float
  cardinality: one

attribute!: &tile-height
  description: Tile height in major grid units (1 unit = 64px)
  the:         xyz.tonk.layout/tile-height
  as:          unsigned-integer
  cardinality: one

attribute!: &tile-entity
  description: Entity the tile's <tonk-display> renders
  the:         xyz.tonk.layout/tile-entity
  as:          entity
  cardinality: one

attribute!: &tile-view
  description: View name forwarded to the tile's <tonk-display>
  the:         xyz.tonk.layout/tile-view
  as:          text
  cardinality: one

attribute!: &tile-model
  description: Concept name forwarded to the tile's <tonk-display>
  the:         xyz.tonk.layout/tile-model
  as:          text
  cardinality: one

concept!: &tile
  description: One cell; mounts a <tonk-display>
  with:
    workspace: tile-workspace
    column:    tile-column
    order:     tile-order
    height:    tile-height
    entity:    tile-entity
    view:      tile-view
    model:     tile-model
```

Notes:

- **`order` is a float.** To insert a column or tile between two
  others, give it an `order` halfway between its neighbours — no
  renumbering needed (fractional indexing).
- **`width` / `height` are integer counts of major grid cells**
  (1 cell = 64px — the bright dots of the graph-paper background).
  A column `width` of `12` is 768px; a tile `height` of `5` is
  320px. Columns lay end to end and the strip scrolls when they
  overflow; tiles have fixed grid heights. The strip's scroll
  snaps to the grid.
- **`tile.workspace`** is a denormalized copy of the tile's
  column's workspace. Keep the two in sync when you move a tile
  across workspaces.

## Seeding a workspace

The element renders whatever the branch holds; it does not create
a workspace for you (yet). To get something on screen, assert one
`workspace` row, at least one `column`, and one `tile`. This
example builds a `default` workspace with two columns — a wide one
holding two stacked tiles, and a narrow one with a single tile.

It assumes a `greeting` concept and a `did:key:zGreeting…` entity
already exist (see the [`<tonk-display>`](../tonk-display) docs for
how to create those).

An entity is derived from its body. Re-asserting an anchor
(`&name`) with a *different* body produces a *new* entity and
re-points the name — which would orphan anything still
referencing the old one. So give each anchored entity its body
**once**; to change a field afterwards, bind the entity with
`this:` and assert against it (that updates in place, no
re-derivation). The snippets below run in order, each against the
branch the previous left behind.

```yaml
# The workspace. `&ws-default` names the entity so the columns
# and tiles can reference it by the bare symbol `ws-default`.
workspace!: &ws-default
  name: "default"
  focus: id:nil
```

```yaml
# Two columns. `&col-left` / `&col-right` name the entities so
# the tiles can reference them. `order` sets strip position;
# `width` is the column width in grid units (1 unit = 64px), so
# `12` is 768px wide and `8` is 512px.
column!: &col-left
  workspace: ws-default
  order:     1.0
  width:     12

column!: &col-right
  workspace: ws-default
  order:     2.0
  width:     8
```

```yaml
# Tiles. Each names its workspace and column, an `order` within
# the column, a `height` in grid units, and the entity its
# <tonk-display> should render.
tile!: &tile-a
  workspace: ws-default
  column:    col-left
  order:     1.0
  height:    5
  entity:    did:key:z6MkEWJRjLtdeain8H18zgnfZLK6rNNxcrbqZcnYbfuWhg6J
  model:     greeting
  view:      "basic"

tile!: &tile-b
  workspace: ws-default
  column:    col-left
  order:     2.0
  height:    5
  entity:    did:key:z6MkEWJRjLtdeain8H18zgnfZLK6rNNxcrbqZcnYbfuWhg6J
  model:     greeting
  view:      "basic"

tile!: &tile-c
  workspace: ws-default
  column:    col-right
  order:     1.0
  height:    8
  entity:    did:key:z6MkEWJRjLtdeain8H18zgnfZLK6rNNxcrbqZcnYbfuWhg6J
  model:     greeting
  view:      "basic"
```

```yaml
# Set the initially focused tile. This updates the existing
# workspace entity in place: `this:` binds it (by name), and the
# assertion writes `focus`. Because `focus` is cardinality-one,
# re-running this with a different tile just moves focus.
workspace!:
  this:  ws-default
  focus: tile-a
```

## Viewing it

The Tonk UI shell exposes `<tonk-layout>` as a route:

```
/space/{space}/branch/{branch}/layout/{workspace}
```

So the workspace seeded above is at:

```
/space/home/branch/main/layout/default
```

The path segments map straight onto the element's attributes —
`{space}` → `space`, `{branch}` → `branch`, `{workspace}` →
`workspace`.

Once layout entities exist, the strip is live: editing a `column`
or `tile` row on the branch (from another tab, another device, or
a `<tonk-code>` cell) re-renders the strip immediately, because
every layout write re-polls the element's subscriptions.

## `data-state`

The element reflects its lifecycle onto the host so stylesheets
can react:

| `data-state` | Meaning                                    |
|--------------|--------------------------------------------|
| `loading`    | Subscriptions opening; no frame yet.       |
| `ready`      | Strip rendered.                            |
| `empty`      | Workspace resolved but has zero columns.   |
| `error`      | Query or network failure.                  |

## Status

The element currently renders a **read-only** strip driven by the
branch. Interaction — keyboard navigation, drag-resize, opening
and closing tiles from the UI — is not yet wired; see the
implementation order in [`/plan/tonk-layout.md`](../../plan/tonk-layout.md).
For now, edit the layout by asserting `column` / `tile` rows
directly.

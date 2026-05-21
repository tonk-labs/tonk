# `<tonk-layout>` — headless workspace primitive

A custom element that holds the **universal state and command
primitives** for a tile-based workspace: which tiles exist, what
content they render, which one is focused, and what linear order
they live in. It has **no rendered DOM of its own** — UIs that
present the workspace (a niri-style strip, a grid, a single-view
spotlight, …) ship as `<tonk-display>` view documents that wrap
`<tonk-layout>` and own all rendering, layout-specific gestures,
and any style-specific overlay state.

The element subscribes to its workspace's universal concepts on
the branch, listens for a small vocabulary of named effects
bubbling up from its subtree, and translates them into atomic
notation writes — applying lex-midpoint math, ULID minting, and
atomic lazy-bootstrap so view authors don't have to.

The element is registered by `tonk-layout`'s `register()` (the
shell does this at startup; view authors don't have to).

This document supersedes `SPEC.md` once the headless split lands;
see `plan/tonk-layout-headless-split.md` for the migration plan.

## Shape

```html
<tonk-layout [workspace="<name>"]
             [space="<space>"]
             [branch="<branch>"]>
</tonk-layout>
```

No children. The element renders nothing into its own subtree;
its purpose is to subscribe, fold, and dispatch.

| Attribute | Required | Default | Meaning |
|---|---|---|---|
| `workspace` | no | `"default"` | Logical name of the workspace. Resolves through the workspace concept's `name` field. |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

Changing any of these after the element is connected aborts
outstanding subscriptions, clears the folded snapshot, and
restarts against the new target — same teardown/restart
discipline as `<tonk-concept>` / `<tonk-display>`.

## Where the UI lives

A WM-style UI is a `<tonk-display>` view document whose template
body embeds `<tonk-layout>` as a child and renders the workspace
in whatever shape it likes. Switching between UIs is just
swapping the wrapping `<tonk-display>`'s `view=` attribute.

```html
<!-- niri-strip UI -->
<tonk-display view="niri-strip" workspace="default">
  <!-- view template body, defined elsewhere:
       <tonk-layout workspace="..."></tonk-layout>
       <div class="niri-strip">...</div>
       <script>...</script> -->
</tonk-display>
```

Each UI declares its own overlay concepts (e.g. niri declares
`niri-column` / `niri-placement` to record column structure and
per-tile placement). UI-specific writes (column resize, drag
reorder, etc.) bypass `<tonk-layout>` entirely — the view's JS
asserts the overlay concepts directly. The layout vocabulary is
only for things that touch the universal tile / workspace state.

Switching UIs leaves the inactive overlay rows latent in dialog;
switch back later and the layout is exactly as you left it.

## Data model

Two universal concepts. They are declared once per repository in
a dialog-yaml `concept!:` document the same way `<tonk-display>`
depends on the `view` concept.

```yaml
concept!: &workspace
  description: A workspace of tiles
  with:
    name:
      description: Workspace name (selects which workspace to render)
      the: xyz.tonk.layout/workspace-name
      as: text
    focus:
      description: Currently focused tile
      the: xyz.tonk.layout/workspace-focus
      as: entity
      cardinality: one

concept!: &tile
  description: One tile in a workspace; renders content via <tonk-display>
  with:
    workspace:
      description: Parent workspace
      the: xyz.tonk.layout/tile-workspace
      as: entity
    order:
      description: Lexicographic linear order within the workspace
      the: xyz.tonk.layout/tile-order
      as: text
    entity:
      description: Entity the tile renders (omit for concept-list tiles)
      the: xyz.tonk.layout/tile-entity
      as: entity
    view:
      description: View name for the tile body
      the: xyz.tonk.layout/tile-view
      as: text
    model:
      description: Model / concept name for the tile body
      the: xyz.tonk.layout/tile-model
      as: text
```

Two things to notice:

- **No column, height, kind, or any placement field.** Those are
  UI-overlay concerns and live in the view document's own
  overlay concepts.
- **`tile.entity` is optional.** Single-entity tiles populate it
  (the view's body is `<tonk-display entity={entity}
  view={view}>`). Concept-listing tiles leave it empty and rely
  on `view` naming a list-rendering view (e.g. `"concept-list"`)
  that internally uses `<tonk-concept source={model}>`.

### Ordering keys

`tile.order` is a **lexicographic text key**, not a number. The
element uses the same LexoRank-style fixed-alphabet midpoint
algorithm as today's column/tile ordering. Authors writing
notation by hand can use plain ASCII letters (`"a"`, `"n"`, `"z"`
for a coarse split).

### Stable identity

Tiles and workspaces are minted with client-side ULIDs embedded
as the `this:` URI, exactly as today:

```yaml
tile!:
  this:      id:01HMT000000000000000000000
  workspace: id:01HMW000000000000000000000
  order:     "n"
  view:      "card"
  model:     "person"
  entity:    id:01HENT00000000000000000000
```

The `id:<ulid>` form is a direct URI literal — the analyser does
*not* content-address the body, so subsequent edits to the same
ULID target the same entity. The same rationale as before
(without it, dialog's default behaviour would orphan a tile on
every `order` update).

### Workspace name resolution

The `workspace` attribute is matched against the workspace
concept's `name` field. The element subscribes to the workspace
concept with `name = "<attribute value>"` pinned as a constant,
picks the first matching row, and uses its `this` URI as the
parent reference for the tile subscription. Same pattern as
`<tonk-display>`'s `view="basic"` resolution.

### Seeding a workspace by hand

A complete two-tile workspace, asserted directly:

```yaml
workspace!:
  this: id:01HMW000000000000000000000
  name: "default"

tile!:
  this:      id:01HMT100000000000000000000
  workspace: id:01HMW000000000000000000000
  order:     "a"
  entity:    id:01HENT00000000000000000000
  view:      "card"
  model:     "person"

tile!:
  this:      id:01HMT200000000000000000000
  workspace: id:01HMW000000000000000000000
  order:     "n"
  entity:    id:01HENT00000000000000000001
  view:      "card"
  model:     "person"
```

Drop that into an `/evaluate` request and any UI view wrapping
`<tonk-layout workspace="default">` will see two tiles in the
expected order.

### Why normalized rather than a JSON blob

Per-attribute merge: two devices reordering different tiles, or
swapping different tile bodies, commit disjoint claims and merge
cleanly on sync. A single JSON-blob workspace would re-hash on
every edit and lose one side's change.

## Bootstrapping an empty workspace

If the `workspace` attribute names a workspace with no entity
yet, the element holds an empty folded snapshot. The first
`open-tile` effect **lazy-mints** the workspace entity, its
`name!` binding, and the first tile in a single `/evaluate`
document — atomic.

This means a fresh branch + `<tonk-layout>` "just works" with no
ceremony. The trade-off: a typo in `workspace="defualt"` silently
creates a new empty workspace rather than failing loudly.
Pre-asserting a workspace (above) sidesteps the risk.

## Effects vocabulary

Six named effects. UIs dispatch them as DOM `CustomEvent`s that
bubble up to `<tonk-layout>`. A small JS helper is shipped
alongside the element for view authors:

```js
const layout = document.querySelector('tonk-layout');
layout.emit('focus-next');
layout.emit('open-tile', { entity, view, model });
```

Under the hood, `emit()` wraps a `dispatchEvent(new
CustomEvent('tonk-layout/<name>', { detail: params, bubbles:
true, composed: true }))`. The element's root listener
dispatches by event type.

The event-name namespace is chosen to match a future
transient-concept transport (per `tonk-labs/tonk` PR #461). When
that lands, `emit()` swaps to writing transient concept
assertions via `/transact`; view call sites don't change.

### `tonk-layout/focus-tile`

Params: `target` (tile entity URI).

Asserts `workspace.focus = target`. No-op if `target` is already
focused.

### `tonk-layout/focus-prev` / `tonk-layout/focus-next`

No params.

Walks the universal linear order from the current focus to the
previous / next tile and asserts `workspace.focus` to it. No-op
if focus is already at the relevant boundary, or if no tile is
focused (use `focus-tile` to set an initial focus).

### `tonk-layout/open-tile`

Required: `view`, `model`.
Optional: `entity`, `before`, `after`.

Mints a fresh tile ULID, computes its `order` per the order-key
rules below, and asserts a new `tile!` row plus
`workspace.focus = <new-tile>` in one atomic `/evaluate`
document. If the workspace doesn't yet exist, the same document
also mints it (lazy bootstrap).

`entity` is optional: omit for concept-list-style tiles where
`view` and `model` are sufficient.

#### Order-key rules

Both `open-tile` and `reorder-tile` resolve their target order
key from the optional `before` / `after` params:

| `before` | `after` | Resolved range | Notes |
|---|---|---|---|
| set | set | midpoint(`after.order`, `before.order`) | Insert strictly between two tiles. |
| set | unset | midpoint(`prev(before).order`, `before.order`) | "Place before this tile." `prev(t)` is the tile immediately before `t` in current linear order, or sentinel-min if `t` is first. |
| unset | set | midpoint(`after.order`, `next(after).order`) | "Place after this tile." `next(t)` is the tile immediately after `t`, or sentinel-max if `t` is last. |
| unset | unset | midpoint(`last.order`, sentinel-max) | Append at the end. |

If `before` / `after` are both set but not adjacent, midpoint is
still computed against the supplied two — the caller opted in.
If a supplied tile reference doesn't resolve in the current
fold, the effect fails loudly via `tonk-layout:error`.

### `tonk-layout/close-tile`

Required: `target` (tile entity URI).

Retracts the tile row. If `target` was focused, advances focus
to the previous tile (or next if previous is gone, or null if
no tiles remain). All in one atomic document.

### `tonk-layout/reorder-tile`

Required: `target`.
Optional: `before`, `after`.

Computes a new `order` per the [order-key
rules](#order-key-rules) and asserts `tile.order` on `target`.
Used by UIs that want to surface "move tile" gestures.

### `tonk-layout/update-tile-content`

Required: `target`.
Optional: `entity`, `view`, `model`.

Asserts whichever of the three fields are provided on `target`.
Used to swap what a tile renders without remounting it.

## Atomic guarantees

Every effect produces exactly one `/evaluate` document.
`open-tile` (workspace bootstrap + tile + focus) and
`close-tile` (retract + focus advance) are the multi-statement
ones; both are atomic so they merge cleanly under concurrent
writers.

## Outbound events

The element dispatches three custom events (all bubble and are
composed):

| Event | When | Detail |
|---|---|---|
| `tonk-layout:changed` | A refold settled. | `{ workspace, focus, tile_count }` |
| `tonk-layout:focus` | The focused tile changed. | `{ tile }` |
| `tonk-layout:error` | Subscription / transport failure or unresolvable effect ref. | `{ kind, message }` |

UIs that need to react to focus changes (e.g. scroll the
focused tile into view) subscribe to `tonk-layout:focus`. UIs
that need a "subscriptions settled" indicator listen for
`tonk-layout:changed`.

The element exposes **no readable JS property** for the folded
state. UIs read tile / workspace rows directly from dialog via
`<tonk-concept>` — the same source `<tonk-layout>` reads from.

## Persistence behaviour

Same model as today: there is no local-only state. Every
mutation goes through `/evaluate` and reaches subscribers via
SSE. Discrete actions write immediately; continuous actions
(e.g. a niri view's drag-resize on `niri-column.width`) are the
view's responsibility to debounce — `<tonk-layout>` itself only
handles atomic discrete writes.

Because every write goes through `/evaluate`, the workspace is
reactive across tabs and devices for free: open a tile on one
device, it appears on the other.

## Concurrency

Same generation/lifecycle discipline as today: attribute
changes bump an internal generation counter, abort outstanding
subscriptions, and clear the folded snapshot. Effects spawned
by a superseded generation no-op before posting.

## Known limitations

- **Typo-creates-empty-workspace.** Pre-assert workspaces if
  loud-fail matters for your deployment.
- **No readable state property.** UIs that need synchronous
  "what's adjacent to T" must subscribe to tiles themselves via
  `<tonk-concept>` (or use `focus-prev` / `focus-next` to let
  the element resolve adjacency).
- **Cross-device "first tile" duplicates.** Two offline devices
  each opening "the first tile" on the same workspace mint two
  distinct tiles — there is no consensus pass.
- **DOM-event transport, not yet dialog-native.** The effects
  vocabulary is dispatched as DOM CustomEvents until PR #461's
  transient-concept transport lands. Event names are chosen so
  the swap is mechanical and view call sites are unaffected.
- **No headless usage outside a UI view.** With no rendered DOM,
  `<tonk-layout>` alone shows nothing on the page. A wrapping
  `<tonk-display>` view is required for any visible workspace.

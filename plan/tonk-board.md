# Tonk board UI

## Scope

A working board at `/space/:space/branch/:branch/board/:board`. The board renders as a horizontal strip of columns; each column is a vertical stack of tiles. Pulling past the bottom of a column reveals a launcher tile inline; picking an item from the launcher creates a new tile in that column. Closing a tile retracts it.

The path is **declarative-first**: most behavior lives in concept + view + rule definitions written in dialog yaml; custom elements only step in where dialog cannot express continuous gesture state.

## Reference

The visual model is a niri-inspired horizontal strip with pull-to-reveal at column ends, an inline app selector, fixed-height tiles, width presets per column, and horizontal scroll snapping.

Consumer elements dispatch `tonk-*` events that bubble through the routing tree; `<tonk-host>` performs IO. Commands are transient concepts consumed by deductive rules. Declarative view templates support `subject={field}` and cardinality-many iteration, while event handlers read transient concepts through the `dom.event.*` namespaces.

## Data model — v1 hierarchical

Three concepts. Each cardinality-many relation drives one level of template iteration.

```yaml
concept!: &board
  description: A named board
  with:
    name:
      the: xyz.tonk.board/name
      cardinality: one
      as: text
    column:
      the: xyz.tonk.board/column
      cardinality: many
      as: entity

concept!: &column
  description: A column on a board
  with:
    order:
      the: xyz.tonk.column/order
      cardinality: one
      as: text                      # LexoRank-style key
    width:
      the: xyz.tonk.column/width
      cardinality: one
      as: unsigned-integer          # grid units (1 unit ≈ 64px)
    tile:
      the: xyz.tonk.column/tile
      cardinality: many
      as: entity

concept!: &tile
  description: One tile in a column
  with:
    order:
      the: xyz.tonk.tile/order
      cardinality: one
      as: text                      # LexoRank key within its column
    view:
      the: xyz.tonk.tile/view
      cardinality: one
      as: text
    model:
      the: xyz.tonk.tile/model
      cardinality: one
      as: text
    entity:
      the: xyz.tonk.tile/entity
      cardinality: one
      as: entity
```

The board owns its columns through `board.column` cardinality-many. The column owns its tiles through `column.tile` cardinality-many. The tile carries presentation triple (view / model / entity) plus its in-column order. No back-references — each level only knows about its children.

### Trade-off (intentional)

The hierarchical shape lets the template engine iterate cleanly today (`subject={column}` then `subject={tile}` nested), but **bakes the layout style into the data model**. A whiteboard layout cannot reuse the same `tile` rows without reshape — the column relation is in the way.

### Future projection model (deferred)

The honest design is *flat tiles + projected concepts*. The schema would be:

```yaml
concept!: &board
  with:
    name: { ..., cardinality: one, as: text }
    tile: { ..., cardinality: many, as: entity }   # flat list of tiles

concept!: &tile
  with:
    column: { ..., cardinality: one, as: entity }  # layout overlay; optional
    order:  { ..., cardinality: one, as: text }
    view:   { ..., cardinality: one, as: text }
    model:  { ..., cardinality: one, as: text }
    entity: { ..., cardinality: one, as: entity }

concept!: &column
  with:
    order: { ... }
    width: { ... }

# Projection concepts — derived from the flat ones via deductive rules.
concept!: &board-view
  with:
    name:   { ... }
    column: { ..., cardinality: many, as: entity }

concept!: &column-view
  with:
    order: { ... }
    width: { ... }
    tile:  { ..., cardinality: many, as: entity }
```

With two rules:

```yaml
rule!:
  description: Project board with its columns (derived from tile membership).
  assert: board-view
  when:
    - assert: board
      where: { this: ?this, name: ?name, tile: ?tile }
    - assert: tile
      where: { this: ?tile, column: ?column }

rule!:
  description: Project column with its tiles (derived from tile.column refs).
  assert: column-view
  when:
    - assert: column
      where: { this: ?this, order: ?order, width: ?width }
    - assert: tile
      where: { this: ?tile, column: ?this }
```

Views would target `model=board-view` and `model=column-view` — same template shape as the hierarchical version. **The view templates do not change.** That's the whole point: the projection model preserves the consumer's worldview while letting the data model stay flat.

A whiteboard layout would project differently:

```yaml
# Hypothetical whiteboard overlay: tile has x/y instead of column.
concept!: &whiteboard-tile
  with:
    x: { the: xyz.tonk.whiteboard/x, as: unsigned-integer, cardinality: one }
    y: { the: xyz.tonk.whiteboard/y, as: unsigned-integer, cardinality: one }

rule!:
  description: Project board with its whiteboard tiles.
  assert: whiteboard-board-view
  when:
    - assert: board
      where: { this: ?this, name: ?name, tile: ?tile }
    - assert: whiteboard-tile
      where: { this: ?tile, x: ?x, y: ?y }
```

Same tile rows, different overlay concept, different projected view — three different layouts can all coexist on the same flat tile set.

**Migration path from hierarchical to flat:** the bootstrap document changes (write `tile!` records flat with `column` field instead of `column!: { tile: [...] }`); add the two rules; views can keep using `model=board-view` / `model=column-view` if we name the hierarchical concepts that way to start (so the rename never happens). I'd lean toward naming the hierarchical concepts the same as the eventual projected ones — `board-view` and `column-view` — so the cut-over is purely a data-side change.

**Open prerequisite:** deductive rules must reliably support cardinality-many output relations. This works today for the counter case (cardinality-one assertion via rule head) but the projection model needs `board-view.column: cardinality many` to be assertable by joining over `board.tile` + `tile.column`. If that's missing, the rule engine needs work first.

## Views

The view templates iterate the hierarchical relations. Three views.

### Board view — horizontal strip

```yaml
view!:
  model: board-view
  name: "basic"
  display: |
    <tonk-strip data-board={this}>
      <tonk-column subject={column} data-column={column}>
        <tonk-display entity={column} model=column-view view=basic />
      </tonk-column>
    </tonk-strip>
```

`<tonk-strip>` and `<tonk-column>` are custom elements (see below). The view passes the column entity down via a nested `<tonk-display>` that resolves the column's basic view.

### Column view — vertical tile stack

```yaml
view!:
  model: column-view
  name: "basic"
  display: |
    <div class="column-stack" style="--col-w: {width};">
      <div class="tile" subject={tile} data-tile={tile}>
        <tonk-display entity={tile} model=tile view=basic />
      </div>
    </div>
```

CSS variable interpolation in the `style` attribute lets the column width come from data. The renderer's recent attribute-vs-property fix handles this correctly: `style="--col-w: 12;"` is a plain attribute write.

### Tile view — bridge to per-entity rendering

```yaml
view!:
  model: tile
  name: "basic"
  display: |
    <tonk-display entity={entity} model={model} view={view} />
```

The tile concept carries `entity`/`model`/`view` strings; the nested `<tonk-display>` mounts the entity's own view. This is where the layout-side machinery hands off to per-tile rendering. **No tile-level chrome at this layer** — close buttons / focus indicators belong in the column view's tile wrapper.

### Tile chrome (close button, focus highlight) lives in the column view

The `.tile` wrapper div in the column view is where chrome lives. For example, with close-tile wired up:

```html
<div class="tile" subject={tile} data-tile={tile}>
  <button class="close" onclick=close-tile data-tile={tile}>×</button>
  <tonk-display entity={tile} model=tile view=basic />
</div>
```

The wrapper is universal layout chrome; the inner `<tonk-display>` is the user's actual app content.

## Custom elements

Two new custom elements, both in the `tonk-layout` crate (since that's where the layout layer already lives) or a new `tonk-board` crate if we want stricter separation.

### `<tonk-strip>`

The horizontal scroll container. Provides:
- Horizontal scroll with snap-on-column.
- CSS for graph-paper background, gap, padding.
- The dot-grid styling.

No subscriptions of its own. No gestures of its own. Pure visual container. Light DOM children (the `<tonk-column>` elements provided by template iteration) are its content. Mostly CSS — could be a `<div class="strip">` with zero element-class logic, but making it a custom element keeps the dispatch namespace tidy and gives us a place to add strip-level behavior later (e.g. global focus management).

### `<tonk-column>`

The vertical scroll container with **pull-to-reveal**. Provides:
- Vertical scroll within the column.
- Continuous overscroll detection past the bottom of the tile stack.
- Rubber-band visual: a `+` slot grows with the pull, capped by a threshold.
- On release past the threshold: dispatch a `tonk-claim` event for a `reveal-launcher` transient with the column's URI.
- On release before the threshold: snap back, no commit.

Gesture state is continuous and high-frequency — DOM-event-derived. The element owns it natively; nothing about scroll position or rubber-band transforms is expressible in dialog. The commit is a single discrete `tonk-claim` dispatch — that's the dialog-native handoff point.

Light DOM children (the iterated tile divs from the column view template) render in the scroll area normally. The launcher reveal-slot is a shadow-DOM element managed by `<tonk-column>` itself.

### A `<tonk-board>` element? Defer.

We don't need a board-level custom element for v1. The board view just renders a `<tonk-strip>`. If we later add board-scoped affordances (keyboard shortcuts, board-level menus) a `<tonk-board>` wrapper might be useful.

## Effects (transients) and rules

Every interaction is a transient assertion that triggers a rule.

### Close a tile

```yaml
concept!: &close-tile
  transient:
  with:
    tile:
      the: dom.event.current-target.dataset/tile
      cardinality: one
      as: entity

rule!:
  description: Retract the closed tile from its column's tile relation.
  retract: column
  where:
    this: ?column
    tile: ?tile
  when:
    - assert: close-tile
      where: { tile: ?tile }
    - assert: column
      where: { this: ?column, tile: ?tile }
```

Click the close button → `close-tile` transient fires → the rule retracts the `column.tile = ?tile` relation. The tile entity itself still exists but is no longer in any column. Renderer's iteration diff detaches it from the DOM.

The tile's claims could be GC'd by a separate rule if we want tile cleanup; not v1.

### Focus a tile

(Deferred to v1.5 once the static + create paths work. Mechanically the same shape — transient + rule that maintains a `board.focus` field.)

### Reveal launcher (the pull-to-open gesture commit)

```yaml
concept!: &reveal-launcher
  transient:
  with:
    column:
      the: dom.event.detail/column
      cardinality: one
      as: entity

rule!:
  description: Create a launcher tile at the end of the pulled column.
  assert: tile
  where:
    this: ?this                  # the transient's URI becomes the new tile's URI
    view: "basic"
    model: "launcher"
    entity: ?this                # launcher tile's content is itself
    order: "z"                   # last position in the column (lex-key heuristic)
  assert: column
  where:
    this: ?column
    tile: ?this                  # add new launcher tile to the column
  when:
    - assert: reveal-launcher
      where: { this: ?this, column: ?column }
```

Two things to flag:

- **The new tile's URI is the transient's URI** (`?this`). This is the pattern from the May 27 journal: the runtime mints a URI for each transient assertion; we reuse it as the durable entity's `this`. After sweep, the transient claim is gone but the URI persists as the tile's identity.
- **The `dom.event.detail/column` namespace.** This is where I need clarification — see "Open questions" below. The gesture commit on `<tonk-column>` is a `CustomEvent` we control, not a standard DOM event, so we can put the column URI anywhere on the event. The `dom.event.detail/*` namespace pattern would be the natural extension of `dom.event.target.dataset/*` from the journal: read a field from `event.detail`. May or may not exist; tracked as an open question.

### Pick an app from the launcher

```yaml
concept!: &open-tile
  transient:
  with:
    column:
      the: dom.event.current-target.dataset/column
      cardinality: one
      as: entity
    view:
      the: dom.event.current-target.dataset/view
      cardinality: one
      as: text
    model:
      the: dom.event.current-target.dataset/model
      cardinality: one
      as: text
    entity:
      the: dom.event.current-target.dataset/entity
      cardinality: one
      as: entity

rule!:
  description: Create a tile from a launcher pick; close the launcher.
  assert: tile
  where:
    this: ?this
    view: ?view
    model: ?model
    entity: ?entity
    order: "z"                   # or computed from launcher tile's order
  assert: column
  where:
    this: ?column
    tile: ?this
  retract: column
  where:
    this: ?column
    tile: ?launcher              # remove the launcher tile from the column
  when:
    - assert: open-tile
      where:
        this: ?this
        column: ?column
        view: ?view
        model: ?model
        entity: ?entity
    - assert: tile
      where:
        this: ?launcher
        model: "launcher"        # find any launcher in this column to close
```

(The "find the launcher in this column" premise is approximate — the real query needs to scope by column. Rule details to iterate on once the easier transients work.)

### Launcher view

```yaml
view!:
  model: launcher
  name: "basic"
  display: |
    <div class="launcher">
      <input type="text"
             placeholder="Pick an app…"
             oninput=launcher-query
             data-launcher={this}/>
      <ul subject={app}>
        <li onclick=open-tile
            data-column={column}
            data-view={view}
            data-model={model}
            data-entity={entity}>
          <tonk-display entity={app} model=app view=label/>
        </li>
      </ul>
    </div>
```

The launcher is itself a tile rendered by `<tonk-display>` like any other. The `subject={app}` iteration over launcher's `app` cardinality-many relation needs the launcher to have a cardinality-many field listing candidate apps — a rule populates it from "all entities with a `view` declared for some `model`."

That rule lives in this plan as future scope. For v1, the launcher's app list can be a static set asserted directly.

## Route

Add a new route:

```
/space/:space/branch/:branch/board/:board
```

The route's view component:

```rust
view! {
    <header slot="main-header" class="space-banner">
        <h1 class="space-banner-title" title=board_name>
            { board_name }
        </h1>
    </header>
    <main class="wa-stack board-view">
        <tonk-repository name=space_name>
            <tonk-branch name=branch_name>
                <tonk-display
                    entity=board_uri
                    model="board-view"
                    view="basic" />
            </tonk-branch>
        </tonk-repository>
    </main>
}
```

Reuse the existing `<tonk-host>` mounted at launcher level. The route resolves `:board` to an entity URI (by name lookup, same pattern as the existing `display.rs` route does for `:subject`) and hands off to a `<tonk-display>` that renders the board's `basic` view.

`<tonk-layout>` is **not used in v1**. The existing layout route stays as-is. The new board route is independent.

(Whether to retire `<tonk-layout>` or keep it as a parallel construct for other layout styles is a follow-up question.)

## Bootstrap data

A starter document for `rust/tonk-board/bootstrap.yaml` (or similar):

```yaml
# Schema
concept!: &board (… as above …)
concept!: &column (… as above …)
concept!: &tile (… as above …)

# Views
view!: (board / basic … as above …)
view!: (column / basic … as above …)
view!: (tile / basic … as above …)

# Transients
concept!: &close-tile (transient, as above)
concept!: &reveal-launcher (transient, as above)
concept!: &open-tile (transient, as above)

# Rules (as above)

# A demo board for screenshot purposes
board!: &demo-board
  name: "demo"
  column: [col-a, col-b]

column!: &col-a
  order: "a"
  width: 12
  tile: [tile-1, tile-2]

column!: &col-b
  order: "n"
  width: 8
  tile: [tile-3]

tile!: &tile-1
  order: "a"
  view: "basic"
  model: "person"
  entity: did:key:zSomePerson

tile!: &tile-2
  order: "n"
  view: "basic"
  model: "person"
  entity: did:key:zSomePerson

tile!: &tile-3
  order: "a"
  view: "basic"
  model: "person"
  entity: did:key:zSomePerson
```

POST this to `/evaluate` to seed a `demo` board with two columns and three tiles. Navigate to `/space/home/branch/main/board/demo` to see the rendered strip.

## Open questions

- **URI minting in rule heads.** The pattern "the transient's `?this` becomes the new entity's URI" needs to actually work for the `reveal-launcher` and `open-tile` rules. The journal pattern shows it for the counter (rule head asserts an existing entity's update). It needs to also work for *creating* a new entity from a `?this` that wasn't otherwise bound. Worth a quick `tonk-evaluator` test before relying on it.

- **The `dom.event.detail/*` namespace.** The current journal note only describes `dom.event.target.dataset/*` and `dom.event.current-target.dataset/*`. The gesture commits on `<tonk-column>` are CustomEvents whose payload we control — we'd want a way to read fields from `event.detail`. Two options:
  - Extend the runtime with a `dom.event.detail/*` namespace.
  - Have `<tonk-column>` set `data-*` attributes on itself before dispatching, then use the existing `dom.event.current-target.dataset/*` namespace. Works today; less elegant.

  v1 starts with option 2; option 1 lands when it bites.

- **Cardinality-many in rule heads.** The `reveal-launcher` rule asserts `column.tile = ?this` — i.e., adds one entry to a cardinality-many relation. This needs to actually *add* rather than *replace* the relation. The rule engine's semantics need to be friendly to this; worth verifying.

- **Tile chrome — view template vs. column-stack-level wrapping.** The column view's iteration `subject={tile}` produces one `.tile` div per tile, with the close button inside. That works. But if the close button needs to live in shadow DOM (so it's not part of the cloned template that re-renders on every tile-content frame), we'd need to move it into a `<tonk-tile>` element. v1 does it the simple way.

- **Pull-to-reveal threshold visual.** Should the "+" indicator animate as the pull progresses, or only appear past threshold? Prototype animates. v1 can ship without animation; add it once the commit path works.

## Future flattening: tracked

When deductive rule support is mature enough to project flat `tile` rows into hierarchical `board-view` + `column-view`:

1. Rename current `board` and `column` concepts to `board-view` and `column-view` (or commit to that naming from the start so no rename needed).
2. Drop the `board.column` and `column.tile` relations from the bootstrap document.
3. Add `tile.column` field. The flat tile carries its column reference directly.
4. Add the two projection rules.
5. Views target `board-view` / `column-view` exactly as before; they don't notice the shift.

The rename direction is the deciding factor: **start with `board-view` and `column-view` as the names**, so the future move is purely a data-model rewire.

## Implementation order

1. Schema yaml (concepts + transients).
2. View yaml (board / column / tile views).
3. `<tonk-strip>` — light wrapper, mostly CSS.
4. `<tonk-column>` — scroll container without the gesture (static layout works).
5. Route + bootstrap demo board. **First checkpoint: visible strip rendering the demo data.**
6. `close-tile` transient + rule. **Second checkpoint: clicking close removes a tile.**
7. `<tonk-column>` pull-to-reveal gesture + `reveal-launcher` rule. **Third checkpoint: pulling past the bottom of a column reveals a launcher tile.**
8. Launcher view + `open-tile` rule. **Fourth checkpoint: picking an app from the launcher creates a tile.**
9. Polish: snap-back animation, threshold visual, focus indicator.

Each checkpoint is independently demoable. Steps 1-5 are pure "static render"; 6 is the smallest interactive step; 7-8 are the signature gesture.

## What this plan deliberately does not address

- **Focus navigation.** Deferred. Would slot in as a transient + rule that maintains a `board.focus` cardinality-one entity field.
- **Drag-to-reorder.** Deferred. Same shape: continuous gesture in a custom element, commit dispatches an `update-order` transient with the new lex-key.
- **Multiple boards.** The route assumes one board at a time. Multiple boards on one page is a future concern.
- **Iframe-bridge integration for tiles.** Tiles that render external content via iframe. Out of scope.
- **The command palette.** The journal note frames every interaction as a command (transient). A command palette is a UI on top of that — listing all `command:` concepts in scope and providing fuzzy search. Out of scope for the board itself; lands as a separate layer.

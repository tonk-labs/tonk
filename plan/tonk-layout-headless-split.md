# Headless `<tonk-layout>` — separating WM logic from UI

## Context

Today's `<tonk-layout>` is a niri-style tiling WM that does three
things at once: subscribes to dialog and folds the workspace state,
interprets keyboard/pointer input into notation writes, and patches
DOM into a niri-strip shape. The niri shape is woven through both the
rendering layer (CSS classes `niri-column` / `niri-tile`, the resize
handle layout in `reconcile.rs`) and the input layer (`R` cycles
column widths, `Ctrl+←/→` moves columns — gestures that only make
sense for a strip-of-columns).

We want a clean separation between state/logic and UI so users can
patch in their own WM preferences at runtime via dialog assertions
and views. Examples of alternate UIs:

- Single-view-at-a-time (just the focused tile, fullscreen).
- Horizontal scroll-off carousel.
- One big grid.

The goal is for any of these (and others not yet imagined) to be
expressible as an HTML+CSS+JS view document — toggleable per
workspace by swapping the `view=` attribute on a wrapping
`<tonk-display>` — with `<tonk-layout>` providing the universal
state and command primitives every WM style needs.

## Decisions

The brainstorm settled the architecture on six interlocking choices:

1. **`<tonk-layout>` becomes truly headless.** No rendered DOM,
   no opinions about gestures. The UI view owns *both* the
   rendered shape *and* the input-to-command mapping.

2. **Universal data: tile registry + focus + linear order.** The
   universal schema in dialog is a `workspace` (with `name`,
   `focus`) and a `tile` (with `workspace`, `order`, `entity`,
   `view`, `model`). One linear `order` lex-key per tile. No
   columns, no widths, no heights, no `kind`. Those are UI-overlay
   concerns.

3. **UI wraps `<tonk-layout>`.** A WM style is a tonk-display
   view whose template body includes `<tonk-layout>` as a child
   element. The UI does the rendering; `<tonk-layout>` is
   invisible glue. Toggling UIs = swapping the wrapping
   `<tonk-display>`'s `view=` attribute.

4. **Effects-only API, no readable property.** `<tonk-layout>`
   exposes a vocabulary of six named effects (DOM CustomEvents
   for now, future-compatible with PR #461's transient-concept
   transport). UIs read dialog state directly via `<tonk-concept>`;
   `<tonk-layout>` is never the state source.

5. **Per-UI overlay concepts.** Each WM style declares its own
   overlay concepts alongside its view. Niri declares
   `niri-column` (workspace, order, width) and `niri-placement`
   (tile, column, order-in-column, height). Other UIs declare
   their own. Switching UIs leaves the inactive overlay rows
   latent in dialog; switch back later and the layout is exactly
   as you left it.

6. **UI interaction lives in the view template's JS.** Per the
   updated view-system direction (see memory:
   project-tonk-view-system-direction), `<tonk-display>` views
   render full HTML + CSS + embedded `<script>`. Keyboard
   handlers, pointer-drag, scroll-to-focused all live inside the
   view document, not in a Rust companion element.

## Element interface

The element's HTML attributes stay the same as today's
`<tonk-layout>`:

```html
<tonk-layout [workspace="<name>"]
             [space="<space>"]
             [branch="<branch>"]>
</tonk-layout>
```

| Attribute | Required | Default | Meaning |
|---|---|---|---|
| `workspace` | no | `"default"` | Logical name of the workspace. Resolves through the workspace concept's `name` field. |
| `space` | no | `"home"` | Repository space (query routing). |
| `branch` | no | `"main"` | Branch (query routing). |

Changing any of these after the element is connected aborts
outstanding subscriptions, clears internal state, and restarts
against the new target — same generation/lifecycle discipline as
today.

Unlike today, the element has no rendered children. It accepts no
slotted UI; UIs live in the wrapping `<tonk-display>` view's body.

## Universal schema

The two universal concepts are *declared* in a dialog-yaml
`concept!:` document that ships with the `tonk-layout` crate. The
shell (e.g. `tonk-ui`) asserts that document into the branch on
first run, the same way other concepts get registered today.
Workspaces and individual tiles, by contrast, are
*lazy-bootstrapped* on first interaction — see the `open-tile`
effect.

```yaml
concept!: &workspace
  description: A workspace of tiles
  with:
    name:    { the: xyz.tonk.layout/workspace-name,  as: text }
    focus:   { the: xyz.tonk.layout/workspace-focus, as: entity, cardinality: one }

concept!: &tile
  description: One tile in a workspace; renders content via <tonk-display>
  with:
    workspace: { the: xyz.tonk.layout/tile-workspace, as: entity }
    order:     { the: xyz.tonk.layout/tile-order,     as: text }
    entity:    { the: xyz.tonk.layout/tile-entity,    as: entity }
    view:      { the: xyz.tonk.layout/tile-view,      as: text }
    model:     { the: xyz.tonk.layout/tile-model,     as: text }
```

`tile.entity` is **optional**. Single-entity tiles (today's
`kind: "display"`) populate it; concept-listing tiles leave it
empty and rely on `model` carrying the concept name and `view`
naming a list-rendering view (e.g. `view: "concept-list"`). The
view template is responsible for handling both shapes — a
`concept-list` view internally uses `<tonk-concept source={model}>`
to do the listing.

Notable differences from the existing SPEC:

- `column` concept gone — moves to the niri overlay.
- `tile.column` → `tile.workspace`. Tiles parent directly under
  workspace.
- `tile.height` gone — niri-specific, moves to the niri overlay.
- `tile.kind` gone — every tile is rendered via `<tonk-display>`.
  The dispatch to "single entity" vs. "concept list" happens in
  the view template, not in the schema.

`focus` stays on workspace.

## Effects vocabulary

Six named effects. Each one is dispatched from the view template
using DOM `CustomEvent` (bubbling, composed), with `<tonk-layout>`
catching them at its root and translating to atomic notation
documents.

| Name | Required params | Optional | What it writes |
|---|---|---|---|
| `tonk-layout/focus-tile` | `target` (tile entity) | — | Asserts `workspace.focus = target`. |
| `tonk-layout/focus-prev` | — | — | Reads current focus + linear order, asserts focus to the previous tile. No-op if first. |
| `tonk-layout/focus-next` | — | — | Same, in the other direction. |
| `tonk-layout/open-tile` | `view`, `model` | `entity`, `before`, `after` | Mints a tile ULID. Computes `order` per the [order-key rules](#order-key-rules-for-open-tile-and-reorder-tile) using `before` / `after`. Lazy-bootstraps the workspace if absent. Asserts `tile!` row + sets `workspace.focus` to the new tile. One atomic doc. `entity` is optional (omit for concept-list-style tiles). |
| `tonk-layout/close-tile` | `target` (tile entity) | — | Retracts the tile row. If `target` was focused, advances focus to the previous tile (or next if previous is gone, or null if no tiles remain). |
| `tonk-layout/reorder-tile` | `target` | `before`, `after` | Computes the new lex-midpoint per the [order-key rules](#order-key-rules-for-open-tile-and-reorder-tile) and asserts `tile.order` on `target`. |
| `tonk-layout/update-tile-content` | `target` | `entity`, `view`, `model` | Asserts whichever of the three fields are provided. |

Niri-overlay writes (column resize, column reorder, niri-placement
edits) are *not* in this vocabulary — the niri view's JS asserts
those directly via standard notation effects on its own concepts.
The layout vocabulary stays universal.

### Order-key rules for `open-tile` and `reorder-tile`

Both effects take optional `before` and `after` parameters
identifying tile entities in the universal linear order. The
resolved order key is computed as follows, where `prev(t)` is the
tile immediately before `t` in current linear order (or
sentinel-min if `t` is first) and `next(t)` is the tile
immediately after (or sentinel-max if `t` is last):

| `before` | `after` | Resolved range | Notes |
|---|---|---|---|
| set | set | midpoint(`after.order`, `before.order`) | Insert strictly between two tiles. |
| set | unset | midpoint(`prev(before).order`, `before.order`) | "Place before this tile." |
| unset | set | midpoint(`after.order`, `next(after).order`) | "Place after this tile." |
| unset | unset | midpoint(`last.order`, sentinel-max) | Append at the end. |

If `before` and `after` are both set but not adjacent, the
midpoint is still computed against the supplied two — the caller
opted into that placement. If the supplied tile references don't
resolve (entity unknown in the current fold), the effect fails
loudly via the error event.

### Transport

For the first PR: DOM `CustomEvent` bubbling up the subtree to
`<tonk-layout>`'s root listener. The view's JS calls a small shim:

```js
layout.emit('focus-next');
layout.emit('open-tile', { entity, view, model });
```

The shim wraps `dispatchEvent(new CustomEvent('tonk-layout/...', {
detail: params, bubbles: true, composed: true }))`. The element's
root listener dispatches by event type.

The event-name namespace is chosen to match the future
transient-concept names from PR #461 (e.g. a `tonk-layout/focus-tile`
event today maps mechanically to a `tonk-layout/focus-tile!: { this:
effect:system, target: ?t }` assertion tomorrow). When PR #461 lands,
`layout.emit()` swaps its implementation to `/transact` writes; the
view template's call sites don't change.

### Atomic guarantees

Every effect produces exactly one `/evaluate` document. `open-tile`
(potentially workspace bootstrap + tile + focus) and `close-tile`
(retract + new focus) are the multi-statement ones; both stay
atomic so they merge cleanly under concurrent writers.

### Outbound events

The element keeps three outbound events:

- `tonk-layout:changed` — fired when a refold settles. Lets UIs
  hook "frame settled" if they want it. Detail: `{ workspace,
  focus, tile_count }`.
- `tonk-layout:focus` — fired when the focused tile changes.
  Detail: `{ tile }`.
- `tonk-layout:error` — fired on subscription/transport failure.
  Detail: the error.

UIs that need scroll-to-focused on focus change subscribe to
`tonk-layout:focus`. UIs that need a "subscriptions are settled"
indicator listen for `tonk-layout:changed`.

### Concurrency

Same generation/lifecycle discipline as today: attribute changes
abort outstanding subscriptions, fresh subscriptions on restart,
effects spawned by a superseded generation are dropped.

## Niri view as a worked example

A niri-strip UI is a single tonk-display view document declaring
its own overlay concepts and a template body with HTML + CSS + JS.
Sketch:

```yaml
# Niri overlay concepts — shipped with the niri view document
concept!: &niri-column
  description: A column in a niri-strip
  with:
    workspace: { the: xyz.tonk.niri/column-workspace, as: entity }
    order:     { the: xyz.tonk.niri/column-order,     as: text }
    width:     { the: xyz.tonk.niri/column-width,     as: float }

concept!: &niri-placement
  description: A tile's placement in a niri column
  with:
    tile:   { the: xyz.tonk.niri/placement-tile,   as: entity }
    column: { the: xyz.tonk.niri/placement-column, as: entity }
    order:  { the: xyz.tonk.niri/placement-order,  as: text }
    height: { the: xyz.tonk.niri/placement-height, as: float }

view!:
  this: id:niri-strip-view
  name: "niri-strip"
  body: |
    <tonk-layout workspace="{workspace}" space="{space}" branch="{branch}"></tonk-layout>
    <div class="niri-strip" tabindex="0">
      <tonk-concept source="niri-column" filter="workspace = {workspace}">
        <template>
          <div class="niri-column" data-entity="{this}" style="flex: {width} 1 0">
            <tonk-concept source="niri-placement" filter="column = {this}">
              <template>
                <div class="niri-tile" data-tile="{tile}" data-placement="{this}"
                     style="flex: {height} 1 0">
                  <tonk-display entity="{tile.entity}" view="{tile.view}" model="{tile.model}"></tonk-display>
                </div>
              </template>
            </tonk-concept>
          </div>
        </template>
      </tonk-concept>
    </div>
    <style>
      .niri-strip { display: flex; overflow-x: auto; height: 100%; }
      .niri-column { display: flex; flex-direction: column; }
      .niri-tile { position: relative; }
      .niri-tile[data-focused] { outline: 2px solid var(--wa-color-brand-fill-loud); }
    </style>
    <script>
      const layout = document.querySelector(':scope > tonk-layout');
      const strip = document.querySelector('.niri-strip');

      // Keyboard handlers
      strip.addEventListener('keydown', (ev) => {
        switch (ev.key) {
          case 'ArrowLeft':  layout.emit('focus-prev'); ev.preventDefault(); break;
          case 'ArrowRight': layout.emit('focus-next'); ev.preventDefault(); break;
          case 'Q':          /* close focused tile */ break;
          case 'Enter':      /* open-tile dialog */ break;
          // Ctrl+Arrow → niri-overlay reorders (asserted directly by this view's JS)
          // R → cycle width on the focused column (niri-column.width edit)
        }
      });

      // Pointer-drag for column resize — local to this view.
      // Asserts niri-column.width directly, debounced.
      // ...

      // Scroll-to-focused on focus change
      layout.addEventListener('tonk-layout:focus', (ev) => {
        const tile = ev.detail.tile;
        const tileEl = strip.querySelector(`.niri-tile[data-tile="${tile}"]`);
        tileEl?.parentElement?.scrollIntoView({ behavior: 'smooth', inline: 'nearest' });
      });

      // Auto-place: any tile lacking a niri-placement gets one in a default column.
      // ...
    </script>
```

A few callouts:

- **Two-level `<tonk-concept>` nesting** drives rendering. The exact
  `filter=` syntax is illustrative; if the current `<tonk-concept>`
  doesn't accept that form, the view's `<script>` does the filtering
  manually after a broader subscription.

- **Niri-overlay writes go through the view's own JS** asserting on
  `niri-column` / `niri-placement` directly. They don't go through
  `<tonk-layout>` effects. The layout vocabulary is only for things
  that touch universal tile / workspace state.

- **Unplaced tiles** — tiles in the workspace without a
  `niri-placement` row need a rendering decision. The view's JS
  auto-asserts a `niri-placement` row putting any unplaced tile in
  a default "incoming" column at the tile's universal `order`
  position. This makes the niri view *converge* — a fresh open-tile
  from any source ends up niri-placed eventually.

## Migration and distribution

### Existing seeded data

The current `workspace` / `column` / `tile` schema differs from the
universal one. A one-shot migration document reads the old rows and
emits a single notation document that:

1. Asserts new universal `tile!:` rows for each existing tile,
   parented under the workspace, with `order = column.order +
   tile.order` (concatenated lex keys yield a unique linear order
   matching the existing visual order).
2. Re-asserts the existing `column!:` rows as niri-overlay
   `niri-column!:` rows (attribute URIs change:
   `xyz.tonk.layout/column-*` → `xyz.tonk.niri/column-*`).
3. Asserts `niri-placement!:` rows from old tile data
   (`tile.column` → `placement.column`, `tile.height` →
   `placement.height`, etc.).
4. Retracts the old `column.*` and `tile.column` / `tile.height` /
   `tile.kind` cells.

Run once per branch with existing layout data. Idempotent (check
for absence of universal tile rows before asserting).

The migration script is the only place in the project that knows
both schemas. It can live as a fixture / helper in the `tonk-layout`
crate, runnable from a CLI or invoked manually via `/evaluate`.

### Concept registration

The universal `workspace` / `tile` concepts ship as a `concept!:`
document that `<tonk-layout>` asserts on first connect (same
lazy-bootstrap pattern as today). Niri overlay concepts ship with
the niri view document — declaring them is part of asserting the
view.

### View distribution

The niri view document is bundled in `tonk-ui` (the application
shell). On first run, the shell checks whether the niri view is
asserted; if not, asserts it. This keeps `tonk-layout` purely
about the universal layer; the niri view is one of (eventually)
many UIs the shell ships.

Users of `tonk-layout` outside the `tonk-ui` shell get no view by
default; they assert their own. Acceptable trade-off for the
cleaner separation.

## Testing strategy

### Native unit tests (`cargo test`)

- **Order key math** (`order.rs`) — already covered; reused as-is
  for the universal `tile.order`.
- **Universal layout fold** — new
  `fold_universal(workspace_frame, tiles_frame) → Option<Layout>`
  mirrors today's `fold_layout` but folds the new schema (no
  columns). Tests assert sorting by `order`, focus carry,
  drop-tiles-without-workspace.
- **Effect document builders** — one builder fn per effect.
  Native tests check the emitted notation strings against
  fixtures. Non-trivial cases (`open-tile` with bootstrap,
  lex-midpoint computation, focus-advance on close) get extra
  coverage.
- **ULID minting** — pure unit; already covered.

All tests use `#[dialog_common::test]` per project convention;
named `it_does_x` and grouped by behaviour.

### WASM integration tests (`wasm_bindgen_test`)

- **Effect dispatch** — mount `<tonk-layout>`, dispatch a
  `tonk-layout/focus-tile` CustomEvent, assert a `/evaluate` POST
  went out with the expected body. Use a stub evaluator so the
  test stays offline.
- **Subscription lifecycle** — attribute change aborts old
  subscriptions; effect dispatch from a superseded generation
  no-ops.
- **Effect → state roundtrip** — dispatch an effect, deliver a
  matching SSE frame, verify `tonk-layout:changed` fires.

### Niri-view integration tests (`thirtyfour`)

- Bring up the niri view in a real browser, drive it through a
  minimal scenario: open three tiles, verify they appear in
  expected columns; arrow-left/right changes focus; Q closes a
  tile and focus advances; drag-resize updates `niri-column.width`
  and the visual layout. Exercises the view's embedded JS *and*
  the layout's effect handlers, end-to-end.
- Migration test: pre-seed a branch with old-schema data, run the
  migration document, mount the niri view, verify it renders the
  same shape the old element did.

## PR sequence

### PR 1 — universal schema and effects API

Structural refactor.

- New universal `workspace` / `tile` concepts (different attribute
  URIs from current).
- `<tonk-layout>` keeps its custom-element identity but strips all
  rendering. Its job is now: subscribe to universal `workspace` +
  `tile`, hold a folded snapshot internally, listen for the six
  effect CustomEvents on its subtree, write atomic notation
  documents in response.
- Outbound events: `tonk-layout:changed`, `tonk-layout:focus`,
  `tonk-layout:error`.
- Tiny JS helper shipping `layout.emit(name, params)` for view
  authors.
- All native tests for new fold + effect-doc builders.
- The existing `tonk-ui` page that mounts `<tonk-layout>` does not
  render anything visible after this PR. Acceptable intermediate
  state.

### PR 2 — niri-strip view document

First UI on top.

- Niri-overlay concepts (`niri-column`, `niri-placement`) ship as
  a yaml document.
- Niri-strip view document with embedded HTML + CSS + JS, bundled
  in `tonk-ui`.
- Auto-seed: on first run, `tonk-ui` checks whether the niri view
  is asserted; if not, asserts it.
- Migration script for existing seeded layout data.
- Integration test (thirtyfour) demonstrating the niri view
  renders correctly and effects round-trip end-to-end.
- At the end of this PR, `tonk-ui` is back to functional parity
  with today.

### PR 3 — proof of pluggability

Add a second UI to demonstrate the toggle.

- A second view document (single-view, or grid — whichever is
  easiest to validate). Doesn't have to be feature-complete; just
  has to demonstrate that toggling `<tonk-display view=…>`
  switches the WM UI without touching layout state.
- Integration test: same workspace, both views, swap between them,
  verify state persists and the alternate UI renders.

This sequence follows "land the smallest first PR" and "transport
before policy" — PR 1 is the transport (effects vocabulary,
headless element); PR 2 is the first concrete policy (the niri
view); PR 3 proves extensibility. Each PR is independently
shippable; PR 1 is the only invasive one.

### Trade-offs flagged

- **PR 1 leaves `tonk-ui` visibly broken** until PR 2 lands.
  Acceptable for a fast-following pair (PR 1 and 2 within days of
  each other); worse if separated. Alternative: keep old niri
  rendering in `<tonk-layout>` as a fallback that activates when
  no view wraps it, ripped out in PR 2. Doubles the
  implementation. Default to the visible-break.

- **Effect transport migrates to PR #461's transients** once that
  lands. Event-name namespace chosen so the swap is mechanical;
  view template call sites don't change.

## Out of scope

- The new `/transact` endpoint and transient-concept transport
  from PR #461 — that's a different effort whose output will get
  integrated here mechanically once it lands.
- Designing alternate UIs beyond the niri-strip + the PR-3 proof
  UI. Users author their own view documents; this plan only
  guarantees the substrate.
- Cross-device deduplication of "should-be-the-same" entities.
  Same v1 limitation as today.

## Open questions

- **`<tonk-concept>` filter syntax.** The view sketch assumes
  `filter="workspace = {workspace}"`. If `<tonk-concept>` doesn't
  support that form today, the niri view's `<script>` filters
  manually after a broader subscription. Resolve in PR 2.
- **`<tonk-display>` field binding into a tile row.** The niri view
  binds `entity={tile.entity}` etc. from a tile row. Resolve
  during PR 2 whether nested binding works or whether each tile
  needs a wrapper that does the lookup.

# tonk-board

Board layout custom elements: `<tonk-board>`, `<tonk-strip>`, `<tonk-column>`.

This crate ships three WASM custom elements that supply the structural shell for
board-style UIs (a horizontal strip of vertical columns of tiles). They are
presentation containers: they own layout and (for the column) gesture behavior,
but they do not subscribe to data themselves. Children are supplied by view
templates rendered through `<tonk-display>`; data flows through the host
abstraction in `tonk-host`. CSS lives in the consuming app's stylesheet; the
elements provide the tag names and structural identity that views target.

Call [`register`](src/lib.rs) once to define all three with the page's custom
element registry. It is idempotent (each element guards on whether its tag is
already registered), and the elements are compiled only for `wasm32`.

## Elements

- **`<tonk-board source="…">`**: the outer wrapper. On mount it resolves the
  `source` board name to an entity URI, then mounts a `<tonk-display>` against
  it. If `source` already looks like a URI (contains a `:`) it is used as-is;
  otherwise the element dispatches a `tonk-query` against the branch's `Name`
  index (matching `xyz.tonk.board/name`) to find the board entity, rendering a
  `section.not-found` if no board matches. The mounted display carries
  `model="board-view"` and no `view` attribute, so the built-in view for that
  model drives the rest of the render. Resolution is memoized and an in-flight
  guard dedupes the back-to-back `attributeChangedCallback` + `connectedCallback`
  that fire at upgrade time. Light DOM (no shadow root).

- **`<tonk-strip>`**: horizontal scroll container, used inside the board view
  template as the host for column children (the template iterates them from
  `board.column`). No attributes and no behavior of its own today; it exists as
  a custom element to keep the dispatch namespace tidy and leave room for
  strip-level behavior (focus management, cross-column keyboard nav).

- **`<tonk-column>`**: vertical scroll container, used inside the column view
  template as the host for tile children (iterated from `column.tile`). v1 is
  just the scroll container; a pull-to-reveal gesture (overscroll past the tile
  stack dispatching a `tonk-claim` transient) is planned as a follow-up.

## Composition

`<tonk-board>` mounts a `<tonk-display>` whose `board-view` template renders a
`<tonk-strip>`, which hosts one `<tonk-column>` per `board.column`, each of which
hosts the column's tiles. The board, strip, and column elements supply the
nested layout shells; the view template chain supplies the data-bound children.

The board schema, view templates, and `demo` board the wrapper resolves against
are seeded into the branch from the standard library at repository creation, not
by these elements.

See `plan/tonk-board.md` at the repository root for the design.

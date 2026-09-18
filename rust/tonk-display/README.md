# tonk-display

A `<tonk-display>` custom element that renders entities from a dialog branch using a view template stored on that branch.

The element is a `cdylib` WASM custom element. You place it in the page, point it at a model concept and (optionally) a single entity, and it resolves the view template for that model, queries the matching entities, and substitutes their fields into the template. Call [`register`] once to register `<tonk-display>` (the orchestrator), `<tonk-view>` (the dumb single-template renderer it drives), and `<tonk-notation>` (the inspector slide). All three are idempotent.

```html
<!-- one entity, default detail view -->
<tonk-display model="person" entity="did:key:z6Mk..."></tonk-display>

<!-- every instance of the model, default directory view -->
<tonk-display model="task"></tonk-display>

<!-- an explicit show facet -->
<tonk-display model="task" view="board"></tonk-display>
```

## Attributes

- `model`: names the subject's model concept, by bookmark name (`person`) or entity URI (anything containing `:`). Required.
- `entity`: the URI of the single entity to render. Absent selects directory mode (every instance of the model).
- `view`: names the *show facet* to render (`label`, `title`, or any key the model's `show` dictionary carries). Omitting it uses the mode default: `ui` in detail mode, `directory` in directory mode. A facet containing `:` is rejected as a descriptor error, since a facet is a plain key and not a concept URI. `view="about:blank"` is checked before that guard and stays reserved for a future carousel mode, which currently errors.

A change to `model`, `entity`, or `view` restarts the resolve/subscribe flow. `data-active` (and other `dom.host/*` context attributes a parent view threads in) is propagated into the already-mounted view in place without restarting.

## The resolution pipeline

Resolving one display walks model -> view -> entities -> bind:

1. **Model.** The `model` attribute is resolved to a concept descriptor. A bookmark name is first turned into its referent URI through the Name concept (`name_query`), then `phase1_query` reads the concept-of-concept row and pulls the descriptor JSON out of its `source` field. This is a *live* subscription: a concept seeded after the element mounts pushes a frame and the display recovers without a reload.

2. **View.** With the model entity in hand, `view_query` pins `this` to it and projects its whole `show` dictionary, one flat row per facet. This too is a live subscription, so editing a template on the branch swaps the rendered DOM. A view instance's `this` IS the model, so there is no `model` field to constrain and no separate view entity to find. `select_rows` folds the rows into one conclusion and `show_template` picks the facet: the `view` attribute names a *facet* (`label`, `title`, ...), not a concept. Detail mode defaults to the `ui` facet, directory mode to `directory`, letting a model declare both independently in the same dictionary.

3. **Entities.** A third live subscription projects every field in the model descriptor's `with:` map. `entity_query` pins `this` to the `entity` URI (detail); `instances_query` leaves `this` unbound (directory), matching every instance. `?key=value` filters on the `model` attribute are applied as constants on those terms.

4. **Bind.** Each entity frame is folded (see below) and handed to the mounted `<tonk-view>` via `el.render(conclusion)`, which inserts on the first call and patches in place afterward. All three subscriptions live in `<tonk-display>`; the slide `<tonk-view>` elements never open their own.

## Template and binding model

A view template is HTML with `{field}` placeholders authored as the children of a `<tonk-view>`:

```html
<tonk-view>
  <p class="greeting">{message}</p>
  <a href="/entity/{this}">{title}</a>
</tonk-view>
```

`<tonk-view>` snapshots its children at connect time into a cloneable fragment plus a *binding plan*: a list of paths from the fragment root to each bound node, each carrying the segment list (literal text and `{field}` references) that produces its value. `{this}` resolves to the conclusion's subject URI; missing fields render as empty strings; single-identifier interpolation only (`{name}`, not `{name + "x"}`).

A binding fills either text content or an element value. Element-value bindings dispatch per value at render time: an `html:foo={x}` author prefix always forces `setAttribute` (booleans map to presence/absence); a single-field non-string JSON value is set as a typed JS property via `Reflect.set`; otherwise the renderer sets a property when the name exists on the element, an attribute when it does not.

The renderer keeps a mounted-state tree so repeated frames patch the DOM in place rather than re-cloning. Each binding caches its last rendered string and skips unchanged writes, preserving node identity (which matters for nested custom elements).

### Repeat and iteration

The plan splits into **chrome** (bindings outside the repeated element, rendered once against the lead conclusion) and a **repeat** element cloned once per subject conclusion. The repeat element is chosen by `this_repeat_root`: the outermost subject-referencing element, with a bare `{this}` marker on it if present, else the fragment root. Each clone is keyed by the conclusion's `this` and stamped `with=<this>` so the repeat boundary is inspectable; adding or removing a subject touches only its row.

Inside a repeat row, cardinality-many *subject fields* iterate their values as a `MountedIteration` keyed by the value's string form, so a many-valued `{tags}` repeats a subtree within one conclusion.

A nested `<tonk-display>` inside a template mounts exactly once: rows build detached and insert with a single `insertBefore` so a move never fires a spurious `disconnected`/`connected` pair.

## Detail vs directory modes

The presence of the `entity` attribute selects the mode:

- **Detail** (`entity` set): pins `this` to that URI, resolves the model's `ui` facet, and renders a single entity. The entity subscription frame is size 0 (entity absent or removed) or 1.
- **Directory** (`entity` absent): leaves `this` unbound, resolves the model's `directory` facet, and renders every instance. A view declared with `this: tonk:_` supplies the facet for any model that lacks its own.

In both modes the query engine emits one flat row per tuple, so cardinality-many fields and multiple subjects arrive as separate rows. [`select_rows`] (in `fold`) groups rows by `this` and folds each group into one conclusion per subject, collapsing multi-valued fields to a list in first-seen order (identical values stay a scalar). Detail mode is then just a one-conclusion frame and directory mode a many-conclusion frame, rendered by the same repeat machinery.

## Introspection

`tonk_display::register()` also registers and mounts `<tonk-introspect>`, an
overlay that makes this pipeline visible at the point of use. Hold Alt and the
display under the pointer outlines, with a **pin** button above its corner;
rest there for 300ms and observation switches on. Click the pin (Alt still
held) to keep the observation while the pointer goes elsewhere; click it again,
or press Escape, to release.

Observation marks two things:

- **Slots** — every interpolation the template filled, labelled with the field
  that fed it and coloured by where that field came from: a concept field,
  `{this}`, a `{dom.host/*}` host attribute, or an iteration key. A value that
  changes flashes where it landed.
- **Commands** — every element binding an `on<event>` or `on:<name>`
  interaction, labelled `click -> space/create`, bouncing when it actually
  posts. A binding whose event declaration did not resolve is drawn *inert*: it
  installs no listener and will never fire, which is otherwise invisible.

A marker is not always a box. A slot that rendered an empty string has nothing
to box, so it ticks the caret position the value would have occupied. A slot
that wrote an element property ticks that element's top edge rather than
filling it — the element is where the value went, but the element is not the
value. Every marker carries its label whatever its placement, and badges that
would collide are pushed down and joined to their anchor by a leader.

Observation also opens a **concept panel**, which answers the question markers
cannot: *why isn't my value showing up?* On a rendered page all four answers
look like the same blank space, so the panel names them — one row per field
either side knows about, whether or not it rendered:

| row | meaning |
| --- | --- |
| *(plain)* | declared, read by a slot, and this subject has a value |
| `no value` | declared and read, but this subject has nothing for it |
| `not in the view` | the concept declares it; no slot reads it |
| `not on the concept` | a slot reads it; the concept does not declare it |

Each row carries the field's declared type and cardinality and the value as the
renderer spelled it — routed through the renderer's own segment substitution,
so the panel can never report a value in a spelling the page did not use.
Resting on a row brings the slots it feeds forward and dims everything else.
In directory mode the panel follows the repeat row under the pointer, and
sticks there while you walk over to read it.

The overlay reads the renderer's binding plan and value cache rather than the
rendered DOM, because the DOM cannot answer the question: a rendered
`with="main@repo"` does not say which half was a field, and a binding applied
as a JS property left no attribute behind at all.

It is inert until Alt goes down — one `mousemove` listener that reads `altKey`
and returns, plus a bool read on the renderer's change path. It takes no
gesture from the page: pinning is a click on its own chrome, not a
modifier-click on your markup. `<tonk-introspect alt-click>` restores
alt-click pinning if you want it. Remove the element to opt out entirely.

See `/plan/display-introspection.md` for the design and the steps still open
(the concept and view panels, editing).

## Modules

- [`element`](src/element.rs): the `<tonk-display>` orchestrator: lifecycle, the three subscriptions, mode selection, slide mounting (wasm only).
- [`resolve`](src/resolve.rs): wire-query construction (`name_query`, `phase1_query`, `view_query`, `entity_query`, `instances_query`) and `source` parsing. Query builders are target-independent.
- [`view`](src/view.rs): the `<tonk-view>` dumb renderer (wasm only).
- [`template`](src/template.rs): segment parsing, the chrome/repeat binding plan, and DOM walking (planning is target-independent; DOM walking is wasm only).
- [`render`](src/render.rs): the mounted-state DOM renderer for a `<tonk-view>` frame (wasm only).
- [`fold`](src/fold.rs): `select_rows`, the multi-row to conclusion-per-subject collapser. Target-independent.
- [`notation_format`](src/notation_format.rs): conclusion-to-`head!:` notation formatter, also used by `tonk-ui`.
- [`introspect`](src/introspect.rs): the `<tonk-introspect>` overlay. The
  state machine and slot description are target-independent; the element is
  wasm only.

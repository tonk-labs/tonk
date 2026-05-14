# `<tonk-display>` — single-entity rendering

## Context

`<tonk-concept>` exists today and renders **many** matched entities into an
author-supplied template (template lives as a child of the element). We
want a sibling element that renders **one** entity, with the template
fetched from a `View` concept stored on the branch rather than authored
in the page.

Target usage:

```html
<tonk-display
    entity="did:key:zGreeting…"
    model="greeting"
    view="basic" />
```

After v1 lands, we likely refactor `<tonk-concept>` to delegate per-row rendering to `<tonk-display>` — but that is out of scope here.

## The `view` concept

The element resolves its template by querying a `view` concept on the
branch. The concept must exist before any `<tonk-display>` can find a
matching template — declare it once per repository, alongside any other
domain concepts:

```yaml
concept!: &view
  description: HTML template used for displaying a concept
  with:
    name:
      description: Name of the view
      the: xyz.tonk.view/name
      as: text
    model:
      description: Concept being displayed
      the: xyz.tonk.view/model
      as: entity
    display:
      description: HTML template used for displaying source entity
      the: xyz.tonk.view/display
      as: text
```

A concrete view is then an assertion against that concept — one assertion
per `(model, name)` pair:

```yaml
# Define the concept being displayed.
concept!: &greeting
  with:
    message:
      description: Message to be displayed
      the: xyz.tonk.greeting/message
      as: text

# Publish a "basic" view for greetings.
view!:
  model: greeting
  name: "basic"
  display: !text/html |
    <p class="greeting">{message}</p>

# Example instance of the greeting concept.
greeting!: &demo
  message: Hello, world!
```

The element finds this view by querying for the `view` row whose
`model` equals the resolved `greeting` concept entity and whose `name`
equals `"basic"`. The `display` field carries the template HTML, which
the element parses and renders with the matched entity's fields
interpolated for `{message}` (and any other `{field}` references).

Authoring multiple views for the same concept — e.g. `name: "card"`,
`name: "tile"` — lets pages pick a presentation by passing a different
`view` attribute, with no element or schema changes.

## Three elements, one orchestrator

The crate ships three custom elements that compose:

- **`<tonk-display>`** — the orchestrator. Owns *all* subscriptions
  for the entity it's been pointed at. Mounts children as
  presentation slides; never paints anything itself.
- **`<tonk-view>`** — a dumb single-template renderer. Snapshots
  its child markup as a binding-plan template at
  `connectedCallback`, exposes a `.render(conclusion)` method that
  patches the cloned template in place. No network, no
  subscriptions.
- **`<tonk-inspector>`** — a Observable-style value renderer.
  Exposes a `.render(value)` method that walks any JS value and
  paints it (quoted strings, bare numbers, italic null/undefined,
  collapsible nested objects/arrays). No network, no
  subscriptions.

`<tonk-display>` decides what slides to mount based on its `view`
attribute and pushes data into them via property method calls. The
slide elements never reach upward; they're pure presentation.

## Element shape

```html
<tonk-display
    entity="<uri>"
    [model="<concept-name-or-uri>"]
    [view="<view-name>"]
    [space="<space>"]
    [branch="<branch>"]>
</tonk-display>
```

Attributes (all observed; changing any restarts the relevant flows):

| Attribute | Required | Meaning |
|---|---|---|
| `entity` | yes | URI of the thing to display. |
| `model` | yes (v1) | Concept name or URI. Used to resolve the descriptor + drive view lookups. Optional `model` is deferred. |
| `view` | no | View name. If omitted, the element opens a "views for this model" subscription and mounts every available view as a slide in a `<wa-carousel>` (see "Carousel mode" below). |
| `space` | no | Defaults to `"home"`. |
| `branch` | no | Defaults to `"main"`. |

No children — the template comes from the branch (or fallback), not the
page.

## URL route

`<tonk-display>` is exposed as a route in the Tonk UI shell:

```
/space/{space}/branch/{branch}/display/{subject}?view=<name>&model=<concept>
```

Path parameters become element attributes; query parameters become the
optional `view` and `model` attributes. The shell does the name → entity
resolution *before* mounting the element, so the route can 404 cleanly
when the bookmark doesn't resolve.

| Segment | Meaning |
|---|---|
| `{space}` | Repository space name. Forwarded as `space`. |
| `{branch}` | Branch name. Forwarded as `branch`. |
| `{subject}` | Either an entity URI (anything containing `:`) or a bookmark name. URIs pass through verbatim. Bookmark names are resolved by the route via a `Name` query (`this = id:<subject>`, read `entity` claim backed by `dialog.name/referent`). A bookmark that doesn't resolve renders a 404 section instead of mounting the element. |
| `?view=<name>` | View name. Forwarded as `view`. |
| `?model=<concept>` | Concept name or URI. Forwarded as `model`. |

Examples (assume the branch contains `name!: demo` → `did:key:zGreeting…`
and a `greeting` concept with a `basic` view):

```
# Resolve "demo" via Name, render with the "basic" view of "greeting":
/space/home/branch/main/display/demo?model=greeting&view=basic

# Same target, but the URI is given directly — no Name lookup:
/space/home/branch/main/display/did:key:zGreeting…?model=greeting&view=basic

# Bookmark exists, no view/model — falls back to generic <dl> rendering:
/space/home/branch/main/display/demo

# Bookmark doesn't exist — 404:
/space/home/branch/main/display/unknown
```

Resolving the name at the route rather than inside `<tonk-display>` keeps
the element decoupled from URL semantics, and gives us a place to render
a "not found" page when the lookup fails — something the element, by
design a live subscription, couldn't express cleanly.

## DOM state signalling

The element reflects its lifecycle into the host as attributes/classes so
stylesheets can react:

| State | Reflected as |
|---|---|
| Initial / resolving | `<tonk-display data-state="loading">` |
| Rendered successfully | `<tonk-display data-state="ready">` |
| Entity not found / empty stream | `<tonk-display data-state="empty">` |
| Concept / view / network failure | `<tonk-display data-state="error">` |

This lets authors write `tonk-display[data-state="empty"] { display: none; }`
to silently hide missing entities, or surface a styled message instead.
The element itself never injects fallback chrome — it only sets the
attribute. Error detail is still dispatched as a custom event for
diagnostics.

## Data flows

Three concurrent flows, each with its own abort handle:

1. **Concept resolution** — one-shot lookup of the concept descriptor
   from `model`. Skipped if `model` is absent (fallback path).
2. **View subscription** — watches the matching `View` row so a template
   edited on the branch swaps the rendered DOM. Skipped if `view` is
   absent (fallback path).
3. **Entity subscription** — watches the entity's attributes. Frame size
   is 0 or 1.

When all three settle the host transitions from `loading` to `ready`.

### Coordinator behaviour

Two independent inputs (template + entity) feed one renderer. The
renderer caches the last entity frame so either input can fire first
and either can change later without losing the other.

```
       ┌─────────────────────┐         ┌─────────────────────┐
       │  view subscription  │         │ entity subscription │
       │   → template HTML   │         │  → field conclusion │
       └──────────┬──────────┘         └──────────┬──────────┘
                  │                               │
              on change                       on change
                  │                               │
                  ▼                               ▼
         rebuild renderer            cache conclusion;
         from new HTML;              data-state ← empty|ready
         re-apply cached  ◄──────── (renderer re-renders
            conclusion                in place when present)
                  │                               │
                  └───────────────┬───────────────┘
                                  ▼
                        host DOM + data-state
```

Three rules govern transitions, and that's the whole behaviour:

- **Template arrives or changes** → rebuild the renderer; re-apply the
  cached entity conclusion if there is one.
- **Entity frame is non-empty** → `data-state = "ready"`; renderer
  patches the DOM in place.
- **Entity frame is empty** → `data-state = "empty"`; clear the DOM but
  keep the renderer and keep listening (entity may reappear).

On attribute changes (`entity`/`model`/`view`/`space`/`branch`), the
coordinator aborts both subscriptions, clears the cached conclusion,
sets `data-state = "loading"`, and re-opens whichever flows the new
attributes call for.

## Fallback rendering

The element must still produce something useful when the author omits
`view` or `model`. There are three cases:

### A. `model` present, `view` absent

Fallback: render each concept field as a `<dt>`/`<dd>` pair (or similar
generic layout). The renderer knows the descriptor and the entity's
field map; it walks every field name and emits text.

```html
<dl class="tonk-display-fallback">
  <dt>message</dt><dd>Hello</dd>
  <dt>recipient</dt><dd>Alice</dd>
</dl>
```

### B. `model` absent, `view` present

Treated as a usage error. View resolution requires knowing which concept
to scope to (the View row's `model` constant). Fire `data-state="error"`
with a clear message. (Could be loosened later if `view` is permitted to
be a globally-unique name.)

### C. Both `model` and `view` absent

Fallback: query *all attributes* on the entity (no concept descriptor)
and render the same generic `<dt>`/`<dd>` list, keyed by attribute URI.

This is the "tonk-inspect" mode — useful for ad-hoc debugging and for
embedding when the author doesn't care about a curated presentation.

The exact wire shape of an "all attributes" query is a worker-side
question (likely a query against the raw `(entity, ?, ?)` triple
pattern); we'll resolve the precise query during implementation. **We
may defer this fallback to a follow-up** if it requires worker changes
— shipping cases A and B is enough to land the element.

### D. `model` absent + entity in worker (interaction with case C)

A live attribute stream that fires whenever any claim on the entity
updates. Same DOM signal as case C, just streaming.

## Rendering

Single-row in-place reconciliation:

- Parse template HTML into a `DocumentFragment`, extract a binding plan.
- On first frame: clone fragment, apply bindings, append to host.
- On subsequent frames: walk bindings, render each, skip writes whose
  value is unchanged (write-deduped), patch the rest.
- On template frame: drop current DOM, parse new HTML, rebuild plan,
  re-render with the cached last conclusion.

`{field}` substitution covers text nodes and attribute values. Field
names match the descriptor's `with:` keys; `{this}` resolves to the
entity URI. Same semantics `<tonk-concept>` already documents — no
expression syntax, no event-handler wiring (out of scope).

## Where the template/binding code lives

The template-parsing and binding-plan code is generic and not specific
to either element. Plan: **extract it from `tonk-concept` into a new
`tonk-template` crate**, then have both `tonk-concept` and `tonk-display`
depend on it. Concretely the new crate would own:

- `Segment`, `parse_segments`, `has_field`
- `Binding`, `BindingKind`, `BindingPlan`
- `extract_plan`, `navigate`, `render_segments`
- (browser-only) the DOM helpers in `tonk-concept`'s `template::dom`

It would **not** own the renderer — diffing strategy differs between
many-row and single-row, and is cheap to write per element.

Shared SSE/error helpers can move alongside (`tonk-template` is the
narrow option; `tonk-rt` or similar is a broader option if more shared
plumbing appears).

This extraction is mechanical and additive; we can do it as the first
commit of the implementation series, before any `tonk-display` code.

## Lifecycle events

| Event | When | Detail |
|---|---|---|
| `tonk-display:connected` | All initial subscriptions opened | none |
| `tonk-display:result` | Entity frame applied | `{ this, fields }` |
| `tonk-display:template` | Template (view) row changed and DOM was rebuilt | `{ name, model }` |
| `tonk-display:error` | Lookup / network / parse failure | `{ kind, message }` |

`data-state` on the host is the canonical signal for styling; events are
for diagnostics.

## Crate layout

```
rust/tonk-template/         # new — shared parse + binding plan
  src/lib.rs
  src/segment.rs            # Segment, parse_segments
  src/plan.rs               # Binding, BindingKind, BindingPlan
  src/dom.rs                # extract_plan, navigate, render_segments (wasm32)

rust/tonk-display/          # new
  Cargo.toml
  src/lib.rs                # pub fn register() — wasm32
  src/element.rs            # CustomElement impl, lifecycle
  src/coordinator.rs        # concept + view + entity flow orchestration
  src/render.rs             # single-row renderer with template swap
  src/resolve.rs            # query builders for view + entity (+ fallbacks)
  src/state.rs              # data-state reflection helper
  src/error.rs

rust/tonk-concept/          # edited
  src/template.rs           # most contents move to tonk-template; thin re-export
  Cargo.toml                # add tonk-template dependency
```

Registered alongside `tonk_concept::register()` in
`rust/tonk-ui/src/bin/ui.rs`.

## Implementation order

1. Extract `tonk-template` (no behaviour change for `tonk-concept`).
2. Skeleton `tonk-display` crate + `register()`; observe attributes; reflect
   `data-state="loading"`.
3. Concept resolution + entity subscription (case A only, ignoring `view`).
4. Single-row renderer with template-swap support.
5. View subscription wired in.
6. Empty-stream → `data-state="empty"` handling.
7. Fallback rendering (case A: `<dl>` from descriptor).
8. Fallback rendering (case C: all-attributes query) — defer if worker
   work needed.
9. Register in `tonk-ui` shell.

## Tests

Native (no DOM):

- query builders: view query constrains `model` + `name`; entity query
  constrains `this`; descriptor field iteration produces correct
  projection terms.

WASM (real DOM):

- renders a single entity into a `view` template
- updates fields in place on a state frame
- swaps DOM wholesale on a template frame
- write-dedupes unchanged fields
- transitions to `data-state="empty"` when the entity disappears
- fallback `<dl>` rendering when `view` is absent

## Open questions

1. **Bookmark for `entity`?** Spec example uses a URI. v1 accepts URIs
   only; missing `:` is an error. Bookmark resolution can be added later
   without breaking callers.
2. **"All attributes" query shape (case C).** Needs alignment with the
   worker's query surface — flagging for implementation.
3. **Empty view vs error.** If `(model, view)` matches zero rows, do we
   fire `:error` or fall back to case A's `<dl>`? Recommend
   **fall back to A** so a typo'd view name still shows the entity.

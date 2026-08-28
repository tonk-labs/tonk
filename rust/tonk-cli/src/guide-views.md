# Views: rendering data

A **view** is an HTML template bound to a concept. The tonk-ui host
renders live branch data through `<tonk-display>` and `<tonk-view>`;
neither needs a framework or a `<script>`. `tonk view add` is the
convenient authoring path and expands to an assertion of the `view`
concept; use `--notation` to inspect that document.

## The `view` concept

`view` is `{model, display}`: the concept it renders and the HTML
template. Assert one per presentation:

```yaml tonk=parse
view!: &person-card
  this: id:person-card
  model: person
  display: !text/html |
    <article>
      <h2>{name}</h2>
      <p>{age}</p>
    </article>
```

`{field}` placeholders interpolate the rendered entity's fields, drawn
from the `model` concept's shape. `&person-card` publishes the anchor
name; the explicit `this: id:person-card` pins the view entity so a
later assertion supersedes its cardinality-one `display` instead of
leaving the old view entity queryable. `tonk view add --anchor person-card`
derives this stable `this: id:person-card` automatically.

## Authoring detail, directory, label, and title views

`tonk view add` authors a detail view by default. Select any standard view
concept explicitly with `--kind detail|directory|label|title`; each kind gets a
stable default anchor (`<model>-view`, `<model>-directory`, `<model>-label`, or
`<model>-title`). Supplying `--anchor person-card` derives
`this: id:person-card`; omitting it generates the stable kind-specific anchor.

```text
tonk view add todo --kind directory --template-file todo.html --home
```

A first detail or directory view automatically surfaces its model while the
home is blank. Label and title views do not. `--home` explicitly replaces an
existing home with this one concept's directory and commits the view plus home
change atomically. Without it, an existing home is always preserved.

## `<tonk-display>` — one entity through a view

`<tonk-display entity=<uri> model=<concept> view=<view-concept>>`
renders a single entity. The resolution that trips people up:

- `entity` must be an entity **URI** — something containing `:`
  (`did:key:…`, `id:foo`, or `{this}`, which interpolates one). The
  browser shell rejects a bare name (`entity=alice`) with
  "`entity` must be an entity URI"; headless `tonk render` is more
  lenient, so a template that SSRs fine can still break live. Always
  write `{this}` or a URI.
- `model` is the entity's concept; it projects the entity's fields.
- `view` is a **view concept** (e.g. the built-in `tonk:view`), NOT a
  specific view instance. `<tonk-display>` resolves the view concept,
  then runs a **model-constrained query** to find every view instance
  whose `model` equals the resolved model. Every matching entry renders in view
  entity order.

So you author the instance with a `model` (above) and point callers at
the concept:

```yaml tonk=illustrative-view-config
# Correct: the sheet/host references the view CONCEPT.
view: tonk:view
# Wrong: referencing a view instance (id:person-card) makes the
# concept-of-concepts lookup miss → "Not found / no concept matched".
```

Omit `view` to fall back to the built-in detail view for the model. A
`<tonk-display>` with a `model` but no `entity` renders a **directory
view** — every instance of the model — using the model's
`view/directory`, or a default carousel.

Three routes reach a view in the shell, and `tonk render` (next
section) takes the same three:

- `/space/<space>/<model>` — the model's directory.
- `/space/<space>/<entity>@<model>` — one entity, model's own view.
- `/space/<space>/<entity>@<model>!<view>` — one entity through an
  explicit view concept.

Handing the repo to someone else is a separate act: `tonk invite`.

## Render to HTML headlessly: `tonk render`

`tonk render <route>` runs the same model → view → entity resolution
the browser `<tonk-display>` runs, and prints the resulting HTML — no
browser, no service worker. The route is the shorthand:

- `tonk render person` — directory: every instance of `person`.
- `tonk render alice@person` — one entity (`{entity}@{model}`).
- `tonk render alice@person!tonk:view/label` — one entity through an
  explicit view concept (`{entity}@{model}!{view}`).

It writes HTML to stdout, or to a file with `--out`. It resolves
`{dom.host/model}`, falls back to the `_:_` default view when a model
has no specific one, and renders nested `<tonk-display>` recursively.

Headless rendering resolves templates and nested `<tonk-display>` elements,
but it does not run custom elements or their JavaScript. In particular, the
seeded `portal` model (below) prints a `<tonk-portal>` element headlessly; only
the browser runtime turns that element into its sandboxed iframe and installs
the `window.tonk` bridge.

## Rendering a reference by name (cross-concept join)

When one concept points at another (an entity reference), a `{field}`
placeholder interpolates the field's **raw value** — the target's URI
(`did:key:…`), not a name. To show the referenced entity's name you must
**nest a `<tonk-display>`** over the reference field; interpolating the
field alone never resolves it.

Render the reference through a small **label view** — a `view/label`
instance. It lives under the built-in `tonk:view/label` concept, so it
doesn't collide with the model's default `tonk:view`:

```yaml tonk=parse
# A comment points at its author (a person).
view!: &comment-card
  this: id:comment-card
  model: comment
  display: !text/html |
    <article>
      <strong><tonk-display entity={author} model=person view=tonk:view/label></tonk-display></strong>
      <p>{body}</p>
    </article>

# The label view the line above resolves: just the person's name.
view/label!: &person-label
  this: id:person-label
  model: person
  display: !text/html |
    {name}
```

`<tonk-display entity={author} …>` follows the `author` reference to the
person entity and renders it through the `tonk:view/label` view
constrained to `model: person`, so the card shows the name, not
`did:key:…`. Writing `{author}` directly would print the URI. The same
nesting renders any reference: `entity` is the reference field, `model`
the referenced concept, and `view` the view concept you want
(`tonk:view/label` for just a name, `tonk:view` for the full detail
card).

## Built-in view elements

Ready-made custom elements a view can drop in, no script needed. Each
has a full page: **`tonk help <element>`**.

| Element | What it is | Bind by |
|---------|-----------|---------|
| `<tonk-display>` | Render an entity (or every instance) through a view. The primitive everything else hangs off. | `model` + `entity` attrs |
| `<tonk-prose>` | Typora-style markdown editor. | text content; `onchange` |
| `<tonk-code>` | CodeMirror code editor with per-language highlighting. | `value`/`language` attrs; `onchange` |
| `<tonk-table>` | IronCalc spreadsheet — live formulas, sheets, per-cell claims. | text (CSV) or `subject` + `<tonk-display>` rows |

```
tonk help tonk-table     # full docs for one element
tonk help tonk-prose
```

The editors persist the same way: bind the store's value in (as element
text or an attribute), fire a command on the element's `change` event
(read `dom.event.detail/…`), and a rule writes it back — the loop in
`tonk help events`. `<tonk-table>` also offers a store-native *claims*
mode (one claim per cell). Your own components (below) are peers of
these.

## Web components

Views can freely use any custom element already registered in the
rendering realm — the built-in `<tonk-*>` elements above and the Web
Awesome `<wa-*>` set (`<wa-icon>`, `<wa-carousel>`, …) — with no
script. A `<script>` written directly in a `display:` template never
executes (templates render through inert fragments), so behaviour the
template language can't express (rich editing, canvas, drag
interactions) is packaged as a **web component** instead.

A component is branch data: a `component` row whose `module` field is
a JS module that defines your element.

```yaml tonk=eval
component!: &tally-widget
  module: |
    customElements.get('tally-widget') || customElements.define('tally-widget',
      class extends HTMLElement {
        connectedCallback() {
          this.addEventListener('click', () => this.dispatchEvent(
            new CustomEvent('bump', { bubbles: true, detail: { amount: 1 } })));
        }
      });
```

Mount the component directory once, in a view that always renders
(typically your root/shell view); it is invisible and loads every
component on the branch:

```html
<tonk-display model=component />
```

`<tonk-component>` executes each module once per realm (de-duplicated
by content), and from then on `<tally-widget>` upgrades wherever any
view renders it. Rules of the road:

- **Guard definitions** with `customElements.get(name) ||` — a custom
  element name cannot be redefined, so an edited component takes
  effect on the next page load.
- **Data flows in** through attributes the view binds (`<tally-widget
  count={count}>`) and through child rows the view renders inside the
  element; **actions flow out** as bubbling `CustomEvent`s, wired
  exactly like clicks — `onbump=<command>` on the element plus
  `dom.event.detail/amount` fields on the command (see `tonk help
  events`). The built-in `<tonk-sheet-binder>` works this way; your
  components are peers of it.
- **One-off inline form**: inside a view's `display`, a
  `<tonk-component>` wrapping an inert holder
  `<script type="tonk/module">…</script>` executes that source the
  same way — handy while prototyping, before promoting the source to
  a `component` row.
- Components **share the realm** with every view on the branch —
  that is the point (they compose with bindings and events). For a
  fully isolated third-party page, use a portal (below) instead.

## Escape hatch: the `portal` model

For an imperative HTML document, assert the always-seeded `portal` concept.
Its `content` may contain scripts and query through `window.tonk` in the live
browser:

```yaml tonk=eval
portal!: &about
  this: id:about
  content: |
    <h1>About</h1>
    <script>
      window.tonk.query().then(console.log)
    </script>
```

Open it at `/space/<space>/about@portal`. `tonk render about@portal` verifies
the outer declarative markup, but cannot execute `<tonk-portal>` or the script.
The live element prepends the bridge bootstrap and mounts the document in an
opaque-origin sandboxed iframe.

`tonk view` is a lower-level, claim-driven inventory: it lists entities carrying
one of the four view-template attributes, plus legacy bare `text/html` claims
served by the worker's guest-host endpoint. A bare `text/html` claim is not a
`tonk render` route and is distinct from the seeded `portal` concept above.

---

For interactivity (clicks, forms) see `tonk help events`. Don't
memorize built-ins — run `tonk show` / `tonk concept` /
`tonk view` to see what's on the branch.

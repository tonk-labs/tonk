# Views: rendering data

A **view** is a model's set of HTML templates, keyed by *facet*. The
tonk-ui host renders live branch data through `<tonk-display>` and
`<tonk-view>`; neither needs a framework or a `<script>`. `tonk view
add` is the convenient authoring path and expands to an assertion of
the `view` concept; use `--notation` to inspect that document.

## The `view` concept

A `view` instance's `this` IS the model — the concept being rendered —
and its one field `show` is a dictionary of templates keyed by facet:
`ui` (the detail presentation), `directory` (every instance), `label`,
`title`, or any facet name you pick. Assert entries together or one at
a time:

```yaml tonk=parse
view!:
  this: person
  show:
    ui: |
      <article>
        <h2>{name}</h2>
        <p>{age}</p>
      </article>
    title: Person {name}
```

`{field}` placeholders interpolate the rendered entity's fields, drawn
from the model concept's shape. Each entry lands as its own fact
(`<model> xyz.tonk.view/<facet> <template>`) with cardinality one, so
re-asserting a facet supersedes that template — there is no separate
view entity to name or pin. The `view` concept itself is seeded by the
standard library, pinned to `tonk:view`.

## Authoring ui, directory, label, and title facets

`tonk view add` authors the `ui` facet by default. Select a facet with
`--kind detail|directory|label|title` (writing `ui`, `directory`,
`label`, `title` respectively):

```text
tonk view add todo --kind directory --template-file todo.html --home
```

A first detail or directory view automatically surfaces its model while the
home is blank. Label and title views do not. `--home` explicitly replaces an
existing home with this one concept's directory and commits the view plus home
change atomically. Without it, an existing home is always preserved.

## `<tonk-display>` — one entity through a view

`<tonk-display entity=<uri> model=<concept> view=<facet>>`
renders a single entity. The resolution that trips people up:

- `entity` must be an entity **URI** — something containing `:`
  (`did:key:…`, `id:foo`, or `{this}`, which interpolates one). The
  browser shell rejects a bare name (`entity=alice`) with
  "`entity` must be an entity URI"; headless `tonk render` is more
  lenient, so a template that SSRs fine can still break live. Always
  write `{this}` or a URI.
- `model` is the entity's concept; it projects the entity's fields
  AND names the view instance — the model entity's `show` dictionary
  is where templates come from.
- `view` is a **facet name** (`label`, `title`, …), NOT a concept or
  an entity. Omit it for the mode default: `ui` when `entity` is set,
  `directory` when it is not (a `<tonk-display>` with a `model` but
  no `entity` renders every instance of the model through the
  `directory` facet, or a default carousel).

Three routes reach a view in the shell, and `tonk render` (next
section) takes the same three:

- `/space/<space>/<model>` — the model's directory.
- `/space/<space>/<entity>@<model>` — one entity, the `ui` facet.
- `/space/<space>/<entity>@<model>!<facet>` — one entity through an
  explicit facet.

Handing the repo to someone else is a separate act: `tonk invite`.

## Render to HTML headlessly: `tonk render`

`tonk render <route>` runs the same model → view → entity resolution
the browser `<tonk-display>` runs, and prints the resulting HTML — no
browser, no service worker. The route is the shorthand:

- `tonk render person` — directory: every instance of `person`.
- `tonk render alice@person` — one entity (`{entity}@{model}`).
- `tonk render alice@person!label` — one entity through an explicit
  facet (`{entity}@{model}!{facet}`).

It writes HTML to stdout, or to a file with `--out`. It resolves
`{dom.host/model}`, falls back to the `tonk:_` default dictionary when
a model's own lacks the facet, and renders nested `<tonk-display>`
recursively.

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

Render the reference through a small **label facet** on the referenced
model — a distinct entry, so it never collides with the model's `ui`:

```yaml tonk=parse
# A comment points at its author (a person).
view!:
  this: comment
  show:
    ui: |
      <article>
        <strong><tonk-display entity={author} model=person view=label></tonk-display></strong>
        <p>{body}</p>
      </article>

# The label facet the line above resolves: just the person's name.
view!:
  this: person
  show:
    label: |
      {name}
```

`<tonk-display entity={author} …>` follows the `author` reference to the
person entity and renders it through `person`'s `label` facet, so the
card shows the name, not `did:key:…`. Writing `{author}` directly would
print the URI. The same nesting renders any reference: `entity` is the
reference field, `model` the referenced concept, and `view` the facet
you want (`label` for just a name, none for the full `ui` card).

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
mode (one claim per cell). Your own elements (below) are peers of
these.

## Web components

Views can freely use any custom element already registered in the
rendering realm — the built-in `<tonk-*>` elements above and the Web
Awesome `<wa-*>` set (`<wa-icon>`, `<wa-carousel>`, …) — with no
script. A `<script>` written directly in a template never executes
(templates render through inert fragments), so behaviour the
template language can't express (rich editing, canvas, drag
interactions) is packaged as a **web component** instead.

A custom element is branch data: an `element` row whose `method`
dictionary holds its functions. The row's identity IS the tag it
defines (`element:<tag>`), the same way a view's identity is the model
it renders — and identity never moves, so a later assertion supersedes
only the methods it names.

```text
tonk element add tally-widget --method-file connected=tally.js
tonk element                       # every element defined on the branch
```

`method` is the same construct as a view's `show`: one fact per entry,
cardinality one. **A view is a dictionary of templates keyed by facet;
an element is a dictionary of functions keyed by method.** Re-authoring
`connected` alone leaves `disconnected` standing, exactly as
re-authoring one view facet leaves the rest of `show` alone.

```yaml tonk=eval
element!: &tally-widget
  this: element:tally-widget
  method:
    connected: |
      (self) => {
        self.textContent = `count: ${self.getAttribute('count') ?? 0}`;
      }
    bump: |
      (self) => self.dispatchEvent(
        new CustomEvent('bump', { bubbles: true, detail: { amount: 1 } }))
```

Each value is a JS arrow function taking the element as its first
argument. Four keys are dispatched by the DOM lifecycle:

| Key | Signature |
|-----|-----------|
| `connected` | `(self) => …` |
| `disconnected` | `(self) => …` |
| `adopted` | `(self) => …` |
| `attribute-changed` | `(self, name, before, after) => …` |

Any other key becomes a method on the element, camelCased —
`attribute-changed` is `self.attributeChanged`, a custom `bump` is
`self.bump()`. Keys stay kebab in the data, matching every other tonk
key; `el['my-method']()` is not callable JS but `el.myMethod()` is. A
key that would shadow a member every element already has (`remove`,
`click`, `id`, `text-content`) is refused at authoring time.

Mount the element directory once, in a view that always renders
(typically your root/shell view); it is invisible and loads every
element on the branch:

```html
<tonk-display model=element />
```

The runtime registers each tag ONCE, with a generated wrapper that
dispatches through a live table (tag → method → function) kept in step
with these facts by a subscription. So editing a method is a fact
write, not a re-registration — `customElements.define` is never called
twice, and no page reload is needed. Rules of the road:

- **Data flows in** through attributes the view binds (`<tally-widget
  count={count}>`) and through child rows the view renders inside the
  element; **actions flow out** as bubbling `CustomEvent`s, wired
  exactly like clicks — `onbump=<command>` on the element plus
  `dom.event.detail/amount` fields on the command (see `tonk help
  events`). The built-in `<tonk-sheet-binder>` works this way; your
  elements are peers of it.
- **The escape hatch is a reserved key.** What a method table cannot
  say — `static formAssociated`, extending a built-in — goes in
  `define`, `() => class extends HTMLElement { … }`, whose returned
  class is registered instead of the wrapper, ignoring every other key.
- Elements **share the realm** with every view on the branch —
  that is the point (they compose with bindings and events). For a
  fully isolated third-party page, use a portal (below) instead.
- The older `component` concept carries one anonymous JS module with no
  identity: an assertion that omits `this:` is keyed by its own body
  digest, so an edit writes a SECOND row, the directory mounts both,
  and whichever module runs first wins. Branches seeded before
  `element` still load their `component` rows (`tonk element` lists
  them); author new definitions as `element`.

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

`tonk view` is a lower-level, claim-driven inventory: it lists every model
carrying `show` entries, plus legacy bare `text/html` claims served by the
worker's guest-host endpoint. A bare `text/html` claim is not a `tonk render`
route and is distinct from the seeded `portal` concept above.

---

For interactivity (clicks, forms) see `tonk help events`. Don't
memorize built-ins — run `tonk show` / `tonk concept` /
`tonk view` to see what's on the branch.

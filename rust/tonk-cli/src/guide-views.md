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
dictionary holds its functions, published under its tag with an
`&anchor`. The browser resolves that name the first time it meets the
tag in a rendered view — nothing is registered ahead of time, and
nothing has to be mounted.

```text
tonk element add tally-widget --description 'A running tally' \
  --method-file connected=tally.js
tonk element                       # every element defined on the branch
```

`method` is the same construct as a view's `show`: one fact per entry,
cardinality one. **A view is a dictionary of templates keyed by facet;
an element is a dictionary of functions keyed by method.**

```yaml tonk=eval
element!: &tally-widget
  description: "A running tally, incremented by its own bump event"
  method:
    connected: |
      (self) => {
        self.textContent = `count: ${self.getAttribute('count') ?? 0}`;
      }
    bump: |
      (self) => self.dispatchEvent(
        new CustomEvent('bump', { bubbles: true, detail: { amount: 1 } }))
```

The `&tally-widget` anchor is where the tag lives. It publishes
`id:tally-widget` over whatever entity the assertion derives, and that
name is the only mutable part: re-author the tag and the name moves to
the new definition, so every instance on every open page follows. The
body carries no copy of the tag — a second, immutable answer to the
same question would only be able to disagree with the first.

`description` is required, the way a concept's is: an element is read
by people and by agents with only the branch to go on, and `<tally-widget>`
does not say what it is for. Quote it, like any text field — a bare
symbol is read as a reference to something else on the branch.

Both fields reach the entity digest, so the entity IS this definition:
change a method and you have a different element, and the anchor
repoints. Editing is a separate road from deriving, and it still works
fact by fact — name the entity and assert only the key you are
changing:

```yaml tonk=illustrative-entity-stands-in-for-a-real-one
element!:
  this: did:key:z6Mk…            # what &tally-widget names today
  method:
    connected: |
      (self) => { … }
```

which supersedes that one fact and leaves the rest standing, exactly as
re-authoring one view facet leaves the rest of `show` alone. `tonk
element add` works in tags rather than entities, so it takes the other
road: it reads the tag's current methods, lays the ones you named over
them, and re-derives — naming one method still edits just that one.

Each value is a JS arrow function taking the element as its first
argument. Four keys are dispatched by the DOM lifecycle:

| Key | Signature |
|-----|-----------|
| `connected` | `(self) => …` |
| `disconnected` | `(self) => …` |
| `adopted` | `(self) => …` |
| `attribute-changed` | `(self, name, before, after) => …` |

Two more cover something the DOM has no notion of: your definition
being **replaced while instances are live**.

| Key | Runs on | For |
|-----|---------|-----|
| `released` | the outgoing definition, before the swap | undo what `connected` set up; stash anything worth keeping on the element |
| `swapped` | the incoming definition, after it | take over the live instance |

Without `released`, a redefinition re-runs `connected` on top of setup
the previous one left behind — a listener, a timer, an observer per
edit, with nothing able to undo them. Without `swapped`, `connected` is
re-run, which is right only when there was nothing to undo; declaring
`swapped` means the new definition decides how to adopt an instance
that is already mounted, and `connected` goes back to meaning a fresh
mount. Both fire on any change to the definition; `connected` is only
re-run when `connected` itself changed.

```yaml tonk=illustrative-fragment-of-a-method-map
released: |
  (self) => { clearInterval(self.__timer); self.dataset.at = self.dataset.count; }
swapped: |
  (self) => { self.textContent = `resuming from ${self.dataset.at}`; }
```

Any other key becomes a method on the element, camelCased —
`attribute-changed` is `self.attributeChanged`, a custom `bump` is
`self.bump()`. Keys stay kebab in the data, matching every other tonk
key; `el['my-method']()` is not callable JS but `el.myMethod()` is. A
key that would shadow a member every element already has (`remove`,
`click`, `id`, `text-content`) is refused at authoring time.

That is also how one method calls another: through the element, as
`self.total()`. The call resolves through the table at call time, not
at definition time, so re-authoring `total` alone changes what an
untouched `connected` computes — the same liveness the lifecycle hooks
get, extended to the methods you name yourself.

There is no `observedAttributes` to declare. `attribute-changed` is
driven by a `MutationObserver` watching every attribute, not by
`attributeChangedCallback`, whose list is read once when the tag is
registered and could therefore never grow with an edited method. Your
hook sees every attribute, including the ones already present when an
instance upgrades (those are replayed with `before` as `null`), and a
hook that writes an attribute does not re-enter itself.

What you *can* declare is a **default**:

```yaml tonk=illustrative-fragment-of-an-element
attribute:
  color: "red"
  size: ""
```

An instance that does not carry the attribute gets it written on
before `connected` runs — into the DOM, so `getAttribute`, a CSS
`[color=red]` rule and devtools all agree. A value the view supplied
wins, and `hasAttribute` is the test, so `count="0"` and `label=""`
count as supplied. `size: ""` above declares a default of the empty
string, which is a real attribute state (`<input disabled="">`) and
not the same as declaring nothing.

Quote the values — a default is *data*, and a bare `red` would be read
as a reference to something else on the branch. Adding a default later
reaches instances already mounted, the way an edited method does.

```text
tonk element add tally-widget --description 'A running tally' \
  --attribute color=red --method-file connected=tally.js
```

The map narrows nothing: it supplies defaults, and that is all. Most
elements declare none and simply leave it out — a keyed collection is
zero-or-more, so an `element!:` body without it is complete, not
partial.

### Properties

`self.total()` is a method call; `el.total` is a property, and a
consumer written against the DOM expects the second. Declare those in
their own maps:

```yaml tonk=illustrative-fragment-of-an-element
getter:
  total: |
    (self) => Number(self.dataset.n ?? 0)
setter:
  total: |
    (self, next) => { self.dataset.n = String(next); }
```

Both maps are keyed by property name: declaring both makes the property
read-write, a getter alone makes it read-only, and a setter alone makes
it write-only — a real shape (a sink that takes a value and renders it),
so it is built rather than refused. Names camelCase like method keys
(`row-count` → `el.rowCount`), and one that would shadow a member every
element already has is refused.

Like a method, each half resolves through the branch's current
definition at access time, so re-authoring a getter changes what a page
already reading the property sees.

### What four dictionaries cost

`tonk query element` answers only for an element that declares *all
four* maps, which almost none do. A generic concept query binds every
field the concept declares, and a keyed collection with no entries binds
nothing. `tonk element` — the listing you actually use — reads the
domains directly and lists them all, which is why it exists; the browser
registry runs one query per dictionary for the same reason.

Nothing registers your element ahead of time. The runtime watches the
document for custom elements nobody has defined (`:not(:defined)`, the
browser's own answer to that question) and looks each tag up by name the
first time it appears — so a view that renders `<tally-widget>` is the
whole trigger. An undefined element is inert until its definition lands
and then upgrades in place, which is what makes resolving after render
safe. Each tag is asked about once per page, whether it renders once or
a hundred times, and a tag with no definition on the branch is asked
about once and left alone.

Once resolved, the tag is registered ONCE, with a generated wrapper that
dispatches through a table the branch keeps current. So editing a method
is a fact write, not a re-registration — `customElements.define` is
never called twice, and no page reload is needed. Concretely:

- A tag rendered before anything defines it sits inert, and registers
  itself when the definition lands. Nothing has to re-render.
- Re-authoring a tag replaces the implementation for instances already
  on the page, including reverting it to a definition used earlier.
- Adding one method to an element already registered gives that method
  to instances already mounted.
- Pointing the tag's name at a different element swaps the whole
  definition; the one it moved away from can no longer change it.

The two halves are separate on purpose. Noticing dispatches a
`tonk-element-needed` event from the element that needs the definition;
a listener installed once at guest bootstrap answers it by resolving the
tag and registering it. Neither knows how the other works — so anywhere
the observer cannot see (a shadow root, a detached fragment) can
announce its own tags and get the same treatment, and the event bubbles
from the element so `with="branch@repo"` context still applies.

Rules of the road:

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
- **The older `component` shape still works, alongside this one.** A
  `component` row is one anonymous JS module that calls
  `customElements.define` itself. The two concepts share nothing — a
  component's facts are `xyz.tonk.component/module`, an element's are
  `xyz.tonk.element.method/<key>` — so a branch carries both, each with
  its own loader, and nothing has to be migrated. `tonk element` lists
  both, naming the concept each row came from. What `component` cannot
  offer is identity: an assertion that omits `this:` is keyed by its own
  body digest, so an edit writes a SECOND row, the directory mounts
  both, and whichever module runs first wins. Prefer `element` for new
  work for that reason, not because `component` stops working.

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

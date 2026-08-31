# View templates: interpolation, iteration, and directories

This documents the template language `<tonk-display>` uses: how an
HTML template with `{placeholder}` holes is filled from a concept's
fields, how cardinality-many fields make a subtree repeat, and how a
single template renders either *one* entity or a *directory* of many.

The planning half lives in `tonk-concept` (`template.rs` parses a
template into a chrome/repeat plan and substitutes fields); `tonk-display`
owns the rendering half (`render.rs` diffs each frame into the DOM).
This doc lives in `tonk-core` because the model is grounded in
`tonk-core`'s `conclusion` primitive, which is the unit a template
renders.

## The shape of a template

A view template — one entry of a model's `show` dictionary — is an
HTML fragment with `{field}` holes:

```html
<article>
  <h1>{title}</h1>
  <p>{summary}</p>
</article>
```

Each hole names a field of the concept being rendered (a key in the
concept's `with:` map), plus the reserved name `{this}` for the
subject's own entity URI. A hole may appear in text (`<h1>{title}</h1>`)
or in an attribute value (`<a href="/x/{this}">`, `data-id={this}`).

Rendering binds the holes from a **conclusion** — the result of
evaluating the concept against a subject:

```
conclusion.this    = the subject entity URI
conclusion.fields  = { title: "...", summary: "...", ... }
```

`{this}` resolves to `conclusion.this`; `{field}` resolves to
`conclusion.fields[field]`.

## Cardinality drives iteration

A field declared `cardinality: many` holds a *list* of values, not a
scalar. The engine renders a many-field by **repeating the smallest
subtree that contains the hole**, once per value:

```html
<ul>
  <li>{ingredient}</li>   <!-- ingredient is cardinality-many -->
</ul>
```

renders as one `<ul>` with one `<li>` per ingredient. The `<li>` is the
**iteration root**: the outermost element whose content depends on the
many-valued field, cloned per value.

The rule that picks the iteration root is purely structural (it does
*not* consult the descriptor at plan time):

1. For each field referenced in the template, collect the host element
   of every hole that mentions it (the smallest enclosing element).
2. Take the **longest common ancestor** of those host elements. That
   element is the field's single iteration root, and every hole
   referencing the field is nested inside it.
3. If two holes for the same field have no shared inner ancestor (they
   are independent siblings), each becomes its own iteration root and
   repeats independently.

At **render** time the engine looks at the actual value: an
`Ipld::List` iterates (one clone per item), a scalar renders once. So
cardinality is observed from the data, not hard-coded in the plan — a
`cardinality: one` field is simply the degenerate case that iterates
exactly once.

Inside a repeated clone, the iterating field is *shadowed* to the
current item: `{ingredient}` resolves to this clone's value, while
fields from the enclosing scope (e.g. `{title}`) stay visible. `{this}`
inside the clone still refers to the subject (see below).

### Choosing what repeats

The iteration root is the *outermost* element that references the
many-field, so you control the repeated unit by where you place the
hole. Given a cardinality-many `step` field, to repeat list items but
not the list:

```html
<ol class="recipe">
  <li>{step}</li>
</ol>
```

The `<li>` is the iteration root; the `<ol>` renders once and one `<li>`
appears per step. Lift the hole to a wrapping element if you want a
larger unit (e.g. a `<li>` containing several holes of the same field
collapses to one `<li>` per value via the longest-common-ancestor
rule).

## `{this}` is the root scope, not a field

`{this}` is the subject — the entity a conclusion is *about*. It is
distinct from the cardinality-many field iteration above: a many-field
repeats a subtree over a list of values *within one conclusion*, while
`{this}` is the **root scope**, one instance per *conclusion*. The
renderer already loops over a frame of conclusions and renders one unit
per conclusion, keyed by `this`. That per-conclusion loop *is* the
`this` iteration; views don't normally bind `{this}` to express it.

A **directory** of N subjects is just N conclusions. To render them
into one outer structure (a table, a list) with shared chrome, a
template names the element that should repeat by binding `{this}` on it:

```html
<table>
  <thead><tr><th>title</th></tr></thead>
  <tbody>
    <tr subject={this}>          <!-- the repeat root -->
      <td>{title}</td>
    </tr>
  </tbody>
</table>
```

The element that binds `{this}` is the **repeat root**: everything
outside it (`<table>`, `<thead>`, `<tbody>`) renders **once** as chrome,
and the repeat root clones **once per conclusion**, each clone resolving
`{this}`/`{field}` against its conclusion.

The marker is *any attribute on the element whose value is exactly
`{this}`* — the attribute's name doesn't matter (`subject={this}`,
`data-with={this}`, `for={this}` are equivalent) and neither does the
element (`<tr>`, `<li>`, `<div>`). What counts is that an attribute
binds `{this}`, which is what lifts that element to be the repeat root.
Use an attribute rather than a bare `{this}` text node so the whole
element repeats, not just a text node inside it; a mixed value like
`href="/x/{this}"` is a URL substitution, not a marker, so it does not
lift a repeat root.

When a template binds **no** `{this}`, the repeat root defaults to the
whole fragment — the entire template clones per conclusion. And in a
**single-entity**
render there is exactly one conclusion, so the template renders once
whether or not it binds `{this}`. Single-entity is the one-conclusion
case of the same per-conclusion loop.

## Two renderers, one entity vs. a directory

`<tonk-display>` resolves which template to use by querying the
subject's model entity for its `show` dictionary — the model IS the
view instance — then picks a facet and renders:

- **Single entity** — `entity` attribute set. The facet is `ui`
  (unless `view=` names another). The query pins `conclusion.this` to
  that one URI; the template renders once. Any `{this}` iteration
  runs over a one-element set.

- **Directory** — `entity` attribute *not* set. There is no single
  subject, so the engine picks the **`directory` facet** and runs a
  query for *all* instances of the model. The matched subjects arrive
  as a frame of N conclusions (one per instance). The directory
  template's chrome renders once and its `{this}` repeat root (the
  `<tr>`) clones per conclusion.

  If the model's dictionary has no `directory` entry,
  `<tonk-display>` falls back to the `tonk:_` default dictionary's
  `directory` facet (the seeded carousel).

The facets differ only in which entry of the dictionary carries the
template (`xyz.tonk.view/ui` vs `xyz.tonk.view/directory`), so a
model declares its detail and directory presentations independently
— and any other facet (`label`, `title`, …) besides.

## Worked example

A `trip` concept with `title` (one) and `stop` (many). Detail view:

```yaml
view!:
  this: trip
  show:
    ui: |
      <section>
        <h1>{title}</h1>
        <ol>
          <li data-stop={stop}>{stop}</li>
        </ol>
      </section>
```

Rendered for one trip: one `<section>`, one `<h1>`, and one `<li>` per
stop.

Directory view over *all* trips:

```yaml
view!:
  this: trip
  show:
    directory: |
      <table>
        <thead><tr><th>trip</th><th>title</th></tr></thead>
        <tbody>
          <tr subject={this}>
            <td>{this}</td>
            <td>{title}</td>
          </tr>
        </tbody>
      </table>
```

`<tonk-display model=trip>` (no `entity`) runs the all-trips query and
gets one conclusion per trip. The `<table>` chrome renders once; the
`<tr subject={this}>` repeat root clones once per conclusion, each
row's `{this}`/`{title}` resolving to that trip. The same template
language; the only difference from a detail view is that the frame
carries many conclusions instead of one, and the template names which
element repeats.

## Empty directories and the fallback region

A directory can have **zero** instances — a fresh repo before anything
is created. The chrome/repeat split already covers this: the chrome
renders once regardless of instance count, and the repeat root simply
produces no rows. So a directory view stays mounted when empty, showing
its chrome with an empty body, and reconciles **in place** the moment the
first instance lands — no reload.

To turn that into a landing page, put a fallback region in the chrome
(any element that references no subject field, so it is not lifted into
the repeat) and gate its visibility on the host's lifecycle state.
`<tonk-display>` reflects that state on itself as `data-state`:
`loading` while resolving, `empty` when the collection has zero
instances, `ready` once at least one is rendered. The fallback shows
under `empty` and the entries show under `ready`:

```yaml
view!:
  this: trip
  show:
    directory: |
      <style>
        .launchpad { display: none; }
        tonk-display[data-state="empty"] .launchpad { display: block; }
        tonk-display[data-state="empty"] .trips    { display: none; }
      </style>
      <ul class="trips">
        <li subject={this}>{title}</li>
      </ul>
      <div class="launchpad">
        <h1>No trips yet</h1>
        <button onclick=create>Plan one</button>
      </div>
```

The `<ul class="trips">` repeat root clones per trip; the
`<div class="launchpad">` is chrome (no `{this}`/`{field}` reference), so
it is always present and the stylesheet decides when it shows. On an
empty repo the host is `data-state="empty"` → the launchpad is visible
and the (empty) list is hidden. When the first trip lands the host flips
to `data-state="ready"` → the list shows and the launchpad hides, live,
because the same mounted view reconciled rather than being torn down and
rebuilt.

### `<tonk-fallback>`: the launchpad without the CSS

`<tonk-fallback>` packages the "show only when empty" half. It finds its
nearest `<tonk-display>` ancestor, reads that host's `data-state`, and
hides itself unless the state is `empty` — watching the host so the flip
is live. The entries stay as the normal repeat; the fallback needs no
stylesheet:

```yaml
view!:
  this: trip
  show:
    directory: |
      <ul>
        <li subject={this}>{title}</li>
      </ul>
      <tonk-fallback>
        <h1>No trips yet</h1>
        <button onclick=create>Plan one</button>
      </tonk-fallback>
```

It is sibling chrome (no subject reference), so it is decoupled from how
the entries render — a `{this}` repeat as above, or nested per-instance
`<tonk-display>`s. Place it in the *outer* directory template, not inside
a nested per-instance display, so `closest()` reads the outer
collection's emptiness.

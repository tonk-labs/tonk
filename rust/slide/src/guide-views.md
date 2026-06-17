# Views: rendering data

A **view** is an HTML template bound to a concept. The tonk-ui host
ships two custom elements that render live branch data into views;
neither needs a framework or a `<script>`. Author views by asserting
the `view` concept — there is no separate write path.

## The `view` concept

`view` is `{model, display}`: the concept it renders and the HTML
template. Assert one per presentation:

```yaml
view!: &person-card
  model: person
  display: !text/html |
    <article>
      <h2>{name}</h2>
      <p>{age}</p>
    </article>
```

`{field}` placeholders interpolate the rendered entity's fields, drawn
from the `model` concept's shape. A view is identified by its **anchor
name** (`&person-card` publishes `id:person-card`); re-asserting the
same anchor with a new `display` re-points the name, so edits replace
in place and never leave duplicate rows.

## `<tonk-display>` — one entity through a view

`<tonk-display entity=<uri> model=<concept> view=<view-concept>>`
renders a single entity. The resolution that trips people up:

- `model` is the entity's concept; it projects the entity's fields.
- `view` is a **view concept** (e.g. the built-in `tonk:view`), NOT a
  specific view instance. `<tonk-display>` resolves the view concept,
  then runs a **model-constrained query** to find the view instance
  whose `model` equals the resolved model.

So you author the instance with a `model` (above) and point callers at
the concept:

```yaml
# Correct: the sheet/host references the view CONCEPT.
view: tonk:view
# Wrong: referencing a view instance (id:person-card) makes the
# concept-of-concepts lookup miss → "Not found / no concept matched".
```

Omit `view` to fall back to the built-in detail view for the model. A
`<tonk-display>` with a `model` but no `entity` renders a **directory
view** — every instance of the model — using the model's
`view/directory`, or a default carousel.

Share a display: `slide share display <entity> --view <view-name>`
(or `--model <concept>` for carousel mode).

## Render to HTML headlessly: `slide render`

`slide render <route>` runs the same model → view → entity resolution
the browser `<tonk-display>` runs, and prints the resulting HTML — no
browser, no service worker. The route is the shorthand:

- `slide render person` — directory: every instance of `person`.
- `slide render alice@person` — one entity (`{entity}@{model}`).
- `slide render alice@person!card` — one entity through an explicit
  view concept (`{entity}@{model}!{view}`).

It writes HTML to stdout, or to a file with `--out`. It resolves
`{dom.host/model}`, falls back to the `_:_` default view when a model
has no specific one, and renders nested `<tonk-display>` recursively.
A `type: text/html` (portal) view is emitted as an `<iframe srcdoc>`;
because there's no live runtime, a portal's own data-fetch doesn't run
under SSR.

## Escape hatch: raw `text/html` views

For a one-off HTML page (no live binding), assert a `text/html` claim
and serve it through the iframe viewer:

```yaml
attribute!: &html-body
  description: "HTML body of a one-off page"
  the:         text/html
  as:          text
  cardinality: many

concept!: &page
  with: { body: html-body }

page!: &about
  body: |
    <h1>About</h1>
```

`slide views` lists every entity carrying a `text/html` claim;
`slide share view <name>` opens it in the iframe viewer. The viewer
shell does not register `<tonk-display>`, so events won't fire there —
use `slide share display` for interactive, data-bound views.

---

For interactivity (clicks, forms) see `slide guide events`. Don't
memorize built-ins — run `slide schema` / `slide concepts` /
`slide views` to see what's on the branch.

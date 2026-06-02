# Rewiring `<tonk-display>` view resolution

Design source: `@gozala/2026-06-01.md` ("Simpler Views"). This plan is the
implementation breakdown; the journal is the spec.

## The new model

The `view` attribute names a **concept** whose `display` field we query
for, scoped by the subject's `model`. Both `view` and `model` are concept
references — qualified (a URI) or named (a name resolved to a URI).

```yaml
# the concept named by view=
attr_view:
  model: attr_model   # which model this view is for
  display: ?template  # the template we bind and render
```

`<tonk-display entity=E model=M view=V>` resolves like this:

1. Resolve concept `V` (the view concept) — name → URI, or use the URI.
2. Resolve concept `M` (the subject's model) — name → URI, or use the URI.
3. Query: find the entity that is an instance of `V` whose `model` field
   equals `M`; project its `display` → `?template`. (If several `V` rows
   share model `M`, pick one — the multiple-entities case is out of scope.)
4. Subscribe to subject `E` under model `M`; render `?template` against it.

`model` is now load-bearing: it constrains the view query. This reverses
today's single-view behavior where `model` was inert and the view declared
its own model.

`view` is **optional**. When omitted, `<tonk-display>` uses the built-in
`view` concept (journal: "if omitted we use built-in `view` concept"). So
the fallback is not a special generic renderer — it's the same resolution
with `V` = the built-in `view` concept (whose `{model, display}` descriptor
is known as a constant, `view_predicate`, so the default needs no resolve).

## Carousel mode (`view="about:blank"`)

`view="about:blank"` is a sentinel selecting **carousel** — enumerate
*every* view defined for the model, across all view concepts, and mount
each as a slide. It queries the **abstract `display` contract**, which is a
concept with **only a `model` field** (no `display` of its own — journal
lines 52-62):

```yaml
concept: &display
  with:
    model: { the: xyz.tonk.view/model, as: entity }
```

Querying `{model}`-constrained-to-M, capturing `display` as a variable,
matches *any* conforming view concept (built-in `view`, custom `preview`,
`page`, …) because they all share the `xyz.tonk.view/model` attribute. This
is strictly better than the old carousel, which enumerated only the one
hardcoded `view` concept. (`about:blank` is the same sentinel a new
artifact's `view` carries until a presentation is chosen.)

## Views are an open-world contract

A view is **any concept that is a superset of the abstract `display`
contract**: a `model` (entity) field plus a `display` text field carrying
the HTML template (journal lines 48-62). The built-in `view` is one such
concept; users define their own (`&preview`, and the built-ins `&opener`,
`&page`) with the same `model` + `display` shape and extra fields of their
own. `<tonk-display>` doesn't hardcode one view concept — it queries
whatever concept `view=V` names, requiring only that it carries `model`
and `display`.

Two other built-in view concepts the journal specifies, both just this
contract with behavior attached:

- **`opener`** — used when `entity` is omitted. Renders a picker of
  entities matching `model`; selecting one emits an `open` command that
  sets the host's `entity` (journal "Opener View"). Needs the `open`
  command + a rule; out of scope for the first rewire.
- **`page`** — `type: text/html`; rendered in a sandboxed iframe instead
  of inline (journal "Page View"). The `type` fork below is the seam.

## What this replaces

Today's single-view path resolves the view by **anchor name**: `view=basic`
→ `id:basic` → `dialog.name/referent` → one view entity → read *its*
`model` + `display`. Three round-trips (`name_target_query` →
`view_fields_query` → `resolve_model`), and the view must be published under
a globally-unique anchor. The carousel path (`views_for_model_query`)
already does the model-constrained shape the new design wants.

So the rewire **converges** the two paths: both query a view concept
constrained by `model`. Single-view pins the predicate to the specific
view concept `V`; carousel uses the generic `view` concept (all views for
the model).

## The `view` concept

`{model, display}`, plus an optional manually-specified `type`
(`xyz.tonk.view/type`, default `text/dialog-ui`). `type` is an ordinary
field the author sets (`type: text/html`); nothing special happens at
lowering — no `!text/html` tag handling.

```yaml
concept!: &view
  with:
    model:   { the: xyz.tonk.view/model,   as: entity, cardinality: one }
    type:    { the: xyz.tonk.view/type,    as: text,   cardinality: one }  # optional
    display: { the: xyz.tonk.view/display, as: text,   cardinality: one }
```

## Where the work lands

- **`resolve.rs`** — replace the name-based builders. Drop
  `name_target_query`; rework single-view to a model-constrained query over
  an arbitrary view-concept predicate (the `view=V` concept's descriptor),
  projecting `display` (+ `type`). `views_for_model_query` stays as the
  carousel; the two now share a shape, parameterized by predicate.
- **`element.rs`** (single-view branch, ~456-510) — replace the three-step
  name resolution with: resolve concept `V` → resolve concept `M` → build
  the constrained query → subscribe. Resolve both `view` and `model` as
  concept refs (name-or-URI) via the existing `resolve_model` mechanism
  (generalized to "resolve concept ref").
- **`type` threading** — carry `type` from the resolved view to the mount
  chokepoint (`mount_view_slide`), add a `type`-keyed fork with the
  `text/html` branch stubbed (mount the template path as today). No iframe
  yet.

## Then: board + workspace fall out

Once `<tonk-display>` resolves `view=V model=M` by querying concept `V`
constrained to model `M`, the board and workspace bootstraps just declare
view concepts and instances in that shape. The board's `view=basic`
references become `view=<view-concept>` with the model carrying the
constraint, and the broken anchor-name lookups go away.

## Scope of the first rewire

In scope: resolution by view-concept + model, `view` optional → built-in
`view` concept, the open-world `{model, display}` contract, the `type`
field threaded to the mount fork (iframe branch stubbed). Then fix board +
workspace to the new shape.

Out of scope (later, per journal): the `opener` view (needs the `open`
command + a rule), the `page` iframe rendering (fill in the stubbed
`text/html` fork), and the `?display`-style future notation (journal
lines 94-129).

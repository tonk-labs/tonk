# `<tonk-concept>` — live concept rendering

A custom element that opens a live subscription against a
tonk-worker `/query` endpoint and renders each match into an
author-supplied row template. One element per live view; no
framework wrapper, no signals, just markup. Rows insert,
update, and remove as the underlying branch changes.

The element is registered by `tonk-concept`'s `register()` (the
shell does this at startup; pages don't have to). Once registered,
drop the tag anywhere in the document.

## Shape

```html
<tonk-concept source="<concept>[?<filters>]"
              [space="<space>"]
              [branch="<branch>"]>
  <!-- chrome + row template (see "Template detection") -->
</tonk-concept>
```

Three observed attributes:

- **`source`** (required) — what to query. Bookmark name
  (`person`) or concept entity URI (anything containing `:`).
  May include a `?key=value` filter string; see "Source syntax"
  below.
- **`space`** (optional) — override the repository the
  subscription opens against. Defaults to the space the
  element is currently rendered inside.
- **`branch`** (optional) — override the branch. Defaults to
  `main`.

Changing any of these attributes after the element is connected
tears down the current subscription and starts a fresh one.

## Source syntax

```
<bookmark-or-uri>[?<key>=<value>[&<key>=<value>...]]
```

- The portion before the optional `?` is the concept name (e.g.
  `person`) or an entity URI (anything containing `:`).
- Each `<key>=<value>` becomes a constant constraint on the live
  query — only matches whose `<key>` field equals `<value>`
  surface.
- Decoding follows `URLSearchParams`: `+` is space, `%xx` is
  hex-decoded. Use percent-encoding for spaces in values.
- Bare `?key` entries (no `=`) are dropped silently. Projection
  is implicit — every concept field is delivered to the template,
  whether or not you reference it in `{field}` interpolations.

Examples:

```html
<!-- Every person on the branch. -->
<tonk-concept source="person">…</tonk-concept>

<!-- Only Alice. -->
<tonk-concept source="person?name=Alice">…</tonk-concept>

<!-- Multiple filters. -->
<tonk-concept source="task?status=open&assignee=alice">…</tonk-concept>

<!-- Direct entity URI (no bookmark lookup). -->
<tonk-concept source="did:key:zHj…">…</tonk-concept>
```

## Template detection

Two modes, detected at `connectedCallback`. The element's child
markup is **structural**: how rows are inserted and where they
live in the DOM both follow from how you arrange it.

### With a `<template>` child (preserves chrome)

If there's a `<template>` element anywhere inside, its `content`
becomes the cloneable per-row fragment. The `<template>`'s parent
becomes the **row container** — all chrome at or above that
parent stays in the DOM as-is. The `<template>` element itself
is removed once snapshotted (it's an instruction, not visible
content).

```html
<tonk-concept source="person">
  <ul>
    <template>
      <li><b>{name}</b> is {age}</li>
    </template>
  </ul>
</tonk-concept>
```

The `<ul>` stays. Each matched person becomes one `<li>` inside
it. The `<template>` tag disappears.

This is the form to reach for when the chrome matters — tables,
lists, cards — because semantic wrappers like `<tbody>` or
`<ul>` survive.

### Without a `<template>` child (host as container)

If no `<template>` is present, every direct child of
`<tonk-concept>` is moved into a fresh fragment and treated as
one row template. Rows are appended directly inside the
`<tonk-concept>` element.

```html
<tonk-concept source="person">
  <article>
    <h3>{name}</h3>
    <p>Age: {age}</p>
  </article>
</tonk-concept>
```

Each matched person becomes one cloned `<article>` (plus
whatever sibling element nodes you put alongside it) inside the
host. There's no chrome boundary — the host element itself is
the container.

## `{field}` substitution

`{field}` placeholders are substituted with the matched row's
field values. Substitution works in two places:

- **Text content** — anywhere text appears in the template,
  including text mixed with other markup:
  ```html
  <li>{name} ({age} years)</li>
  ```
- **Attribute values** — full or partial:
  ```html
  <a href="/people/{name}" class="card-{kind}">{name}</a>
  ```

### Field-name rules

- `{` opens an interpolation; `}` closes it. Everything in
  between is the **literal field name** — no expressions,
  arithmetic, function calls, or property access.
- `{name + "x"}` is treated as a field literally named
  `name + "x"`. The lookup misses; the substitution renders empty.
- An unterminated `{name` (no closing `}`) is emitted as literal
  text — the element does not error.
- There is **no escape for a literal `{`**. If you need a `{` in
  your output, the element can't currently express it.
- Field names are case-sensitive and match the concept's `with:`
  map exactly.

### Missing-field semantics

If a frame's row doesn't carry a value for a referenced field,
the substitution renders empty. The element does not warn or
inject a placeholder — the row just stays partially blank.
Define the concept's `with:` fields to match what your template
references.

## Update behaviour

Each frame from the subscription is reconciled against the
current row set:

- **Identity** is the entity URI on the conclusion (the `this`
  binding). Rows persist across frames as long as their entity
  stays in the result set.
- **New entity** → clone the template, fill in fields, append at
  the end of the container.
- **Existing entity, changed field** → update only the changed
  bindings in place. No DOM thrash.
- **Existing entity, unchanged field** → write-deduped; the
  element compares the new rendering against the cached one and
  skips the DOM write.
- **Entity dropped from the result** → remove that row's nodes.

Insertion order follows the order rows arrive in the frame.
There's no client-side sort; if you need ordering, express it
in the concept query or sort the markup after the fact.

## Lifecycle events

The element dispatches three custom events on itself; pages can
listen for them to wire diagnostics, loading indicators, or
analytics.

| Event | When | Detail |
|---|---|---|
| `tonk-concept:connected` | `connectedCallback` ran and the SSE subscription is opening | none |
| `tonk-concept:result` | Each frame applied to the DOM | `{ count: <row count> }` |
| `tonk-concept:error` | Parse / resolve / network failure | `{ kind: <ErrorKind>, message: <human-readable> }` |

All three bubble and are composed; standard `addEventListener`
on the host (or an ancestor) sees them.

## Known limitations

- **No literal `{` in output.** Tracked in
  `template.rs::parse_segments` rustdoc; flagged for a future
  escape rule.
- **No expressions in `{…}`.** Single bare identifiers only.
- **No client-side sort or pagination.** The element renders
  whatever the subscription delivers, in delivery order.
- **No `<template>` discovery inside Shadow DOM.** The element
  itself doesn't use Shadow DOM (`shadow() = false`), so
  light-DOM authoring works as documented; this matters only if
  the element is hosted inside someone else's Shadow root.

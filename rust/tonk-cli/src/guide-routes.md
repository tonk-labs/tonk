# Routes: pages at paths

A **route** gives a page in a space its own path. It maps a path
pattern to a **route model**: a concept whose `ui` view is the page.
Opening `/space/<space>/notes` renders whatever route claims `/notes`.

Without a route of your own, the standard library already answers
three shapes (see `tonk help views`): `/<model>` for a directory,
`/<entity>@<model>` for one entity, and `/<entity>@<model>!<facet>`.
Write a route when a page deserves a plain path, or when the page is
more than one model's view.

## A notes index and a note page

This defines a `note`, a page at `/notes` listing every note, and a
page at `/notes/<note>` showing one:

```yaml tonk=eval
concept!: &note
  description: A short note
  with:
    title:
      description: The note's title
      the: xyz.example.note/title
      as: text
    body:
      description: The note's text
      the: xyz.example.note/body
      as: text

view!:
  this: note
  show:
    ui: |
      <article><h1>{title}</h1><p>{body}</p></article>
    link: |
      <p><a href="/notes/{this}">{title}</a></p>

# The page at /notes. Its fields are read off the tab's site entity.
concept!: &notes/index-page
  this: id:notes/index-page
  description: The notes index page
  with:
    repo:
      description: The space repository, picked off the site entity
      the: xyz.tonk.site/repo
      as: text
    branch:
      description: The space branch, picked off the site entity
      the: xyz.tonk.site/branch
      as: text

view!:
  this: notes/index-page
  show:
    ui: |
      <h1>Notes</h1>
      <tonk-display with="{branch}@{repo}" model=note view=link></tonk-display>

route!:
  this: id:route/notes
  path: "/notes"
  concept: notes/index-page

# The page at /notes/<note>. `{*entity}` captures the note's URI.
concept!: &notes/note-page
  this: id:notes/note-page
  description: One note on its own page
  with:
    entity:
      description: The note from the path, picked off the site entity
      the: xyz.tonk.site/entity
      as: entity
    repo:
      description: The space repository, picked off the site entity
      the: xyz.tonk.site/repo
      as: text
    branch:
      description: The space branch, picked off the site entity
      the: xyz.tonk.site/branch
      as: text

view!:
  this: notes/note-page
  show:
    ui: |
      <p><a href="/notes">All notes</a></p>
      <tonk-display with="{branch}@{repo}" entity={entity} model=note></tonk-display>

route!:
  this: id:route/note
  path: "/notes/{*entity}"
  concept: notes/note-page

# A note to list.
note!:
  title: "First"
  body: "Routes are data too."
```

Read the routes back:

```yaml tonk=parse
route:
  this: ?route
  path: ?path
  concept: ?concept
```

Open `/space/<space>/notes`, then follow a link. The index lists every
note through its `link` facet, whose link is an absolute path
(`/notes/{this}`). The page resolves it inside the space, so
`/notes/did:key:…` lands on `/space/<space>/notes/did:key:…` and the
note page renders.

## How a path becomes a page

1. The tab's path below `/space/<space>` is matched against the
   space's `route` rows. The match picks a route and captures its
   params.
2. The worker stamps the tab's **site entity** (`tonk:site`, one per
   tab, kept in the session overlay and never synced) with the matched
   route model as its `concept`, and each captured param as an
   `xyz.tonk.site/<name>` fact. It also stamps `xyz.tonk.site/repo`
   and `xyz.tonk.site/branch`.
3. The page renders `<tonk-display entity={site} model={concept}>`:
   the route model resolves on the site entity and its `ui` facet
   renders.

So a route model is an ordinary concept whose fields name site
attributes. It may only **require** attributes the site carries for
that route; a required field the site lacks fails to resolve and the
page shows a concept mismatch. Put an optional one under `maybe:`.

Pin every route model with `this:`, as above. A concept's identity
comes from its fields alone, not its description, and most route
models declare the same few site fields: two unpinned ones with the
same fields are one concept, sharing one `ui` view, so one page would
replace the other.

A navigation re-stamps the same site in place: the captured params of
the old route are dropped, the new route's view renders, and the page
does not reload.

## Reference

### The `route` concept

| Field | Attribute | Meaning |
|-------|-----------|---------|
| `path` | `xyz.tonk.route/path` | The path pattern, relative to the space |
| `concept` | `xyz.tonk.route/concept` | The route model to render when it matches |

Give a route an explicit `this:` (`id:route/notes` above). Asserting
again with the same `this:` replaces its `path` or `concept` in place;
without it, an edited route is a second row on the same path, and the
two tie.

### Patterns

- Literal text matches itself: `/notes`.
- `{name}` captures one path segment: it stops at `/`.
- `{*name}` captures a span that may contain `/`, up to the next
  literal or the end. Use it for entity URIs (`id:notes/first`) and
  namespaced models (`tonk/person`).
- Several params may share one segment, separated by literals:
  `/{*entity}@{*model}!{*view}` is how the library's routes read
  `alice@person!label`.

A captured value is percent-decoded before it is stamped.

### Params on the site

Each captured param lands on the site entity as
`xyz.tonk.site/<name>`. A param named `entity` is stamped as an entity,
so the route model declares it `as: entity`; every other param is
stamped as text, so the model declares it `as: text`. Name the param
that carries an entity URI `entity`.

Besides params, a route model can pick these site fields:

| Field | Attribute | Type |
|-------|-----------|------|
| path | `xyz.tonk.site/path` | text: the path below the space |
| repo | `xyz.tonk.site/repo` | text: the space, for `with="{branch}@{repo}"` |
| branch | `xyz.tonk.site/branch` | text: the branch name |
| replica | `xyz.tonk.site/replica` | entity: this device's replica |
| route | `xyz.tonk.site/route` | entity: the matched route row |

### Precedence

The most specific pattern wins, whatever order the routes were written
in:

1. A pattern without params beats one with `{name}`, which beats one
   with `{*name}`.
2. Among those, more literal text wins: `/notes/{*entity}` beats the
   library's `/{*model}` for `/notes/id:a`.
3. Then fewer params.

So `/notes` claims that path from the library's `/{*model}` directory
route, and `/<model>` keeps working for every other model.

Two routes with the same pattern tie, and the router settles the tie
by entity URI, not by intent. The library ships its routes
(`/`, the directory, artifact and ad-hoc patterns) only for paths no
route claims yet. A route the space writes for one of those paths
stands beside the library's until the next library upgrade, which keeps
the space's. Prefer a new path to taking over a library one; for the
space's home page at `/`, use the home recipe in `tonk help views`.

### Rendering and links

- The route model's `ui` facet is the page. Pass
  `with="{branch}@{repo}"` on the `<tonk-display>` elements it nests,
  as above: elements that build worker URLs (uploads, media) need that
  pair on their ancestors.
- Write in-space links as absolute paths (`href="/notes/{this}"`). A
  relative link resolves against the space's root rather than the
  current page. `/join`, `/account`, `/inspector` and `/diagnose` are
  profile pages: a link to them from a space page leaves the space, so
  do not use them as space paths.
- `tonk render` takes the `entity@model!facet` shorthand, not paths,
  and does not consult routes. Check a route by opening its path.

---

For the templates a route renders, see `tonk help views`. For the
commands its pages dispatch, see `tonk help events`.

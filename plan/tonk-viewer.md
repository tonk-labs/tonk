# Tonk Viewer — bottom-tabs viewer over artifacts

## Context

We have a wireframe for the viewer ("Layout F · bottom tabs"): one repo
open at a time, a top bar with the repo title and a sync chip, a bottom
**tab strip** with one tab per *artifact*, and a **canvas** that renders
the selected artifact. The mockup carries many more states (members list,
activity feed, pause-sync, offline / partial-replication, conflict,
agent-editing), but the spine is simple: tabs of artifacts, click one,
see it rendered.

That spine maps onto what we already have. Each tab/sheet is a
`<tonk-display>` pointed at an entity with a model and a view. The
*artifact* concept is what the shell queries to know which tabs to show,
what to title them, and what each one points at. So an artifact is really
just `<tonk-display>` state plus a little tab metadata.

This plan covers the MVP: **tabs + canvas**. Members, activity, pause-sync
and the offline states are deferred. The two pieces of groundwork the MVP
needs — an open-world view contract and a unified page (iframe) view kind
— are pulled forward because the artifact/tab work sits cleanly on top of
them.

The data model below is drawn from the [2026-06-01 journal][journal].

[journal]: https://github.com/tonk-labs/tonk/blob/journal/%40gozala/2026-06-01.md

## Where we are today

`<tonk-display>` is most of the way there. It resolves a view by anchor
name, reads the view's own `model` + `display`, and renders the `display`
template with `{field}` binding (the binding engine lives in
`tonk-concept`). With no `view` attribute it falls back to a carousel of
every view for the model.

What's missing:

- **The view type is hardcoded.** The element queries one fixed `view`
  concept shape — `{model, display}` over `xyz.tonk.view/*`. There is no
  open-world notion of "any concept that is a superset of an abstract
  `display` concept is a view," and no `type` discriminator to tell a
  template view from a page view.

- **The `!text/html` tag is dropped.** Views already author
  `display: !text/html | …`, but the tag is discarded during lowering, so
  the content type never reaches the claim. Unifying the page view depends
  on carrying it through.

- **The page (iframe) view is a separate route.** A sandboxed-iframe
  viewer exists at `view/:entity`, backed by a `page` concept in
  `tonk-schema` and the worker's iframe bridge (the `globalThis.tonk`
  postMessage + `MessagePort` channel). It is not reachable through
  `<tonk-display>`, so it has its own URL, its own mounting, and doesn't
  participate in the view system.

- **There is no artifact concept, and no tab shell.** The shell is a
  space-rail with a single routed main area. The closest things to
  "multiple things at once" are `tonk-board` (strip → columns → tiles,
  each tile a nested `<tonk-display>`) and the headless `<tonk-layout>`
  (workspace + tile state, currently with no companion view so it renders
  nothing). Neither is a tab strip over artifacts.

- **Stale `name` field.** The `view` concept in `tonk-board`'s bootstrap
  still declares a `name` field that the resolver no longer reads — view
  identity moved to the anchor name. Worth removing while we're here.

The single chokepoint we'll keep returning to: the place inside
`<tonk-display>` where a `display` string becomes mounted DOM. That's the
one fork point for "template view vs page view," and the seam the whole
unification turns on.

## The view concept and its identity

A view carries a `model` (entity), a `display` (text), and an optional
`type` that selects how the template mounts.

```yaml
concept!: &view
  description: |
    A display template for a concept. `type` selects how it mounts:
    text/dialog-ui (inline template, default) or text/html (sandboxed page).
  with:
    model:
      description: Concept this view is defined for
      the: xyz.tonk.view/model
      as: entity
    type:
      description: text/dialog-ui (inline) or text/html (sandboxed page)
      the: xyz.tonk.view/type
      as: text
      cardinality: one
    display:
      description: HTML template for the view
      the: xyz.tonk.view/display
      as: text
      cardinality: one
```

The catch is identity. Nothing stops several views being published for the
same model, and if resolution then enumerates "all views for this model"
it has to pick one arbitrarily — the ambiguity we hit before. We fix this
by deriving a view's identity from its full body. Two views for the same
model with different templates are therefore different entities; there's no
collision to resolve.

```yaml
view!:
  model: counter
  display: !text/html |
    <span>{count}</span>
```

`type` defaults to `text/dialog-ui` when absent, so existing views render
unchanged. A page is the same concept with `type: text/html` and a
raw-HTML `display`.

```yaml
# A page view — rendered in a sandboxed iframe.
view!:
  model: counter
  type: text/html
  display: |
    <html>
      <body><h1 id="count"></h1>
        <script>
          parent.model.subscribe(c => count.textContent = c.count)
        </script>
      </body>
    </html>
```

```yaml
# A template view — rendered inline, as today.
view!: &counter-basic
  model: counter
  display: !text/html |
    <span>{count}</span>

# A page view — same contract, rendered in a sandboxed iframe.
view!: &counter-page
  model: counter
  type: text/html
  display: |
    <html>
      <body><h1 id="count"></h1>
        <script>
          parent.model.subscribe(c => count.textContent = c.count)
        </script>
      </body>
    </html>
```

A page reuses the worker's existing iframe bridge — the iframe content
subscribes to its bound `{repo, branch}` over `globalThis.tonk`, the same
channel the standalone page viewer already uses.

## The artifact concept

An artifact binds an entity to a model and a view, plus the bit of
metadata the tab needs.

```yaml
concept!: &artifact
  description: An entity displayed as a sheet/tab in the viewer.
  with:
    title:
      description: Title shown in the tab
      the: xyz.tonk.artifact/title
      as: text
    icon:
      description: Icon shown in the tab
      the: xyz.tonk.artifact/icon
      as: text
    entity:
      description: Entity displayed in the sheet (about:blank on a new artifact)
      the: xyz.tonk.artifact/entity
      as: entity
    model:
      description: Model concept used to display the entity (about:blank on a new artifact)
      the: xyz.tonk.artifact/model
      as: entity
    view:
      description: View concept used to display the entity (about:blank on a new artifact)
      the: xyz.tonk.artifact/view
      as: entity
```

An artifact's `(entity, model, view)` maps one-to-one onto
`<tonk-display entity model view>`. The tab strip subscribes to "every
artifact on this branch" and renders one tab per row; selecting a tab
mounts a `<tonk-display>` from that artifact's three fields. A freshly
created artifact carries `about:blank` placeholders and lands the user on
an empty-artifact canvas.

## The shell

A new viewer surface (route `…/viewer`) renders the Layout-F shape:

- **Top bar** — repo title, sync chip. (The members/share/activity
  controls are deferred; leave room for them.)
- **Tab strip** — subscribes to the artifact query, one tab per artifact:
  title, a `+` "new artifact" affordance that expands into an inline name
  prompt, ⌘N shortcuts, close ×.
- **Canvas** — the selected artifact rendered as
  `<tonk-display entity={artifact.entity} model={artifact.model}
  view={artifact.view}>` inside a status strip showing the artifact name.

The sheet/map/doc "flavors" in the mockup are not shell components — they
are just different views (templates) chosen by the artifact's `view`
field. The shell stays flavor-agnostic; the presentation comes from the
view documents.

We do *not* build the viewer on `<tonk-layout>` for the MVP — a flat
artifact query is simpler and gets us to "tabs render artifacts" fastest.
If we later want tab reorder, focus persistence, or split panes, that is
exactly what `<tonk-layout>` provides, and we revisit then.

## PR sequence

### PR 1 — derive view identity from fields

Make a view's identity derivable so multiple views for one model can't
collide: the implicit digest includes all resolved fields and the
notation/wire paths converge. Repoint
`<tonk-display>`'s view resolution to the derived entity instead of
enumerating views-for-model and picking one. Remove the stale `name` field
from the `view` concept in `tonk-board`'s bootstrap and migrate its
`view!:` instances.

### PR 2 — carry the content type through

Stop discarding the `!text/html` tag during lowering, and add the optional
`type` field to the view concept (`xyz.tonk.view/type`, default
`text/dialog-ui`). Thread `type` from the resolved view down to the mount
chokepoint and add a `type`-keyed fork there with the `text/html` branch
stubbed (mount the template path as today). No user-visible change yet;
existing views render exactly as before.

### PR 3 — page view through `<tonk-display>`

Fill in the `text/html` branch: mount a sandboxed iframe instead of the
inline template, reusing the worker's existing iframe bridge so the page
content gets its data over `globalThis.tonk`. Reconcile the existing
`page` concept in `tonk-schema` with the view contract (fold `content`
into `display`). At the end of this PR one rendering path —
`<tonk-display>` — covers both template and page views, and the separate
`view/:entity` route is redundant.

### PR 4 — artifact concept and query

Ship the `artifact` concept via the bootstrap pattern (a `claim!`-compiled
document seeded into the branch on repo creation, the way `tonk-board`'s
schema is). Seed a few demo artifacts mirroring the mockup
(Itinerary/sheet, Lodging map/map, Budget/sheet) so the shell has
something to show. Add the "all artifacts on this branch" subscription.

### PR 5 — the viewer shell

Build the bottom-tabs surface: top bar, tab strip subscribed to the
artifact query, canvas mounting a `<tonk-display>` per the selected
artifact. Inline new-artifact prompt → transact a new artifact with
`about:blank` placeholders → land on the empty-artifact canvas. This is
the MVP: open a branch, see artifact tabs, click one, the artifact
renders. It matches the mockup's `default`, `create-artifact`, and
`empty-artifact` states.

## Out of scope (post-MVP)

- **Opener view** — a view with `model` but no `entity` that renders a
  picker of matching entities and dispatches an `open` command to set the
  host's `entity`. Because a parent reacting to a child's command routes
  through the worker as a transient command plus a rule, this couples to
  the deductive-rules work and should follow it. The system fallback
  opener (generic picker when a model has none) comes with it.

- **The rest of the mockup chrome** — members list, activity feed +
  restore, pause-sync, offline / index-only states, conflict and
  agent-editing presence. Leave seams (a `materialized` flag on the tab, a
  lock set) but don't build them in the MVP.

## Open questions

- **Page src strategy.** The `text/html` branch can reuse the existing
  worker `guest` navigation URL shape (so the iframe wrapping + bridge
  apply unchanged), or introduce a `<tonk-portal>` element that owns the
  iframe and the bridge handshake and is mounted by `<tonk-display>`.
  Reusing `guest` is less new surface; resolve in PR 3.

- **Where artifacts live.** Whether the artifact concept ships in its own
  viewer bootstrap or extends an existing one. Either way it follows the
  `claim!` + seed-on-repo-creation pattern.

- **Tabs on `<tonk-layout>` later.** The MVP uses a flat artifact query.
  If reorder/focus/split become requirements, the tab strip moves onto
  `<tonk-layout>`'s tile state. Flag, don't decide now.

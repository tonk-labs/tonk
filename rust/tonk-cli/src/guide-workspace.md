# Building for the tonk-ui workspace

> App-layer and subject to change: the workspace shell is being
> reworked. The concepts below are how it works today. Always confirm
> against `tonk schema` rather than relying on this list.

The tonk-ui shell renders a **workspace** (`workspace`) as a strip of
**sheets** (tabs). Each sheet displays one entity through a model and a
view.

> Don't confuse `workspace` with `space`. `workspace` (pinned to
> `tonk:workspace`) is this tab surface, `{name, sheet (many), active}`.
> `space` is a separate profile-side record the Hub lists joinable
> repos from, `{name, subject, kind}` — not what you assert here.

## Concept catalogue

Run `tonk schema` for the authoritative shape. At a glance:

- `workspace`        — a workspace: `{name, sheet (many), active}`.
- `workspace/sheet` — a tab: `{title, subtitle, icon, order, entity, model, view}`.
- `artifact`        — the generic "entity shown as a tab" shape sheets build on.
- `view`, `view/directory` — display templates (see `tonk guide views`).
- `board`, `column-view`, `tile` — the kanban-style board layout.
- `portal`          — a sandboxed-iframe HTML document.
- `inspector`       — a built-in space-inspector tile.
- `workspace/create-sheet`, `workspace/activate-sheet`,
  `workspace/close-sheet` — the `command!:` kinds the tab strip fires
  (see `tonk guide events`).
- `workspace/active-sheet` — the durable `{active}` fact a rule writes
  in response to `activate-sheet`. (Durable, so not a `projection!:` —
  that word means an event-to-argument mapping, which is never stored.)

## The sheet recipe

A sheet points at the entity to show, the entity's model concept, and
the **view concept** to resolve a template through:

```yaml
workspace/sheet!: &sheet-alice
  title:    "Alice"
  subtitle: "person"
  icon:     "user"
  order:    "a"
  entity:   alice        # the entity to display
  model:    person       # its concept
  view:     tonk:view    # the VIEW CONCEPT, not a specific view
```

`view: tonk:view` is the part agents get wrong. The sheet's `view`
field is the *view concept*; `<tonk-display>` then runs a
model-constrained query to pick the actual view instance whose `model`
matches. So author the view instance with a `model`, and let the sheet
reference the concept:

```yaml
view!: &person-card        # the instance, selected by its model
  model: person
  display: !text/html |
    <article><h2>{name}</h2><p>{age}</p></article>
```

See `tonk guide views` for the resolution rule in full.

## Wiring sheets into a workspace

`workspace.sheet` is cardinality-many, and the notation has no list
syntax — add each member with its own `this:`-bound assertion. A fresh
anchored concept assertion must also set every field (or add `..: _`):

```yaml
workspace!: &lab
  name:   "Lab"
  active: sheet-alice
  sheet:  sheet-alice     # first member inline so every field is set

workspace!:
  this:  lab              # this: present, so a partial body is fine
  sheet: sheet-bob
```

Open it in the shell at `/space/<space>/<entity>@workspace` —
`lab@workspace` for the one above. `tonk render lab@workspace` prints
the same HTML headlessly. To put someone else in it, hand them the
repo with `tonk invite`.

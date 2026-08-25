# Building a tabbed workspace

The always-seeded core provides the outer `tonk:workspace/shell`. It is a
site-level route shell: it mounts the active replica through the `tonk/space`
name. On a fresh space that name points at `tonk:blank`, the lean starting
canvas. It is not a user-authored `workspace` record.

The optional sheets module in `rust/tonk-core/assets/library/sheets.yaml`
re-points `tonk/space` to `tonk:binder` and adds a tabbed workspace. The CLI
does not seed that module automatically; from a source checkout, install it
with:

```text
tonk eval rust/tonk-core/assets/library/sheets.yaml
```

Run `tonk show tonk/binder` and `tonk show workspace/sheet` after installation
for their authoritative fields.

## Current model

- `tonk/binder` is the active replica projected as `{subject, active?}`.
  `active` is the persisted default tab; the custom element owns the immediate
  live selection in DOM state.
- `artifact` is an entity that can be displayed as a tab:
  `{title, subtitle, icon, entity, model, view}`.
- `workspace/sheet` is an artifact plus the lexicographic `order` key that
  makes it a visible tab. Sheets are discovered by querying this concept;
  there is no `workspace.sheet` collection field.
- `empty-artifact` is the placeholder model used for a newly-created sheet.
- `workspace/create-sheet`, `workspace/activate-sheet`, and
  `workspace/close-sheet` are transient commands emitted by
  `<tonk-sheet-binder>` (see `tonk help events`).
- `workspace/sheet-order` is the one-field projection retracted when a tab is
  closed. The artifact facts survive, so closing demotes a sheet rather than
  deleting its content.

## Authoring a sheet

A sheet points at the entity to display, that entity's model concept, and the
view concept used to resolve a model-specific template:

```yaml tonk=parse
workspace/sheet!: &sheet-alice
  this: id:sheet-alice
  title:    "Alice"
  subtitle: "person"
  icon:     "user"
  order:    "a"
  entity:   alice
  model:    person
  view:     tonk:view
```

`view: tonk:view` names the view concept, not a particular view entity.
`<tonk-display>` queries that concept for the view whose `model` is `person`.
Author the model-specific view with the CLI:

```text
tonk view add person --name person-card --template '<article><h2>{name}</h2></article>'
```

The CLI pins the view to `id:person-card`, so re-running it updates the same
view. In raw notation, include the equivalent stable `this:` yourself:

```yaml tonk=parse
view!: &person-card
  this: id:person-card
  model: person
  display: !text/html |
    <article><h2>{name}</h2></article>
```

The binder's directory view queries every `workspace/sheet` and orders the
tabs by `order`; no separate assertion adds the sheet to a workspace object.
The sheet above displays through the sheets home after the module has re-pointed
`tonk/space` to `tonk:binder`.

`tonk render <entity>@workspace/sheet` renders one sheet wrapper headlessly,
but live tab selection, creation, and closing require the browser runtime.
Share the underlying space separately with `tonk invite`.

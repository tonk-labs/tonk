# `<tonk-display>` — render an entity through a view

The core rendering element. Point it at a model concept and (optionally)
a single entity; it resolves that model's view template, queries the
matching entities, and binds their fields into the template. Everything
you render ultimately hangs off a `<tonk-display>`.

```html
<!-- one entity, its detail view -->
<tonk-display model="person" entity="did:key:z6Mk…"></tonk-display>

<!-- every instance, the directory view -->
<tonk-display model="task"></tonk-display>

<!-- one entity through the built-in label facet -->
<tonk-display model="person" entity="did:key:z6Mk…" view="label"></tonk-display>
```

## Attributes

| Name | Meaning |
|------|---------|
| `model` | The concept to render — bookmark name (`person`) or entity URI. **Required.** |
| `entity` | The single entity to render. Absent → **directory mode** (every instance). |
| `view` | The *show facet* to render (`label`, `title`, or any key the model's `show` dictionary carries) — a plain key, never a concept URI. Omitted uses the mode default: `ui` (`entity` present) or `directory` (absent). |

All three are live subscriptions: seed a concept, edit a template, or add
an instance after mount and the display updates without a reload. A
change to `model`/`entity`/`view` restarts resolution; `dom.host/*`
context attributes thread into the mounted view in place.

## Nesting (following references)

To render a reference field, nest a display over it — `entity` is the
reference, `model` the referenced concept, `view` the facet you want:

```html
<strong>
  <tonk-display entity={author} model=person view=label></tonk-display>
</strong>
```

The `label` facet renders just a name; omitting `view` renders the `ui` facet, the full detail card.

## Directory data-rows

`<tonk-display model=… >` with no `entity` renders one row per instance —
the pattern the interactive elements use to feed themselves (a hidden
`<tonk-display>` per concept inside `<tonk-table>` / a board canvas). Each
row's `subject={this}` marks the repeat root.

Full resolution pipeline, the `<tonk-view>` binding model, and repeat/fold
semantics: `rust/tonk-display/README.md`. See also `tonk help views`.

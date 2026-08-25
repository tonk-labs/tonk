# `<tonk-code>` — code editor

A CodeMirror 6-backed code editor with per-language syntax highlighting,
as a custom element. Language packs load on demand.

## Using it in a view

Seed the buffer with the `value` attribute (or its text) and persist
edits by firing a command on `change`:

```html
<tonk-code
  language="yaml"
  value={source}
  data-subject={this}
  onchange=snippet/edit
  placeholder="Type here…"></tonk-code>
```

The `change` event's `event.detail.value` is the new document text; read
it in the command with `dom.event.detail/value` and write it back onto
the entity with a rule. (`value` is also a live property:
`el.value` round-trips the document.)

## Attributes

| Name | Meaning |
|------|---------|
| `value` | Initial document; round-trips on the `value` property. |
| `language` | Language id (e.g. `yaml`, `sql`); resolves a lazily-loaded pack. A missing pack falls back to plain text. |
| `source` | Document URI used for LSP attachment. Without it the editor remains standalone. |
| `readonly` | Presence locks the editor (selection/copy still work). |
| `placeholder` | Ghost text shown while empty. |
| `line-numbers`, `active-line` | Toggle the gutter / active-line highlight. |
| `auto-focus` | Focus the editor after mount. |

## Events

| Name | `detail` |
|------|----------|
| `change` | `{ value, doc }` — fires on user edits only. |
| `ready` | `{ view }` — once, after mount (the CodeMirror `EditorView`). |
| `run` | Editor "run" affordance (Shift-Enter or Mod-Enter), carrying `{ value, doc }`. |
| `diagnostics` | LSP diagnostics, when a diagnostics provider is wired. |

## Notes

- Language packs are separate chunks fetched the first time a `language`
  is used; adding one is a build-time step (see
  `rust/tonk-code/README.md`).
- LSP requires both `source` and an ancestor `<tonk-diagnostics-provider>`;
  `language` supplies the LSP language id as well as highlighting.
- Theming/host integration mirror the other editors. Full API and the
  language-authoring recipe: `rust/tonk-code/README.md`.

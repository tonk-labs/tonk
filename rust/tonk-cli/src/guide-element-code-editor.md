# `<code-editor>`: code editor

A CodeMirror 6 code editor with per-language syntax highlighting, as a
custom element. It is branch data, not part of the host: install the
`code-editor` library component
(`rust/tonk-core/assets/library/code-editor.yaml`) and the element and
its editor core come with it. `<prose-editor>` embeds it in fenced code
blocks.

## Using it in a view

Seed the buffer with the `value` attribute and persist edits by firing a
command on `change`:

```html
<code-editor
  language="yaml"
  value={source}
  on:snippet-change=snippet/edit
  placeholder="Type here…"></code-editor>
```

The `change` event's `event.detail.value` is the new document text.
Declare how it fills your command, and write it back onto the entity
with a rule:

```yaml
event!: &on/snippet-change
  type: "change"
  where:
    subject: "{this}"
    source: ".detail.value"
```

The command (`snippet/edit`) declares exactly `subject` (an `entity`)
and `source` (`text`); see `tonk help events`. (`value` is also a live
property: `el.value` round-trips the document, and a write to it patches
only the span that differs, so the caret stays put and no `change`
fires.)

## Languages

A language is a record in the space, not part of the element. The
component ships two, `yaml` and `dialog-yaml` (also named `dialog`);
every other language is a component of its own, `language/<id>` (for
example `language/python`), installed from the palette. `language`
names one by any of its words (`python`, `py`). A language the space
lacks leaves the editor plain text until it is installed, and then it
redraws.

## Attributes

| Name | Meaning |
|------|---------|
| `value` | Initial document; reactive after mount. |
| `language` | A word naming one of the space's languages (e.g. `yaml`, `python`). Missing → plain text. |
| `source` | Document URI (`tonk-buffer:///<repo>/<branch>/<name>`) that puts the document on the language server. Without it the editor stays standalone. |
| `language-server` | Language server endpoint; `/api/language-server` by default. |
| `readonly` | Presence locks the editor (selection/copy still work). |
| `placeholder` | Ghost text shown while empty. |
| `line-numbers`, `active-line` | Toggle the gutter / active-line highlight. |
| `auto-focus` | Focus the editor after mount. |

## Events

| Name | `detail` |
|------|----------|
| `change` | `{ value, doc }`: fires on user edits only. |
| `run` | `{ value, doc }`: Shift-Enter or Mod-Enter. |
| `ready` | `{ view }`: once, after mount (the CodeMirror `EditorView`). |
| `diagnostics` | `{ value, doc, count, errorCount }`: each fresh frame from the language server, the verdict on the text as typed. |
| `code-editor-connect` | `{ source, language, url }`: the editor is about to open `source` on a language server. |
| `code-editor-disconnect` | `{ source }`: the editor closed `source`. |

Properties: `value`, `view` (the CodeMirror `EditorView`, null until
mounted) and `errorCount` (errors in the latest diagnostics frame).

## Notes

- With a `source`, the editor announces itself with a cancelable,
  bubbling `code-editor-connect`. An ancestor that claims it
  (`preventDefault`) supplies the client by calling `el.connect(client)`;
  otherwise the editor attaches to its core's own session for
  `language-server`. `language` doubles as the LSP language id.
- Theming runs through `--tonk-code-*` custom properties set on the
  element (`code-editor { --tonk-code-bg: … }`). Source and build:
  `rust/tonk-code/README.md`.

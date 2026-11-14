# `<prose-editor>`: markdown editor

A ProseMirror-backed, Typora-style markdown editor: the document renders
rich, and the markdown syntax around the caret reveals itself for editing
and collapses when the caret leaves. It is branch data, not part of the
host: install the `prose-editor` library component
(`rust/tonk-core/assets/library/prose-editor.yaml`) and the element, its
editor core and a `prose` document concept come with it.

## Using it in a view

The element's **text content** is the document, the way a `<textarea>`
carries its value. Bind the store's content as element text (newline- and
markup-safe, unlike an attribute) and fire a command on each idle edit:

```html
<prose-editor
  on:prose-change=prose/edit
  placeholder="Write something…"
  auto-focus>{content}</prose-editor>
```

`on/prose-change` is the component's event declaration: it listens for
`change` and fills `prose/edit`'s `subject` from `{this}` and its
`content` from `.detail.content`, so the element needs no `data-subject`.

The `change` event carries the new document on `event.detail.content` as
a **versioned envelope** (markdown + HLC ETag). Store that verbatim and
feed it back as the element's text: the element recognizes its own
round-tripped echo by the version and drops it, so the caret is never
disturbed. Seeding with a bare markdown string also works (no version →
always adopted). The component's `prose` concept stores documents in this
shape and renders them with exactly the view above.

## Attributes

| Name | Meaning |
|------|---------|
| text / `content` | Bare markdown or the versioned envelope; text is the primary store-binding channel. |
| `value` | Bare markdown convenience channel; reactive after mount. |
| `readonly` | Presence locks the editor. |
| `placeholder` | Ghost text shown while empty. |
| `auto-focus` | Focus the editor on mount (`caret="end"` puts the caret at the end). |
| `switcher` | The leading heading picks or names a document from `candidates`; read once, at mount. |

## Events

| Name | `detail` |
|------|----------|
| `change` | `{ value, content }`: `value` is the markdown, `content` the versioned envelope. Fires after edits go idle (debounced); programmatic writes don't refire. |
| `ready` | `{ editor }`: once, after the editor mounts. |
| `switch`, `create`, `suggest` | The heading switcher (with `switcher`): a candidate picked, a new title named (`{ title, document }`), the suggestions shown (`{ rows, active }`). |

Properties: `value` (current markdown), `content` (the versioned envelope,
for lossless store round-trips), `version`, `editor` (the live handle,
null until `ready`) and `candidates` (the documents the switcher offers,
set by the host).

## Notes

- Inline syntax (`**bold**`, `*em*`, `` `code` ``, `[text](url)`), block
  syntax (`> `, `- `, `1. `, `## `, `---`), task lists, and images all
  convert as you type.
- Code blocks embed `<code-editor>` when the space has the `code-editor`
  component (see `tonk help code-editor`); otherwise they stay editable
  plain text, and redraw as editors once it is installed.
- Theming runs through `--tonk-prose-*` custom properties. Source and
  build: `rust/tonk-prose/README.md`.

# `<tonk-prose>` — markdown editor

A ProseMirror-backed, Typora-style markdown editor: the document renders
rich, and the markdown syntax around the caret reveals itself for editing
and collapses when the caret leaves.

## Using it in a view

The element's **text content** is the document — the way a `<textarea>`
carries its value. Bind the store's content as element text (newline- and
markup-safe, unlike an attribute) and fire a command on each idle edit:

```html
<tonk-prose
  onchange=prose/edit
  data-subject={this}
  placeholder="Write something…"
  auto-focus>{content}</tonk-prose>
```

The `change` event carries the new document on `event.detail.content` as
a **versioned envelope** (markdown + HLC ETag). Store that verbatim and
feed it back as the element's text: the element recognizes its own
round-tripped echo by the version and drops it, so the caret is never
disturbed. Seeding with a bare markdown string also works (no version →
always adopted). This is the `prose` library module
(`rust/tonk-core/assets/library/prose.yaml`).

## Attributes

| Name | Meaning |
|------|---------|
| `value` / text | Markdown source (the text child is the primary channel). |
| `readonly` | Presence locks the editor. |
| `placeholder` | Ghost text shown while empty. |
| `auto-focus` | Focus the editor on mount. |

## Events

| Name | `detail` |
|------|----------|
| `change` | `{ value, content }` — `value` is the markdown, `content` the versioned envelope. Fires after edits go idle (debounced); programmatic writes don't refire. |
| `ready` | `{ editor }` — once, after the editor mounts. |

## Notes

- Inline syntax (`**bold**`, `*em*`, `` `code` ``, `[text](url)`), block
  syntax (`> `, `- `, `1. `, `## `, `---`), task lists, and images all
  convert as you type.
- Code blocks upgrade to embedded `<tonk-code>` editors when that element
  is defined on the page (see `tonk guide views tonk-code`); otherwise
  they stay editable plain text.
- Theming runs through `--tonk-prose-*` custom properties. Full API:
  `rust/tonk-prose/README.md`.

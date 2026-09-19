# `<tonk-prose>` — markdown editor

A ProseMirror-backed, Typora-style markdown editor: the document renders
rich, and the markdown syntax around the caret reveals itself for editing
and collapses when the caret leaves.

## Document mode — a body that merges (the `prose` library)

```html
<tonk-prose subject={this} placeholder="Write something…" auto-focus></tonk-prose>
```

With `subject`, the body is an **automerge document** named after the
entity, not a claim: no command binding, no rule. Two people typing at once
keep both edits, a branch shows its own version, and editing writes no body
into branch history. This is what `rust/tonk-core/assets/library/prose.yaml`
uses. The entity needs `format: "automerge/text@1"`
(`xyz.tonk.document/format`).

To edit the text yourself, assert a command — the same one from a page or
from the CLI:

```yaml
document/replace!:
  document: id:prose/doc
  find: "a passage that occurs exactly once"
  with: "its replacement"
```

`document/insert` (`after`, `text`) and `document/splice` (`heads`, `at`,
`delete`, `text`; UTF-16 positions) work the same way. A `find` that matches
nothing, or twice, changes nothing and says so.

To read it, query the mirror or the history:

```
tonk query document/content
tonk query document/versions --term document=id:prose/doc
tonk query document/content  --term document=id:prose/doc --term heads=<heads>
tonk query document/diff     --term document=id:prose/doc --term from=<heads> --term to=<heads>
```

`document/restore` (`document`, `heads`) brings a past version back as a new
change. These need `rust/tonk-core/assets/library/document.yaml` evaluated
into the space.

To SHOW a past version, give the element its heads. It is then read-only
and follows nothing; remove `at` to go back to the live version:

```html
<tonk-prose subject={this} at="<heads from document/versions>"></tonk-prose>
```

A document takes no more edits once it is stored as 8 MiB (automerge keeps
every edit, so a document only grows); the edit is refused and nothing
changes. `tonk export` carries the document with the branch, and
`tonk import` puts it back.

## Standalone — content as element text

Without `subject` the element is a plain editor whose content you store
yourself.


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
| text / `content` | Bare markdown or the versioned envelope; text is the primary store-binding channel. |
| `value` | Bare markdown convenience channel; reactive after mount. |
| `readonly` | Presence locks the editor. |
| `placeholder` | Ghost text shown while empty. |
| `auto-focus` | Focus the editor on mount. |

## Events

| Name | `detail` |
|------|----------|
| `change` | `{ value, content }` — `value` is the markdown, `content` the versioned envelope. Fires after edits go idle (debounced); programmatic writes don't refire. |
| `ready` | `{ editor }` — once, after the editor mounts. |

The `value` property returns current markdown. The `content` property returns
the versioned envelope suitable for lossless store round-trips.

## Notes

- Inline syntax (`**bold**`, `*em*`, `` `code` ``, `[text](url)`), block
  syntax (`> `, `- `, `1. `, `## `, `---`), task lists, and images all
  convert as you type.
- Code blocks upgrade to embedded `<tonk-code>` editors when that element
  is defined on the page (see `tonk help tonk-code`); otherwise
  they stay editable plain text.
- Theming runs through `--tonk-prose-*` custom properties. Full API:
  `rust/tonk-prose/README.md`.

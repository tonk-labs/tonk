# tonk-prose

The core of the library's `<prose-editor>`
(`rust/tonk-core/assets/library/prose-editor.yaml`): a ProseMirror-backed,
Typora-style markdown editor. A space gets one by installing the
`prose-editor` component; the host ships none.

The document renders rich; the markdown syntax of the block under
the caret reveals itself for editing (muted `**`, `# `, `[…](url)`
markers) and collapses again when the caret leaves. The architecture
follows [allusion](https://github.com/Gozala/allusion), modernized:

- **Markers are literal text.** Syntax characters are stored as text
  nodes carrying a `markup` mark, hidden/revealed *purely via CSS*
  keyed off two selection-driven decorations: the caret's block
  (reveals block markers like the heading `# `) and the caret's
  *edit range* — the mark run it touches, resolved through the
  markers' `of` attr (allusion's `marks`) so a caret on a bare `**`
  counts as inside the strong span beside it. Only that span's
  markers show: caret in bold reveals its `**`, caret in a link
  reveals `[` and `](url)`, everything else stays rendered. Cursor
  movement never mutates the document.
- **A debounced reparse loop instead of inline input rules.** Because
  markers are text, a textblock's `textContent` *is* its markdown
  source. ~120ms after you stop typing, edited blocks are reparsed
  and swapped when their structure changed — typing `**bold**` makes
  a strong span, deleting one `*` degrades it back, `# ` makes a
  heading. Caret position survives because in a pure-text block,
  document offsets are text offsets.
- **Serialization strips markers first**, then regenerates delimiters
  from the marks — so output markdown is always normalized.

## As branch data

`<prose-editor>` is an `element!:` in `prose-editor.yaml`. Its methods
in `library/prose-editor/*.js` are the custom-element class (text
channel, versioned envelope, debounced `change`, deferred teardown,
switcher events), and its `core` method,
`library/prose-editor/core.js`, is built here: `src-js/editor/` as an
expression returning `createEditor`. Code blocks embed the library's
`<code-editor>` when the space has it; otherwise they stay a plain
editable `<pre>`.

## Layout

| Path | Purpose |
| --- | --- |
| `src-js/editor/` | The editor core (schema, markup layer, reparse loop, reveal, input rules, code blocks, heading switcher). |
| `src-js/editor/hlc.ts`, `content.ts` | The versioned-content protocol the element's envelope follows. |
| `scripts/build.mjs` | esbuild driver. Writes `library/prose-editor/core.js`. |
| `scripts/test.mjs` | Bundles and runs every `*.test.ts` with Node's test runner. |

The element in a real browser is covered by
`rust/tonk-code/tests/library-element.mjs` (with `<code-editor>` in its
code blocks) and `rust/tonk-code/tests/notebook-elements.mjs`.

## Building

```bash
cd rust/tonk-prose
npm install
npm run build      # core.js into the library
npm run watch      # rebuild on source changes
npm run check      # typecheck only
npm test           # node tests
```

`core.js` is committed beside `prose-editor.yaml`: regenerate it
whenever `src-js/editor/` or a ProseMirror dependency changes, and
commit the result alongside the source change.

## Element API

See `tonk help prose-editor` for attributes, properties and events. In
short: the element's text content (or `content` / `value`) is the
document, and `change` carries `{ value, content }`, the markdown and
its versioned envelope, after each idle edit.

## Editing behavior

- Inline syntax (`**bold**`, `*em*`, `` `code` ``, `[text](url)`)
  converts via the reparse loop; deleting a marker converts back.
- Block syntax: `> `, `- `/`* `/`+ `, `1. ` wrap on typing (input
  rules; one Backspace undoes the wrap). `## ` headings convert via
  the loop. `---` becomes a rule.
- Images are *expanded* (allusion's `expandedImage`): the document
  stores the literal `![alt](src)` source under an `image_markup`
  mark, hidden/revealed like any marker, while a widget decoration
  renders the picture right after it. Typing image syntax converts
  through the loop, editing the source re-points the preview, and
  breaking the syntax degrades it to text.
- Code blocks: type ` ```lang ` (with a space) or ` ``` `/` ```lang `
  followed by Enter. Arrow keys walk in and out of the embedded
  editor at its edges; Backspace in an empty block turns it back
  into a paragraph.
- `Mod-b` / `Mod-i` / `` Mod-` `` insert literal markers around the
  selection — one code path with typed syntax.
- Pasted plain text is parsed as markdown (except into code blocks).
- Plain-text copy yields markdown source for free (markers are text).

## Theming

All visuals route through `--tonk-prose-*` CSS custom properties set
on the element (`--tonk-prose-bg`, `--tonk-prose-fg`,
`--tonk-prose-accent`, `--tonk-prose-marker`, `--tonk-prose-code-bg`,
`--tonk-prose-font`, `--tonk-prose-max-width`, …), with
GitHub-flavored light/dark defaults. The element imports nothing
from the consumer's design system; the variables are the contract.

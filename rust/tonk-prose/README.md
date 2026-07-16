# tonk-prose

`<tonk-prose>`: a ProseMirror-backed, Typora-style markdown editor
packaged as a custom element, plus a tiny Rust shim that injects the
bundle's loader script into the page.

The document renders rich; the markdown syntax of the block under
the caret reveals itself for editing (muted `**`, `# `, `[…](url)`
markers) and collapses again when the caret leaves. The architecture
follows [allusion](https://github.com/Gozala/allusion), modernized:

- **Markers are literal text.** Syntax characters are stored as text
  nodes carrying a `markup` mark, hidden/revealed *purely via CSS*
  keyed off one selection-driven node decoration. Cursor movement
  never mutates the document.
- **A debounced reparse loop instead of inline input rules.** Because
  markers are text, a textblock's `textContent` *is* its markdown
  source. ~120ms after you stop typing, edited blocks are reparsed
  and swapped when their structure changed — typing `**bold**` makes
  a strong span, deleting one `*` degrades it back, `# ` makes a
  heading. Caret position survives because in a pure-text block,
  document offsets are text offsets.
- **Serialization strips markers first**, then regenerates delimiters
  from the marks — so output markdown is always normalized.

## Lazy loading

The main bundle (`tonk-prose.js`, ~4 kB) is a shell: it registers the
custom element and nothing else. The ProseMirror machinery lives in
`tonk-prose-editor.js` (loaded via dynamic import the first time an
element connects, once per page). Code blocks embed `<tonk-code>`
when that element is defined, inheriting its lazily-loaded
per-language chunks; otherwise they fall back to a plain editable
`<pre>`.

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | Rust crate. The `web` feature exposes [`install`] which appends `<script type="module">` for the bundle. |
| `src-js/index.ts` | The shell: registers `tonk-prose`, lazy-loads the core. |
| `src-js/editor/` | The editor core chunk (schema, markup layer, reparse loop, reveal, input rules, code blocks). |
| `scripts/build.mjs` | esbuild driver. Produces `assets/tonk-prose.js` + `assets/tonk-prose-editor.js`. |
| `assets/` | Build output. **Committed** so consumers (and CI) don't need a Node toolchain to build the Rust workspace. |

## Building the bundle

```bash
cd rust/tonk-prose
npm install
npm run build      # production build → assets/
npm run watch      # rebuild on source changes
npm run check      # typecheck only
```

`assets/` is a build artifact but committed: regenerate it whenever
`src-js/` or any ProseMirror dependency changes, and commit the
result alongside the source change.

## Element API

```html
<tonk-prose placeholder="Write something…"
            value="# Hello&#10;&#10;Some **markdown**."></tonk-prose>
```

Attributes:

| Name | Type | Notes |
| --- | --- | --- |
| `value` | string | Markdown source. Reactive: writes after connect replace the document. |
| `readonly` | boolean | Presence locks the editor. |
| `placeholder` | string | Ghost text shown while the document is empty. |
| `auto-focus` | boolean | Focus the editor once mounted. |

Properties:

| Name | Type |
| --- | --- |
| `value` | `string` (round-trips with the document as markdown) |
| `editor` | live editor handle (`null` until `ready`); `.view` exposes the raw ProseMirror `EditorView` |

Events:

| Name | `detail` |
| --- | --- |
| `change` | `{ value: string }` — markdown after every user edit (programmatic writes don't refire). |
| `ready` | `{ editor }` — fires once, after the lazy core chunk loaded and the view mounted. |

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
on the host (`--tonk-prose-bg`, `--tonk-prose-fg`,
`--tonk-prose-accent`, `--tonk-prose-marker`, `--tonk-prose-code-bg`,
`--tonk-prose-font`, `--tonk-prose-max-width`, …), with
GitHub-flavored light/dark defaults. The element imports nothing
from the consumer's design system; the variables are the contract.

## Serving alongside `<tonk-code>`

Both bundles resolve sibling chunks relative to their own module URL.
Copy each package's `assets/` to its own directory and load the shell
with the crate's `install("/tonk-prose/tonk-prose.js")` or a module
script. `tonk-ui`'s `index.html` already ships the bundle at
`/tonk-prose/` (a `copy-dir` link next to tonk-code's, with a
matching `Trunk.toml` watch entry), so host code or guest bridges can
load it from there. Install `tonk-code` too to get embedded code
editors with language highlighting; without it, code blocks stay
editable as plain text.

# tonk-code

The core of the library's `<code-editor>`
(`rust/tonk-core/assets/library/code-editor.yaml`): a CodeMirror 6
editor, and the language packs it draws with. A space gets an editor by
installing the `code-editor` component; the host ships none.

## As branch data

`<code-editor>` is an `element!:` in `code-editor.yaml`. Its methods in
`library/code-editor/*.js` are the custom-element class (language
records, language server announcement, deferred teardown); the rest is
built here:

| Output | Role |
| --- | --- |
| `library/code-editor/core.js` | `src-js/library.ts`: the editor core, the CodeMirror modules packs share, `loadPack` and the language server session, as an expression returning its exports. |
| `library/code-editor/language/<id>.js` | The two packs the editor ships, `yaml` and `dialog-yaml`. |
| `library/language/<id>.{yaml,js}` | One installable component per `@codemirror/language-data` language: its `code-editor/language` record and the pack it includes. |
| `code-editor.yaml` (generated region) | The language concept, the shipped languages' records, and the palette's list of the rest. |

A pack is the text of `(shared) => LanguageSupport`: its imports of
the CodeMirror modules the core shares resolve to `shared`, so every
pack uses the core's CodeMirror (CodeMirror checks extensions by
`instanceof`, so a second copy would be rejected). Packs share nothing
with each other, so each language component is one self-contained file.

## Layout

| Path | Purpose |
| --- | --- |
| `src-js/library.ts` | The core entry: `createEditor`, `SHADOW_STYLESHEET`, `shared`, `loadPack`, `languageServer`. |
| `src-js/editor.ts` | The CodeMirror editor: state, view, theme, per-shadow stylesheet, runtime compartments. |
| `src-js/lsp/` | The language server session, client and HTTP transport. |
| `src-js/lang/*.ts` | The hand-written packs (`yaml`, `dialog-yaml`); catalog packs are generated into `src-js/lang/generated/` on every build. |
| `scripts/build.mjs` | esbuild driver. Writes everything in the table above. |
| `tests/library-element.mjs` | `<code-editor>` (and `<prose-editor>` embedding it) in Chromium, defined from the library by the real element runtime. |
| `tests/console-elements.mjs`, `tests/notebook-elements.mjs` | The inspector console and notebook elements that build on it. |

## Building

```bash
cd rust/tonk-code
npm install
npm run build      # core, packs and language components into the library
npm run watch      # rebuild the core on source changes
node tests/library-element.mjs   # the element in Chromium
```

The outputs are committed beside `code-editor.yaml`: regenerate them
whenever `src-js/` or a CodeMirror dependency changes, and commit the
result alongside the source change.

## Element API

See `tonk help code-editor` for attributes, events and the language
server handshake. Theming runs through `--tonk-code-*` custom properties
set on the element (see `SHADOW_STYLESHEET` in `src-js/editor.ts`).

## Adding a language

Every language in `@codemirror/language-data` is already an installable
component. For another one:

1. `npm install @codemirror/lang-<id>`.
2. Add `src-js/lang/<id>.ts` exporting a `LanguageSupport` instance:
   ```ts
   import { sql } from "@codemirror/lang-sql";
   export default sql();
   ```
3. Add `{ id: "<id>", entry: "src-js/lang/<id>.ts" }` to
   `customLanguages` in `scripts/build.mjs`, so its id wins over a
   catalog alias, and an entry with its `name` and `words` to
   `builtin`, which ships it with the editor.
4. `npm run build`.

A language is a record anyone can assert, so a space can also carry
one built elsewhere, as long as its pack has the `(shared) =>
LanguageSupport` shape.

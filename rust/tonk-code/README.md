# tonk-code

`<tonk-code>`: a CodeMirror 6-backed code editor packaged as a
custom element, plus a tiny Rust shim that injects the bundle's
loader script into the page.

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | Rust crate. The `web` feature exposes [`install`] which appends `<script type="module">` for the bundle. |
| `src-js/index.ts` | Custom element implementation (registers `tonk-code`). |
| `src-js/lang/*.ts` | One file per language pack, bundled as a separate ESM chunk and loaded on demand. |
| `scripts/build.mjs` | esbuild driver. Produces `assets/tonk-code.js` plus per-language chunks and a shared CodeMirror runtime chunk. |
| `scripts/source-fingerprint.mjs` | Computes the checked-in source identity stamped into production bundles; the UI Node suite rejects stale artifacts. |
| `assets/` | Build output. **Committed** to source control so consumers (and CI) don't need a Node toolchain to build the Rust workspace. |

## Building the bundle

```bash
cd rust/tonk-code
npm install
npm run build      # production build → assets/
npm run watch      # rebuild on source changes
```

`assets/` is a build artifact but committed: regenerate it whenever
`src-js/` or any of the CodeMirror dependencies change, and commit
the result alongside the source change.

## Element API

```html
<tonk-code mode="yaml" placeholder="Type a query…"></tonk-code>
```

Attributes:

| Name | Type | Notes |
| --- | --- | --- |
| `value` | string | Initial document. Reflected on the property. |
| `mode` | string | Language id; resolved to `./tonk-code-lang-<mode>.js`. Missing chunks fall back silently to plain text. |
| `readonly` | boolean | Presence locks the editor. |
| `placeholder` | string | Ghost text shown while the buffer is empty. |

Properties:

| Name | Type |
| --- | --- |
| `value` | `string` (round-trips with the document) |

Events:

| Name | `detail` |
| --- | --- |
| `change` | `{ value: string, doc: Text }` (fires on user edits only). |
| `ready` | `{ view: EditorView }` (fires once after mount). |

## Adding a language

1. `npm install @codemirror/lang-<id>`.
2. Add `src-js/lang/<id>.ts` exporting a `LanguageSupport` instance:
   ```ts
   import { sql } from "@codemirror/lang-sql";
   export default sql();
   ```
3. Append `{ id: "<id>", entry: "src-js/lang/<id>.ts" }` to the
   `languages` list in `scripts/build.mjs`.
4. `npm run build`, and a new `assets/tonk-code-lang-<id>.js` lands.
5. Use it: `<tonk-code mode="<id>"></tonk-code>`.

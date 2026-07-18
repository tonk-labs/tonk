# On-demand CodeMirror language packs in the sealed guest

## Problem

`<tonk-code>` code blocks highlight via CodeMirror language packs, loaded
lazily as `tonk-code-lang-<id>.js` chunks. Today only two exist (`yaml`,
`dialog-yaml`), so a ```lisp (or python/rust/js/…) block gets no
highlighting — the chunk 404s. Loading is already async; the gap is (a)
the chunks don't exist, and (b) the sealed guest can't fetch them.

## Two layers

### 1. Build: a chunk per language (tonk-code)

Generate the `languages` list in `scripts/build.mjs` from
`@codemirror/language-data` (each descriptor knows its npm package +
loader). Emit one esbuild entry per language → `tonk-code-lang-<id>.js`,
sharing the split CodeMirror-core chunk (identity requirement, see the
build comment). Also emit `src-js/lang/chunks.generated.ts` — a
`LANGUAGE_CHUNKS` map from every name/alias/extension (lower-cased) to
its chunk id — statically bundled into the main element so resolving a
`language` attribute needs no fetch. Keep the custom `dialog-yaml` pack.

`loadLanguage(language)` (index.ts) already updated: alias-map lookup →
dynamic import of the chunk. Unknown id → reject → editor stays plain.

### 2. Bridge: fetch a language chunk on demand (tonk-portal / guest)

The sealed guest has an opaque origin and cannot `import()` a network
URL; runtime chunk URLs are rewritten to guest-minted blob URLs
(`window.__tonkCodeLang`). Today the host pre-fetches a HARDCODED file
list (`tonk-code.js` + `tonk-code-lang-dialog-yaml.js`,
bridge.rs:956) at boot and mints their blobs. Shipping ~140 language
chunks eagerly would bloat boot and defeat laziness.

Instead, load a language chunk THROUGH THE BRIDGE on demand:

- Guest side: when `__tonkCodeLang(id)` has no blob yet, request it over
  the existing postMessage bridge (a new `code-lang` message carrying the
  id), await the host's reply (the chunk source + any not-yet-present
  shared split chunks), mint the blob(s) in dependency order (reuse
  `mintGraph`), cache by id, and return the blob URL. `loadLanguage`
  already awaits, so making `__tonkCodeLang` async (return a Promise) fits
  — adjust the rewrite target accordingly.
- Host side: handle the `code-lang` request by
  `fetch_bundle_graph("/tonk-code", &["tonk-code-lang-<id>.js"])` (minus
  chunks already sent), returning `{name, source}` pairs. The host is
  trusted and same-origin, so it CAN fetch; this is the same relay shape
  other guest→host operations use.

Shared split chunks (`chunk-*.js`) must be minted once and reused across
languages — track which are already in the guest blob map so a second
language only ships its own entry + any new shared chunk.

## Fallback / safety

- Unknown language id: no highlighting, no thrown error (already handled).
- Bridge fetch failure: warn, leave plain — never break the editor.
- Non-sealed pages (plain http): `import.meta.url` resolution still works
  directly; the bridge path is guest-only.

## Tests

- tonk-code: alias map resolves `js`→javascript chunk, `rs`→rust, etc.;
  loadLanguage imports the right chunk (browser test with a couple of
  real packs).
- Bridge: guest requests a chunk, host relays source, blob minted, pack
  applied — verify in the sealed app (highlighting appears for a
  ```python block).

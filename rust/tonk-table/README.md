# tonk-table

`<tonk-table>`: the [IronCalc](https://github.com/ironcalc/IronCalc)
spreadsheet engine packaged as a custom element with a framework-free
grid, plus a tiny Rust shim that injects the bundle's loader script
into the page.

The engine is the spreadsheet: formula evaluation, the dependency
graph, number formatting, undo/redo, clipboard parsing, and even
selection/navigation semantics (`onArrowDown`, `onExpandSelectedRange`,
`onAreaSelecting`, …) live in IronCalc's wasm — a headless *view
model*. The grid only draws that state into DOM and routes input
events back, so editing behaves like a spreadsheet because it *is*
one, not a re-implementation.

Why the headless engine and not `@ironcalc/workbook` (IronCalc's own
UI): that component is React + MUI + Emotion, draws on a `<canvas>`
outside any design-token contract, pins an older engine, and injects
styles into `document.head` — all of which fight the shadow-root,
`--wa-*`-token, sealed-guest architecture every tonk element follows.
The engine alone is MIT/Apache-2.0, dependency-free, and initializes
**from bytes**, which is the property everything below leans on.

## Lazy loading

Three chunks, two lazy seams:

| Chunk | Size | Role |
| --- | --- | --- |
| `tonk-table.js` | ~9 kB | The shell: registers the element, nothing else. |
| `tonk-table-grid.js` | ~33 kB | Grid UI + IronCalc JS glue. Dynamically imported on the first element connect. |
| `tonk-table-engine.js` | ~3.8 MB | The engine wasm, base64-embedded in a pure data leaf. Pulled by the grid, decoded, and passed to `init({ module_or_path: bytes })`. |

Pages that ship the bundle but never render a `<tonk-table>` pay only
for the shell. The engine leaf changes ONLY on an IronCalc version
bump, so grid iteration never rewrites the multi-megabyte artifact.
The engine instantiates from bytes — never from a URL fetch — so the
whole graph works at origins where fetch is dead (sealed guests).

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | Rust crate. The `web` feature exposes [`install`] which appends `<script type="module">` for the bundle. |
| `src-js/index.ts` | The shell: registers `tonk-table`, lazy-loads the grid core. |
| `src-js/grid/` | The grid core chunk (engine bootstrap, DOM grid, workbook helpers). |
| `src-js/engine.ts` | The engine-bytes leaf entry. |
| `src-js/hlc.ts`, `content.ts`, `b64.ts` | The versioned-content protocol (shared shape with tonk-prose). |
| `scripts/build.mjs` | esbuild driver. Produces the three `assets/` chunks. |
| `assets/` | Build output. **Committed** so consumers (and CI) don't need a Node toolchain to build the Rust workspace. |

## Building the bundle

```bash
cd rust/tonk-table
npm install
npm run build      # production build → assets/
npm run watch      # rebuild on source changes
npm run check      # typecheck only
npm test           # node tests, including the REAL engine (initSync from bytes)
```

`assets/` is a build artifact but committed: regenerate it whenever
`src-js/` or the IronCalc dependency changes, and commit the result
alongside the source change.

## Element API

```html
<tonk-table>a,b,total
1,2,=A2+B2</tonk-table>
```

The element's **text content** is its content channel (the way
`<textarea>` carries its value). Two body forms are accepted:

- the versioned `content` envelope (`Tonk-Table-Version` header + HLC
  `ETag`) around **base64 of the engine's binary workbook
  serialization** — the lossless channel the element emits and
  round-trips through a store;
- **bare CSV** — the hand-authorable seed form; formulas ride as `=…`
  cell text. No version → always adopted.

Attributes:

| Name | Type | Notes |
| --- | --- | --- |
| `content` | string | Content source, same parsing as the text channel (text child wins when both are present). |
| `value` | string | Legacy/convenience content source, same parsing. |
| `readonly` | boolean | Presence locks the grid (selection + copy still work). |
| `auto-focus` | boolean | Focus the grid once mounted. |

Properties:

| Name | Type |
| --- | --- |
| `value` | `string` — CSV of the ACTIVE sheet (raw cell content, formulas survive). Human-readable, **lossy**. |
| `content` | `string` — the versioned envelope around the **lossless** workbook bytes. |
| `version` | `string` — the current HLC as a decimal string. |
| `grid` | live grid handle (`null` until `ready`); `.model` exposes the raw IronCalc `Model`. |

Events:

| Name | `detail` |
| --- | --- |
| `change` | `{ value, content }` — after every idle edit burst (debounced); programmatic writes don't refire. Round-trip `content` through a store and the element drops its own echo by the HLC. |
| `ready` | `{ grid }` — fires once per mount, after the engine instantiated and the grid rendered. |

## Editing behavior

- Click / arrows / Tab / Enter / PageUp/PageDown move the selection;
  Shift+arrows and drag extend a range (engine semantics).
- Type to replace a cell, `F2` / double-click to edit in place,
  `Escape` cancels, `Delete`/`Backspace` clears the selected range.
- The formula bar edits the active cell's raw content; `Enter`
  commits.
- Formulas: anything starting `=` evaluates live (`=A1*3+1`,
  `=SUM(D2:D3)`, …); errors render as `#REF!`-style values.
- Copy/cut/paste: copy emits the engine's clipboard text; paste
  accepts tab-separated (spreadsheet clipboards) or RFC 4180 CSV.
  Parsing is done element-side — the engine's `pasteCsvText` splits
  on tabs only, which the tests pin.
- Sheets: tabs along the bottom — click to switch, `+` to add,
  double-click to rename inline, `×` on the active tab to delete it
  behind an inline confirmation (all in-page: sealed guests have no
  `window.prompt`/`confirm`, and sheet deletion is not undoable, so a
  stray click must never be enough). Deletion is standalone-mode only
  until the claims schema grows a delete-sheet command, and the last
  sheet can't be deleted.
- `Mod-z` / `Shift-Mod-z` / `Mod-y` undo/redo through the engine.

## Content protocol

Same seam as tonk-prose: a MIME-style envelope whose `ETag` is a
Hybrid Logical Clock. The element adopts an incoming write only when
its HLC is newer than anything it has issued or seen — so its own
store echo is dropped and never reloads the grid under the user —
and replaces the workbook (selection preserved) when a genuinely
newer remote write arrives. `Content-Type:
application/vnd.ironcalc` marks a base64-bytes body; anything else
(or no envelope) reads as CSV.

The fixed model locale (`en`/`UTC`) keeps the serialized bytes
deterministic across peers; display localization is future rendering
work, not a storage concern.

## Theming & restyling

Three layers, weakest to strongest — the whole look is overridable:

1. **Tokens** — `--tonk-table-*` custom properties on the host
   (`--tonk-table-bg`, `--tonk-table-accent`,
   `--tonk-table-selection`, `--tonk-table-header-bg`,
   `--tonk-table-height`, …), defaulting to the page's WebAwesome
   `--wa-*` tokens with GitHub-flavored light/dark fallbacks. Plain
   host CSS works too (`tonk-table { border: none; height: 30rem }`).
2. **Parts** — every structural element exports a `part`, so outside
   CSS restyles any piece with full power:

   ```css
   tonk-table::part(formula-bar) { display: none }
   tonk-table::part(column-header) { text-transform: lowercase }
   tonk-table::part(cell selected) { outline-color: hotpink }
   tonk-table::part(tab active) { font-weight: 700 }
   ```

   Part names: `frame`, `formula-bar`, `cell-reference`,
   `formula-input`, `body`, `grid`, `corner`, `column-header`,
   `row-header`, `row`, `cell` (state tokens `number` / `error` /
   `selected` / `range`), `tab-strip`, `tab` (token `active`),
   `tab-delete`, `add-sheet`, `cell-editor`, `sheet-rename`,
   `sheet-confirm`, `sheet-confirm-delete`, `sheet-confirm-keep`.
3. **User stylesheets** — any `<style>` child of `<tonk-table>` is
   adopted into the shadow root, cascading LAST (it wins over the
   built-in sheets at equal specificity), for rules parts can't
   express:

   ```html
   <tonk-table subject={this}>
     <style>
       tbody tr:nth-child(even) td { background: #00000006; }
     </style>
     …
   </tonk-table>
   ```

   Live-updated; the internal class names (`.formula`, `.scroller`,
   `.tabs`, `td.num`, …) are a semi-stable contract for this layer.
   Views restyle per-use by authoring a style child.

## Serving

Copy `assets/` to its own directory and load the shell with the
crate's `install("/tonk-table/tonk-table.js")` or a module script;
all three chunks resolve as siblings of the shell's URL. `tonk-ui`'s
`index.html` already ships the bundle at `/tonk-table/` (a `copy-dir`
link next to tonk-prose's, with a matching `Trunk.toml` watch entry).

## Inside `<tonk-site>` guests

tonk-portal wires the element into every sealed guest iframe
(`bridge.rs`), *preserving the lazy split*: the boot payload carries
only the registration shell, so guests that never render a table
never pay for one. When the first `<tonk-table>` connects, the shell
calls `window.__tonkTableGrid` — in guests, a function that asks the
trusted parent for the grid core (`need-table`), mints blob URLs from
the `inject-table` reply (the grid's relative import of the engine
leaf is rewritten to its blob), and resolves the grid's blob URL. The
engine then instantiates from the leaf's embedded bytes — no fetch,
which is why it works at the opaque origin at all. The parent fetches
the two core files BY NAME rather than walking the import graph: the
grid chunk embeds the wasm import-object key `"./wasm_bg.js"`, which
a graph walk would chase into the SPA's HTML fallback and the blob
rewrite would then corrupt. Failures reject and retry on the next
connect instead of wedging the runtime.

## Library module

`rust/tonk-core/assets/library/table.yaml` defines the `tonk:table`
concept, the `table/edit` command + persistence rule, instance and
directory views, and a seeded demo workbook — the same shape as
`prose.yaml`.

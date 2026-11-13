# tonk-table

The core of the library's `<table-editor>`
(`rust/tonk-core/assets/library/table.yaml`): the
[IronCalc](https://github.com/ironcalc/IronCalc) spreadsheet engine
with a framework-free grid. A space gets a spreadsheet by installing
the `table` component; the host ships none.

The engine is the spreadsheet: formula evaluation, the dependency
graph, number formatting, undo/redo, clipboard parsing, and even
selection/navigation semantics (`onArrowDown`, `onExpandSelectedRange`,
`onAreaSelecting`, …) live in IronCalc's wasm, a headless *view
model*. The grid only draws that state into DOM and routes input
events back, so editing behaves like a spreadsheet because it *is*
one, not a re-implementation.

Why the headless engine and not `@ironcalc/workbook` (IronCalc's own
UI): that component is React + MUI + Emotion, draws on a `<canvas>`
outside any design-token contract, pins an older engine, and injects
styles into `document.head`, all of which fight the shadow-root,
`--wa-*`-token, sealed-guest architecture every tonk element follows.
The engine alone is MIT/Apache-2.0, dependency-free, and initializes
**from bytes**, which is what lets it travel as branch data.

## As branch data

`<table-editor>` is an `element!:` in `table.yaml`. Its methods in
`library/table-editor/*.js` are the custom-element class (content
channel, claim rows, versioned envelope, debounced `change`, deferred
teardown), and two of them are built here:

| Method | Size | Role |
| --- | --- | --- |
| `core` | ~55 kB | `src-js/library.ts`: the grid and what the methods share, as an expression returning its exports. |
| `engine` | ~3.9 MB | The IronCalc engine wasm as base64 (~1.1 MB gzipped). Decoded and instantiated once per page, from bytes. |

The engine is its own method so it changes only on an IronCalc
version bump: grid edits never rewrite the multi-megabyte file.

## Layout

| Path | Purpose |
| --- | --- |
| `src-js/library.ts` | The core entry: `createGrid`, the claim-row and content helpers, the host stylesheet. |
| `src-js/grid/` | The grid (engine bootstrap, DOM grid, workbook helpers, claims plumbing). |
| `src-js/hlc.ts`, `content.ts`, `b64.ts` | The versioned-content protocol (shared shape with tonk-prose). |
| `scripts/build.mjs` | esbuild driver. Writes `library/table-editor/core.js` and `engine.js`. |
| `tests/library-element.mjs` | The element in Chromium, defined from `table.yaml` by the real element runtime. |

## Building

```bash
cd rust/tonk-table
npm install
npm run build      # core.js + engine.js into the library
npm run watch      # rebuild the core on source changes
npm run check      # typecheck only
npm test           # node tests, including the REAL engine (initSync from bytes)
node tests/library-element.mjs   # the element in Chromium
```

The outputs are committed beside `table.yaml`: regenerate them
whenever `src-js/` or the IronCalc dependency changes, and commit the
result alongside the source change.

## Element API

```html
<table-editor>a,b,total
1,2,=A2+B2</table-editor>
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
| `subject` | entity | The workbook whose claims the hidden light-DOM rows carry (claims mode, as `table.yaml` renders it). Absent: standalone. |
| `content` | string | Content source, same parsing as the text channel (text child wins when both are present). |
| `value` | string | Legacy/convenience content source, same parsing. |
| `readonly` | boolean | Presence locks the grid (selection + copy still work). |
| `auto-focus` | boolean | Focus the grid once mounted. |
| `min-rows`, `min-cols` | integer | Minimum rendered extent (default 100×26). |

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
   host CSS works too (`table-editor { border: none; height: 30rem }`).
2. **Parts** — every structural element exports a `part`, so outside
   CSS restyles any piece with full power:

   ```css
   table-editor::part(formula-bar) { display: none }
   table-editor::part(column-header) { text-transform: lowercase }
   table-editor::part(cell selected) { outline-color: hotpink }
   table-editor::part(tab active) { font-weight: 700 }
   ```

   Part names: `frame`, `formula-bar`, `cell-reference`,
   `formula-input`, `body`, `grid`, `corner`, `column-header`,
   `row-header`, `row`, `cell` (state tokens `number` / `error` /
   `selected` / `range`), `tab-strip`, `tab` (token `active`),
   `tab-delete`, `add-sheet`, `cell-editor`, `sheet-rename`,
   `sheet-confirm`, `sheet-confirm-delete`, `sheet-confirm-keep`.
3. **User stylesheets** — any `<style>` child of `<table-editor>` is
   adopted into the shadow root, cascading LAST (it wins over the
   built-in sheets at equal specificity), for rules parts can't
   express:

   ```html
   <table-editor subject={this}>
     <style>
       tbody tr:nth-child(even) td { background: #00000006; }
     </style>
     …
   </table-editor>
   ```

   Live-updated; the internal class names (`.formula`, `.scroller`,
   `.tabs`, `td.num`, …) are a semi-stable contract for this layer.
   Views restyle per-use by authoring a style child.

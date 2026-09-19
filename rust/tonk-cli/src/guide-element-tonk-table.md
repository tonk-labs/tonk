# `<tonk-table>` — spreadsheet

An IronCalc-backed spreadsheet: live formulas, multiple sheets, and
selection/clipboard/undo semantics, as a self-contained custom element.
Two ways to hold its data — pick per use.

## Standalone (self-contained, no store)

The element's **text content** is the workbook. Bare CSV is the easy
seed; formulas ride as `=…` cell text:

```html
<tonk-table>Item,Qty,Total
Widget,2,=B2*10</tonk-table>
```

Edits fire a debounced `change` event carrying `{ value, content }` —
`value` is the active sheet as CSV, `content` is a versioned envelope
(base64 workbook bytes + HLC) you can persist and feed back verbatim;
the element drops its own echo by the version. This is the same
one-attribute pattern as `<tonk-prose>`; good for embedding a table in a
document or a `board` card.

## Document mode — a workbook that merges (the `table` library)

```html
<tonk-table subject={this} document></tonk-table>
```

With `subject` and `document`, the workbook is ONE **automerge document**
named after the entity: no command bindings, no hidden data rows. Two people
who fill the same empty cell write the same key, so there is one cell and the
other value is kept as a conflict (`cellconflict` event, `detail.cells`). A
branch shows its own version, and editing writes no cells into branch
history. This is what `rust/tonk-core/assets/library/table.yaml` uses. The
entity needs `format: "automerge/table@1"`.

Cells stay readable as facts — the host mirrors the workbook into
`table/sheet` and `table/cell`:

```
tonk query table/cell
```

The mirror is read-only. To write a cell, assert a command:

```yaml
document/put!:
  document: id:table/book
  path: "sheets/<sheet>/cells/B2"
  value: "=A1*2"
```

`document/remove` (`document`, `path`) clears a cell or drops a sheet;
paths also cover `name`, `order`, `styles/<A1>`, `widths/<col>`,
`heights/<row>`. History works as for prose (`tonk query document/versions
--term document=id:table/book`). These need
`rust/tonk-core/assets/library/document.yaml` evaluated into the space.

## Claims mode (individuated, store-backed)

The earlier store-backed mode, still supported: every cell is its own
claim. Concurrent fills of one empty cell leave two cell entities for one
address, which is what document mode removes.


Add a `subject` and the workbook lives in the store as **one claim per
sheet and per non-empty cell** (raw input only; values are recomputed on
every replica). Cells are addressable and concurrent editors merge at
cell grain. Wire it like the board: hidden `<tonk-display>` rows feed
the element, and each mutation event maps to a command.

```html
<tonk-table subject={this}
  oncreatecell=table/create-cell
  oneditcell=table/edit-cell
  onclearcell=table/clear-cell
  oncreatesheet=table/create-sheet
  onrenamesheet=table/rename-sheet>
  <div hidden>
    <tonk-display model=tonk:table/sheet></tonk-display>
    <tonk-display model=tonk:table/cell></tonk-display>
    <tonk-display model=tonk:table/column></tonk-display>
    <tonk-display model=tonk:table/row></tonk-display>
  </div>
</tonk-table>
```

The full concept/command/rule set is the `table` library module — seed
it with `tonk eval` from `rust/tonk-core/assets/library/table.yaml`, then
`tonk query table/cell --json` lists cells. Update one by copying its entity
and running `tonk assert table/cell <ENTITY> --content '=A1*2'`; create one
with `tonk assert table/cell --sheet <SHEET_ENTITY> --at B2 --content
'=A1*2'`.

## Attributes

| Name | Meaning |
|------|---------|
| `subject` | Workbook entity → **claims mode**. Absent → standalone. |
| `content` / text | Standalone workbook: CSV, or the versioned envelope. |
| `value` | Standalone convenience channel: CSV for the active sheet. |
| `readonly` | Lock the grid (selection + copy still work). |
| `min-rows`, `min-cols` | Minimum rendered extent (default 100×26). |
| `auto-focus` | Focus the grid on mount. |

## Events (claims mode → bind each to a command)

`createcell` `{cellSheet, cellAt, cellContent, time}` · `editcell`
`{editCell, editContent}` · `clearcell` `{clearCell}` · `createsheet`
`{sheetTable, sheetName, sheetOrder, time}` · `renamesheet`
`{renameSheet, renameName}` · `createcolumn`/`resizecolumn`,
`createrow`/`resizerow` (sizing). Both modes emit `selectionchange`
`{sheet, at, from, to}`. Standalone emits `change` `{value, content}`.

## Programmatic control (`el.grid`, null until `ready`)

`setCell(at, content)` · `getCell(at)` · `addSheet(name?)` ·
`deleteSheet(name?)` (standalone only) · `setColumnWidth(col, px)` ·
`getColumnWidth(col)` · `setRowHeight(row, px)` · `getRowHeight(row)` ·
`insertRows`/`deleteRows`/`insertColumns`/`deleteColumns` · `select(at,
to?)` · `selection` · `toCsv()` · `serialize()` · `model` (raw IronCalc).
Every mutating method routes through the same commit path as typing, so it
repaints, emits events, and persists identically.

## Restyling

Three layers: `--tonk-table-*` custom properties (theming);
`::part()` on every structural element — `frame`, `formula-bar`, `body`,
`grid`, `column-header`, `row-header`, `cell` (state tokens `number` /
`error` / `selected` / `range`), `tab`, … ; and a `<style>` child
adopted into the shadow root for arbitrary rules (e.g. zebra rows). See
`rust/tonk-table/README.md` for the full contract.

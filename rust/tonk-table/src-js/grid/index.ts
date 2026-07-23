// The heavy chunk: the IronCalc engine + a framework-free DOM grid for
// `<tonk-table>`. Loaded by the shell (`../index.ts`) via dynamic
// import on the first element connect — nothing here may be imported
// *statically* from the shell (types excepted).
//
// Division of labor: the ENGINE is the spreadsheet — formula
// evaluation, dependency graph, formatting, undo/redo, selection
// semantics (arrow/page/range-expansion), clipboard parsing. This
// module only draws its state into DOM and routes input events to it.
// The engine is also a headless *view model*: selection lives in the
// model (`getSelectedView`, `onArrowDown`, `onAreaSelecting`, …), so
// grid behaviors follow canonical spreadsheet semantics for free.
//
// Two document modes (see `GridMode` in api.ts):
//
// - STANDALONE — the element's content channel carries the whole
//   workbook; edits signal `onChange` and the shell round-trips the
//   envelope.
// - CLAIMS — the workbook lives in the dialog store as individuated
//   claims. The shell feeds parsed data rows in (`applyRows`); every
//   user mutation flows out as a typed event (`emit`) that a tonk
//   view binds to a command. One commit path (`#commitAt`) serves
//   gestures, paste, undo diffs, and the public `setCell` — so
//   programmatic control is indistinguishable from typing.
//
// The engine wasm arrives as BYTES from the sibling
// `tonk-table-engine.js` data leaf (see `../engine.ts`), never via a
// URL fetch — which is what lets this chunk come up inside a sealed,
// opaque-origin portal guest where relative fetches are dead. The
// relative import below is rewritten to a blob URL by tonk-portal's
// injector in that environment.

import init, { Model, columnNameFromNumber } from "@ironcalc/wasm";
import type {
  CellRow,
  ColumnRow,
  GridMode,
  GridOptions,
  HostEmit,
  RowSizeRow,
  Selection,
  SheetRow,
  TableGrid,
  TableSource,
} from "./api";
import { modelFromSource, newModel, pasteText, toCsv, usedRange } from "./workbook";
import { parseDelimited } from "./workbook";
import {
  PendingLedger,
  afterAll,
  columnName,
  columnNumber,
  diffSnapshots,
  formatAddress,
  parseAddress,
} from "./claims";

/** Single in-flight / resolved instantiation of the engine wasm,
 *  shared by every grid on the page. Cleared on failure so a later
 *  connect can retry after e.g. a network blip. */
let enginePromise: Promise<void> | null = null;

function ensureEngine(): Promise<void> {
  if (!enginePromise) {
    enginePromise = (async () => {
      // Resolve the bytes leaf next to this chunk. In sealed guests the
      // literal below is rewritten to a blob URL at injection time, and
      // an absolute blob URL wins over the (dead) import.meta.url base.
      const url = new URL("./tonk-table-engine.js", import.meta.url).href;
      const mod = (await import(/* @vite-ignore */ url)) as {
        default: Uint8Array;
      };
      await init({ module_or_path: mod.default });
    })();
    enginePromise.catch(() => {
      enginePromise = null;
    });
  }
  return enginePromise;
}

/** Mount a grid into `parent`. The first call also instantiates the
 *  engine wasm, once per page. */
export async function createGrid(
  parent: HTMLElement,
  options: GridOptions,
): Promise<TableGrid> {
  await ensureEngine();
  return new Grid(parent, options);
}

/** Default minimum grid extent (independent of content) and growth
 *  padding — an empty sheet still looks like a spreadsheet, not a
 *  stub. Overridable per instance via the element's `min-rows` /
 *  `min-cols` attributes (small embeds want a small grid). */
const DEFAULT_MIN_ROWS = 100;
const DEFAULT_MIN_COLS = 26;
const PAD_ROWS = 20;
const PAD_COLS = 4;

/** Hard bounds on the DOM extent. Every rendered cell is a live node,
 *  so a huge loaded workbook must not translate 1:1 into DOM — beyond
 *  these caps the data is intact (the byte channel is unbounded) but
 *  cells outside the window aren't painted. Viewport windowing is the
 *  eventual fix; these keep worst-case DOM ≈ 33k cells until then. */
const MAX_ROWS = 512;
const MAX_COLS = 64;

/** IronCalc cell-type codes (Excel `TYPE()` convention) this grid
 *  distinguishes for presentation. */
const CELL_TYPE_NUMBER = 1;
const CELL_TYPE_ERROR = 16;

/** Row-header column width, px. Fixed (not em) so the table's total
 *  width — which must be definite for `table-layout: fixed` to engage —
 *  is an exact sum. */
const ROW_HEADER_WIDTH = 48;

/** Monotonic nonce for create commands: identical field values must
 *  not collapse into one command entity (the board's `time` lesson). */
let nonceLast = 0;
function nonce(): number {
  nonceLast = Math.max(nonceLast + 1, Date.now());
  return nonceLast;
}

/** One user-committed cell mutation, in engine coordinates. */
type Commit = { engineSheet: number; row: number; column: number; content: string };

/** Accept a column as a letter ("B") or 1-based number. */
function normalizeColumn(column: number | string): number | null {
  if (typeof column === "number") {
    return Number.isInteger(column) && column >= 1 ? column : null;
  }
  return columnNumber(column);
}

/** A min-extent override: default when absent/invalid, capped at the
 *  DOM bound, floored at 1. */
function clampExtent(value: number | undefined, dflt: number, max: number): number {
  if (value === undefined || !Number.isFinite(value) || value < 1) return dflt;
  return Math.min(Math.floor(value), max);
}

class Grid implements TableGrid {
  #model: Model;
  #readOnly: boolean;
  readonly #mode: GridMode;
  readonly #emitHost: HostEmit;
  readonly #onChange: (() => void) | undefined;
  #minRows = DEFAULT_MIN_ROWS;
  #minCols = DEFAULT_MIN_COLS;

  /** Everything this grid put into the parent, removed on destroy. */
  readonly #root: HTMLDivElement;
  readonly #refBox: HTMLSpanElement;
  readonly #fx: HTMLInputElement;
  readonly #scroller: HTMLDivElement;
  readonly #table: HTMLTableElement;
  readonly #colgroup: HTMLTableColElement[] = [];
  readonly #headRow: HTMLTableRowElement;
  readonly #tbody: HTMLTableSectionElement;
  readonly #tabs: HTMLDivElement;

  /** Rendered DOM extent (grows on demand, never shrinks). */
  #rows = 0;
  #cols = 0;

  /** Cells currently carrying selection classes, so a selection move
   *  clears exactly what it painted instead of sweeping the table. */
  #selected: HTMLTableCellElement[] = [];

  /** The in-cell editor overlay, when open. */
  #editor: { input: HTMLInputElement; row: number; column: number } | null =
    null;
  /** Reentrancy guard: closing the editor blurs its input, and blur
   *  commits — without the flag a commit would recurse. */
  #closingEditor = false;

  /** Range-selection drag in progress (mousedown on a cell, not yet
   *  released). */
  #dragging = false;

  // --- Claims-mode state ----------------------------------------------

  /** Current sheet claims, in order (claims mode). */
  #sheetRows: SheetRow[] = [];
  /** Cell claims by sheet entity → address (claims mode). */
  #cellIndex = new Map<string, Map<string, CellRow>>();
  /** Column/row sizing claims by sheet entity → letter / row number. */
  #columnIndex = new Map<string, Map<string, ColumnRow>>();
  #rowSizeIndex = new Map<string, Map<string, RowSizeRow>>();
  /** Echo ledger for sizing writes, keyed `col:B` / `row:2` — same
   *  masking discipline as cell contents. */
  readonly #pendingSizes = new PendingLedger();
  /** Engine sheet index per sheet claim entity, and the reverse. The
   *  engine's own sheet order is creation order; the CLAIMS order (the
   *  `order` keys) drives the tab strip, so the two need a mapping. */
  #engineIndexByEntity = new Map<string, number>();
  #entityByEngineIndex: (string | null)[] = [];
  /** Optimistic-echo ledger: outbound cell writes masked over the rows
   *  until the claim round-trips (or expires). */
  readonly #pending = new PendingLedger();
  /** Locally-created sheets awaiting their claim echo, by name →
   *  engine index. Adopted (mapped to the claim entity) when the sheet
   *  row arrives. */
  readonly #pendingSheets = new Map<string, number>();
  /** Cell commits made on a sheet whose claim hasn't round-tripped
   *  yet; flushed by `applyRows` once the entity is known. */
  #queuedCommits: Commit[] = [];
  /** Rows arrived while a cell editor was open and their application
   *  to that cell was deferred. Re-applied when the editor closes. */
  #lastRows: {
    sheets: SheetRow[];
    cells: CellRow[];
    columns: ColumnRow[];
    rowSizes: RowSizeRow[];
  } | null = null;
  #rowsDeferred = false;

  /** Last announced selection, so `selectionchange` fires only on
   *  actual movement (repaints re-run the painter constantly). */
  #lastSelectionKey = "";

  /** One controller aborts every listener this grid installed. */
  readonly #abort = new AbortController();
  readonly #resize: ResizeObserver;

  #destroyed = false;

  constructor(parent: HTMLElement, options: GridOptions) {
    this.#mode = options.mode;
    this.#model =
      options.mode.kind === "standalone"
        ? modelFromSource(options.mode.source)
        : newModel();
    this.#readOnly = options.readOnly;
    this.#emitHost = options.emit;
    this.#onChange = options.onChange;
    this.setMinExtent(options.minRows, options.minCols);

    // --- DOM scaffold -------------------------------------------------
    // Every structural element carries a `part` name (plus state
    // tokens toggled by the painters), so outside CSS can restyle any
    // piece with full power: `tonk-table::part(cell selected) { … }`.
    // The part names — frame, formula-bar, cell-reference,
    // formula-input, body, grid, corner, column-header, row-header,
    // row, cell (tokens: number/error/selected/range), tab-strip, tab
    // (token: active), add-sheet, cell-editor, sheet-rename — are the
    // stable restyling contract.
    this.#root = document.createElement("div");
    this.#root.className = "table-root";
    this.#root.setAttribute("part", "frame");

    const style = document.createElement("style");
    style.textContent = GRID_STYLESHEET;

    const formula = document.createElement("div");
    formula.className = "formula";
    formula.setAttribute("part", "formula-bar");
    this.#refBox = document.createElement("span");
    this.#refBox.className = "ref";
    this.#refBox.setAttribute("part", "cell-reference");
    this.#fx = document.createElement("input");
    this.#fx.className = "fx";
    this.#fx.setAttribute("part", "formula-input");
    this.#fx.spellcheck = false;
    this.#fx.setAttribute("aria-label", "Formula");
    formula.append(this.#refBox, this.#fx);

    this.#scroller = document.createElement("div");
    this.#scroller.className = "scroller";
    this.#scroller.setAttribute("part", "body");
    this.#scroller.tabIndex = 0;

    this.#table = document.createElement("table");
    this.#table.setAttribute("part", "grid");
    const colgroupEl = document.createElement("colgroup");
    const rowHeadCol = document.createElement("col");
    rowHeadCol.className = "rowhead";
    colgroupEl.append(rowHeadCol);
    const thead = document.createElement("thead");
    this.#headRow = document.createElement("tr");
    const corner = document.createElement("th");
    corner.setAttribute("part", "corner");
    this.#headRow.append(corner);
    thead.append(this.#headRow);
    this.#tbody = document.createElement("tbody");
    this.#table.append(colgroupEl, thead, this.#tbody);
    this.#scroller.append(this.#table);

    this.#tabs = document.createElement("div");
    this.#tabs.className = "tabs";
    this.#tabs.setAttribute("part", "tab-strip");

    this.#root.append(style, formula, this.#scroller, this.#tabs);
    parent.append(this.#root);

    this.#wireEvents();

    // Tell the engine how big its window is so PageUp/PageDown move by
    // a real page. Kept current by the observer; display itself doesn't
    // depend on it.
    this.#resize = new ResizeObserver(() => {
      if (this.#destroyed) return;
      this.#model.setWindowHeight(this.#scroller.clientHeight);
      this.#model.setWindowWidth(this.#scroller.clientWidth);
    });
    this.#resize.observe(this.#scroller);

    this.#refresh();
  }

  // --- TableGrid ------------------------------------------------------

  serialize(): Uint8Array {
    return this.#model.toBytes();
  }

  toCsv(): string {
    return toCsv(this.#model, this.#model.getSelectedSheet());
  }

  load(source: TableSource): void {
    if (this.#mode.kind !== "standalone") return;
    // Capture the view so a remote write doesn't teleport the user.
    const view = this.#model.getSelectedView();
    this.#closeEditor();
    const next = modelFromSource(source);
    this.#model.free();
    this.#model = next;
    try {
      const sheets = next.getWorksheetsProperties().length;
      next.setSelectedSheet(Math.min(view.sheet, Math.max(0, sheets - 1)));
      next.setSelectedCell(view.row, view.column);
    } catch {
      // A shrunken workbook can reject the old view; the engine's
      // default selection is fine.
    }
    this.#refresh();
  }

  /** Reconcile the engine against the store's data rows (claims mode).
   *  Sheets are created/renamed/adopted to match; cell differences —
   *  resolved through the pending ledger so our own in-flight writes
   *  aren't clobbered — apply as individual `setUserInput`s, never a
   *  model reload. The cell under an open editor is deferred until the
   *  editor closes. */
  applyRows(
    sheets: SheetRow[],
    cells: CellRow[],
    columns: ColumnRow[],
    rowSizes: RowSizeRow[],
  ): void {
    if (this.#mode.kind !== "claims" || this.#destroyed) return;
    this.#lastRows = { sheets, cells, columns, rowSizes };
    this.#sheetRows = sheets;

    // 1. Sheets: adopt locally-created ones, create unknown ones,
    //    track remote renames. (Sheet deletion has no command yet, so
    //    disappearance is not handled — the tab just stops rendering.)
    for (const row of sheets) {
      const known = this.#engineIndexByEntity.get(row.id);
      if (known !== undefined) {
        const props = this.#model.getWorksheetsProperties();
        if (props[known] && props[known].name !== row.name) {
          try {
            this.#model.renameSheet(known, row.name);
          } catch (err) {
            console.warn("[tonk-table] remote rename rejected:", err);
          }
        }
        continue;
      }
      const local = this.#pendingSheets.get(row.name);
      if (local !== undefined) {
        // Our own create round-tripped: bind the claim entity to the
        // engine sheet we already made.
        this.#pendingSheets.delete(row.name);
        this.#mapSheet(row.id, local);
        continue;
      }
      // Remote sheet: create it in the engine. The FIRST claim adopts
      // the engine's seed sheet (a fresh model always has one) instead
      // of appending next to an unused blank.
      if (this.#entityByEngineIndex.length === 0 && this.#pendingSheets.size === 0) {
        try {
          this.#model.renameSheet(0, row.name);
        } catch {
          /* engine seed sheet may already carry this name */
        }
        this.#mapSheet(row.id, 0);
        continue;
      }
      this.#model.newSheet();
      const index = this.#model.getWorksheetsProperties().length - 1;
      try {
        this.#model.renameSheet(index, row.name);
      } catch (err) {
        console.warn("[tonk-table] sheet name rejected:", err);
      }
      this.#mapSheet(row.id, index);
    }

    // 2. Flush commits queued while their sheet claim was in flight.
    if (this.#queuedCommits.length > 0) {
      const queued = this.#queuedCommits;
      this.#queuedCommits = [];
      this.#commitMany(queued);
    }

    // 3. Cells: index by sheet, then diff desired-vs-engine and apply.
    this.#cellIndex = new Map();
    for (const cell of cells) {
      let bySheet = this.#cellIndex.get(cell.sheet);
      if (!bySheet) {
        bySheet = new Map();
        this.#cellIndex.set(cell.sheet, bySheet);
      }
      bySheet.set(cell.at, cell);
    }

    const current = this.#snapshotContent();
    const desired = new Map<string, Map<string, string>>();
    for (const [entity, engineIndex] of this.#engineIndexByEntity) {
      const want = new Map<string, string>();
      const rows = this.#cellIndex.get(entity);
      const have = current.get(entity) ?? new Map<string, string>();
      const addresses = new Set<string>([
        ...(rows?.keys() ?? []),
        ...have.keys(),
      ]);
      for (const at of addresses) {
        const value = this.#pending.resolve(entity, at, rows?.get(at)?.content ?? "");
        if (value !== "") want.set(at, value);
      }
      desired.set(entity, want);
      void engineIndex;
    }

    const editing = this.#editor
      ? formatAddress(this.#editor.row, this.#editor.column)
      : null;
    const editingSheet = this.#activeSheetEntity();
    for (const delta of diffSnapshots(current, desired)) {
      const engineIndex = this.#engineIndexByEntity.get(delta.sheet);
      if (engineIndex === undefined) continue;
      if (
        editing !== null &&
        delta.sheet === editingSheet &&
        delta.at === editing
      ) {
        // Never yank the cell out from under the caret; re-applied
        // from #lastRows when the editor closes.
        this.#rowsDeferred = true;
        continue;
      }
      const pos = parseAddress(delta.at);
      if (!pos) continue;
      try {
        this.#model.setUserInput(engineIndex, pos.row, pos.column, delta.content);
      } catch (err) {
        console.warn("[tonk-table] remote cell rejected:", err);
      }
    }

    // 4. Sizing claims: apply column widths / row heights that differ
    //    from the engine (through the sizing echo ledger). Only
    //    explicitly-sized lines carry claims; everything else keeps
    //    the engine default.
    this.#columnIndex = new Map();
    for (const col of columns) {
      let bySheet = this.#columnIndex.get(col.sheet);
      if (!bySheet) {
        bySheet = new Map();
        this.#columnIndex.set(col.sheet, bySheet);
      }
      bySheet.set(col.at, col);
      const engineIndex = this.#engineIndexByEntity.get(col.sheet);
      const n = columnNumber(col.at);
      if (engineIndex === undefined || n === null) continue;
      const want = Number(this.#pendingSizes.resolve(col.sheet, `col:${col.at}`, col.width));
      if (want > 0 && Math.abs(this.#model.getColumnWidth(engineIndex, n) - want) > 0.5) {
        try {
          this.#model.setColumnsWidth(engineIndex, n, n, want);
        } catch (err) {
          console.warn("[tonk-table] remote column width rejected:", err);
        }
      }
    }
    this.#rowSizeIndex = new Map();
    for (const row of rowSizes) {
      let bySheet = this.#rowSizeIndex.get(row.sheet);
      if (!bySheet) {
        bySheet = new Map();
        this.#rowSizeIndex.set(row.sheet, bySheet);
      }
      bySheet.set(row.at, row);
      const engineIndex = this.#engineIndexByEntity.get(row.sheet);
      const n = Number(row.at);
      if (engineIndex === undefined || !(n > 0)) continue;
      const want = Number(this.#pendingSizes.resolve(row.sheet, `row:${row.at}`, row.height));
      if (want > 0 && Math.abs(this.#model.getRowHeight(engineIndex, n) - want) > 0.5) {
        try {
          this.#model.setRowsHeight(engineIndex, n, n, want);
        } catch (err) {
          console.warn("[tonk-table] remote row height rejected:", err);
        }
      }
    }

    this.#refresh();
  }

  setReadOnly(readOnly: boolean): void {
    this.#readOnly = readOnly;
    if (readOnly) this.#closeEditor();
    this.#fx.disabled = readOnly;
  }

  focus(): void {
    this.#scroller.focus();
  }

  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    this.#abort.abort();
    this.#resize.disconnect();
    this.#closeEditor();
    this.#root.remove();
    this.#model.free();
  }

  get model(): Model {
    return this.#model;
  }

  // --- Programmatic control -------------------------------------------

  setCell(at: string, content: string, sheetName?: string): void {
    const pos = parseAddress(at);
    if (!pos) return;
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (engineSheet === null) return;
    this.#commitMany([
      { engineSheet, row: pos.row, column: pos.column, content },
    ]);
  }

  getCell(
    at: string,
    sheetName?: string,
  ): { content: string; value: string } | null {
    const pos = parseAddress(at);
    if (!pos) return null;
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (engineSheet === null) return null;
    return {
      content: this.#model.getCellContent(engineSheet, pos.row, pos.column),
      value: this.#model.getFormattedCellValue(engineSheet, pos.row, pos.column),
    };
  }

  addSheet(name?: string): void {
    this.#addSheetGesture(name);
  }

  // --- Sizing ---------------------------------------------------------

  setColumnWidth(column: number | string, px: number, sheetName?: string): void {
    const n = normalizeColumn(column);
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (n === null || engineSheet === null || !(px > 0)) return;
    try {
      this.#model.setColumnsWidth(engineSheet, n, n, px);
    } catch (err) {
      console.warn("[tonk-table] column width rejected:", err);
      return;
    }
    this.#emitColumnSize(engineSheet, n);
    if (this.#mode.kind === "standalone") this.#onChange?.();
    this.#refresh();
  }

  getColumnWidth(column: number | string, sheetName?: string): number | null {
    const n = normalizeColumn(column);
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (n === null || engineSheet === null) return null;
    return this.#model.getColumnWidth(engineSheet, n);
  }

  setRowHeight(row: number, px: number, sheetName?: string): void {
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (engineSheet === null || !(row >= 1) || !(px > 0)) return;
    try {
      this.#model.setRowsHeight(engineSheet, row, row, px);
    } catch (err) {
      console.warn("[tonk-table] row height rejected:", err);
      return;
    }
    this.#emitRowSize(engineSheet, row);
    if (this.#mode.kind === "standalone") this.#onChange?.();
    this.#refresh();
  }

  getRowHeight(row: number, sheetName?: string): number | null {
    const engineSheet = this.#sheetIndexByName(sheetName);
    if (engineSheet === null || !(row >= 1)) return null;
    return this.#model.getRowHeight(engineSheet, row);
  }

  /** Emit (and echo-mask) one column's current engine width as a
   *  sizing claim — create or resize depending on whether the claim
   *  exists yet. Width travels as decimal text, rounded so echo
   *  comparison is exact. */
  #emitColumnSize(engineSheet: number, n: number): void {
    if (this.#mode.kind !== "claims") return;
    const entity = this.#entityByEngineIndex[engineSheet] ?? null;
    if (entity === null) return; // sheet claim still in flight
    const at = columnName(n);
    const width = String(Math.round(this.#model.getColumnWidth(engineSheet, n)));
    this.#pendingSizes.record(entity, `col:${at}`, width);
    const row = this.#columnIndex.get(entity)?.get(at);
    if (row) {
      this.#emit("resizecolumn", { resizeColumn: row.id, resizeWidth: width });
    } else {
      this.#emit("createcolumn", {
        columnSheet: entity,
        columnAt: at,
        columnWidth: width,
        time: nonce(),
      });
    }
  }

  #emitRowSize(engineSheet: number, n: number): void {
    if (this.#mode.kind !== "claims") return;
    const entity = this.#entityByEngineIndex[engineSheet] ?? null;
    if (entity === null) return;
    const at = String(n);
    const height = String(Math.round(this.#model.getRowHeight(engineSheet, n)));
    this.#pendingSizes.record(entity, `row:${at}`, height);
    const row = this.#rowSizeIndex.get(entity)?.get(at);
    if (row) {
      this.#emit("resizerow", { resizeRow: row.id, resizeHeight: height });
    } else {
      this.#emit("createrow", {
        rowSheet: entity,
        rowAt: at,
        rowHeight: height,
        time: nonce(),
      });
    }
  }

  // --- Structure ------------------------------------------------------
  // Each op wraps the engine mutation in a content diff: addresses
  // shift, formulas get rewritten, and in claims mode every changed
  // cell must emit. Sizing claims for the sheet are re-swept afterward
  // (the engine shifts widths/heights along with the cells).

  insertRows(before: number, count = 1): void {
    if (this.#readOnly || !(before >= 1) || !(count >= 1)) return;
    const sheet = this.#model.getSelectedSheet();
    this.#withDiff(() => this.#model.insertRows(sheet, before, count));
  }

  deleteRows(row: number, count = 1): void {
    if (this.#readOnly || !(row >= 1) || !(count >= 1)) return;
    const sheet = this.#model.getSelectedSheet();
    this.#withDiff(() => this.#model.deleteRows(sheet, row, count));
  }

  insertColumns(before: number | string, count = 1): void {
    const n = normalizeColumn(before);
    if (this.#readOnly || n === null || !(count >= 1)) return;
    const sheet = this.#model.getSelectedSheet();
    this.#withDiff(() => this.#model.insertColumns(sheet, n, count));
  }

  deleteColumns(column: number | string, count = 1): void {
    const n = normalizeColumn(column);
    if (this.#readOnly || n === null || !(count >= 1)) return;
    const sheet = this.#model.getSelectedSheet();
    this.#withDiff(() => this.#model.deleteColumns(sheet, n, count));
  }

  /** Run an engine mutation whose cell effects aren't enumerable up
   *  front (structural ops, undo/redo): snapshot around it, commit the
   *  content diff through the normal path, then re-sweep this sheet's
   *  sizing claims against the engine (structure shifts sizes too).
   *  Standalone always notifies — the workbook changed even when no
   *  cell content moved. */
  #withDiff(mutate: () => void): void {
    const before = this.#snapshotContent();
    try {
      mutate();
    } catch (err) {
      console.warn("[tonk-table] mutation rejected:", err);
      this.#refresh();
      return;
    }
    const after = this.#snapshotContent();
    const commits: Commit[] = [];
    for (const delta of diffSnapshots(before, after)) {
      const pos = parseAddress(delta.at);
      if (!pos) continue;
      const engineSheet =
        this.#mode.kind === "claims"
          ? this.#engineIndexByEntity.get(delta.sheet)
          : this.#model.getSelectedSheet();
      if (engineSheet === undefined) continue;
      commits.push({
        engineSheet,
        row: pos.row,
        column: pos.column,
        content: delta.content,
      });
    }
    this.#commitMany(commits);
    if (this.#mode.kind === "claims") {
      this.#resweepSizes();
    } else if (commits.length === 0) {
      this.#onChange?.();
    }
  }

  /** After a structural op, sizing claims may point at lines whose
   *  engine width/height moved (insertColumns shifts widths right).
   *  Re-emit every claim whose engine value now disagrees. Bounded by
   *  the number of EXPLICITLY sized lines, which is small. */
  #resweepSizes(): void {
    for (const [entity, bySheet] of this.#columnIndex) {
      const engineSheet = this.#engineIndexByEntity.get(entity);
      if (engineSheet === undefined) continue;
      for (const [at, claim] of bySheet) {
        const n = columnNumber(at);
        if (n === null) continue;
        const engine = Math.round(this.#model.getColumnWidth(engineSheet, n));
        if (Math.abs(engine - Number(claim.width)) > 0.5) {
          this.#emitColumnSize(engineSheet, n);
        }
      }
    }
    for (const [entity, bySheet] of this.#rowSizeIndex) {
      const engineSheet = this.#engineIndexByEntity.get(entity);
      if (engineSheet === undefined) continue;
      for (const [at, claim] of bySheet) {
        const n = Number(at);
        if (!(n > 0)) continue;
        const engine = Math.round(this.#model.getRowHeight(engineSheet, n));
        if (Math.abs(engine - Number(claim.height)) > 0.5) {
          this.#emitRowSize(engineSheet, n);
        }
      }
    }
  }

  // --- Selection ------------------------------------------------------

  select(at: string, to?: string): void {
    const a = parseAddress(at);
    if (!a) return;
    this.#model.setSelectedCell(a.row, a.column);
    if (to !== undefined) {
      const b = parseAddress(to);
      if (b) {
        this.#model.setSelectedRange(a.row, a.column, b.row, b.column);
      }
    }
    this.#afterNavigate();
  }

  get selection(): Selection {
    const view = this.#model.getSelectedView();
    const [r1, c1, r2, c2] = view.range;
    const props = this.#model.getWorksheetsProperties();
    return {
      sheet: props[view.sheet]?.name ?? "",
      at: formatAddress(view.row, view.column),
      from: formatAddress(Math.min(r1, r2), Math.min(c1, c2)),
      to: formatAddress(Math.max(r1, r2), Math.max(c1, c2)),
    };
  }

  setMinExtent(minRows?: number, minCols?: number): void {
    this.#minRows = clampExtent(minRows, DEFAULT_MIN_ROWS, MAX_ROWS);
    this.#minCols = clampExtent(minCols, DEFAULT_MIN_COLS, MAX_COLS);
    // Constructed yet? (The constructor sizes before the DOM exists;
    // attribute changes after mount want an immediate repaint. The
    // extent only ever GROWS the DOM — a shrunk minimum applies to the
    // next mount.)
    if (this.#root) this.#refresh();
  }

  #sheetIndexByName(name?: string): number | null {
    if (name === undefined) return this.#model.getSelectedSheet();
    const props = this.#model.getWorksheetsProperties();
    for (let i = 0; i < props.length; i++) {
      if (props[i].name === name) return i;
    }
    return null;
  }

  // --- Claims plumbing ------------------------------------------------

  #mapSheet(entity: string, engineIndex: number): void {
    this.#engineIndexByEntity.set(entity, engineIndex);
    while (this.#entityByEngineIndex.length <= engineIndex) {
      this.#entityByEngineIndex.push(null);
    }
    this.#entityByEngineIndex[engineIndex] = entity;
  }

  #activeSheetEntity(): string | null {
    return this.#entityByEngineIndex[this.#model.getSelectedSheet()] ?? null;
  }

  /** Claims-command events: only meaningful when a store backs the
   *  element. Observability events (`selectionchange`) bypass this and
   *  use `#emitHost` directly in both modes. */
  #emit(type: string, detail: Record<string, unknown>): void {
    if (this.#mode.kind === "claims") this.#emitHost(type, detail);
  }

  /** Content snapshot of every CLAIM-MAPPED sheet (claims mode) or the
   *  active sheet keyed `""` (standalone — only undo diffing uses it
   *  there). */
  #snapshotContent(): Map<string, Map<string, string>> {
    const out = new Map<string, Map<string, string>>();
    if (this.#mode.kind === "claims") {
      for (const [entity, engineIndex] of this.#engineIndexByEntity) {
        out.set(entity, this.#sheetContent(engineIndex));
      }
    } else {
      out.set("", this.#sheetContent(this.#model.getSelectedSheet()));
    }
    return out;
  }

  #sheetContent(engineIndex: number): Map<string, string> {
    const cells = new Map<string, string>();
    const used = usedRange(this.#model, engineIndex);
    for (let r = 1; r <= used.rows; r++) {
      for (let c = 1; c <= used.cols; c++) {
        const content = this.#model.getCellContent(engineIndex, r, c);
        if (content !== "") cells.set(formatAddress(r, c), content);
      }
    }
    return cells;
  }

  /** THE commit path. Every mutation — editor commit, formula bar,
   *  paste, clear, undo/redo diff, `setCell` — lands here after the
   *  engine already holds the new state. Standalone: signal the shell.
   *  Claims: record pending + emit one typed event per cell, queueing
   *  when the sheet's claim hasn't round-tripped yet. */
  #commitMany(commits: Commit[]): void {
    if (this.#mode.kind === "claims") {
      for (const commit of commits) {
        const entity = this.#entityByEngineIndex[commit.engineSheet] ?? null;
        const at = formatAddress(commit.row, commit.column);
        if (entity === null) {
          // Sheet claim in flight (or not yet requested): make sure a
          // create is out, and queue the cell for the echo.
          this.#ensureSheetClaim(commit.engineSheet);
          this.#queuedCommits.push(commit);
          continue;
        }
        this.#pending.record(entity, at, commit.content);
        const row = this.#cellIndex.get(entity)?.get(at);
        if (row) {
          if (commit.content === "") {
            this.#emit("clearcell", { clearCell: row.id });
          } else {
            this.#emit("editcell", { editCell: row.id, editContent: commit.content });
          }
        } else if (commit.content !== "") {
          this.#emit("createcell", {
            cellSheet: entity,
            cellAt: at,
            cellContent: commit.content,
            time: nonce(),
          });
        }
      }
    } else if (commits.length > 0) {
      this.#onChange?.();
    }
    this.#refresh();
  }

  /** Ask the store to mint a claim for an engine sheet that has none
   *  (the bootstrap sheet of a fresh doc, or one raced ahead of its
   *  echo). Idempotent per name. */
  #ensureSheetClaim(engineIndex: number): void {
    if (this.#mode.kind !== "claims") return;
    const props = this.#model.getWorksheetsProperties();
    const name = props[engineIndex]?.name;
    if (name === undefined || this.#pendingSheets.has(name)) return;
    this.#pendingSheets.set(name, engineIndex);
    this.#emit("createsheet", {
      sheetTable: this.#mode.subject,
      sheetName: name,
      sheetOrder: afterAll(this.#sheetRows),
      time: nonce(),
    });
  }

  /** Rows deferred while editing get re-applied on editor exit. */
  #afterEditorClosed(): void {
    if (this.#rowsDeferred && this.#lastRows) {
      this.#rowsDeferred = false;
      const { sheets, cells, columns, rowSizes } = this.#lastRows;
      this.applyRows(sheets, cells, columns, rowSizes);
    }
  }

  // --- Painting -------------------------------------------------------

  /** Repaint everything from the model: extent, cells, selection,
   *  formula bar, tabs. The full sweep runs on data changes; pure
   *  selection moves take the cheaper `#paintSelection` path. */
  #refresh(): void {
    const sheet = this.#model.getSelectedSheet();
    const used = usedRange(this.#model, sheet);
    const view = this.#model.getSelectedView();
    const wantRows = Math.min(
      MAX_ROWS,
      Math.max(this.#minRows, used.rows + PAD_ROWS, view.row + PAD_ROWS),
    );
    const wantCols = Math.min(
      MAX_COLS,
      Math.max(this.#minCols, used.cols + PAD_COLS, view.column + PAD_COLS),
    );
    this.#growTo(wantRows, wantCols);
    this.#paintCells(sheet);
    this.#paintSelection();
    this.#paintTabs();
  }

  /** Ensure the table has at least `rows` × `cols` cells (append-only). */
  #growTo(rows: number, cols: number): void {
    const colgroupEl = this.#table.querySelector("colgroup");
    const makeCell = () => {
      const td = document.createElement("td");
      td.setAttribute("part", "cell");
      return td;
    };
    while (this.#cols < cols) {
      const column = ++this.#cols;
      const col = document.createElement("col");
      this.#colgroup.push(col);
      colgroupEl?.append(col);
      const th = document.createElement("th");
      th.setAttribute("part", "column-header");
      th.textContent = columnNameFromNumber(column);
      this.#headRow.append(th);
      // Extend every existing body row.
      for (const tr of Array.from(this.#tbody.rows)) {
        tr.append(makeCell());
      }
    }
    while (this.#rows < rows) {
      const row = ++this.#rows;
      const tr = document.createElement("tr");
      tr.setAttribute("part", "row");
      const th = document.createElement("th");
      th.setAttribute("part", "row-header");
      th.textContent = String(row);
      tr.append(th);
      for (let c = 0; c < this.#cols; c++) {
        tr.append(makeCell());
      }
      this.#tbody.append(tr);
    }
  }

  /** Paint values, number/error presentation, column widths, and row
   *  heights. Writes only what changed so the common repaint touches
   *  little DOM. */
  #paintCells(sheet: number): void {
    // `table-layout: fixed` only engages when the table has a definite
    // width — with `width: auto` the browser content-sizes columns, so
    // a long value WIDENS its column instead of clipping. Sum the
    // engine's column widths (plus the px-fixed row header) and pin
    // the table to it; the colgroup then divides it exactly and cell
    // overflow clips/ellipsizes as intended.
    let total = ROW_HEADER_WIDTH;
    for (let c = 1; c <= this.#cols; c++) {
      const px = this.#model.getColumnWidth(sheet, c);
      total += px;
      const width = `${px}px`;
      const col = this.#colgroup[c - 1];
      if (col.style.width !== width) col.style.width = width;
    }
    const tableWidth = `${total}px`;
    if (this.#table.style.width !== tableWidth) {
      this.#table.style.width = tableWidth;
    }
    for (let r = 1; r <= this.#rows; r++) {
      const tr = this.#tbody.rows[r - 1];
      const height = `${this.#model.getRowHeight(sheet, r)}px`;
      if (tr.style.height !== height) tr.style.height = height;
      for (let c = 1; c <= this.#cols; c++) {
        const td = tr.cells[c];
        const text = this.#model.getFormattedCellValue(sheet, r, c);
        if (td.textContent !== text) td.textContent = text;
        const type = text === "" ? 0 : this.#model.getCellType(sheet, r, c);
        td.classList.toggle("num", type === CELL_TYPE_NUMBER);
        td.classList.toggle("err", type === CELL_TYPE_ERROR);
        // Mirror presentation state into part tokens so outside CSS
        // can target `::part(cell number)` etc.
        td.part.toggle("number", type === CELL_TYPE_NUMBER);
        td.part.toggle("error", type === CELL_TYPE_ERROR);
      }
    }
    this.#paintFormula();
  }

  /** Move the selection highlight (active cell + range) and keep the
   *  formula bar in step. Cheap enough for every arrow key. */
  #paintSelection(): void {
    for (const td of this.#selected) {
      td.classList.remove("sel", "sel-range");
      td.part.remove("selected", "range");
    }
    this.#selected = [];
    const view = this.#model.getSelectedView();
    const [r1, c1, r2, c2] = view.range;
    const rowStart = Math.max(1, Math.min(r1, r2));
    const rowEnd = Math.min(this.#rows, Math.max(r1, r2));
    const colStart = Math.max(1, Math.min(c1, c2));
    const colEnd = Math.min(this.#cols, Math.max(c1, c2));
    for (let r = rowStart; r <= rowEnd; r++) {
      const tr = this.#tbody.rows[r - 1];
      for (let c = colStart; c <= colEnd; c++) {
        const td = tr.cells[c];
        td.classList.add("sel-range");
        td.part.add("range");
        this.#selected.push(td);
      }
    }
    const active = this.#cellAt(view.row, view.column);
    if (active) {
      active.classList.add("sel");
      active.part.add("selected");
      if (!this.#selected.includes(active)) this.#selected.push(active);
    }
    this.#paintFormula();

    // Announce actual movement (repaints re-run this constantly; only
    // a changed selection is an event). Observability, both modes.
    const selection = this.selection;
    const key = `${selection.sheet} ${selection.at} ${selection.from} ${selection.to}`;
    if (key !== this.#lastSelectionKey) {
      this.#lastSelectionKey = key;
      this.#emitHost("selectionchange", { ...selection });
    }
  }

  #paintFormula(): void {
    const view = this.#model.getSelectedView();
    this.#refBox.textContent = `${columnNameFromNumber(view.column)}${view.row}`;
    // Don't clobber an entry the user is composing in the bar.
    if (document.activeElement !== this.#fx || this.#fx.disabled) {
      this.#fx.value = this.#model.getCellContent(
        view.sheet,
        view.row,
        view.column,
      );
    }
  }

  /** The tab strip. Standalone: engine order. Claims: the claim rows'
   *  `order` keys drive the strip (plus locally-created sheets still
   *  awaiting their echo, rename disabled until adopted). */
  #paintTabs(): void {
    this.#tabs.textContent = "";
    const selected = this.#model.getSelectedSheet();
    const strip: { engineIndex: number; name: string; canRename: boolean }[] =
      [];
    if (this.#mode.kind === "claims") {
      for (const row of this.#sheetRows) {
        const engineIndex = this.#engineIndexByEntity.get(row.id);
        if (engineIndex === undefined) continue;
        strip.push({ engineIndex, name: row.name, canRename: true });
      }
      for (const [name, engineIndex] of this.#pendingSheets) {
        strip.push({ engineIndex, name, canRename: false });
      }
      // A fresh doc with no claims yet: show the engine's bootstrap
      // sheet so the grid still looks like a spreadsheet.
      if (strip.length === 0) {
        const props = this.#model.getWorksheetsProperties();
        props.forEach((p, i) => {
          if (p.state === "visible") {
            strip.push({ engineIndex: i, name: p.name, canRename: false });
          }
        });
      }
    } else {
      this.#model.getWorksheetsProperties().forEach((p, i) => {
        if (p.state === "visible") {
          strip.push({ engineIndex: i, name: p.name, canRename: true });
        }
      });
    }

    for (const entry of strip) {
      const tab = document.createElement("button");
      tab.type = "button";
      tab.className = "tab";
      tab.setAttribute("part", "tab");
      tab.textContent = entry.name;
      if (entry.engineIndex === selected) {
        tab.classList.add("active");
        tab.part.add("active");
      }
      tab.addEventListener("click", () => {
        if (this.#model.getSelectedSheet() === entry.engineIndex) return;
        this.#closeEditor();
        this.#model.setSelectedSheet(entry.engineIndex);
        this.#refresh();
      });
      if (entry.canRename) {
        tab.addEventListener("dblclick", () =>
          this.#renameTab(tab, entry.engineIndex),
        );
      }
      this.#tabs.append(tab);
    }

    const add = document.createElement("button");
    add.type = "button";
    add.className = "tab-add";
    add.setAttribute("part", "add-sheet");
    add.textContent = "+";
    add.title = "Add sheet";
    add.disabled = this.#readOnly;
    add.addEventListener("click", () => {
      if (this.#readOnly) return;
      this.#addSheetGesture();
    });
    this.#tabs.append(add);
  }

  #addSheetGesture(name?: string): void {
    this.#model.newSheet();
    const index = this.#model.getWorksheetsProperties().length - 1;
    if (name !== undefined) {
      try {
        this.#model.renameSheet(index, name);
      } catch (err) {
        console.warn("[tonk-table] sheet name rejected:", err);
      }
    }
    this.#model.setSelectedSheet(index);
    if (this.#mode.kind === "claims") {
      this.#ensureSheetClaim(index);
    } else {
      this.#onChange?.();
    }
    this.#refresh();
  }

  /** Swap a sheet tab for an inline rename input — in-page, because
   *  sealed guests have no `window.prompt`. */
  #renameTab(tab: HTMLButtonElement, engineIndex: number): void {
    if (this.#readOnly) return;
    const current = tab.textContent ?? "";
    const input = document.createElement("input");
    input.className = "rename";
    input.setAttribute("part", "sheet-rename");
    input.value = current;
    tab.replaceWith(input);
    input.focus();
    input.select();
    let done = false;
    const finish = (commit: boolean) => {
      if (done) return;
      done = true;
      const name = input.value.trim();
      if (commit && name !== "" && name !== current) {
        try {
          this.#model.renameSheet(engineIndex, name);
          if (this.#mode.kind === "claims") {
            const entity = this.#entityByEngineIndex[engineIndex];
            if (entity !== null && entity !== undefined) {
              this.#emit("renamesheet", {
                renameSheet: entity,
                renameName: name,
              });
            }
          } else {
            this.#onChange?.();
          }
          this.#refresh();
          return;
        } catch (err) {
          console.warn("[tonk-table] rename rejected:", err);
        }
      }
      this.#paintTabs();
    };
    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") finish(true);
      else if (e.key === "Escape") finish(false);
    });
    input.addEventListener("blur", () => finish(true));
  }

  // --- Model mutation plumbing -----------------------------------------

  #cellAt(row: number, column: number): HTMLTableCellElement | null {
    if (row < 1 || row > this.#rows || column < 1 || column > this.#cols) {
      return null;
    }
    return this.#tbody.rows[row - 1].cells[column] as HTMLTableCellElement;
  }

  /** Resolve a pointer event to 1-based grid coordinates, or null when
   *  it didn't land on a body cell. */
  #cellFromEvent(event: Event): { row: number; column: number } | null {
    const target = event.target as Element | null;
    const td = target?.closest("td");
    if (!td || !this.#tbody.contains(td)) return null;
    const tr = td.parentElement as HTMLTableRowElement;
    const row = tr.rowIndex; // thead row is 0, body rows are 1-based
    const column = td.cellIndex; // row-header <th> is 0
    if (row < 1 || column < 1) return null;
    return { row, column };
  }

  #scrollSelectionIntoView(): void {
    const view = this.#model.getSelectedView();
    this.#cellAt(view.row, view.column)?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }

  /** Selection possibly moved past the rendered extent (arrow-hold,
   *  Ctrl+edge nav): grow if so, then repaint the highlight. */
  #afterNavigate(): void {
    const view = this.#model.getSelectedView();
    if (view.row + PAD_ROWS > this.#rows || view.column + PAD_COLS > this.#cols) {
      this.#refresh();
    } else {
      this.#paintSelection();
    }
    this.#scrollSelectionIntoView();
  }

  // --- In-cell editor ---------------------------------------------------

  /** Open the overlay editor on the active cell. `seed` replaces the
   *  content (type-to-edit); otherwise the raw cell content is loaded
   *  for editing (F2 / double-click). */
  #openEditor(seed?: string): void {
    if (this.#readOnly) return;
    this.#closeEditor();
    const view = this.#model.getSelectedView();
    const td = this.#cellAt(view.row, view.column);
    if (!td) return;
    const input = document.createElement("input");
    input.className = "cell-editor";
    input.setAttribute("part", "cell-editor");
    input.spellcheck = false;
    input.value =
      seed ?? this.#model.getCellContent(view.sheet, view.row, view.column);
    this.#editor = { input, row: view.row, column: view.column };
    td.append(input);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);

    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault();
        this.#commitEditor();
        this.#model.onArrowDown();
        this.#afterNavigate();
        this.#scroller.focus();
      } else if (e.key === "Tab") {
        e.preventDefault();
        this.#commitEditor();
        if (e.shiftKey) this.#model.onArrowLeft();
        else this.#model.onArrowRight();
        this.#afterNavigate();
        this.#scroller.focus();
      } else if (e.key === "Escape") {
        e.preventDefault();
        this.#closeEditor();
        this.#afterEditorClosed();
        this.#scroller.focus();
      }
    });
    input.addEventListener("blur", () => {
      // Click-away commits, mirroring every spreadsheet.
      if (!this.#closingEditor) this.#commitEditor();
    });
  }

  #commitEditor(): void {
    const editor = this.#editor;
    if (!editor) return;
    const value = editor.input.value;
    const sheet = this.#model.getSelectedSheet();
    this.#closeEditor();
    try {
      this.#model.setUserInput(sheet, editor.row, editor.column, value);
    } catch (err) {
      console.warn("[tonk-table] input rejected:", err);
      this.#afterEditorClosed();
      return;
    }
    this.#commitMany([
      { engineSheet: sheet, row: editor.row, column: editor.column, content: value },
    ]);
    this.#afterEditorClosed();
  }

  #closeEditor(): void {
    const editor = this.#editor;
    if (!editor) return;
    this.#editor = null;
    this.#closingEditor = true;
    editor.input.remove();
    this.#closingEditor = false;
  }

  // --- Events -----------------------------------------------------------

  #wireEvents(): void {
    const signal = this.#abort.signal;

    // Selection: click, then drag to extend.
    this.#scroller.addEventListener(
      "mousedown",
      (e) => {
        const cell = this.#cellFromEvent(e);
        if (!cell) return;
        // Keep focus on the scroller (preventDefault suppresses the
        // implicit focus a mousedown would set).
        e.preventDefault();
        if (this.#editor) this.#commitEditor();
        this.#scroller.focus();
        this.#model.setSelectedCell(cell.row, cell.column);
        this.#dragging = true;
        this.#paintSelection();
      },
      { signal },
    );
    this.#scroller.addEventListener(
      "mousemove",
      (e) => {
        if (!this.#dragging) return;
        const cell = this.#cellFromEvent(e);
        if (!cell) return;
        this.#model.onAreaSelecting(cell.row, cell.column);
        this.#paintSelection();
      },
      { signal },
    );
    window.addEventListener("mouseup", () => (this.#dragging = false), {
      signal,
    });
    this.#scroller.addEventListener(
      "dblclick",
      (e) => {
        if (this.#cellFromEvent(e)) this.#openEditor();
      },
      { signal },
    );

    this.#scroller.addEventListener(
      "keydown",
      (e) => this.#onKeydown(e),
      { signal },
    );

    // Clipboard. While the overlay editor or the formula bar owns
    // focus these must stay native, so the handlers step aside.
    this.#scroller.addEventListener(
      "copy",
      (e) => {
        if (this.#editor) return;
        const clip = this.#model.copyToClipboard();
        e.clipboardData?.setData("text/plain", clip.csv);
        e.preventDefault();
      },
      { signal },
    );
    this.#scroller.addEventListener(
      "cut",
      (e) => {
        if (this.#editor) return;
        const clip = this.#model.copyToClipboard();
        e.clipboardData?.setData("text/plain", clip.csv);
        e.preventDefault();
        if (this.#readOnly) return;
        this.#clearSelectedRange();
      },
      { signal },
    );
    this.#scroller.addEventListener(
      "paste",
      (e) => {
        if (this.#editor || this.#readOnly) return;
        const text = e.clipboardData?.getData("text/plain");
        e.preventDefault();
        if (!text) return;
        // Anchor at the selected range's top-left corner.
        const view = this.#model.getSelectedView();
        const row = Math.min(view.range[0], view.range[2]);
        const column = Math.min(view.range[1], view.range[3]);
        const cells = parseDelimited(text);
        try {
          pasteText(this.#model, view.sheet, row, column, text);
        } catch (err) {
          console.warn("[tonk-table] paste rejected:", err);
          return;
        }
        const commits: Commit[] = [];
        for (let i = 0; i < cells.length; i++) {
          for (let j = 0; j < cells[i].length; j++) {
            commits.push({
              engineSheet: view.sheet,
              row: row + i,
              column: column + j,
              content: cells[i][j],
            });
          }
        }
        this.#commitMany(commits);
      },
      { signal },
    );

    // Formula bar: Enter commits to the active cell, Escape restores.
    this.#fx.addEventListener(
      "keydown",
      (e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          if (this.#readOnly) return;
          const view = this.#model.getSelectedView();
          try {
            this.#model.setUserInput(
              view.sheet,
              view.row,
              view.column,
              this.#fx.value,
            );
          } catch (err) {
            console.warn("[tonk-table] input rejected:", err);
            return;
          }
          this.#scroller.focus();
          this.#commitMany([
            {
              engineSheet: view.sheet,
              row: view.row,
              column: view.column,
              content: this.#fx.value,
            },
          ]);
        } else if (e.key === "Escape") {
          e.preventDefault();
          this.#scroller.focus();
          this.#paintFormula();
        }
      },
      { signal },
    );
    this.#fx.disabled = this.#readOnly;
  }

  #clearSelectedRange(): void {
    const view = this.#model.getSelectedView();
    const [r1, c1, r2, c2] = view.range;
    const rowStart = Math.min(r1, r2);
    const rowEnd = Math.max(r1, r2);
    const colStart = Math.min(c1, c2);
    const colEnd = Math.max(c1, c2);
    const commits: Commit[] = [];
    for (let r = rowStart; r <= rowEnd; r++) {
      for (let c = colStart; c <= colEnd; c++) {
        if (this.#model.getCellContent(view.sheet, r, c) !== "") {
          commits.push({ engineSheet: view.sheet, row: r, column: c, content: "" });
        }
      }
    }
    this.#model.rangeClearContents(view.sheet, rowStart, colStart, rowEnd, colEnd);
    this.#commitMany(commits);
  }

  /** Undo/redo mutate the engine without saying what changed; the
   *  snapshot diff in `#withDiff` recovers the per-cell events claims
   *  mode needs. (Snapshot scope: claim-mapped sheets in claims mode,
   *  active sheet standalone.) */
  #undoRedo(redo: boolean): void {
    if (redo ? !this.#model.canRedo() : !this.#model.canUndo()) return;
    this.#withDiff(() => {
      if (redo) this.#model.redo();
      else this.#model.undo();
    });
  }

  #onKeydown(e: KeyboardEvent): void {
    const mod = e.metaKey || e.ctrlKey;

    if (mod && (e.key === "z" || e.key === "Z" || e.key === "y")) {
      e.preventDefault();
      if (this.#readOnly) return;
      const redo = e.key === "y" || (e.shiftKey && (e.key === "z" || e.key === "Z"));
      this.#undoRedo(redo);
      return;
    }
    // Native copy/cut/paste shortcuts must reach the clipboard events.
    if (mod) return;

    const arrows: Record<string, () => void> = {
      ArrowUp: () => this.#model.onArrowUp(),
      ArrowDown: () => this.#model.onArrowDown(),
      ArrowLeft: () => this.#model.onArrowLeft(),
      ArrowRight: () => this.#model.onArrowRight(),
    };

    if (e.key in arrows) {
      e.preventDefault();
      if (e.shiftKey) this.#model.onExpandSelectedRange(e.key);
      else arrows[e.key]();
      this.#afterNavigate();
      return;
    }
    switch (e.key) {
      case "Enter":
        e.preventDefault();
        this.#model.onArrowDown();
        this.#afterNavigate();
        return;
      case "Tab":
        e.preventDefault();
        if (e.shiftKey) this.#model.onArrowLeft();
        else this.#model.onArrowRight();
        this.#afterNavigate();
        return;
      case "PageDown":
        e.preventDefault();
        this.#model.onPageDown();
        this.#afterNavigate();
        return;
      case "PageUp":
        e.preventDefault();
        this.#model.onPageUp();
        this.#afterNavigate();
        return;
      case "F2":
        e.preventDefault();
        this.#openEditor();
        return;
      case "Delete":
      case "Backspace":
        e.preventDefault();
        if (!this.#readOnly) this.#clearSelectedRange();
        return;
    }
    // Type-to-edit: any printable character replaces the cell.
    if (e.key.length === 1 && !e.altKey) {
      e.preventDefault();
      this.#openEditor(e.key);
    }
  }
}

/** Grid stylesheet, appended inside the element's shadow root. Colors,
 *  fonts, and radii all route through the `--tonk-table-*` custom
 *  properties the shell defines on the host (which in turn inherit the
 *  page's WebAwesome tokens with self-contained fallbacks). */
const GRID_STYLESHEET = `
  .table-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    font-family: var(--tonk-table-font);
    font-size: var(--tonk-table-font-size);
    color: var(--tonk-table-fg);
    background: var(--tonk-table-bg);
  }

  .formula {
    display: flex;
    align-items: center;
    flex: none;
    border-block-end: 1px solid var(--tonk-table-border);
  }
  .formula .ref {
    min-inline-size: 5em;
    padding: 4px 8px;
    text-align: center;
    font-family: var(--tonk-table-mono);
    font-size: 0.85em;
    color: var(--tonk-table-fg-muted);
    border-inline-end: 1px solid var(--tonk-table-border);
  }
  .formula .fx {
    flex: 1;
    min-inline-size: 0;
    padding: 4px 8px;
    border: none;
    outline: none;
    background: transparent;
    color: inherit;
    font-family: var(--tonk-table-mono);
    font-size: 0.9em;
  }

  .scroller {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    outline: none;
  }

  table {
    border-collapse: separate;
    border-spacing: 0;
    table-layout: fixed;
  }
  th, td {
    box-sizing: border-box;
    border-inline-end: 1px solid var(--tonk-table-grid-line);
    border-block-end: 1px solid var(--tonk-table-grid-line);
    padding: 0 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: normal;
    text-align: left;
  }
  col.rowhead { width: 48px; }
  thead th {
    position: sticky;
    inset-block-start: 0;
    z-index: 2;
    background: var(--tonk-table-header-bg);
    color: var(--tonk-table-header-fg);
    text-align: center;
    font-size: 0.85em;
  }
  tbody th {
    position: sticky;
    inset-inline-start: 0;
    z-index: 1;
    background: var(--tonk-table-header-bg);
    color: var(--tonk-table-header-fg);
    text-align: center;
    font-size: 0.85em;
  }
  thead th:first-child {
    inset-inline-start: 0;
    z-index: 3;
  }

  td { position: relative; }
  td.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.err {
    color: var(--tonk-table-error);
    text-align: center;
  }
  td.sel-range { background: var(--tonk-table-selection); }
  td.sel {
    outline: 2px solid var(--tonk-table-accent);
    outline-offset: -2px;
  }
  td .cell-editor {
    position: absolute;
    inset: 0;
    z-index: 4;
    border: 2px solid var(--tonk-table-accent);
    padding: 0 2px;
    font: inherit;
    background: var(--tonk-table-bg);
    color: inherit;
    outline: none;
  }

  .tabs {
    display: flex;
    align-items: stretch;
    flex: none;
    overflow-x: auto;
    border-block-start: 1px solid var(--tonk-table-border);
    background: var(--tonk-table-header-bg);
  }
  .tab, .tab-add {
    border: none;
    border-inline-end: 1px solid var(--tonk-table-border);
    padding: 4px 12px;
    background: transparent;
    color: var(--tonk-table-fg-muted);
    font: inherit;
    font-size: 0.85em;
    cursor: pointer;
    white-space: nowrap;
  }
  .tab.active {
    background: var(--tonk-table-bg);
    color: var(--tonk-table-fg);
    box-shadow: inset 0 2px 0 var(--tonk-table-accent);
  }
  .tab-add:disabled { cursor: default; opacity: 0.5; }
  .tabs .rename {
    border: 1px solid var(--tonk-table-accent);
    margin: 2px;
    padding: 2px 6px;
    font: inherit;
    font-size: 0.85em;
    background: var(--tonk-table-bg);
    color: inherit;
    outline: none;
    inline-size: 8em;
  }
`;

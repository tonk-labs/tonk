// Contract between the `<tonk-table>` shell (`../index.ts`) and the
// lazily-loaded grid core (`./index.ts`). The shell imports ONLY types
// from this module — anything with a runtime footprint must stay out
// of it, or the shell chunk stops being tiny.
//
// DESIGN CONTRACT (the element is a base component): every capability
// has a sane default AND is programmable from outside — attributes for
// declarative knobs, these handle methods for imperative control,
// typed events for observability. Programmatic mutations route through
// the SAME commit path as user gestures, so host code driving the grid
// is indistinguishable from typing: repaint, events, and persistence
// all fire identically.

import type { Model } from "@ironcalc/wasm";
import type { CellRow, ColumnRow, RowSizeRow, SheetRow } from "./claims";

/** A parsed initial document for standalone mode: either the engine's
 *  own binary serialization (the lossless envelope channel) or CSV
 *  text (the human-authorable channel; formulas ride as `=…` cell
 *  text). The shell owns envelope/base64 decoding; the grid owns the
 *  engine. */
export type TableSource =
  | { kind: "workbook"; bytes: Uint8Array }
  | { kind: "csv"; csv: string };

/** Dispatch one typed CustomEvent on the host element (bubbling,
 *  composed). In claims mode each mutation event is bound by a tonk
 *  view to a command (`oneditcell=table/edit-cell`); any other host
 *  just listens. Observability events (`selectionchange`) flow through
 *  the same channel in BOTH modes. Detail keys are camelCase and
 *  command-specific (`editCell` ↔ `dom.event.detail/edit-cell`). */
export type HostEmit = (type: string, detail: Record<string, unknown>) => void;

/** How the grid holds its document.
 *
 *  - `standalone`: the element's text/attribute channel carries the
 *    whole workbook (CSV or the versioned envelope); edits round-trip
 *    through the debounced `change` event. Self-contained — no store.
 *  - `claims`: the workbook lives in the dialog store as individuated
 *    claims (one per sheet, per non-empty cell, per explicitly-sized
 *    column/row). The shell feeds parsed data rows in via `applyRows`;
 *    user mutations flow out as typed events. No envelope, no
 *    `change` — the store is the document. */
export type GridMode =
  | { kind: "standalone"; source: TableSource }
  | { kind: "claims"; subject: string };

/** Options the shell hands to `createGrid`. */
export type GridOptions = {
  mode: GridMode;
  /** Host-element event dispatcher (both modes). */
  emit: HostEmit;
  /** Lock the grid (no edits; selection and copy still work). */
  readOnly: boolean;
  /** Minimum rendered extent. Defaults keep an empty sheet looking
   *  like a spreadsheet (100×26); small embeds shrink it via the
   *  element's `min-rows`/`min-cols` attributes. */
  minRows?: number;
  minCols?: number;
  /** Standalone-mode dirty signal, called after every user-committed
   *  mutation. Carries no payload: the shell debounces and then PULLS
   *  `serialize()`/`toCsv()`. Not called in claims mode (mutations
   *  emit typed events instead) nor for programmatic loads. */
  onChange?: () => void;
};

/** The current selection, in the same A1 idiom the claims use. */
export type Selection = {
  /** Active sheet's display name. */
  sheet: string;
  /** Active cell ("B2"). */
  at: string;
  /** Selected range corners, normalized ("A1", "C3"); equal to `at`
   *  for a single-cell selection. */
  from: string;
  to: string;
};

/** Live grid handle returned by `createGrid`. */
export interface TableGrid {
  /** The engine's binary serialization of the whole workbook (every
   *  sheet, formulas, formats) — the lossless export channel. */
  serialize(): Uint8Array;
  /** CSV of the ACTIVE sheet's used range, raw cell content (so
   *  formulas survive as `=…`). The human-readable, lossy channel. */
  toCsv(): string;
  /** Standalone mode: replace the workbook (a genuinely newer remote
   *  write). Preserves the selection best-effort; does not fire
   *  `onChange`. No-op in claims mode. */
  load(source: TableSource): void;
  /** Claims mode: reconcile the engine against the store's current
   *  data rows (already filtered to this subject). Applies sheet,
   *  cell, and sizing differences — resolved through the optimistic-
   *  echo ledgers, deferring the cell under an open editor — and
   *  repaints. No-op in standalone mode. */
  applyRows(
    sheets: SheetRow[],
    cells: CellRow[],
    columns: ColumnRow[],
    rowSizes: RowSizeRow[],
  ): void;

  // --- Data ----------------------------------------------------------

  /** Write a cell by A1 address on the active (or named) sheet. */
  setCell(at: string, content: string, sheetName?: string): void;
  /** Read a cell's raw content and formatted value, or null when the
   *  address is malformed / the sheet is unknown. */
  getCell(
    at: string,
    sheetName?: string,
  ): { content: string; value: string } | null;
  /** Append a sheet (engine-named `SheetN` unless `name` given). */
  addSheet(name?: string): void;

  // --- Sizing (persisted as `table/column` / `table/row` claims in
  //     claims mode) ---------------------------------------------------

  /** Set a column's width in px. `column` is a letter ("B") or 1-based
   *  number. */
  setColumnWidth(column: number | string, px: number, sheetName?: string): void;
  getColumnWidth(column: number | string, sheetName?: string): number | null;
  /** Set a row's height in px. */
  setRowHeight(row: number, px: number, sheetName?: string): void;
  getRowHeight(row: number, sheetName?: string): number | null;

  // --- Structure ------------------------------------------------------
  // Structural ops shift addresses; in claims mode the resulting cell
  // (and sizing) differences are diffed and emitted automatically.

  insertRows(before: number, count?: number): void;
  deleteRows(row: number, count?: number): void;
  insertColumns(before: number | string, count?: number): void;
  deleteColumns(column: number | string, count?: number): void;

  // --- Selection ------------------------------------------------------

  /** Select a cell, or a range when `to` is given. */
  select(at: string, to?: string): void;
  /** The current selection. Changes (user or programmatic) fire a
   *  `selectionchange` event on the host. */
  readonly selection: Selection;

  setReadOnly(readOnly: boolean): void;
  /** Adjust the minimum rendered extent (the `min-rows`/`min-cols`
   *  attributes route here). */
  setMinExtent(minRows?: number, minCols?: number): void;
  /** Move keyboard focus into the grid. */
  focus(): void;
  /** Tear down the DOM, release listeners, free the engine model. */
  destroy(): void;
  /** The underlying IronCalc model. Power-user escape hatch — reads
   *  are always safe; writes bypass repaint and persistence. */
  readonly model: Model;
}

/** Shape of the dynamically-imported grid-core module. `createGrid`
 *  is async: the first call also instantiates the engine wasm (from
 *  the `tonk-table-engine.js` bytes leaf), once per page. */
export type GridModule = {
  createGrid(parent: HTMLElement, options: GridOptions): Promise<TableGrid>;
};

export type { CellRow, ColumnRow, RowSizeRow, SheetRow };

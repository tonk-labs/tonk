// DOM-free engine helpers: model construction from a `TableSource`,
// used-range discovery, CSV round-trip, and paste routing. Split from
// the grid's DOM code so the node test runner can exercise the REAL
// engine (instantiated from bytes via `initSync`) without a browser.

import { Model } from "@ironcalc/wasm";
import type { TableSource } from "./api";

/** Fixed model locale/timezone/language. The workbook travels between
 *  peers inside the content envelope, so per-client locale would make
 *  the same document parse differently on each end (decimal commas,
 *  date order). One canonical locale keeps the bytes deterministic;
 *  display localization can come later as a rendering concern. */
export const LOCALE = "en";
export const TIMEZONE = "UTC";
export const LANGUAGE = "en";

/** Columns scanned when discovering the used range (and therefore the
 *  CSV serialization width). The LOSSLESS byte channel has no such
 *  bound — this caps only the lossy CSV view and the painted grid's
 *  auto-extent. 256 columns ≫ any human-authored sheet. */
export const SCAN_COLS = 256;

/** A fresh empty workbook (one sheet). */
export function newModel(name = "Workbook"): Model {
  return new Model(name, LOCALE, TIMEZONE, LANGUAGE);
}

/** Build a model from a parsed source. Degrades rather than throws:
 *  corrupt workbook bytes or an unparseable CSV yield a fresh empty
 *  model (with a console warning) — a bad store write must render an
 *  empty grid, not crash the element. */
export function modelFromSource(source: TableSource): Model {
  if (source.kind === "workbook") {
    try {
      return Model.from_bytes(source.bytes, LANGUAGE);
    } catch (err) {
      console.warn("[tonk-table] corrupt workbook bytes; starting empty:", err);
      return newModel();
    }
  }
  const model = newModel();
  if (source.csv.trim() !== "") {
    try {
      seedCsv(model, 0, source.csv);
    } catch (err) {
      console.warn("[tonk-table] failed to seed CSV; starting empty:", err);
      return newModel();
    }
  }
  return model;
}

/** Parse RFC 4180 CSV into rows of fields: quoted fields may carry
 *  commas, newlines, and doubled quotes. Done HERE, not by the engine —
 *  IronCalc's `pasteCsvText` splits on tabs only (it's really "paste
 *  clipboard text"; pinned by the tests), so routing comma CSV through
 *  it lands whole lines in single cells. Degrades gracefully: an
 *  unclosed quote keeps its accumulated text; a trailing newline is
 *  not an extra empty row. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i++;
        continue;
      }
      field += ch;
      i++;
      continue;
    }
    if (ch === '"' && field === "") {
      inQuotes = true;
      i++;
      continue;
    }
    if (ch === ",") {
      row.push(field);
      field = "";
      i++;
      continue;
    }
    if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      field = "";
      rows.push(row);
      row = [];
      i++;
      continue;
    }
    field += ch;
    i++;
  }
  if (field !== "" || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

/** Parse tab-separated text (the spreadsheet clipboard lingua franca):
 *  naive line/tab split — the TSV convention carries no quoting. A
 *  trailing newline (every spreadsheet emits one) is not an extra
 *  empty row. */
function parseTsv(text: string): string[][] {
  const lines = text.split(/\r?\n/);
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line) => line.split("\t"));
}

/** Parse clipboard/authored tabular text: tab-separated when any tab
 *  is present (spreadsheet copies), RFC 4180 CSV otherwise. */
export function parseDelimited(text: string): string[][] {
  return text.includes("\t") ? parseTsv(text) : parseCsv(text);
}

/** Write a parsed cell block into the sheet anchored at `(row,
 *  column)`, with evaluation paused across the burst so a big block
 *  costs one recompute. */
function applyCells(
  model: Model,
  sheet: number,
  row: number,
  column: number,
  cells: string[][],
): void {
  if (cells.length === 0) return;
  model.pauseEvaluation();
  try {
    for (let i = 0; i < cells.length; i++) {
      const fields = cells[i];
      for (let j = 0; j < fields.length; j++) {
        model.setUserInput(sheet, row + i, column + j, fields[j]);
      }
    }
  } finally {
    model.resumeEvaluation();
  }
  model.evaluate();
}

/** Seed CSV text into `sheet` anchored at A1. */
export function seedCsv(model: Model, sheet: number, csv: string): void {
  applyCells(model, sheet, 1, 1, parseCsv(csv));
}

/** The sheet's used range as `{ rows, cols }` (1-based inclusive
 *  extents; both 0 when the sheet is empty). Composed from
 *  `getRowsWithData` per column — the engine exposes no direct
 *  used-range call — so it sees the first `SCAN_COLS` columns. */
export function usedRange(model: Model, sheet: number): { rows: number; cols: number } {
  let rows = 0;
  let cols = 0;
  for (let column = 1; column <= SCAN_COLS; column++) {
    const withData = model.getRowsWithData(sheet, column);
    if (withData.length === 0) continue;
    cols = column;
    for (const row of withData) rows = Math.max(rows, row);
  }
  return { rows, cols };
}

/** Quote one CSV field per RFC 4180: wrap when it contains a comma,
 *  quote, or line break; double embedded quotes. */
export function quoteCsvField(text: string): string {
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

/** CSV of the sheet's used range, from RAW cell content
 *  (`getCellContent`) rather than formatted values — so formulas
 *  survive as `=…` and the text re-seeds a workbook losslessly enough
 *  for the human channel. Empty sheet serializes to `""`. */
export function toCsv(model: Model, sheet: number): string {
  const { rows, cols } = usedRange(model, sheet);
  if (rows === 0) return "";
  const lines: string[] = [];
  for (let row = 1; row <= rows; row++) {
    const fields: string[] = [];
    for (let column = 1; column <= cols; column++) {
      fields.push(quoteCsvField(model.getCellContent(sheet, row, column)));
    }
    lines.push(fields.join(","));
  }
  return lines.join("\n");
}

/** Paste clipboard text anchored at `(row, column)`: tab-separated or
 *  comma-separated, parsed by `parseDelimited`. */
export function pasteText(
  model: Model,
  sheet: number,
  row: number,
  column: number,
  text: string,
): void {
  applyCells(model, sheet, row, column, parseDelimited(text));
}

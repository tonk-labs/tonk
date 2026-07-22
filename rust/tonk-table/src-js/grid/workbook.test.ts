// Exercises the REAL IronCalc engine: the test bundle embeds the wasm
// bytes (test.mjs configures the same `binary` loader as the build) and
// instantiates them exactly the way production does — from bytes, no
// fetch. Everything the grid assumes about the engine is pinned here:
// formula evaluation, byte round-trips, the CSV channel, paste routing,
// and the `SelectedView.range` tuple order.

import { test, before } from "node:test";
import assert from "node:assert/strict";
import { initSync, Model } from "@ironcalc/wasm";
import engineBytes from "../engine";
import {
  modelFromSource,
  newModel,
  parseCsv,
  parseDelimited,
  pasteText,
  quoteCsvField,
  toCsv,
  usedRange,
} from "./workbook";

before(() => {
  initSync({ module: engineBytes });
});

test("formulas evaluate through setUserInput", () => {
  const model = newModel();
  model.setUserInput(0, 1, 1, "23");
  model.setUserInput(0, 1, 2, "=A1*3+1");
  assert.equal(model.getFormattedCellValue(0, 1, 2), "70");
  model.free();
});

test("toBytes / from_bytes round-trips values and formulas", () => {
  const model = newModel();
  model.setUserInput(0, 1, 1, "23");
  model.setUserInput(0, 1, 2, "=A1*3+1");
  model.setUserInput(0, 2, 1, "hello");
  const bytes = model.toBytes();
  model.free();

  const back = Model.from_bytes(bytes, "en");
  assert.equal(back.getFormattedCellValue(0, 1, 2), "70");
  assert.equal(back.getCellContent(0, 1, 2), "=A1*3+1");
  assert.equal(back.getFormattedCellValue(0, 2, 1), "hello");
  back.free();
});

test("modelFromSource degrades corrupt workbook bytes to an empty model", () => {
  const model = modelFromSource({
    kind: "workbook",
    bytes: new Uint8Array([1, 2, 3, 4, 5]),
  });
  assert.equal(usedRange(model, 0).rows, 0);
  model.free();
});

test("CSV seeds a workbook; formulas ride as cell text", () => {
  const model = modelFromSource({
    kind: "csv",
    csv: "a,b\n1,=A2*2",
  });
  assert.equal(model.getFormattedCellValue(0, 1, 1), "a");
  assert.equal(model.getFormattedCellValue(0, 2, 1), "1");
  assert.equal(model.getCellContent(0, 2, 2), "=A2*2");
  assert.equal(model.getFormattedCellValue(0, 2, 2), "2");
  model.free();
});

test("toCsv serializes the used range with RFC 4180 quoting", () => {
  const model = newModel();
  model.setUserInput(0, 1, 1, "plain");
  model.setUserInput(0, 1, 2, "with, comma");
  model.setUserInput(0, 2, 1, 'quote "inner"');
  model.setUserInput(0, 3, 3, "=SUM(1,2)");
  const csv = toCsv(model, 0);
  assert.equal(
    csv,
    'plain,"with, comma",\n' + '"quote ""inner""",,\n' + ',,"=SUM(1,2)"',
  );
  model.free();
});

test("toCsv output re-seeds an equivalent workbook", () => {
  const model = newModel();
  model.setUserInput(0, 1, 1, "2");
  model.setUserInput(0, 1, 2, "=A1*10");
  const csv = toCsv(model, 0);
  model.free();

  const back = modelFromSource({ kind: "csv", csv });
  assert.equal(back.getFormattedCellValue(0, 1, 2), "20");
  back.free();
});

test("empty sheet serializes to an empty string", () => {
  const model = newModel();
  assert.equal(toCsv(model, 0), "");
  model.free();
});

test("parseCsv handles quoting, embedded delimiters, and line endings", () => {
  assert.deepEqual(parseCsv("a,b\n1,2"), [
    ["a", "b"],
    ["1", "2"],
  ]);
  assert.deepEqual(parseCsv('a,"x,y",c'), [["a", "x,y", "c"]]);
  assert.deepEqual(parseCsv('"multi\nline",b'), [["multi\nline", "b"]]);
  assert.deepEqual(parseCsv('say ""hi"" style,"esc ""q"""'), [
    ['say ""hi"" style', 'esc "q"'],
  ]);
  assert.deepEqual(parseCsv("a,b\r\n1,2\r\n"), [
    ["a", "b"],
    ["1", "2"],
  ]);
  assert.deepEqual(parseCsv(""), []);
  // A trailing newline is not an extra empty row.
  assert.deepEqual(parseCsv("a,b\n"), [["a", "b"]]);
});

test("parseDelimited picks TSV when tabs are present", () => {
  assert.deepEqual(parseDelimited("a\tb\n1\t2\n"), [
    ["a", "b"],
    ["1", "2"],
  ]);
  assert.deepEqual(parseDelimited("a,b"), [["a", "b"]]);
});

test("quoteCsvField wraps only when needed", () => {
  assert.equal(quoteCsvField("plain"), "plain");
  assert.equal(quoteCsvField("a,b"), '"a,b"');
  assert.equal(quoteCsvField('say "hi"'), '"say ""hi"""');
  assert.equal(quoteCsvField("two\nlines"), '"two\nlines"');
});

test("pasteText routes tab-separated text cell by cell", () => {
  const model = newModel();
  pasteText(model, 0, 2, 2, "a\tb\n1\t=B2&\"!\"\n");
  assert.equal(model.getFormattedCellValue(0, 2, 2), "a");
  assert.equal(model.getFormattedCellValue(0, 2, 3), "b");
  assert.equal(model.getFormattedCellValue(0, 3, 2), "1");
  // The formula pasted at C3 references B2 and evaluated.
  assert.equal(model.getFormattedCellValue(0, 3, 3), "a!");
  model.free();
});

test("pasteText routes comma text through the engine CSV parser", () => {
  const model = newModel();
  pasteText(model, 0, 1, 1, 'a,"x,y"\n1,2');
  assert.equal(model.getFormattedCellValue(0, 1, 2), "x,y");
  assert.equal(model.getFormattedCellValue(0, 2, 2), "2");
  model.free();
});

test("SelectedView.range is [rowStart, colStart, rowEnd, colEnd]", () => {
  // The grid's range handling (clear, highlight, paste anchor) assumes
  // this order; pin it against the engine so a binding change fails
  // loudly here instead of scrambling selections.
  const model = newModel();
  model.setSelectedCell(2, 3);
  model.onExpandSelectedRange("ArrowDown");
  const view = model.getSelectedView();
  assert.equal(view.row, 2);
  assert.equal(view.column, 3);
  assert.deepEqual(Array.from(view.range), [2, 3, 3, 3]);
  model.free();
});

test("sheet management: add, rename, switch", () => {
  const model = newModel();
  model.newSheet();
  const sheets = model.getWorksheetsProperties();
  assert.equal(sheets.length, 2);
  model.renameSheet(1, "Budget");
  assert.equal(model.getWorksheetsProperties()[1].name, "Budget");
  model.setSelectedSheet(1);
  assert.equal(model.getSelectedSheet(), 1);
  model.setUserInput(1, 1, 1, "42");
  assert.equal(toCsv(model, 1), "42");
  assert.equal(toCsv(model, 0), "");
  model.free();
});

test("insertRows shifts content and rewrites formulas (the diff churn)", () => {
  // Structural ops move addresses AND rewrite dependent formulas —
  // the reason claims mode diffs snapshots around them instead of
  // guessing. Pin both effects.
  const model = newModel();
  model.setUserInput(0, 1, 1, "1");
  model.setUserInput(0, 2, 1, "2");
  model.setUserInput(0, 1, 2, "=A2*10");
  model.insertRows(0, 2, 1);
  assert.equal(model.getCellContent(0, 3, 1), "2"); // shifted down
  assert.equal(model.getCellContent(0, 2, 1), ""); // inserted row empty
  assert.equal(model.getCellContent(0, 1, 2), "=A3*10"); // formula rewritten
  assert.equal(model.getFormattedCellValue(0, 1, 2), "20");
  model.free();
});

test("column widths and row heights read back what was set", () => {
  const model = newModel();
  const before = model.getColumnWidth(0, 2);
  model.setColumnsWidth(0, 2, 2, 140);
  assert.equal(Math.round(model.getColumnWidth(0, 2)), 140);
  assert.equal(Math.round(model.getColumnWidth(0, 3)), Math.round(before));
  model.setRowsHeight(0, 3, 3, 42);
  assert.equal(Math.round(model.getRowHeight(0, 3)), 42);
  model.free();
});

test("undo restores the previous value after a committed input", () => {
  const model = newModel();
  model.setUserInput(0, 1, 1, "first");
  model.setUserInput(0, 1, 1, "second");
  assert.ok(model.canUndo());
  model.undo();
  assert.equal(model.getFormattedCellValue(0, 1, 1), "first");
  model.redo();
  assert.equal(model.getFormattedCellValue(0, 1, 1), "second");
  model.free();
});

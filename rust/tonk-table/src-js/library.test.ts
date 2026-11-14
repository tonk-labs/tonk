// The helpers the <table-editor> methods share through the core: the
// content channel read off the light DOM, the claim rows, and the
// envelope the element emits.

import { test, before } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { initSync } from "@ironcalc/wasm";
import engineBytes from "@ironcalc/wasm/wasm_bg.wasm";
import { lightContent, parseContent, readRows, toSource, userStyles } from "./library";
import { formatContent, WORKBOOK_TYPE } from "./content";
import { bytesToBase64 } from "./b64";
import { newModel } from "./grid/workbook";

before(() => {
  initSync({ module: engineBytes });
});

function host(html: string) {
  const { document } = parseHTML(`<html><body><table-editor>${html}</table-editor></body></html>`);
  return document.querySelector("table-editor")!;
}

test("the content channel is the direct text, not the children's", () => {
  const el = host(`a,b\n<style>td { color: red }</style>1,2<div class="table-cell-row" data-content="x">ignored</div>`);
  assert.equal(lightContent(el), "a,b\n1,2");
});

test("user styles are the direct style children, in order", () => {
  const el = host(`<style>td { color: red }</style><div><style>nested {}</style></div><style>th {}</style>`);
  assert.equal(userStyles(el), "td { color: red }\nth {}\n");
});

test("readRows keeps only the subject's sheets and their lines", () => {
  const el = host(`
    <div class="table-sheet-row" subject="s1" data-table="book" data-name="One" data-order="m"></div>
    <div class="table-sheet-row" subject="s2" data-table="other" data-name="Two" data-order="n"></div>
    <div class="table-cell-row" subject="c1" data-sheet="s1" data-at="A1" data-content="1"></div>
    <div class="table-cell-row" subject="c2" data-sheet="s2" data-at="A1" data-content="2"></div>
    <div class="table-column-row" subject="k1" data-sheet="s1" data-at="B" data-width="120"></div>
    <div class="table-rowsize-row" subject="r1" data-sheet="s2" data-at="3" data-height="40"></div>`);
  const rows = readRows(el, "book");
  assert.deepEqual(rows.sheets.map((s) => s.id), ["s1"]);
  assert.deepEqual(rows.cells.map((c) => c.id), ["c1"]);
  assert.deepEqual(rows.columns.map((c) => c.id), ["k1"]);
  assert.deepEqual(rows.rowSizes, []);
});

test("bare text is a CSV source", () => {
  assert.deepEqual(toSource(parseContent("a,b\n1,2")), { kind: "csv", csv: "a,b\n1,2" });
});

test("a workbook envelope is its bytes", () => {
  const bytes = newModel().toBytes();
  const text = formatContent({ hlc: 7n, contentType: WORKBOOK_TYPE, value: bytesToBase64(bytes) });
  const parsed = parseContent(text);
  assert.equal(parsed.hlc, 7n);
  assert.deepEqual(toSource(parsed), { kind: "workbook", bytes });
});

test("a corrupt workbook body is an empty sheet, not a throw", () => {
  const text = formatContent({ hlc: 1n, contentType: WORKBOOK_TYPE, value: "!!not base64!!" });
  assert.deepEqual(toSource(parseContent(text)), { kind: "csv", csv: "" });
});

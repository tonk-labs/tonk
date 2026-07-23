import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import {
  PendingLedger,
  afterAll,
  between,
  byOrder,
  columnName,
  columnNumber,
  diffSnapshots,
  formatAddress,
  parseAddress,
  readCellRows,
  readColumnRows,
  readRowSizeRows,
  readSheetRows,
} from "./claims";

test("column letters round-trip through bijective base-26", () => {
  for (const [name, n] of [
    ["A", 1],
    ["Z", 26],
    ["AA", 27],
    ["AZ", 52],
    ["BA", 53],
    ["ZZ", 702],
    ["AAA", 703],
  ] as const) {
    assert.equal(columnNumber(name), n, name);
    assert.equal(columnName(n), name, String(n));
  }
});

test("addresses parse and format", () => {
  assert.deepEqual(parseAddress("B2"), { row: 2, column: 2 });
  assert.deepEqual(parseAddress("AA10"), { row: 10, column: 27 });
  assert.equal(formatAddress(10, 27), "AA10");
  // Malformed forms (hand-written claims) parse to null, not garbage.
  assert.equal(parseAddress("b2"), null);
  assert.equal(parseAddress("2B"), null);
  assert.equal(parseAddress("B0"), null);
  assert.equal(parseAddress("B02"), null);
  assert.equal(parseAddress(""), null);
});

test("between produces keys that sort strictly between the bounds", () => {
  const k = between("", "");
  assert.ok(k > "" && k < "{");
  const mid = between("a", "c");
  assert.ok(mid > "a" && mid < "c", mid);
  // Adjacent keys still admit an in-between.
  const tight = between("a", "b");
  assert.ok(tight > "a" && tight < "b", tight);
  // afterAll appends past the (unsorted) maximum.
  const order = afterAll([
    { order: "m", id: "x" },
    { order: "c", id: "y" },
  ]);
  assert.ok(order > "m", order);
});

test("byOrder sorts by key, tiebreaking on id", () => {
  const rows = [
    { order: "m", id: "b" },
    { order: "c", id: "z" },
    { order: "m", id: "a" },
  ];
  assert.deepEqual(
    [...rows].sort(byOrder).map((r) => r.id),
    ["z", "a", "b"],
  );
});

test("diffSnapshots yields adds, changes, and clears across sheets", () => {
  const prev = new Map([
    ["s1", new Map([["A1", "1"], ["B1", "2"]])],
    ["s2", new Map([["A1", "x"]])],
  ]);
  const next = new Map([
    ["s1", new Map([["A1", "1"], ["B1", "3"], ["C1", "new"]])],
    ["s2", new Map<string, string>()],
  ]);
  const deltas = diffSnapshots(prev, next).sort((a, b) =>
    `${a.sheet}${a.at}`.localeCompare(`${b.sheet}${b.at}`),
  );
  assert.deepEqual(deltas, [
    { sheet: "s1", at: "B1", content: "3" },
    { sheet: "s1", at: "C1", content: "new" },
    { sheet: "s2", at: "A1", content: "" },
  ]);
});

test("diffSnapshots of identical snapshots is empty", () => {
  const snap = new Map([["s1", new Map([["A1", "=B1*2"]])]]);
  assert.deepEqual(diffSnapshots(snap, snap), []);
});

test("PendingLedger masks a write until its echo, then steps aside", () => {
  let now = 1000;
  const ledger = new PendingLedger(() => now);
  ledger.record("s1", "A1", "42");
  // Store still carries the old value: pending wins.
  assert.equal(ledger.resolve("s1", "A1", "old"), "42");
  // Echo arrives: entry clears, store value flows through.
  assert.equal(ledger.resolve("s1", "A1", "42"), "42");
  assert.equal(ledger.size, 0);
  assert.equal(ledger.resolve("s1", "A1", "remote"), "remote");
});

test("PendingLedger entries expire, so a rejected write stops masking", () => {
  let now = 1000;
  const ledger = new PendingLedger(() => now);
  ledger.record("s1", "A1", "willfail");
  assert.equal(ledger.resolve("s1", "A1", ""), "willfail");
  now += PendingLedger.EXPIRY_MS + 1;
  assert.equal(ledger.resolve("s1", "A1", ""), "");
  assert.equal(ledger.size, 0);
});

// --- Row readers (linkedom) ---------------------------------------------

function rootOf(html: string): ParentNode {
  const { document } = parseHTML(`<html><body>${html}</body></html>`);
  return document;
}

test("readSheetRows filters by workbook subject and sorts by order", () => {
  const root = rootOf(`
    <div class="table-sheet-row" subject="s2" data-table="book" data-name="Later" data-order="m"></div>
    <div class="table-sheet-row" subject="s1" data-table="book" data-name="First" data-order="c"></div>
    <div class="table-sheet-row" subject="sX" data-table="OTHER" data-name="Foreign" data-order="a"></div>
    <div class="table-sheet-row" data-table="book" data-name="NoEntity" data-order="a"></div>
  `);
  const rows = readSheetRows(root, "book");
  assert.deepEqual(
    rows.map((r) => [r.id, r.name]),
    [
      ["s1", "First"],
      ["s2", "Later"],
    ],
  );
});

test("readColumnRows / readRowSizeRows validate their line and size", () => {
  const root = rootOf(`
    <div class="table-column-row" subject="w1" data-sheet="s1" data-at="B" data-width="140"></div>
    <div class="table-column-row" subject="w2" data-sheet="s1" data-at="2" data-width="99"></div>
    <div class="table-column-row" subject="w3" data-sheet="s1" data-at="C" data-width="-5"></div>
    <div class="table-column-row" subject="w4" data-sheet="sX" data-at="D" data-width="80"></div>
    <div class="table-rowsize-row" subject="h1" data-sheet="s1" data-at="3" data-height="42.5"></div>
    <div class="table-rowsize-row" subject="h2" data-sheet="s1" data-at="B" data-height="42"></div>
    <div class="table-rowsize-row" subject="h3" data-sheet="s1" data-at="0" data-height="42"></div>
  `);
  const sheets = new Set(["s1"]);
  assert.deepEqual(
    readColumnRows(root, sheets).map((r) => [r.id, r.at, r.width]),
    [["w1", "B", "140"]],
  );
  assert.deepEqual(
    readRowSizeRows(root, sheets).map((r) => [r.id, r.at, r.height]),
    [["h1", "3", "42.5"]],
  );
});

test("readCellRows keeps only known sheets and valid addresses", () => {
  const root = rootOf(`
    <div class="table-cell-row" subject="c1" data-sheet="s1" data-at="A1" data-content="=B1*2"></div>
    <div class="table-cell-row" subject="c2" data-sheet="s1" data-at="bogus" data-content="dropped"></div>
    <div class="table-cell-row" subject="c3" data-sheet="sX" data-at="A1" data-content="foreign"></div>
    <div class="table-cell-row" subject="c4" data-sheet="s1" data-at="B2" data-content="7" data-style='{"b":true}'></div>
  `);
  const rows = readCellRows(root, new Set(["s1"]));
  assert.deepEqual(
    rows.map((r) => [r.id, r.at, r.content, r.style]),
    [
      ["c1", "A1", "=B1*2", ""],
      ["c4", "B2", "7", '{"b":true}'],
    ],
  );
});

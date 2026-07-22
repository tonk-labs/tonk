import { test } from "node:test";
import assert from "node:assert/strict";
import {
  WORKBOOK_TYPE,
  formatContent,
  isEnvelope,
  isWorkbookType,
  parseContent,
} from "./content";

test("bare text parses as HLC-less CSV", () => {
  const parsed = parseContent("a,b\n1,2");
  assert.equal(parsed.hlc, null);
  assert.equal(parsed.contentType, null);
  assert.equal(parsed.value, "a,b\n1,2");
});

test("envelope round-trips hlc, content type, and body", () => {
  const body = "QUJDRA=="; // arbitrary base64ish body
  const wire = formatContent({
    hlc: 123456789n,
    contentType: WORKBOOK_TYPE,
    value: body,
  });
  assert.ok(isEnvelope(wire));
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, 123456789n);
  assert.equal(parsed.contentType, WORKBOOK_TYPE);
  assert.equal(parsed.value, body);
});

test("null hlc formats as the bare value", () => {
  const wire = formatContent({ hlc: null, contentType: null, value: "x,y" });
  assert.equal(wire, "x,y");
});

test("null contentType omits the header and reads back null", () => {
  const wire = formatContent({ hlc: 7n, contentType: null, value: "a,b" });
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, 7n);
  assert.equal(parsed.contentType, null);
  assert.equal(parsed.value, "a,b");
});

test("header casing is normalized away", () => {
  const wire =
    "tonk-table-version: 1\n" +
    'etag: "42"\n' +
    "CONTENT-TYPE: application/vnd.ironcalc\n" +
    "\n" +
    "Qk9EWQ==";
  assert.ok(isEnvelope(wire));
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, 42n);
  assert.ok(isWorkbookType(parsed.contentType));
  assert.equal(parsed.value, "Qk9EWQ==");
});

test("LF-only envelopes parse; body blank lines are preserved", () => {
  const wire = 'Tonk-Table-Version: 1\nETag: "9"\n\nline1\n\nline3';
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, 9n);
  // Only the FIRST blank line separates headers from body.
  assert.equal(parsed.value, "line1\n\nline3");
});

test("a malformed envelope (no separator) degrades to bare text", () => {
  const wire = 'Tonk-Table-Version: 1\nETag: "9"';
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, null);
  assert.equal(parsed.value, wire);
});

test("a garbled ETag parses as unversioned, keeping the body", () => {
  const wire = "Tonk-Table-Version: 1\r\nETag: \"not-a-number\"\r\n\r\nbody";
  const parsed = parseContent(wire);
  assert.equal(parsed.hlc, null);
  assert.equal(parsed.value, "body");
});

test("CSV that merely mentions the header mid-text is not an envelope", () => {
  const text = "note\nTonk-Table-Version: 1\n\nrest";
  assert.equal(isEnvelope(text), false);
  assert.equal(parseContent(text).value, text);
});

test("isWorkbookType matches suffixed variants and rejects CSV", () => {
  assert.ok(isWorkbookType("application/vnd.ironcalc"));
  assert.ok(isWorkbookType("Application/VND.IronCalc;v=2"));
  assert.equal(isWorkbookType("text/csv"), false);
  assert.equal(isWorkbookType(null), false);
});

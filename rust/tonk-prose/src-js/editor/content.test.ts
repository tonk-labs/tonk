import { test } from "node:test";
import assert from "node:assert/strict";
import { parseContent, formatContent, isEnvelope } from "./content";
import { pack } from "./hlc";

test("it round-trips an envelope", () => {
  const hlc = pack(1780982815290, 5);
  const value = "hello **world**\n\nsecond paragraph";
  const parsed = parseContent(formatContent({ hlc, value }));
  assert.equal(parsed.hlc, hlc);
  assert.equal(parsed.value, value);
});

test("it preserves a body containing blank lines", () => {
  const hlc = pack(1, 0);
  const value = "line1\n\nline2\n\nline3";
  const parsed = parseContent(formatContent({ hlc, value }));
  assert.equal(parsed.value, value);
});

test("it treats bare markdown as an un-versioned value", () => {
  const parsed = parseContent("just plain markdown\n\nno envelope");
  assert.equal(parsed.hlc, null);
  assert.equal(parsed.value, "just plain markdown\n\nno envelope");
});

test("it recognizes an envelope whose headers were lowercased", () => {
  // A layer between store and element (DOM attribute reflection, a
  // normalizing store) may lowercase header names. The envelope must
  // still parse — else its own headers leak into the document.
  const hlc = pack(999, 0);
  const good = formatContent({ hlc, value: "# Body" });
  const lowercased = good.toLowerCase().replace("# body", "# Body");
  assert.equal(isEnvelope(lowercased), true);
  const parsed = parseContent(lowercased);
  assert.equal(parsed.hlc, hlc);
  assert.equal(parsed.value, "# Body");
});

test("it parses an envelope delimited by LF instead of CRLF", () => {
  const hlc = pack(42, 1);
  const good = formatContent({ hlc, value: "body" });
  const lfOnly = good.replace(/\r\n/g, "\n");
  const parsed = parseContent(lfOnly);
  assert.equal(parsed.hlc, hlc);
  assert.equal(parsed.value, "body");
});

test("an envelope with no header/body separator degrades to bare markdown", () => {
  // If every newline is stripped there's no separator; rather than
  // throw, the whole string is treated as bare markdown (hlc null).
  const joined = 'Tonk-Prose-Version: 1ETag: "5"Content-Type: text/markdown# Body';
  const parsed = parseContent(joined);
  assert.equal(parsed.hlc, null);
});

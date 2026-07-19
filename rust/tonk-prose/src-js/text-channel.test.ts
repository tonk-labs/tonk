// The content channel is the element's light-DOM text (like <textarea>).
// These tests verify the property that fixes the store round-trip bug:
// content set as element text survives verbatim and never parses markup,
// and a versioned envelope carried that way parses back cleanly (headers
// don't leak into the document). They exercise the channel + envelope
// parse directly through a real DOM, without the ProseMirror editor
// (which needs a layout engine); the editor's own adoption is covered by
// content.test.ts's HLC tests.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { parseContent, formatContent } from "./editor/content";
import { pack } from "./editor/hlc";

function host() {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  const el = document.createElement("tonk-prose");
  document.body.appendChild(el);
  return el;
}

test("content set as element text round-trips through parseContent", () => {
  const hlc = pack(1234, 2);
  const value = "# Title\n\nA paragraph with **bold**.\n\n> quote";
  const el = host();
  // How the view writes it: set_text_content(envelope).
  el.textContent = formatContent({ hlc, value });
  const parsed = parseContent(el.textContent ?? "");
  assert.equal(parsed.hlc, hlc);
  assert.equal(parsed.value, value);
});

test("markdown containing HTML markup stays literal in the text channel", () => {
  // The hazard textContent avoids: `<div>` must NOT become a child
  // element. If it did, the document would be corrupted and the
  // markdown lost.
  const value = "# Note\n\n<div>literal</div> and <b>x</b> & <script>y</script>";
  const el = host();
  el.textContent = formatContent({ hlc: pack(1, 0), value });
  // No markup was parsed into elements.
  assert.equal(el.children.length, 0);
  // And it parses back to exactly the markdown.
  const parsed = parseContent(el.textContent ?? "");
  assert.equal(parsed.value, value);
});

test("CRLF in the envelope survives the text channel", () => {
  // The confirmed corruption vector was newline stripping. Text content
  // preserves every byte, so the header/body separator survives and the
  // envelope parses (rather than degrading to bare markdown and leaking
  // its own headers into the document).
  const el = host();
  const envelope = formatContent({ hlc: pack(7, 0), value: "body\n\nmore" });
  assert.match(envelope, /\r\n\r\n/); // it has a real CRLF separator
  el.textContent = envelope;
  assert.equal(el.textContent, envelope); // preserved verbatim
  const parsed = parseContent(el.textContent ?? "");
  assert.equal(parsed.hlc, pack(7, 0));
  assert.equal(parsed.value, "body\n\nmore");
});

test("a bare-markdown text child (no envelope) parses as un-versioned", () => {
  const el = host();
  el.textContent = "# Seed\n\nplain markdown, no envelope";
  const parsed = parseContent(el.textContent ?? "");
  assert.equal(parsed.hlc, null);
  assert.equal(parsed.value, "# Seed\n\nplain markdown, no envelope");
});

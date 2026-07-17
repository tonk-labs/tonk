// DOM-boundary tests for how content survives the store → element path.
//
// The failure we're chasing: an envelope (multi-line, headers) fed back
// into <tonk-prose> comes out with its structure mangled and the headers
// leak into the document. These tests pin down which DOM channel
// (attribute value vs textContent) preserves the content, and whether
// HTML markup inside the content is a hazard — the two questions that
// decide the right content channel for the element.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";

/** A fresh DOM per test — linkedom gives us document/Element/etc. */
function dom() {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  return document;
}

const MULTILINE = "Tonk-Prose-Version: 1\r\nETag: \"5\"\r\n\r\n# Body\n\npara";

test("setAttribute preserves a value's CRLF verbatim", () => {
  // setAttribute is NOT the HTML *parser*; per spec it stores the value
  // string as-is. Newlines are only normalized when an attribute is
  // parsed from HTML source text, not when set via the DOM API.
  const el = dom().createElement("div");
  el.setAttribute("data-x", MULTILINE);
  assert.equal(el.getAttribute("data-x"), MULTILINE);
});

test("a property set preserves the value verbatim", () => {
  const el = dom().createElement("div") as unknown as Record<string, unknown>;
  el.content = MULTILINE;
  assert.equal(el.content, MULTILINE);
});

test("textContent preserves CRLF and never parses markup", () => {
  // The <textarea>/<pre> approach: content as a text child. textContent
  // set/get is verbatim, and — crucially — setting it never interprets
  // `<...>` as markup (it creates a Text node), so markdown containing
  // HTML is safe.
  const el = dom().createElement("div");
  const withMarkup = "# Title\n\n<div>literal</div> & <b>x</b>\n\nmore";
  el.textContent = withMarkup;
  assert.equal(el.textContent, withMarkup);
  // No child elements were created — the `<div>`/`<b>` are literal text.
  assert.equal(el.children.length, 0);
});

test("innerHTML with the same content DOES parse the markup (the hazard)", () => {
  // Contrast: if content ever reaches the element as innerHTML (e.g. a
  // template that puts {content} between the tags without escaping), the
  // `<div>`/`<b>` become real elements and the markdown is corrupted.
  // This is the case textContent avoids and the one to guard against.
  const el = dom().createElement("div");
  el.innerHTML = "# Title\n\n<div>literal</div> & <b>x</b>";
  assert.ok(el.children.length > 0, "innerHTML parsed markup into elements");
});

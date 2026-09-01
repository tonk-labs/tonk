import { test } from "node:test";
import assert from "node:assert/strict";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { headingTitle } from "./heading-switcher";

/** The document the switcher opens on. */
const START = "# ";

test("the starting document IS a heading", () => {
  const doc = parseMarkdown(START);
  assert.equal(doc.firstChild?.type.name, "heading");
});

test("the heading's raw text carries the literal marker", () => {
  // This is why the switcher cannot filter on `textContent`: the marker is
  // materialized as literal text, so a raw read searches for titles
  // containing "# " and matches nothing.
  const doc = parseMarkdown(START);
  assert.equal(doc.firstChild?.textContent, "# ");
});

test("headingTitle strips the marker", () => {
  assert.equal(headingTitle(parseMarkdown("# Notes").firstChild!), "Notes");
  assert.equal(headingTitle(parseMarkdown(START).firstChild!), "");
});

test("headingTitle keeps text that merely looks like a marker", () => {
  // `#tag` is not a marker: no space, so it is ordinary content and must
  // survive into the title.
  assert.equal(headingTitle(parseMarkdown("# #tag rules").firstChild!), "#tag rules");
});

test("an empty heading survives a round trip", () => {
  const out = serializeMarkdown(parseMarkdown(START));
  assert.ok(out.startsWith("#"), `expected a heading, got ${JSON.stringify(out)}`);
});

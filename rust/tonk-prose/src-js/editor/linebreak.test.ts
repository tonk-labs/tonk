import { test } from "node:test";
import assert from "node:assert/strict";
import { parseMarkdown, serializeMarkdown } from "./markdown";

/** Markdown in, markdown out — the load-edit-save cycle with no edit. */
function roundTrip(src: string): string {
  return serializeMarkdown(parseMarkdown(src));
}

test("a single newline inside a paragraph survives a round trip", () => {
  const src = "one\ntwo";
  assert.equal(
    roundTrip(src),
    src,
    "under CommonMark this is a soft break and collapses to a space",
  );
});

test("a run of line breaks survives a round trip", () => {
  const src = "one\ntwo\nthree";
  assert.equal(roundTrip(src), src);
});

test("a line break is not escaped with a backslash", () => {
  assert.ok(
    !roundTrip("one\ntwo").includes("\\"),
    "the default hard_break serializer writes `\\` before the newline",
  );
});

test("blank-line paragraph separation still splits blocks", () => {
  const src = "one\n\ntwo";
  assert.equal(
    roundTrip(src),
    src,
    "breaks must not turn a paragraph break into a line break",
  );
});

test("line breaks inside a list item survive", () => {
  const src = "- one\n  two";
  assert.equal(roundTrip(src), src);
});

test("a fenced block's newlines are untouched", () => {
  const src = "```dialog-yaml\nconcept:\n  this: id:x\n```";
  assert.equal(roundTrip(src), src);
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { Selection } from "prosemirror-state";
import { parseMarkdown } from "./markdown";

// `caretToEnd` dispatches through the view, which needs a real
// `root.getSelection()` — linkedom has none. The part worth pinning is the
// POSITION it computes, which is pure: `Selection.atEnd(doc)`.

test("the end position lands in the document's last block", () => {
  const doc = parseMarkdown("# Counter\n\nbody");
  const at = Selection.atEnd(doc);
  assert.equal(at.$from.parent.textContent, "body");
  assert.ok(at.empty, "a caret, not a range");
});

test("the end position is inside the last block, not past it", () => {
  // The end of a document is NOT `doc.content.size`: that is past the
  // final node, and setting a text selection there throws.
  const doc = parseMarkdown("# Counter\n\n");
  const at = Selection.atEnd(doc);
  assert.ok(at.from < doc.content.size, `${at.from} < ${doc.content.size}`);
});

test("a trailing blank line is not a block to land in", () => {
  // Markdown collapses trailing blank lines, so a document written with an
  // empty block at the end parses back WITHOUT it. That is why the create
  // writes one block and the caret does the rest: an empty second block
  // would simply vanish.
  const doc = parseMarkdown("# Counter\n\n\n");
  assert.equal(doc.childCount, 1, "the blank line is not a node");
  assert.equal(Selection.atEnd(doc).$from.parent.textContent, "# Counter");
});

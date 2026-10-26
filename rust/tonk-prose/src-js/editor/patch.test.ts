import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { patchMarkdown } from "./patch";

/** A state over `markdown` with the caret right after `marker`. */
function caretAfter(markdown: string, marker: string): EditorState {
  const doc = parseMarkdown(markdown);
  let at = -1;
  doc.descendants((node, pos) => {
    if (at < 0 && node.isText && node.text!.includes(marker)) {
      at = pos + node.text!.indexOf(marker) + marker.length;
    }
  });
  assert.ok(at > 0, `${marker} is in the document`);
  return EditorState.create({ doc, selection: TextSelection.create(doc, at) });
}

/** The document text on either side of the caret. */
function around(state: EditorState): [string, string] {
  const { $head } = state.selection;
  const text = $head.parent.textContent;
  return [text.slice(0, $head.parentOffset), text.slice($head.parentOffset)];
}

test("another replica's edit to the caret's paragraph leaves the caret in place", () => {
  // Inline code makes this paragraph more than plain text, which is the
  // case the block-level narrowing replaced wholesale.
  const state = caretAfter("A `cell` paragraph. Mine", "Mine");
  const tr = patchMarkdown(state, "Theirs: A `cell` paragraph. Mine");
  assert.ok(tr);
  const after = state.apply(tr!);
  assert.equal(serializeMarkdown(after.doc), "Theirs: A `cell` paragraph. Mine");
  const [before] = around(after);
  assert.ok(before.endsWith("Mine"), `the caret still follows what we typed: ${before}`);
});

test("text inserted at the caret lands after it", () => {
  const state = caretAfter("Shared `x`. A-one", "A-one");
  const after = state.apply(patchMarkdown(state, "Shared `x`. A-one B-one")!);
  const [before, rest] = around(after);
  assert.ok(before.endsWith("A-one"), before);
  assert.equal(rest, " B-one");
});

test("an edit in another block leaves the caret's block alone", () => {
  const state = caretAfter("first\n\nsecond `x` here", "here");
  const after = state.apply(patchMarkdown(state, "first, edited\n\nsecond `x` here")!);
  const [before] = around(after);
  assert.ok(before.endsWith("here"), before);
});

test("an identical value is no transaction", () => {
  const state = caretAfter("same `x` text", "text");
  assert.equal(patchMarkdown(state, serializeMarkdown(state.doc)), null);
});

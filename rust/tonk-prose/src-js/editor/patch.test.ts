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

/** A state over `markdown` with an empty paragraph opened after top-level
 *  node `index`, the caret in it: the author has just pressed Enter. */
function enterAfter(markdown: string, index: number): EditorState {
  const state = EditorState.create({ doc: parseMarkdown(markdown) });
  let pos = 0;
  for (let i = 0; i <= index; i++) pos += state.doc.child(i).nodeSize;
  const tr = state.tr.insert(pos, state.schema.nodes.paragraph.create());
  tr.setSelection(TextSelection.create(tr.doc, pos + 1));
  return state.apply(tr);
}

test("an empty paragraph the author just opened survives an edit elsewhere", () => {
  const state = enterAfter("first\n\nsecond", 1);
  const after = state.apply(patchMarkdown(state, "first, edited\n\nsecond")!);
  assert.equal(after.doc.childCount, 3, "the empty paragraph is still there");
  assert.equal(after.doc.child(2).content.size, 0);
  assert.equal(after.selection.$head.index(0), 2, "the caret is still in it");
  assert.equal(serializeMarkdown(after.doc), "first, edited\n\nsecond");
});

test("an empty paragraph stays beside its neighbour when that neighbour is edited", () => {
  const state = enterAfter("first\n\nsecond\n\nthird", 0);
  const after = state.apply(patchMarkdown(state, "first\n\nsecond\n\nthird, edited")!);
  assert.equal(after.doc.child(1).content.size, 0, "still right after `first`");
  assert.equal(after.selection.$head.index(0), 1);
});

test("an empty paragraph after a block another replica edited keeps its place", () => {
  const state = enterAfter("first\n\nsecond", 0);
  const after = state.apply(patchMarkdown(state, "first, edited\n\nsecond")!);
  assert.equal(after.doc.childCount, 3);
  assert.equal(after.doc.child(1).content.size, 0, "between the two blocks");
  assert.equal(after.selection.$head.index(0), 1);
});

test("an empty document stays one empty paragraph", () => {
  const state = EditorState.create({ doc: parseMarkdown("") });
  const tr = patchMarkdown(state, "");
  const after = tr ? state.apply(tr) : state;
  assert.equal(after.doc.childCount, state.doc.childCount);
});

test("a value arriving into an empty document replaces its placeholder", () => {
  const state = EditorState.create({ doc: parseMarkdown("") });
  const after = state.apply(patchMarkdown(state, "loaded")!);
  assert.equal(serializeMarkdown(after.doc), "loaded");
  assert.equal(after.doc.childCount, 1, "no empty paragraph left over");
});

test("a paragraph holding only the space the author just typed survives the stored value", () => {
  const opened = enterAfter("first\n\nsecond", 0);
  const state = opened.apply(opened.tr.insertText(" "));
  const tr = patchMarkdown(state, "first\n\nsecond");
  const after = tr ? state.apply(tr) : state;
  assert.equal(after.doc.childCount, 3, "the paragraph is still there");
  assert.equal(after.doc.child(1).textContent, " ");
  assert.equal(after.selection.$head.index(0), 1, "the caret is still in it");
});

test("a paragraph of only an image is not taken for a blank one", () => {
  const state = EditorState.create({ doc: parseMarkdown("first\n\n![alt](x.png)") });
  const after = state.apply(patchMarkdown(state, "first")!);
  assert.equal(serializeMarkdown(after.doc), "first");
});

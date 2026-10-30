import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import type { Node } from "prosemirror-model";
import { parseMarkdown, serializeMarkdown } from "./markdown";

/** A document of `first` and a paragraph the author typed `text` into. */
function typed(text: string): Node {
  const state = EditorState.create({ doc: parseMarkdown("first") });
  const end = state.doc.content.size;
  const tr = state.tr.insert(end, state.schema.nodes.paragraph.create());
  tr.setSelection(TextSelection.create(tr.doc, end + 1)).insertText(text);
  return tr.doc;
}

/** The nodes a document parses back to from its own markdown. */
function reparsed(doc: Node): Node {
  return parseMarkdown(serializeMarkdown(doc));
}

test("a paragraph typed after four spaces stays a paragraph", () => {
  const doc = typed("    let a = 1;");
  const back = reparsed(doc);
  assert.equal(back.child(1).type.name, "paragraph", "not an indented code block");
  assert.ok(back.eq(doc));
});

test("a paragraph typed after a tab stays a paragraph", () => {
  const doc = typed("\tlet a = 1;");
  assert.ok(reparsed(doc).eq(doc));
});

test("a paragraph keeps the spaces it was typed after", () => {
  const doc = typed("  indented");
  assert.ok(reparsed(doc).eq(doc));
});

test("a line after a break keeps the spaces it starts with", () => {
  const { schema } = parseMarkdown("").type;
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.text("first"),
      schema.node("hard_break"),
      schema.text("    second"),
    ]),
  ]);
  assert.ok(reparsed(doc).eq(doc));
});

test("a list item keeps the spaces its text starts with", () => {
  const state = EditorState.create({ doc: parseMarkdown("- &#32;   x") });
  assert.ok(reparsed(state.doc).eq(state.doc));
  assert.equal(serializeMarkdown(state.doc), "- &#32;   x");
});

test("a paragraph without leading whitespace is written as it was", () => {
  assert.equal(serializeMarkdown(typed("plain text")), "first\n\nplain text");
});

test("leading spaces in inline code are not spelled as an entity", () => {
  const doc = parseMarkdown("first\n\n`  x`");
  assert.ok(!serializeMarkdown(doc).includes("&#"));
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState } from "prosemirror-state";
import type { Node } from "prosemirror-model";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { reparse, flushReparse } from "./reparse";

/** A state whose doc is `src` materialized, with the reparse plugin. */
function stateFor(src: string): EditorState {
  return EditorState.create({ doc: parseMarkdown(src), plugins: [reparse()] });
}

/** Doc position just inside the first textblock whose text starts with
 *  `prefix`. */
function posOfLine(doc: Node, prefix: string): number {
  let found = -1;
  doc.descendants((node, pos) => {
    if (found >= 0) return false;
    if (node.isTextblock && node.textContent.startsWith(prefix)) {
      found = pos + 1;
      return false;
    }
    return true;
  });
  if (found < 0) throw new Error(`no line starting ${JSON.stringify(prefix)}`);
  return found;
}

/** Delete the leading block marker (`marker` chars) from the line that
 *  starts with `find`, run the reparse, and return the resulting
 *  markdown. `find` locates the line; `marker` is the prefix removed
 *  (e.g. find "> b" and remove "> "). */
function deletePrefix(src: string, find: string, marker: string): string {
  const state = stateFor(src);
  const from = posOfLine(state.doc, find);
  const edited = state.apply(state.tr.delete(from, from + marker.length));
  return serializeMarkdown(flushReparse(edited).doc).trim();
}

/** Insert `text` at the start of the line beginning with `at`, reparse,
 *  return markdown. */
function insertPrefix(src: string, at: string, text: string): string {
  const state = stateFor(src);
  const from = posOfLine(state.doc, at);
  const edited = state.apply(state.tr.insertText(text, from));
  return serializeMarkdown(flushReparse(edited).doc).trim();
}

test("deleting > on a middle quote line splits the quote", () => {
  assert.equal(deletePrefix("> a\n\n> b\n\n> c", "> b", "> "), "> a\n\nb\n\n> c");
});

test("deleting - [ ] on a todo lifts it out of the list", () => {
  assert.equal(deletePrefix("- [ ] a\n\n- [x] b", "- [ ] a", "- [ ] "), "a\n\n- [x] b");
});

test("deleting - on a middle bullet lifts it out", () => {
  assert.equal(deletePrefix("- a\n\n- b\n\n- c", "- b", "- "), "- a\n\nb\n\n- c");
});

test("an inline edit inside a quoted line keeps the quote", () => {
  // Delete the `*` closing an emphasis inside a quote — inline change,
  // structure unchanged.
  const state = stateFor("> a *b* c");
  const from = posOfLine(state.doc, "> a");
  // "> a *b* c" — remove the second `*` (index of the closing delimiter).
  const text = state.doc.textBetween(from, from + 9);
  const star = text.lastIndexOf("*");
  const edited = state.apply(state.tr.delete(from + star, from + star + 1));
  const md = serializeMarkdown(flushReparse(edited).doc).trim();
  // Still a quote; emphasis degraded (only one `*` left).
  assert.ok(md.startsWith(">"), `expected quote, got ${JSON.stringify(md)}`);
});

test("underscore emphasis converts (materializes to the * form)", () => {
  // `_for_` materializes to `*for*`; the guard must allow the delimiter
  // normalization instead of rejecting the swap (else it stays literal
  // and later serializes to an escaped `\_for\_`).
  const state = stateFor("x");
  const edited = state.apply(state.tr.insertText(" _for_", state.doc.content.size - 1));
  const after = flushReparse(edited);
  let hasEm = false;
  after.doc.descendants((n) => {
    if (n.isText && n.marks.some((m) => m.type.name === "em")) hasEm = true;
  });
  assert.ok(hasEm, "underscore emphasis should convert to an em mark");
});

test("a literal underscore word is not turned into emphasis", () => {
  const state = stateFor("x");
  const edited = state.apply(
    state.tr.insertText(" some_var_name", state.doc.content.size - 1),
  );
  const after = flushReparse(edited);
  let hasEm = false;
  after.doc.descendants((n) => {
    if (n.isText && n.marks.some((m) => m.type.name === "em")) hasEm = true;
  });
  assert.ok(!hasEm, "a mid-word underscore must stay literal");
});

test("typing > before a paragraph between two quotes joins them", () => {
  const md = insertPrefix("> a\n\nb\n\n> c", "b", "> ");
  // All three lines now quoted.
  assert.equal(md, "> a\n\n> b\n\n> c");
});

test("unrelated bullets are untouched when one is lifted", () => {
  assert.equal(
    deletePrefix("- a\n\n- b\n\n- c\n\n- d", "- c", "- "),
    "- a\n\n- b\n\nc\n\n- d",
  );
});

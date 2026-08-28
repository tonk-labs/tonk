import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import type { Command } from "prosemirror-state";
import type { Node } from "prosemirror-model";
import { parseMarkdown, serializeMarkdown } from "./markdown";
import { listBackspace, listEnter } from "./keymap";
import { flushReparse, reparse } from "./reparse";
import { reveal, revealKey } from "./reveal";

// Storybook journey UI-04: list editing inside a rendered space must keep
// its caret and structural list markers stable across ordinary key actions.

function stateFor(source: string): EditorState {
  return EditorState.create({
    doc: parseMarkdown(source),
    plugins: [reparse(), reveal()],
  });
}

function lines(doc: Node): string[] {
  const out: string[] = [];
  doc.descendants((node) => {
    if (node.isTextblock) {
      out.push(node.textContent);
      return false;
    }
    return true;
  });
  return out;
}

function lineEnd(doc: Node, text: string): number {
  let found: number | null = null;
  doc.descendants((node, pos) => {
    if (found !== null) return false;
    if (node.isTextblock && node.textContent === text) {
      found = pos + 1 + node.content.size;
      return false;
    }
    return true;
  });
  if (found === null) throw new Error(`no line ${JSON.stringify(text)}`);
  return found;
}

function select(state: EditorState, pos: number): EditorState {
  return state.apply(
    state.tr.setSelection(TextSelection.create(state.doc, pos)),
  );
}

function run(state: EditorState, command: Command): EditorState {
  let next: EditorState | null = null;
  assert.equal(
    command(state, (tr) => { next = state.apply(tr); }),
    true,
    "command should handle the selection",
  );
  assert.ok(next);
  return next;
}

test("Enter materializes the next ordered marker before typing", () => {
  let state = stateFor("1. one");
  state = select(state, lineEnd(state.doc, "1. one"));
  state = run(state, listEnter);

  assert.deepEqual(lines(state.doc), ["1. one", "2. "]);
  assert.equal(state.selection.$head.parent.textContent, "2. ");
  assert.equal(state.selection.$head.parentOffset, 3);

  state = state.apply(state.tr.insertText("two"));
  state = flushReparse(state);
  assert.equal(serializeMarkdown(state.doc).trim(), "1. one\n\n2. two");
  assert.equal(state.selection.$head.parent.textContent, "2. two");
});

test("Enter materializes the next bullet marker before typing", () => {
  let state = stateFor("- one");
  state = select(state, lineEnd(state.doc, "- one"));
  state = run(state, listEnter);
  assert.deepEqual(lines(state.doc), ["- one", "- "]);

  state = state.apply(state.tr.insertText("two"));
  state = flushReparse(state);
  assert.equal(serializeMarkdown(state.doc).trim(), "- one\n\n- two");
});

test("ordered lists preserve a non-one starting number on Enter", () => {
  let state = stateFor("3. three");
  state = select(state, lineEnd(state.doc, "3. three"));
  state = run(state, listEnter);
  assert.deepEqual(lines(state.doc), ["3. three", "4. "]);
});

test("splitting an ordered item renumbers following materialized markers", () => {
  let state = stateFor("1. one\n\n2. three");
  state = select(state, lineEnd(state.doc, "1. one"));
  state = run(state, listEnter);
  assert.deepEqual(lines(state.doc), ["1. one", "2. ", "3. three"]);
});

test("a second Enter exits a marker-only top-level list item", () => {
  let state = stateFor("1. one");
  state = select(state, lineEnd(state.doc, "1. one"));
  state = run(state, listEnter);
  state = run(state, listEnter);

  assert.deepEqual(
    Array.from({ length: state.doc.childCount }, (_, i) =>
      state.doc.child(i).type.name,
    ),
    ["ordered_list", "paragraph"],
  );
  assert.equal(state.selection.$head.parent.type.name, "paragraph");
  assert.equal(state.selection.$head.parent.content.size, 0);
});

test("Backspace exits a loaded marker-only top-level list item", () => {
  let state = stateFor("1. one\n\n2.");
  state = select(state, lineEnd(state.doc, "2. "));
  state = run(state, listBackspace);
  assert.deepEqual(
    Array.from({ length: state.doc.childCount }, (_, i) =>
      state.doc.child(i).type.name,
    ),
    ["ordered_list", "paragraph"],
  );
  assert.equal(state.selection.$head.parent.content.size, 0);
});

test("deleting final content leaves one stable marker, then Backspace exits", () => {
  let state = stateFor("1. one");
  state = select(state, lineEnd(state.doc, "1. one"));
  state = run(state, listEnter);
  state = state.apply(state.tr.insertText("x"));
  state = flushReparse(state);

  const head = state.selection.head;
  state = state.apply(state.tr.delete(head - 1, head));
  state = flushReparse(state);
  assert.deepEqual(lines(state.doc), ["1. one", "2. "]);

  state = run(state, listBackspace);
  assert.equal(state.selection.$head.parent.type.name, "paragraph");
  assert.equal(state.selection.$head.parent.content.size, 0);
});

test("loaded marker-only items receive a non-document caret anchor", () => {
  const state = stateFor("1. one\n\n2.");
  assert.deepEqual(lines(state.doc), ["1. one", "2. "]);
  const anchors = revealKey
    .getState(state)!
    .find(undefined, undefined, (spec) => spec.emptyMarkerCaret === true);
  assert.equal(anchors.length, 1);
  assert.equal(anchors[0].from, lineEnd(state.doc, "2. "));
  assert.equal(serializeMarkdown(state.doc).trim(), "1. one\n\n2.");
});

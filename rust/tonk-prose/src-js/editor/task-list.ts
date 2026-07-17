// Rendered checkboxes for GFM task-list items.
//
// A task item stores its state as literal source text — `[ ] ` or
// `[x] ` under a `markup` block marker at the start of the list
// item's line (markup.ts `markTaskPrefix`). This plugin draws the
// checkbox itself: one widget decoration placed right after that
// prefix, reflecting the checked state. At rest the `[ ] ` source is
// hidden (like every marker) and only the checkbox shows; with the
// caret in the line the source reveals for editing, exactly like the
// heading `# ` and blockquote `> ` prefixes.
//
// Clicking the checkbox flips the source character (`[ ]` ↔ `[x]`)
// in place. The edit rides the normal reparse loop — the block's
// text changes, re-materializes, and re-serializes — so the checkbox
// is never a source of truth of its own.

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { EditorView } from "prosemirror-view";
import type { Node } from "prosemirror-model";
import { isMarkup } from "./markup";

// prosemirror-view resolves a widget's DOM lazily and hands it the
// live view, so the checkbox can dispatch without the plugin owning
// a view instance — the decoration set is derived purely from the
// document, exactly like the image-preview plugin.

export const taskListKey = new PluginKey<DecorationSet>("tonk-prose-task-list");

/** A task-list prefix marker (`[ ] ` / `[x] `), if `node` is one.
 *  Returns the checked state and the position of the state character
 *  (the space or `x` inside the brackets) relative to the node start. */
function taskPrefix(node: Node): { checked: boolean; stateOffset: number } | null {
  if (!node.isText || !isMarkup(node) || !node.text) return null;
  const match = /^\[([ xX])\] $/.exec(node.text);
  if (!match) return null;
  return { checked: match[1] !== " ", stateOffset: 1 };
}

/** Build the checkbox element. `pos` is the doc position of the state
 *  character (inside the brackets), captured so the click handler can
 *  replace exactly that one character. */
function makeCheckbox(checked: boolean, statePos: number) {
  return (view: EditorView): HTMLElement => {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "md-task-checkbox";
    box.checked = checked;
    // The document owns the state; the input is a view of it. Prevent
    // the native toggle and drive the change through a transaction so
    // undo/serialize/reparse all see it.
    box.addEventListener("mousedown", (event) => event.preventDefault());
    box.addEventListener("click", (event) => {
      event.preventDefault();
      const { state, dispatch } = view;
      const ch = state.doc.textBetween(statePos, statePos + 1);
      const next = ch === " " ? "x" : " ";
      dispatch(state.tr.insertText(next, statePos, statePos + 1));
      view.focus();
    });
    return box;
  };
}

function computeCheckboxes(doc: Node): DecorationSet {
  const decorations: Decoration[] = [];
  doc.descendants((node, pos) => {
    const task = taskPrefix(node);
    if (!task) return true;
    const statePos = pos + task.stateOffset;
    decorations.push(
      Decoration.widget(pos, makeCheckbox(task.checked, statePos), {
        // side: -1 → the checkbox renders before the (hidden) `[ ] `
        // source, so a caret at the line start lands after it.
        side: -1,
        key: `task:${node.text}:${statePos}`,
      }),
    );
    return true;
  });
  return DecorationSet.create(doc, decorations);
}

export function taskList(): Plugin<DecorationSet> {
  return new Plugin<DecorationSet>({
    key: taskListKey,
    state: {
      init: (_config, state) => computeCheckboxes(state.doc),
      apply(tr, prev) {
        return tr.docChanged ? computeCheckboxes(tr.doc) : prev;
      },
    },
    props: {
      decorations(state) {
        return taskListKey.getState(state);
      },
    },
  });
}

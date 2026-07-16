// Key bindings. Deliberately thin: inline markdown conversions come
// from the reparse loop, so the classic Mod-B/Mod-I bindings don't
// toggle marks — they wrap the selection in *literal markers*, and
// the loop converts the result exactly as if the user had typed the
// characters. One code path for "text became styled", regardless of
// how the markers got there.

import { keymap } from "prosemirror-keymap";
import {
  baseKeymap,
  chainCommands,
  exitCode,
  joinUp,
  joinDown,
  lift,
  selectParentNode,
} from "prosemirror-commands";
import { undo, redo } from "prosemirror-history";
import { undoInputRule } from "prosemirror-inputrules";
import {
  splitListItem,
  liftListItem,
  sinkListItem,
} from "prosemirror-schema-list";
import { TextSelection } from "prosemirror-state";
import type { Command } from "prosemirror-state";
import type { Plugin } from "prosemirror-state";
import { schema } from "./schema";

/** Enter at the end of a textblock reading "```" or "```lang"
 *  converts it to a code block — the path the fence input rule
 *  can't cover, because input rules never see Enter. */
const codeFenceEnter: Command = (state, dispatch) => {
  const { $from, empty } = state.selection;
  if (!empty || !$from.parent.isTextblock || $from.parent.type.spec.code) {
    return false;
  }
  if ($from.parentOffset !== $from.parent.content.size) return false;
  const match = /^```([a-zA-Z][\w+#-]*)?$/.exec($from.parent.textContent);
  if (!match) return false;
  const index = $from.index(-1);
  const codeBlock = schema.nodes.code_block;
  if (!$from.node(-1).canReplaceWith(index, index + 1, codeBlock)) {
    return false;
  }
  if (dispatch) {
    const pos = $from.before();
    const tr = state.tr.replaceWith(
      pos,
      pos + $from.parent.nodeSize,
      codeBlock.create(match[1] ? { params: match[1] } : null),
    );
    tr.setSelection(TextSelection.create(tr.doc, pos + 1));
    dispatch(tr.scrollIntoView());
  }
  return true;
};

/** Wrap the selection in literal markdown delimiters. With a caret
 *  (empty selection) inserts the pair and parks the caret between
 *  them; with a range, fences the range. The reparse loop picks the
 *  text up and applies the real mark ~120ms later. */
function wrapMarker(delim: string): Command {
  return (state, dispatch) => {
    const { from, to, empty } = state.selection;
    // Only meaningful inside a plain textblock (not code).
    const $from = state.selection.$from;
    if (!$from.parent.isTextblock || $from.parent.type.spec.code) return false;
    if (dispatch) {
      const tr = state.tr;
      if (empty) {
        tr.insertText(delim + delim, from);
        tr.setSelection(TextSelection.create(tr.doc, from + delim.length));
      } else {
        tr.insertText(delim, to);
        tr.insertText(delim, from);
        tr.setSelection(
          TextSelection.create(
            tr.doc,
            from + delim.length,
            to + delim.length,
          ),
        );
      }
      dispatch(tr.scrollIntoView());
    }
    return true;
  };
}

export function buildKeymap(): Plugin {
  return keymap({
    "Mod-z": undo,
    "Shift-Mod-z": redo,
    "Mod-y": redo,
    // Undo an input-rule conversion first (one Backspace restores
    // the literal `> ` / `- ` / "```" text), then normal backspace.
    Backspace: chainCommands(undoInputRule, baseKeymap.Backspace),
    Enter: chainCommands(
      codeFenceEnter,
      splitListItem(schema.nodes.list_item),
      baseKeymap.Enter,
    ),
    "Mod-Enter": exitCode,
    "Shift-Enter": exitCode,
    Tab: sinkListItem(schema.nodes.list_item),
    "Shift-Tab": liftListItem(schema.nodes.list_item),
    "Mod-b": wrapMarker("**"),
    "Mod-i": wrapMarker("*"),
    "Mod-`": wrapMarker("`"),
    "Alt-ArrowUp": joinUp,
    "Alt-ArrowDown": joinDown,
    "Mod-BracketLeft": lift,
    Escape: selectParentNode,
  });
}

export { baseKeymap };

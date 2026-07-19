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
import type { Command, EditorState } from "prosemirror-state";
import type { Plugin } from "prosemirror-state";
import { liftTarget } from "prosemirror-transform";
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

/** How deep in blockquotes the caret sits, and the `> ` prefix text a
 *  new line at that depth needs. `depth` 0 means "not in a quote". */
function quoteContext(state: EditorState): {
  depth: number;
  prefix: string;
} {
  const { $head } = state.selection;
  let depth = 0;
  for (let d = $head.depth; d >= 0; d--) {
    if ($head.node(d).type === schema.nodes.blockquote) depth++;
  }
  return { depth, prefix: "> ".repeat(depth) };
}

/** A `> ` (or `> > ` …) leading block marker node for a new quote line. */
function quoteMarker(prefix: string) {
  return schema.text(prefix, [
    schema.marks.markup.create({ kind: "block", of: [] }),
  ]);
}

/** True when the caret's textblock is an EMPTY quote line — its only
 *  content is the `> ` prefix marker(s). A second Enter here exits the
 *  quote instead of adding another quoted line. */
function onEmptyQuoteLine(
  state: EditorState,
  prefix: string,
): boolean {
  const { $head } = state.selection;
  return $head.parent.isTextblock && $head.parent.textContent === prefix;
}

/** Split the current quote line and seed the new paragraph with a `> `
 *  marker, parking the caret right after it — the "continue the quote"
 *  action shared by Enter and Shift+Enter. */
function continueQuote(prefix: string): Command {
  return (state, dispatch) => {
    const { $head } = state.selection;
    if (dispatch) {
      const tr = state.tr;
      tr.split($head.pos, 1);
      const insertAt = tr.mapping.map($head.pos);
      const marker = quoteMarker(prefix);
      tr.insert(insertAt, marker);
      tr.setSelection(TextSelection.create(tr.doc, insertAt + marker.nodeSize));
      dispatch(tr.scrollIntoView());
    }
    return true;
  };
}

/** Exit the blockquote from an empty quote line: remove the `> ` marker
 *  and lift the paragraph out of the innermost blockquote. */
const exitQuote: Command = (state, dispatch) => {
  const { prefix } = quoteContext(state);
  const { $head } = state.selection;
  const start = $head.start();
  if (dispatch) {
    const tr = state.tr;
    // Drop the `> ` prefix so the lifted paragraph is plain.
    tr.delete(start, start + prefix.length);
    const range = tr.selection.$from.blockRange();
    if (range) {
      const target = liftTarget(range);
      if (target !== null) tr.lift(range, target);
    }
    dispatch(tr.scrollIntoView());
  }
  return true;
};

/** Enter inside a blockquote. Continues the quote: splits the line and
 *  seeds the new paragraph with a `> ` marker (so it reads `> |`, and
 *  typing lands after the marker). A second Enter on an already-empty
 *  quote line (`> |`) EXITS the quote instead — it lifts that empty line
 *  out to a plain paragraph, so double-Enter ends the quote like every
 *  editor. Declines (returns false) outside a blockquote so the default
 *  Enter runs. */
const blockquoteEnter: Command = (state, dispatch) => {
  const { depth, prefix } = quoteContext(state);
  if (depth === 0) return false;
  const { $head, empty } = state.selection;
  if (!empty || !$head.parent.isTextblock) return false;
  if (onEmptyQuoteLine(state, prefix)) return exitQuote(state, dispatch);
  return continueQuote(prefix)(state, dispatch);
};

/** Shift+Enter inside a blockquote: always continue the quote (add a new
 *  `> ` line), never exit. Outside a quote, declines so the default
 *  (`exitCode`, the code-block soft break) runs. */
const quoteShiftEnter: Command = (state, dispatch) => {
  const { depth, prefix } = quoteContext(state);
  if (depth === 0) return false;
  const { $head, empty } = state.selection;
  if (!empty || !$head.parent.isTextblock) return false;
  return continueQuote(prefix)(state, dispatch);
};

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
      blockquoteEnter,
      splitListItem(schema.nodes.list_item),
      baseKeymap.Enter,
    ),
    "Mod-Enter": exitCode,
    "Shift-Enter": chainCommands(quoteShiftEnter, exitCode),
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

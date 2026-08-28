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
import { Selection, TextSelection } from "prosemirror-state";
import type {
  Command,
  EditorState,
  Transaction,
} from "prosemirror-state";
import type { Plugin } from "prosemirror-state";
import type { ResolvedPos } from "prosemirror-model";
import { liftTarget } from "prosemirror-transform";
import { schema } from "./schema";
import {
  demarkupDoc,
  materializeDoc,
} from "./markup";
import {
  isBlockMarkerOnly,
  leadingBlockMarkerSize,
} from "./block-markers";

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

/** Markdown quote prefix implied by `$pos`'s structural ancestors. */
function quotePrefixAt($pos: ResolvedPos): string {
  let depth = 0;
  for (let d = $pos.depth; d >= 0; d--) {
    if ($pos.node(d).type === schema.nodes.blockquote) depth++;
  }
  return "> ".repeat(depth);
}

/** Closest list around `$pos`. */
function listDepthAt($pos: ResolvedPos): number {
  for (let d = $pos.depth - 1; d >= 1; d--) {
    const node = $pos.node(d);
    if (
      node.type === schema.nodes.bullet_list ||
      node.type === schema.nodes.ordered_list
    ) {
      return d;
    }
  }
  return -1;
}

/** Position after the materialized prefix in `itemIndex`'s first
 *  textblock. `listFrom` points immediately before `list`. */
function itemContentStart(
  listFrom: number,
  list: import("prosemirror-model").Node,
  itemIndex: number,
): number | null {
  if (itemIndex < 0 || itemIndex >= list.childCount) return null;
  let itemOffset = 0;
  for (let i = 0; i < itemIndex; i++) itemOffset += list.child(i).nodeSize;
  const item = list.child(itemIndex);
  const itemFrom = listFrom + 1 + itemOffset;
  let target: number | null = null;
  item.descendants((node, offset) => {
    if (target !== null) return false;
    if (node.isTextblock) {
      target = itemFrom + 1 + offset + 1 + leadingBlockMarkerSize(node);
      return false;
    }
    return true;
  });
  return target;
}

/** `splitListItem` changes the structural list but knows nothing about
 *  Tonk-Prose's literal source markers. Re-materialize the closest list in
 *  the SAME transaction, giving the new item its `- `/`N. ` prefix and
 *  renumbering later ordered items before the view paints an inconsistent
 *  state. */
function materializeSplitList(tr: Transaction): boolean {
  const $head = tr.selection.$head;
  const listDepth = listDepthAt($head);
  if (listDepth < 1) return false;
  const list = $head.node(listDepth);
  const itemIndex = $head.index(listDepth);
  const listFrom = $head.before(listDepth);
  const rebuilt = materializeDoc(demarkupDoc(list), quotePrefixAt($head));
  tr.replaceWith(listFrom, listFrom + list.nodeSize, rebuilt);
  const target = itemContentStart(listFrom, rebuilt, itemIndex);
  if (target !== null) {
    tr.setSelection(TextSelection.create(tr.doc, target));
  }
  return true;
}

/** Append `after` (created against `base.doc`) to `base`, preserving the
 *  command's final selection while keeping the user action one transaction
 *  and therefore one undo step/change event. */
function appendTransaction(base: Transaction, after: Transaction): void {
  for (const step of after.steps) base.step(step);
  base.setSelection(Selection.fromJSON(base.doc, after.selection.toJSON()));
  if (after.storedMarksSet) base.setStoredMarks(after.storedMarks);
  if (after.scrolledIntoView) base.scrollIntoView();
}

const nativeListEnter = splitListItem(schema.nodes.list_item);
const nativeEmptyListEnter = chainCommands(
  nativeListEnter,
  baseKeymap.Enter,
);

/** Run a native command against a temporary state where a marker-only list
 *  paragraph is genuinely empty, then replay its steps into the marker
 *  deletion transaction. */
function runMarkerOnlyListCommand(
  state: EditorState,
  dispatch: ((tr: Transaction) => void) | undefined,
  command: Command,
): boolean {
  const { $from, $to, empty } = state.selection;
  const inListItem =
    empty &&
    $from.sameParent($to) &&
    $from.depth >= 2 &&
    $from.node(-1).type === schema.nodes.list_item;
  if (!inListItem || !isBlockMarkerOnly($from.parent)) return false;

  const start = $from.start();
  const clean = state.tr.delete(start, start + $from.parent.content.size);
  clean.setSelection(TextSelection.create(clean.doc, start));
  const cleanState = state.apply(clean);
  if (!dispatch) return command(cleanState);
  let after: Transaction | null = null;
  if (!command(cleanState, (tr) => { after = tr; }) || !after) return false;
  appendTransaction(clean, after);
  // A nested empty item may have outdented into its parent list. Restore
  // that list's source markers too; at top level there is no surrounding
  // list and this is intentionally a no-op.
  materializeSplitList(clean);
  dispatch(clean);
  return true;
}

/** Enter in a list, adapted to the materialized-marker invariant.
 *
 *  - A normal split is immediately re-materialized, so the new item has
 *    its bullet/number before any typing or debounced reparse can occur.
 *  - A marker-only item is semantically empty. Its hidden source marker is
 *    removed in-memory, native empty-item behavior runs, and both transforms
 *    are combined into one transaction. Thus a second Enter exits a
 *    top-level list (or outdents a nested one) instead of minting endless
 *    marker-only items. */
export const listEnter: Command = (state, dispatch) => {
  if (runMarkerOnlyListCommand(state, dispatch, nativeEmptyListEnter)) {
    return true;
  }

  if (!dispatch) return nativeListEnter(state);
  return nativeListEnter(state, (tr) => {
    materializeSplitList(tr);
    dispatch(tr);
  });
};

/** Backspace on a marker-only item follows structural empty-list behavior
 *  instead of deleting one hidden source character at a time. */
export const listBackspace: Command = (state, dispatch) =>
  runMarkerOnlyListCommand(
    state,
    dispatch,
    liftListItem(schema.nodes.list_item),
  );

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
    Backspace: chainCommands(
      undoInputRule,
      listBackspace,
      baseKeymap.Backspace,
    ),
    Enter: chainCommands(
      codeFenceEnter,
      blockquoteEnter,
      listEnter,
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

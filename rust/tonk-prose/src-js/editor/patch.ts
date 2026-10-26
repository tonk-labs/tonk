// Applying an out-of-band markdown value (a store round-trip, another
// replica's edit) to a live document without disturbing the author.

import { TextSelection } from "prosemirror-state";
import type { EditorState, Transaction } from "prosemirror-state";
import { parseMarkdown } from "./markdown";

/** The transaction that turns `state`'s document into `markdown`, or
 *  null when they already match.
 *
 *  Replaces only what differs, down to the character, whatever the
 *  blocks contain. Narrowing by whole top-level blocks (with a text diff
 *  only for blocks of plain text) replaced any paragraph holding inline
 *  code, a link or emphasis wholesale whenever another replica edited
 *  it, and the author's caret in that paragraph was mapped to its edge:
 *  the next keystrokes landed somewhere else. */
export function patchMarkdown(
  state: EditorState,
  markdown: string,
): Transaction | null {
  const next = parseMarkdown(markdown);
  const current = state.doc;
  const start = current.content.findDiffStart(next.content);
  if (start === null) return null;
  const end = current.content.findDiffEnd(next.content);
  if (!end) return null;
  let { a: endA, b: endB } = end;
  // The two scans can cross where the change repeats its context
  // (typing "aa" after "a"); push both ends past the start.
  const overlap = start - Math.min(endA, endB);
  if (overlap > 0) {
    endA += overlap;
    endB += overlap;
  }
  const tr = state.tr.replace(start, endA, next.slice(start, endB));

  // A caret at the edge of the change stays on its own side (`-1`), so
  // text another replica added where the author is typing lands after
  // the caret instead of carrying it along. A selection inside a code
  // block is left to ProseMirror's own mapping: the embedded editor owns
  // that caret, and resetting it would move it.
  const selection = state.selection;
  if (
    selection instanceof TextSelection &&
    !selection.$head.parent.type.spec.code
  ) {
    const anchor = tr.mapping.map(selection.anchor, -1);
    const head = tr.mapping.map(selection.head, -1);
    tr.setSelection(
      TextSelection.between(tr.doc.resolve(anchor), tr.doc.resolve(head)),
    );
  }
  return tr;
}

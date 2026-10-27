// Applying an out-of-band markdown value (a store round-trip, another
// replica's edit) to a live document without disturbing the author.

import { TextSelection } from "prosemirror-state";
import type { EditorState, Transaction } from "prosemirror-state";
import { Fragment } from "prosemirror-model";
import type { Node } from "prosemirror-model";
import { parseMarkdown } from "./markdown";

/** Whether a top-level node is a blank paragraph: empty, or holding
 *  only spaces and tabs. Markdown has no spelling for one (a blank line
 *  separates blocks), so it only ever exists in the live document: the
 *  author pressed Enter and has not typed yet, or has typed a space. */
function isEmptyParagraph(node: Node): boolean {
  if (node.type.name !== "paragraph") return false;
  let blank = true;
  node.forEach((child) => {
    if (!child.isText || !/^[ \t]*$/.test(child.text ?? "")) blank = false;
  });
  return blank;
}

/** `next` with `current`'s empty top-level paragraphs carried into it.
 *
 *  `next` is parsed from markdown, so it never has one, and diffing
 *  against it read the paragraph the author had just opened as deleted:
 *  any outside value (another replica's edit, a store round-trip) took it
 *  out, and what they typed next joined the block above. Each run of
 *  empty paragraphs goes back after the node it followed, found by
 *  content, else before the node it preceded, else where it was. */
function keepEmptyParagraphs(current: Node, next: Node): Node {
  const nodes: Node[] = [];
  next.forEach((node) => nodes.push(node));
  const runs: { before: Node | null; after: Node | null; at: number; run: Node[] }[] = [];
  let kept = 0;
  for (let i = 0; i < current.childCount; i++) {
    const node = current.child(i);
    if (!isEmptyParagraph(node)) {
      kept++;
      continue;
    }
    const last = runs[runs.length - 1];
    if (last && last.at === kept && last.after === null) {
      last.run.push(node);
    } else {
      runs.push({ before: i > 0 ? current.child(i - 1) : null, after: null, at: kept, run: [node] });
    }
    const following = i + 1 < current.childCount ? current.child(i + 1) : null;
    if (following && !isEmptyParagraph(following)) runs[runs.length - 1].after = following;
  }
  // A document of nothing but empty paragraphs is the editor's
  // placeholder for an empty value, not something the author opened.
  if (runs.length === 0 || kept === 0) return next;

  // Insert from the end so earlier indices stay valid; `at` is where the
  // run sat among the non-empty nodes, which is where `next` most likely
  // still has its neighbours.
  for (const { before, after, at, run } of runs.reverse()) {
    const near = (target: Node | null): number => {
      if (!target) return -1;
      let best = -1;
      nodes.forEach((node, index) => {
        if (node.eq(target) && (best < 0 || Math.abs(index - at) < Math.abs(best - at))) best = index;
      });
      return best;
    };
    let index: number;
    if (!before) index = 0;
    else if (near(before) >= 0) index = near(before) + 1;
    else if (near(after) >= 0) index = near(after);
    else index = Math.min(at, nodes.length);
    nodes.splice(index, 0, ...run);
  }
  return next.copy(Fragment.fromArray(nodes));
}

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
  const current = state.doc;
  const next = keepEmptyParagraphs(current, parseMarkdown(markdown));
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

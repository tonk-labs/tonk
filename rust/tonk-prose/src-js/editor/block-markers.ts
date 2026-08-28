// Inspection helpers for materialized block-source markers. Kept separate
// from markup.ts because that file deliberately contains a literal NUL in a
// mark identity key, which makes Git classify edits to it as binary.

import type { Node } from "prosemirror-model";
import { schema } from "./schema";

const markupType = schema.marks.markup;

/** True when `node` is literal BLOCK syntax (`# `, `> `, `- `, `N. `),
 *  as opposed to an inline delimiter such as `**`. */
export function isBlockMarkup(node: Node): boolean {
  const mark = node.isText ? markupType.isInSet(node.marks) : null;
  return Boolean(mark && mark.attrs.kind === "block");
}

/** A textblock whose entire content is block syntax. Semantically this is
 *  empty: the common case is an empty list item containing only `- ` or
 *  `2. `. ProseMirror must treat it like an empty block for Enter/caret
 *  behavior even though the materialized editor document contains text. */
export function isBlockMarkerOnly(node: Node): boolean {
  if (!node.isTextblock || node.childCount === 0) return false;
  let onlyMarkers = true;
  node.content.forEach((child) => {
    if (!isBlockMarkup(child)) onlyMarkers = false;
  });
  return onlyMarkers;
}

/** Size of the leading run of materialized block syntax in `node`. */
export function leadingBlockMarkerSize(node: Node): number {
  let size = 0;
  let done = false;
  node.content.forEach((child) => {
    if (done) return;
    if (isBlockMarkup(child)) size += child.nodeSize;
    else done = true;
  });
  return size;
}

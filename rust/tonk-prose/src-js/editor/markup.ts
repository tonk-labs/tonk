// Marker materialization — the allusion technique, modernized.
//
// Markdown syntax characters live in the document as literal text
// nodes carrying the `markup` mark:
//
//   heading      "# " …            (block prefix)
//   strong       "**" … "**"
//   em           "*" … "*"
//   code         "`" … "`"
//   link         "[" … "](url)"    (closing marker carries the href)
//
// Two consequences fall out of "markers are text":
//
//   1. A textblock's `textContent` IS its markdown source. The
//      reparse loop (reparse.ts) exploits this: it parses the text
//      the user actually sees and swaps the block when its structure
//      changed — that's how `**bold**` becomes bold with no inline
//      input rules, and deleting one `*` degrades it back.
//
//   2. Caret positions survive structural swaps when tracked as
//      text offsets, because the swap preserves the text exactly.
//
// `materialize*` adds markers to clean (parser-produced) nodes;
// `demarkup*` strips them for serialization. Both are pure.

import { Fragment, Mark, Node } from "prosemirror-model";
import { schema } from "./schema";

const markupType = schema.marks.markup;

/** True when `node` is a marker text node. */
export function isMarkup(node: Node): boolean {
  return node.isText && Boolean(markupType.isInSet(node.marks));
}

function marker(text: string): Node {
  return schema.text(text, [markupType.create()]);
}

/** Opening delimiter for a content mark, in source syntax. */
function openDelim(mark: Mark): string {
  switch (mark.type.name) {
    case "strong":
      return "**";
    case "em":
      return "*";
    case "code":
      return "`";
    case "link":
      return "[";
    default:
      return "";
  }
}

/** Closing delimiter for a content mark. The link's closing marker
 *  carries the destination — revealing it makes the href editable
 *  as text, and the reparse loop folds edits back into the mark. */
function closeDelim(mark: Mark): string {
  switch (mark.type.name) {
    case "strong":
      return "**";
    case "em":
      return "*";
    case "code":
      return "`";
    case "link": {
      const href = (mark.attrs.href as string) ?? "";
      const title = mark.attrs.title as string | null;
      return title ? `](${href} "${title}")` : `](${href})`;
    }
    default:
      return "";
  }
}

/** Content marks (delimiter-bearing), in the order they appear in
 *  `marks` — ProseMirror keeps mark arrays sorted by schema rank,
 *  so this order is stable across nodes and transitions nest
 *  properly for parser-produced fragments. */
function delimited(marks: readonly Mark[]): Mark[] {
  return marks.filter((m) => openDelim(m) !== "");
}

/** Materialize inline markers for a textblock's content.
 *
 *  Walks the children tracking a *stack* of open delimiter-bearing
 *  marks. A node's `marks` array is ordered by schema rank, not by
 *  nesting, so the stack is compared as a set with stack discipline:
 *  at each boundary we close every mark from the top of the stack
 *  down to the deepest mark that ended (re-opening survivors), then
 *  open the marks that begin. Parser output is properly nested, so
 *  re-opens are rare, but the algorithm stays correct either way. */
export function materializeInline(content: Fragment): Fragment {
  const out: Node[] = [];
  const stack: Mark[] = [];

  content.forEach((child) => {
    const marks = delimited(child.marks);
    const has = (m: Mark) => marks.some((x) => x.eq(m));

    // Deepest stack entry that survives into this child.
    let keep = 0;
    while (keep < stack.length && has(stack[keep])) keep++;

    // Close from the top down to `keep`; remember which of the
    // closed marks continue (they were merely trapped above a
    // closing one) and re-open them.
    const reopen: Mark[] = [];
    while (stack.length > keep) {
      const mark = stack.pop()!;
      out.push(marker(closeDelim(mark)));
      if (has(mark)) reopen.unshift(mark);
    }
    for (const mark of reopen) {
      out.push(marker(openDelim(mark)));
      stack.push(mark);
    }

    // Open marks that begin at this child.
    for (const mark of marks) {
      if (!stack.some((x) => x.eq(mark))) {
        out.push(marker(openDelim(mark)));
        stack.push(mark);
      }
    }

    out.push(child);
  });

  while (stack.length > 0) {
    out.push(marker(closeDelim(stack.pop()!)));
  }

  return Fragment.from(out);
}

/** Block-prefix marker for a textblock, if any. */
function blockPrefix(node: Node): string | null {
  if (node.type === schema.nodes.heading) {
    return `${"#".repeat(node.attrs.level as number)} `;
  }
  return null;
}

/** Materialize one textblock: block prefix + inline markers.
 *  Non-textblocks and code blocks pass through untouched (code
 *  block content is code, not markdown). */
export function materializeBlock(node: Node): Node {
  if (!node.isTextblock || node.type.spec.code) return node;
  let content = materializeInline(node.content);
  const prefix = blockPrefix(node);
  if (prefix) {
    content = Fragment.from(marker(prefix)).append(content);
  }
  return node.copy(content);
}

/** Materialize every textblock in a document (or subtree). */
export function materializeDoc(node: Node): Node {
  if (node.isTextblock) return materializeBlock(node);
  if (node.isLeaf) return node;
  const children: Node[] = [];
  node.content.forEach((child) => children.push(materializeDoc(child)));
  return node.copy(Fragment.from(children));
}

/** Strip marker text from a textblock's inline content. */
function demarkupInline(content: Fragment): Fragment {
  const out: Node[] = [];
  content.forEach((child) => {
    if (!isMarkup(child)) out.push(child);
  });
  return Fragment.from(out);
}

/** Strip all markers from a document, producing the clean doc the
 *  markdown serializer understands. */
export function demarkupDoc(node: Node): Node {
  if (node.isTextblock) return node.copy(demarkupInline(node.content));
  if (node.isLeaf) return node;
  const children: Node[] = [];
  node.content.forEach((child) => children.push(demarkupDoc(child)));
  return node.copy(Fragment.from(children));
}

/** A textblock is loop-eligible when its content is pure text —
 *  inline leaves (images, hard breaks) have no faithful text form,
 *  so `textContent` would lie about the source and a reparse could
 *  destroy them. Those blocks keep their structure; only explicit
 *  edits change them. */
export function isPlainTextblock(node: Node): boolean {
  if (!node.isTextblock || node.type.spec.code) return false;
  let plain = true;
  node.content.forEach((child) => {
    if (!child.isText) plain = false;
  });
  return plain;
}

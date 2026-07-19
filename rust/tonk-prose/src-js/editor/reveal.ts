// Selection-driven marker reveal (allusion's decoration trick, both
// halves of it).
//
// Two decorations, recomputed per selection change:
//
//  1. A node decoration tagging the caret's textblock `md-active`.
//     CSS reveals *block* markers (the heading `# ` prefix) inside
//     it — block syntax belongs to the whole line, so it shows
//     whenever you're on the line.
//
//  2. An inline decoration tagging the caret's *edit range*
//     `md-edit`. The edit range is the contiguous run of content
//     sharing marks with the caret's position — computed via
//     `effectiveMarks`, which resolves a bare `**` marker to the
//     strong span it belongs to (the `of` attr, allusion's `marks`).
//     CSS reveals inline markers only inside this range: the caret
//     in bold text shows that span's `**`; the caret in a link
//     shows its `[` and `](url)`; everything else stays rendered.
//
// Visibility itself is pure CSS keyed off these two decorations —
// cursor movement never mutates the document.

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { EditorState } from "prosemirror-state";
import type { Node, ResolvedPos } from "prosemirror-model";
import { schema } from "./schema";
import { effectiveMarks } from "./markup";

export const revealKey = new PluginKey<DecorationSet>("tonk-prose-reveal");

/** The edit range around `$head`, in absolute positions — the
 *  contiguous neighbor run whose effective marks intersect the
 *  marks at the caret. Null when the caret touches no marked
 *  content (plain text, empty block, non-textblock selection). */
function findEditRange($head: ResolvedPos): { from: number; to: number } | null {
  const parent = $head.parent;
  const offset = $head.parentOffset;

  // Mark keys "at" the caret: union over the child before and after
  // it (a caret between two children belongs to both sides).
  //
  // A marker anchors its span SYMMETRICALLY — from either side. The
  // caret adjacent to a span (just past its closing `**`, or just
  // before its opening `**`) reveals that span's markers, so the
  // syntax appears as the caret approaches rather than only once
  // inside. That is what makes the control characters read as if
  // always visible: `text |**bold**` and `**bold**| text` both show
  // the `**`, and because the markers are then real visible text the
  // browser's own caret movement and typing land at the boundary with
  // no hidden node to skip over.
  const before = parent.childBefore(offset);
  const after = parent.childAfter(offset);
  const anchor = new Set<string>();
  const addKeys = (keys: readonly string[]) => {
    for (const key of keys) anchor.add(key);
  };
  if (before.node) addKeys(effectiveMarks(before.node));
  if (after.node) addKeys(effectiveMarks(after.node));
  if (anchor.size === 0) return null;

  const intersects = (node: Node) =>
    effectiveMarks(node).some((key) => anchor.has(key));

  // Child index the expansion starts from.
  const startIndex = before.node ? before.index : after.index;

  // Child offsets within the parent, for position arithmetic.
  const children: { node: Node; offset: number }[] = [];
  parent.content.forEach((node, childOffset) =>
    children.push({ node, offset: childOffset }),
  );
  if (!children[startIndex]) return null;

  let first = startIndex;
  while (first > 0 && intersects(children[first - 1].node)) first--;
  let last = startIndex;
  // The caret can sit at the boundary where `after` continues the
  // run `before` ends; make sure the expansion covers both sides.
  while (last + 1 < children.length && intersects(children[last + 1].node)) {
    last++;
  }

  const base = $head.start();
  return {
    from: base + children[first].offset,
    to: base + children[last].offset + children[last].node.nodeSize,
  };
}

function computeDecorations(state: EditorState): DecorationSet {
  const { $head } = state.selection;
  if (!$head.parent.isTextblock) return DecorationSet.empty;

  const decorations: Decoration[] = [];
  const blockStart = $head.before();
  decorations.push(
    Decoration.node(blockStart, blockStart + $head.parent.nodeSize, {
      class: "md-active",
    }),
  );

  // A blockquote reveals ALL its `> ` markers when the caret is anywhere
  // inside it — the whole quote reads as its markdown while you edit it,
  // not just the one line the caret sits on. Find the OUTERMOST enclosing
  // blockquote and mark every textblock in it `md-active` (the class that
  // reveals block markers), so all their `> ` prefixes show at once.
  let quoteDepth = -1;
  for (let d = $head.depth - 1; d >= 0; d--) {
    if ($head.node(d).type === schema.nodes.blockquote) quoteDepth = d;
  }
  if (quoteDepth >= 0) {
    const quote = $head.node(quoteDepth);
    const quoteStart = $head.before(quoteDepth);
    quote.descendants((node, offset) => {
      if (node.isTextblock) {
        const from = quoteStart + 1 + offset;
        decorations.push(
          Decoration.node(from, from + node.nodeSize, { class: "md-active" }),
        );
        return false;
      }
      return true;
    });
  }

  if (!$head.parent.type.spec.code) {
    const range = findEditRange($head);
    if (range && range.from < range.to) {
      decorations.push(
        Decoration.inline(
          range.from,
          range.to,
          { class: "md-edit" },
          // Text typed at the range edges shouldn't inherit the
          // decoration — it gets its own on the next recompute.
          { inclusiveStart: false, inclusiveEnd: false },
        ),
      );
    }
  }

  return DecorationSet.create(state.doc, decorations);
}

export function reveal(): Plugin<DecorationSet> {
  return new Plugin<DecorationSet>({
    key: revealKey,
    state: {
      init: (_config, state) => computeDecorations(state),
      apply(tr, prev, _old, state) {
        if (!tr.docChanged && !tr.selectionSet) return prev;
        return computeDecorations(state);
      },
    },
    props: {
      decorations(state) {
        return revealKey.getState(state);
      },
    },
  });
}

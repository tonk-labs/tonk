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
  const before = parent.childBefore(offset);
  const after = parent.childAfter(offset);
  const anchor = new Set<string>();
  const addAll = (node: Node | null) => {
    if (!node) return;
    for (const key of effectiveMarks(node)) anchor.add(key);
  };
  addAll(before.node);
  addAll(after.node);
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

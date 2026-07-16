// Selection-driven marker reveal (allusion's decoration trick).
//
// Markers (`.md-markup` spans, see schema.ts) are hidden by CSS at
// rest. This plugin decorates the textblock containing the selection
// head with the `md-active` class, and the stylesheet reveals the
// markers inside it:
//
//     .md-markup            { display: none; }
//     .md-active .md-markup { display: inline; }
//
// Visibility is *pure CSS keyed off one node decoration* — cursor
// movement never mutates the document, and the reveal costs one
// class toggle per selection change.

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { EditorState } from "prosemirror-state";

export const revealKey = new PluginKey<DecorationSet>("tonk-prose-reveal");

function activeBlockDecorations(state: EditorState): DecorationSet {
  const { $head } = state.selection;
  // The deepest textblock the head sits in. Gap cursors / node
  // selections have no textblock parent — nothing to reveal.
  if (!$head.parent.isTextblock) return DecorationSet.empty;
  const start = $head.before();
  const deco = Decoration.node(start, start + $head.parent.nodeSize, {
    class: "md-active",
  });
  return DecorationSet.create(state.doc, [deco]);
}

export function reveal(): Plugin<DecorationSet> {
  return new Plugin<DecorationSet>({
    key: revealKey,
    state: {
      init: (_config, state) => activeBlockDecorations(state),
      apply(tr, prev, _old, state) {
        if (!tr.docChanged && !tr.selectionSet) return prev;
        return activeBlockDecorations(state);
      },
    },
    props: {
      decorations(state) {
        return revealKey.getState(state);
      },
    },
  });
}

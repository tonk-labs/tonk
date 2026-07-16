// Ghost-text placeholder, shown while the document is empty. The
// text lives in plugin state (not a closure) so the shell can update
// it after mount via a transaction meta — see `setPlaceholder` in
// `index.ts`.

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { Node } from "prosemirror-model";

export const placeholderKey = new PluginKey<string>("tonk-prose-placeholder");

/** A doc is "empty" when it holds a single childless paragraph. */
function isEmpty(doc: Node): boolean {
  return (
    doc.childCount === 1 &&
    doc.firstChild !== null &&
    doc.firstChild.type.name === "paragraph" &&
    doc.firstChild.childCount === 0
  );
}

export function placeholder(initial: string): Plugin<string> {
  return new Plugin<string>({
    key: placeholderKey,
    state: {
      init: () => initial,
      apply: (tr, prev) => {
        const next = tr.getMeta(placeholderKey);
        return typeof next === "string" ? next : prev;
      },
    },
    props: {
      decorations(state) {
        const text = placeholderKey.getState(state);
        if (!text || !isEmpty(state.doc)) return null;
        // Node decoration spanning the lone empty paragraph.
        const deco = Decoration.node(0, state.doc.firstChild!.nodeSize, {
          "data-placeholder": text,
          class: "tonk-prose-empty",
        });
        return DecorationSet.create(state.doc, [deco]);
      },
    },
  });
}

// Rendered <img> previews for expanded images.
//
// The document stores an image as its literal source text
// (`![alt](src)` under the `image_markup` mark — see markup.ts).
// This plugin renders the picture itself: one widget decoration
// placed right after each source occurrence. At rest the source
// text is hidden (like every marker) and only the picture shows;
// with the caret in the block, source and picture show side by
// side and edits to the source re-point the preview — allusion's
// collapsed/expanded image pair, with the reparse loop keeping
// text and preview coherent.
//
// Widgets are keyed by their source string, so retyping an
// unrelated part of the block reuses the existing <img> element
// (no flicker, no image re-fetch).

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { Node } from "prosemirror-model";
import { isImageMarkup, matchImages } from "./markup";

export const imagePreviewKey = new PluginKey<DecorationSet>(
  "tonk-prose-image-preview",
);

function makePreview(alt: string, src: string, title: string | undefined) {
  return (): HTMLElement => {
    const img = document.createElement("img");
    img.className = "md-image-preview";
    img.src = src;
    img.alt = alt;
    if (title) img.title = title;
    img.draggable = false;
    // Broken sources keep a visible, styleable footprint instead of
    // the browser's broken-image glyph soup.
    img.addEventListener("error", () => img.classList.add("md-image-broken"), {
      once: true,
    });
    return img;
  };
}

function computePreviews(doc: Node): DecorationSet {
  const decorations: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (!node.isText || !isImageMarkup(node) || !node.text) return true;
    for (const match of matchImages(node.text)) {
      const [source, alt, src, title] = match;
      decorations.push(
        Decoration.widget(
          pos + match.index + source.length,
          makePreview(alt, src, title),
          // side: 1 → the widget sits after the source text, and a
          // caret at that position lands before the widget, next to
          // the text it edits. `key` enables element reuse.
          { side: 1, key: `img:${source}` },
        ),
      );
    }
    return true;
  });
  return DecorationSet.create(doc, decorations);
}

export function imagePreview(): Plugin<DecorationSet> {
  return new Plugin<DecorationSet>({
    key: imagePreviewKey,
    state: {
      init: (_config, state) => computePreviews(state.doc),
      apply(tr, prev) {
        // Recompute only when the document changed; selection-only
        // transactions map the existing set for free.
        return tr.docChanged ? computePreviews(tr.doc) : prev;
      },
    },
    props: {
      decorations(state) {
        return imagePreviewKey.getState(state);
      },
    },
  });
}

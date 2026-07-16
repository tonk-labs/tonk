// The editor schema: `prosemirror-markdown`'s CommonMark node/mark
// set plus one extra mark, `markup`, for literal syntax characters.
//
// Markdown markers (`**`, `` ` ``, `[`, `](url)`, `# `) are stored
// as *literal text nodes* carrying the `markup` mark — the allusion
// technique. They are hidden by CSS everywhere except the block
// under the caret (reveal.ts), which is what produces the Typora
// feel: rich text at rest, raw syntax where you're editing.
//
// This module contains ONLY the schema. The markdown round-trip
// lives in markdown.ts and marker materialization in markup.ts —
// both import the schema from here, so this file must not import
// either of them (module-eval cycle).

import { schema as baseSchema } from "prosemirror-markdown";
import { Schema } from "prosemirror-model";

export const schema = new Schema({
  nodes: baseSchema.spec.nodes,
  marks: baseSchema.spec.marks
    .addToEnd("markup", {
      attrs: {
        // "inline" markers (`**`, `` ` ``, `[`, `](url)`) reveal per
        // edit range; "block" markers (the heading `# ` prefix)
        // reveal whenever their block is active.
        kind: { default: "inline" },
        // The content marks this marker belongs to (allusion's
        // `marks` attr): identity keys (markup.ts `markKey`) of the
        // delimiter run(s) enclosing it, innermost last. This is
        // what lets the reveal plugin know that the caret sitting
        // on a bare `**` text node is "inside" the strong span next
        // to it. Flat strings on purpose — attr values must stay
        // cycle-free (the DOM serializer walks them recursively).
        of: { default: [] },
      },
      // Text typed at a marker's edge must not inherit the mark.
      inclusive: false,
      // No parseDOM: markers exist only when materialized by us.
      toDOM: (mark) => [
        "span",
        {
          class:
            mark.attrs.kind === "block"
              ? "md-markup md-block"
              : "md-markup",
        },
        0,
      ],
    })
    // Images, expanded (allusion's `expandedImage`, as a mark): the
    // editor document stores an image as its literal source text
    // `![alt](src)` carrying this mark. The text is revealed/hidden
    // exactly like other markers; the rendered <img> is a widget
    // decoration that follows the text (image-preview.ts); demarkup
    // folds the text back into a real image node for serialization.
    // Because the source is text, blocks with images stay eligible
    // for the reparse loop — typing or editing `![alt](src)` just
    // works, and breaking the syntax degrades it to plain text.
    .addToEnd("image_markup", {
      inclusive: false,
      toDOM: () => ["span", { class: "md-markup md-image-src" }, 0],
    }),
});

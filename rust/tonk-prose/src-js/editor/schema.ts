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
  marks: baseSchema.spec.marks.addToEnd("markup", {
    // Text typed at a marker's edge must not inherit the mark.
    inclusive: false,
    // No parseDOM: markers exist only when materialized by us.
    toDOM: () => ["span", { class: "md-markup" }, 0],
  }),
});

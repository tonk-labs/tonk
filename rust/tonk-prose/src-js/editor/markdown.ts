// Markdown round-trip:
//
//   markdown --parse--> clean doc --materialize--> editor doc
//   editor doc --demarkup--> clean doc --serialize--> markdown
//
// where materialize/demarkup (markup.ts) add/strip the literal
// marker text. The serializer never sees markers (they're stripped
// first) — it regenerates delimiters from the marks themselves,
// which also normalizes whatever the user literally typed.

import {
  defaultMarkdownParser,
  defaultMarkdownSerializer,
  MarkdownParser,
  MarkdownSerializer,
} from "prosemirror-markdown";
import MarkdownIt from "markdown-it";
import { Fragment } from "prosemirror-model";
import type { Node } from "prosemirror-model";
import { schema } from "./schema";
import { materializeDoc, demarkupDoc } from "./markup";

/** markdown-it tuned like `prosemirror-markdown`'s default (CommonMark,
 *  no raw HTML) but with the GFM extensions we support turned back on:
 *  strikethrough (`~~`). The `commonmark` preset disables these, so we
 *  start from the default preset and re-lock everything else off. */
const markdownIt = MarkdownIt("commonmark", { html: false }).enable([
  "strikethrough",
]);

/** Parser bound to our extended schema. Reuses `defaultMarkdownParser`'s
 *  token map, extended with the GFM tokens our schema adds. Produces
 *  *clean* docs (no marker text). */
const parser = new MarkdownParser(schema, markdownIt, {
  ...defaultMarkdownParser.tokens,
  s: { mark: "strikethrough" },
});

/** A leading GFM task-list checkbox (`[ ] ` / `[x] `) in a list item's
 *  first paragraph. Demarkup keeps this as literal text (markup.ts),
 *  but the default text serializer would escape the brackets
 *  (`\[ \]`), which no longer reads as a checkbox. The paragraph
 *  serializer below writes the prefix raw and escapes only the rest. */
const TASK_PREFIX = /^\[[ xX]\] /;

/** Serializer for clean docs. Overrides `paragraph` to emit an
 *  unescaped task-list checkbox prefix; the `markup` entry is
 *  belt-and-braces (demarkup strips markers, so it should never fire,
 *  but a survivor emits verbatim rather than crashing). */
export const serializer = new MarkdownSerializer(
  {
    ...defaultMarkdownSerializer.nodes,
    paragraph(state, node, parent, index) {
      const first = node.firstChild;
      const inItem = parent.type === schema.nodes.list_item && index === 0;
      const match =
        inItem && first?.isText && first.text
          ? TASK_PREFIX.exec(first.text)
          : null;
      if (match) {
        // Write the checkbox verbatim, then serialize the paragraph
        // with the prefix shaved off its first text node.
        state.write(match[0]);
        const rest = node.copy(shaveTaskPrefix(node.content, match[0].length));
        state.renderInline(rest);
        state.closeBlock(node);
        return;
      }
      state.renderInline(node);
      state.closeBlock(node);
    },
  },
  {
    ...defaultMarkdownSerializer.marks,
    strikethrough: {
      open: "~~",
      close: "~~",
      mixable: true,
      expelEnclosingWhitespace: true,
    },
    markup: {
      open: "",
      close: "",
      mixable: false,
      expelEnclosingWhitespace: false,
    },
  },
);

/** Drop `n` leading characters from a fragment's first text child —
 *  the task-prefix shave for the paragraph serializer. */
function shaveTaskPrefix(content: Fragment, n: number): Fragment {
  const first = content.firstChild;
  if (!first?.isText || !first.text) return content;
  const rest = first.text.slice(n);
  const out: Node[] = [];
  if (rest) out.push(schema.text(rest, first.marks));
  content.forEach((child, _o, i) => {
    if (i > 0) out.push(child);
  });
  return Fragment.from(out);
}

/** Parse markdown into a *clean* document (no marker text). */
export function parseCleanMarkdown(source: string): Node {
  const doc = parser.parse(source);
  if (doc) return doc;
  return schema.node(
    "doc",
    null,
    schema.node("paragraph", null, source ? [schema.text(source)] : []),
  );
}

/** Parse markdown into an editor document with markers materialized. */
export function parseMarkdown(source: string): Node {
  return materializeDoc(parseCleanMarkdown(source));
}

/** Serialize an editor document back to markdown source. */
export function serializeMarkdown(doc: Node): string {
  return serializer.serialize(demarkupDoc(doc), { tightLists: true });
}

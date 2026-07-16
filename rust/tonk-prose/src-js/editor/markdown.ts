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
import type { Node } from "prosemirror-model";
import { schema } from "./schema";
import { materializeDoc, demarkupDoc } from "./markup";

/** Parser bound to our extended schema, reusing the CommonMark
 *  tokenizer + token map from `defaultMarkdownParser`. Produces
 *  *clean* docs (no marker text). */
const parser = new MarkdownParser(
  schema,
  defaultMarkdownParser.tokenizer,
  defaultMarkdownParser.tokens,
);

/** Serializer for clean docs. The `markup` entry is belt-and-braces:
 *  demarkup strips marker text before serialization, so it should
 *  never fire, but if a marker survives we emit its text verbatim
 *  rather than crashing on an unknown mark. */
export const serializer = new MarkdownSerializer(
  defaultMarkdownSerializer.nodes,
  {
    ...defaultMarkdownSerializer.marks,
    markup: {
      open: "",
      close: "",
      mixable: false,
      expelEnclosingWhitespace: false,
    },
  },
);

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

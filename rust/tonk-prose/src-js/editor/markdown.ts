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
import markdownItMark from "markdown-it-mark";
import { Fragment } from "prosemirror-model";
import type { Node } from "prosemirror-model";
import { schema } from "./schema";
import { materializeDoc, demarkupDoc } from "./markup";

/** markdown-it tuned like `prosemirror-markdown`'s default (CommonMark,
 *  no raw HTML) but with the extensions we support turned back on: GFM
 *  strikethrough (`~~`) and highlight (`==`, via markdown-it-mark). The
 *  `commonmark` preset disables the built-ins, so we start from it and
 *  re-enable only what we want.
 *
 *  Note markdown-it's `breaks` option does NOT help here: it is a
 *  *renderer* option, applied when markdown-it writes HTML. We consume
 *  tokens, so a single newline arrives as a `softbreak` token either way.
 *  The parser's token map below is where it becomes a line break. */
const markdownIt = MarkdownIt("commonmark", { html: false })
  .enable(["strikethrough"])
  .use(markdownItMark);

/** Parser bound to our extended schema. Reuses `defaultMarkdownParser`'s
 *  token map, extended with the tokens our schema adds. Produces *clean*
 *  docs (no marker text). */
const parser = new MarkdownParser(schema, markdownIt, {
  ...defaultMarkdownParser.tokens,
  s: { mark: "strikethrough" },
  mark: { mark: "highlight" },
  // A single newline is a LINE BREAK, not a space.
  //
  // CommonMark calls it a "soft" break and folds it to a space, so a
  // newline the author typed vanishes the moment the document is
  // serialized back: the editor silently reflows their text. Mapping the
  // token to `hard_break` keeps the break in the document, and the
  // serializer below writes it back as a bare newline.
  softbreak: { node: "hard_break" },
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
    // Emit `-` bullets (not the default `*`), matching the `- ` markers
    // materialization stamps and the reparse loop reads back.
    bullet_list(state, node) {
      state.renderList(node, "  ", () => "- ");
    },
    // A bare newline, not the default `\`-then-newline. With `breaks` on,
    // every line break the author types parses as a `hard_break`, so the
    // default escape would write a backslash into text that never had one.
    // Trailing breaks are still dropped: they carry no content and would
    // accumulate blank lines across an edit cycle.
    hard_break(state, node, parent, index) {
      for (let i = index + 1; i < parent.childCount; i++) {
        if (parent.child(i).type !== node.type) {
          state.write("\n");
          return;
        }
      }
    },
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
    highlight: {
      open: "==",
      close: "==",
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

/** The leading `> ` blockquote runs of a line (`"> > x"` → `"> > "`). */
function quotePrefixOf(line: string): string {
  const match = /^(?:> )*/.exec(line);
  return match ? match[0] : "";
}

/** Re-materialize a run of block source lines into editor blocks — the
 *  whole-wrapper reparse (reparse.ts). Each line is a textblock's
 *  `textContent`, already carrying its `>`/`-`/`1.`/`[ ] ` prefix
 *  (materializeDoc), so the lines ARE markdown source.
 *
 *  Consecutive lines are separated by a BLANK line — a bare `\n` lets
 *  CommonMark's lazy continuation fuse neighbors into one paragraph
 *  (`> foo\n> bar` → one `"foo bar"`), so each line needs its own block.
 *  But a plain blank line also BREAKS a blockquote, which would split
 *  every multi-line quote into separate quote blocks. The rule the
 *  editor wants: consecutive `>` lines stay ONE blockquote, and the
 *  quote breaks only at a non-`>` line. So the separator between two
 *  lines carries their COMMON `> ` depth: two `> ` lines are joined by a
 *  quoted blank line (`>`), keeping one blockquote with two paragraphs;
 *  a `>` line followed by a plain line is separated by a bare blank,
 *  splitting the quote. Lists need no marker in the separator — a blank
 *  line keeps a list going as long as the next line still has its
 *  `- `/`1. `. Returns the top-level materialized blocks. */
export function reparseBlockLines(lines: readonly string[]): Fragment {
  let source = lines[0] ?? "";
  for (let i = 1; i < lines.length; i++) {
    // Common blockquote depth of the two adjacent lines — the separator
    // stays inside a quote the two share, and drops to a bare blank when
    // one leaves the quote.
    const a = quotePrefixOf(lines[i - 1]);
    const b = quotePrefixOf(lines[i]);
    const common = a.length < b.length ? a : b;
    const shared = b.startsWith(common) && a.startsWith(common) ? common : "";
    // A quoted blank line is `>` (trailing space trimmed) so it reads as
    // an empty quoted paragraph rather than trailing whitespace.
    const separator = shared ? shared.trimEnd() : "";
    source += `\n${separator}\n${lines[i]}`;
  }
  return materializeDoc(parseCleanMarkdown(source)).content;
}

// `dialog-yaml` language pack.
//
// Dialog notation is YAML at the syntax level — the difference is
// semantic (head → fields shape, query/assertion expression
// flavours, `?var` unification, `&anchor` name publication, bare
// symbols as name-table references, etc.). Where dialog notation
// diverges visually from generic YAML is a small set of
// recognizable surface forms; we decorate them here so the editor
// communicates the dialect at a glance.
//
// Decoration classes today:
//
// - **Variables** — `?<word>` and bare `_`. Both denote unbound
//   logic positions; coloring them identically signals their
//   shared role even though their syntax differs.
//
// - **Entities** — colon-bearing URI tokens (`did:key:zX`,
//   `id:alice`, `db:concept`) and attribute URIs in
//   `domain/name` form (`xyz.tonk.person/name`). Underlining
//   communicates "direct reference to a thing" without competing
//   with the role-based color of the surrounding token.
//
// - **Names** — bare lowercase symbols in field-value position
//   (the name-table reference shape from the guide), and the
//   identifier portion of an `&<name>` anchor. The shared color
//   tells the eye "this resolves through the name table" —
//   distinguishing it from a literal string (which stays
//   uncolored).
//
// - **Name sigil** — the leading `&` of an anchor. Distinct from
//   the name itself so the eye reads "publish under this name"
//   without losing the name's identity.
//
// - **Effect marker** — the *whole head* of an assertion or
//   retraction: `person!`, `xyz.tonk!`, `db:concept!`. The
//   effect color paints both the name and the trailing `!` so
//   the eye is drawn to the entire token that flips meaning
//   from query to mutation. A `rule!:` body nests two more
//   effect heads (`assert!:` / `retract!:`) one indent deep —
//   those are decorated the same way, so the rule's polarity
//   line reads with the same weight as a top-level head.
//
// - **Rule keyword** — the structural keys of a `rule!:` body:
//   `when:`, `unless:`, `description:`, and the `assert:` /
//   `where:` keys of each premise. These aren't mutation heads
//   (no `!`) and aren't name references — they're the rule's
//   skeleton. A dedicated color sets them apart from ordinary
//   YAML keys so the shape of a rule reads at a glance.
//
// Future iterations can layer additional decorations by walking
// the syntax tree from `lang-yaml` (or, eventually, semantic
// tokens emitted by the language server) instead of regex-matching
// the raw text. For now the patterns are unambiguous enough that a
// regex pass over visible ranges is correct without false
// positives in any reasonable document.

import { yamlLanguage } from "@codemirror/lang-yaml";
import { htmlLanguage } from "@codemirror/lang-html";
import { markdownLanguage } from "@codemirror/lang-markdown";
import {
  LanguageSupport,
  LRLanguage,
  getIndentUnit,
  indentService,
} from "@codemirror/language";
import { parseMixed } from "@lezer/common";
import type { Parser, SyntaxNodeRef, Input } from "@lezer/common";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";

/** Variable decoration — `?<word>` or bare `_`. */
const variableMark = Decoration.mark({ class: "tonk-cm-variable" });

/** Entity decoration — colon-bearing URI tokens and attribute
 *  URIs (`domain/name`). Underlined to signal "direct
 *  reference" without claiming a color slot in the role-based
 *  palette. */
const entityMark = Decoration.mark({ class: "tonk-cm-entity" });

/** Name-sigil decoration — the leading `&` of an anchor.
 *  Distinct from the anchor's name so the eye reads
 *  "publication" without losing the name's identity. */
const nameSigilMark = Decoration.mark({ class: "tonk-cm-name-sigil" });

/** Name decoration — bare-symbol references in field-value
 *  position, and the identifier portion of an `&<name>`
 *  anchor. */
const nameMark = Decoration.mark({ class: "tonk-cm-name" });

/** Effect-marker decoration — covers the entire head of an
 *  assertion or retraction (`person!`, `xyz.tonk!`,
 *  `db:concept!`). Painting the full head — name *and* `!` —
 *  matches the conceptual unit: the head is what flips from
 *  query to mutation, not just the trailing punctuation. */
const effectMark = Decoration.mark({ class: "tonk-cm-effect" });

/** Rule-keyword decoration — the structural keys of a `rule!:`
 *  body (`when:`, `unless:`, `description:`) and of each premise
 *  (`assert:`, `where:`). These are the rule's skeleton, not
 *  mutation heads or name references; a dedicated color makes a
 *  rule's shape legible at a glance. */
const keywordMark = Decoration.mark({ class: "tonk-cm-keyword" });

/** Match a `?` followed by one or more word characters. */
const VARIABLE_NAMED = /\?\w+/g;

/** Match a bare `_` token: word-boundary on either side so we
 *  don't catch the underscore inside `did:key:zAlice` or
 *  `__init__`. JavaScript regex `\b` treats `_` as a word
 *  character, so we use lookarounds with explicit word-char
 *  classes instead. */
const VARIABLE_BLANK = /(?<![A-Za-z0-9_])_(?![A-Za-z0-9_])/g;

/** Match a colon-bearing identifier-ish token (`did:key:zX`,
 *  `id:alice`, `db:concept`). A sequence of identifier chars
 *  (and dots, slashes) that contains at least one colon. The
 *  trailing colon that ends a YAML key isn't part of the value
 *  text, so we won't catch it. */
const ENTITY_URI = /[A-Za-z][A-Za-z0-9._/-]*:[A-Za-z0-9._/:-]+/g;

/** Match an attribute URI in `domain/name` form
 *  (`xyz.tonk.person/name`, `dialog.meta/description`).
 *  Required: at least one dot in the prefix, exactly one slash,
 *  and a name part. We require start-of-token (whitespace, `:`,
 *  comma, brackets) to avoid catching a `name/age` substring of
 *  some larger token. */
const ATTRIBUTE_URI =
  /(?<=^|[\s,[{:])([a-z][a-z0-9+-]*(?:\.[a-z0-9+-]+)+\/[a-z][a-z0-9+.-]*)/gm;

/** Match the head of an assertion or retraction — name plus
 *  the trailing `!`. The head's name may contain dots
 *  (`xyz.tonk`), slashes (rare), and a colon for URI-form heads
 *  (`db:concept`, `did:key:zX`). The `!` is required: query
 *  heads aren't decorated.
 *
 *  Leading whitespace is allowed and not captured — a top-level
 *  head sits at column 0, but a `rule!:` body nests `assert!:`
 *  / `retract!:` one indent deep, and those are effect heads
 *  too. The match must be the first non-whitespace token on its
 *  line (a `name!` in value position is something else).
 *
 *  The capture is the *entire* head including the `!`, so the
 *  effect mark covers the whole token. The lookahead for `:`
 *  pins the match to a YAML key, not a value somewhere else. */
const EFFECT_HEAD = /^[ \t]*([A-Za-z][A-Za-z0-9._/:-]*!)(?=:)/gmd;

/** Match a `rule!:` body keyword as a YAML key — the first
 *  non-whitespace token on its line, followed by `:`. The set
 *  is closed: `when` / `unless` / `description` scaffold the
 *  rule; `assert` / `where` scaffold a premise. (`assert!:` is
 *  *not* in this set — the `!` makes it an effect head, caught
 *  by `EFFECT_HEAD`.)
 *
 *  These keywords are also valid keys outside a rule (a concept
 *  field could be named `description`), but in practice the
 *  notation reserves them; decorating every occurrence keeps
 *  the regex pass simple and is correct for any well-formed
 *  document. The capture excludes the leading indentation so
 *  only the keyword itself is colored. */
const RULE_KEYWORD = /^[ \t]*(when|unless|description|assert|where)(?=:)/gmd;

/** Match an `&<name>` anchor. Anchors appear on the value side
 *  of an assertion head (`head!: &alice`). We require the `&`
 *  to follow `:` and whitespace so we don't accidentally catch
 *  a stray `&` elsewhere. The capture groups split the sigil
 *  and the name. The `d` flag exposes per-group ranges via
 *  `match.indices`.
 *
 *  The name charset mirrors the parser's `is_anchor_char`
 *  (alphanumerics plus `-`, `_`, `+`, `.`, `/`): a concept named
 *  `demo/stuff` is anchored `&demo/stuff`, so the `/` and the
 *  rest of the name must be part of the match, not left to
 *  highlight as something else. */
const ANCHOR = /(?<=:\s+)(&)([a-z][a-z0-9+./_-]*)/gd;

/** Match a bare-symbol name reference in field-value position.
 *  A symbol per the guide starts with `[a-z]` and continues
 *  with `[a-z0-9.+-]`. We approximate the parser's charset
 *  with a regex; the user can quote a value when they need a
 *  literal that happens to match the symbol shape.
 *
 *  Constraints to stay out of trouble:
 *  - Must follow `:` and whitespace (i.e. value position).
 *  - Must end at whitespace or end-of-line (so we don't catch a
 *    prefix of a longer token like `id:foo`).
 *  - Must NOT contain `/` — those are attribute URIs, caught
 *    elsewhere. (We forbid `/` by excluding it from the body
 *    charset.)
 *  - Must NOT contain `:` — those are entity URIs.
 *
 *  An entity URI match runs first and short-circuits this one
 *  via the range-set builder's first-write-wins behavior… but
 *  the builder doesn't actually offer that; instead we add
 *  decorations in priority order and trust that overlapping
 *  marks merge cleanly. To be safe we keep the symbol charset
 *  strict (no `:`, no `/`) so the patterns don't overlap. */
const SYMBOL_REF = /(?<=:\s+)([a-z][a-z0-9.+-]*)(?=\s|$)/gm;

/** Walk every visible range and emit a decoration for each
 *  match. `RangeSetBuilder` requires its inputs in order; we
 *  collect into an array, sort, and add in one pass to keep the
 *  builder's invariant. */
function buildDecorations(view: EditorView): DecorationSet {
  type Hit = { from: number; to: number; mark: Decoration };
  const hits: Hit[] = [];
  for (const { from, to } of view.visibleRanges) {
    const text = view.state.doc.sliceString(from, to);
    const collect = (re: RegExp, mark: Decoration) => {
      re.lastIndex = 0;
      for (let m; (m = re.exec(text)); ) {
        hits.push({
          from: from + m.index,
          to: from + m.index + m[0].length,
          mark,
        });
      }
    };
    collect(VARIABLE_NAMED, variableMark);
    collect(VARIABLE_BLANK, variableMark);
    collect(ENTITY_URI, entityMark);

    // Attribute URIs (`domain/name`) — captured group, not the
    // full match (the lookbehind compensates for one form, the
    // alternation a leading anchor for the other). Use the
    // capture's range so the leading separator isn't decorated.
    ATTRIBUTE_URI.lastIndex = 0;
    for (let m; (m = ATTRIBUTE_URI.exec(text)); ) {
      const indices = (
        m as RegExpExecArray & { indices?: Array<[number, number]> }
      ).indices;
      const range = indices?.[1];
      if (!range) continue;
      const [start, end] = range;
      hits.push({
        from: from + start,
        to: from + end,
        mark: entityMark,
      });
    }

    // Effect heads — the full `name!` token, first on its line
    // (a top-level head, or an `assert!:` / `retract!:` nested
    // in a `rule!:` body). Use the capture's range so neither
    // the leading indentation nor the trailing `:` (checked by
    // the lookahead, not included) is decorated.
    EFFECT_HEAD.lastIndex = 0;
    for (let m; (m = EFFECT_HEAD.exec(text)); ) {
      const range = (
        m as RegExpExecArray & { indices?: Array<[number, number]> }
      ).indices?.[1];
      if (!range) continue;
      hits.push({
        from: from + range[0],
        to: from + range[1],
        mark: effectMark,
      });
    }

    // Rule-body keywords — `when:` / `unless:` / `description:`
    // and the `assert:` / `where:` of a premise. Capture range
    // excludes the leading indentation and the trailing `:`.
    RULE_KEYWORD.lastIndex = 0;
    for (let m; (m = RULE_KEYWORD.exec(text)); ) {
      const range = (
        m as RegExpExecArray & { indices?: Array<[number, number]> }
      ).indices?.[1];
      if (!range) continue;
      hits.push({
        from: from + range[0],
        to: from + range[1],
        mark: keywordMark,
      });
    }

    // Anchors — `&<name>` on the value side of a head. The
    // sigil and the name get distinct decorations.
    ANCHOR.lastIndex = 0;
    for (let m; (m = ANCHOR.exec(text)); ) {
      const indices = (
        m as RegExpExecArray & { indices?: Array<[number, number]> }
      ).indices;
      if (!indices || !indices[1] || !indices[2]) continue;
      const [sigilStart, sigilEnd] = indices[1];
      const [nameStart, nameEnd] = indices[2];
      hits.push({
        from: from + sigilStart,
        to: from + sigilEnd,
        mark: nameSigilMark,
      });
      hits.push({
        from: from + nameStart,
        to: from + nameEnd,
        mark: nameMark,
      });
    }

    // Bare-symbol references in field-value position.
    collect(SYMBOL_REF, nameMark);
  }
  hits.sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const hit of hits) {
    builder.add(hit.from, hit.to, hit.mark);
  }
  return builder.finish();
}

/** ViewPlugin that maintains the decoration set in sync with the
 *  visible viewport. Reruns on document change and viewport
 *  scroll; the regex pass is bounded by the viewport size so
 *  cost stays constant for large documents. */
const dialectDecorations = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
  },
);

/** Theme rules for the decoration classes. Routed through the
 *  element's `--tonk-code-*` variable contract so consumers can
 *  retheme without touching this file.
 *
 *  Each rule also targets descendants (`& *`) because the host
 *  YAML grammar wraps the same range in its own token spans
 *  (e.g. a head's `propertyName` span sits inside our effect
 *  range); the inner span's `color` would otherwise win the
 *  cascade and the decoration's color would be invisible.
 *  `font-style` / `text-decoration` inherit by default and don't
 *  need this. */
const dialectTheme = EditorView.theme({
  ".tonk-cm-variable, .tonk-cm-variable *": {
    color: "var(--tonk-code-variable)",
    fontStyle: "italic",
  },
  ".tonk-cm-entity, .tonk-cm-entity *": {
    color: "var(--tonk-code-entity)",
    textDecoration: "underline",
    textDecorationColor: "var(--tonk-code-entity)",
    textDecorationStyle: "solid",
    textDecorationThickness: "1px",
    textUnderlineOffset: "2px",
  },
  ".tonk-cm-name-sigil, .tonk-cm-name-sigil *": {
    color: "var(--tonk-code-name-sigil)",
  },
  ".tonk-cm-name, .tonk-cm-name *": {
    color: "var(--tonk-code-name)",
  },
  ".tonk-cm-effect, .tonk-cm-effect *": {
    color: "var(--tonk-code-effect)",
    fontWeight: "bold",
  },
  ".tonk-cm-keyword, .tonk-cm-keyword *": {
    color: "var(--tonk-code-keyword)",
    fontWeight: "600",
  },
});

/** Map of YAML tags to the lezer parser to apply to the tagged
 *  scalar's content. Two flavors of tag are accepted:
 *
 *  - **Short forms** — `!html`, `!css`, `!js`, `!md`, etc.
 *  - **MIME-type forms** — `!text/html`, `!text/css`,
 *    `!application/javascript`, `!text/markdown`. These mirror
 *    the `type: text/html` shape used elsewhere in the data
 *    model so users don't have to remember a separate vocabulary.
 *
 *  The HTML parser handles embedded `<style>` and `<script>` itself
 *  via its own `parseMixed`, so a single `!html` tag picks up
 *  CSS/JS coloring inside the document automatically. CSS-only and
 *  JS-only tags still route through the HTML parser today — it's
 *  what we have bundled and the highlighting works correctly for
 *  pure CSS/JS content embedded as the document body too. */
const TAG_PARSERS: Record<string, Parser> = {
  "!html": htmlLanguage.parser,
  "!text/html": htmlLanguage.parser,
  "!text/xml": htmlLanguage.parser,
  "!css": htmlLanguage.parser,
  "!text/css": htmlLanguage.parser,
  "!js": htmlLanguage.parser,
  "!javascript": htmlLanguage.parser,
  "!application/javascript": htmlLanguage.parser,
  "!text/javascript": htmlLanguage.parser,
  "!md": markdownLanguage.parser as Parser,
  "!markdown": markdownLanguage.parser as Parser,
  "!text/markdown": markdownLanguage.parser as Parser,
};

/** Resolve the parser to apply to a YAML scalar's content range,
 *  given the scalar node's location in the tree and the editor's
 *  current input. Two dispatch sources, in priority order:
 *
 *  1. A sibling `Tag` node — `template: !html |\n  …` forces
 *     HTML highlighting regardless of the content shape.
 *  2. A heuristic on the content's first non-whitespace char —
 *     `<` → HTML. (Markdown has no reliable single-char marker;
 *     we don't auto-dispatch it.)
 *
 *  Returns the parser to use, or `null` to leave the scalar as
 *  plain YAML text. */
function resolveEmbeddedParser(
  node: SyntaxNodeRef,
  input: Input,
): Parser | null {
  // Walk up past BlockLiteral to find a Tagged ancestor that
  // wraps it — that's where the `!html`-style tag lives.
  let cursor = node.node.parent;
  while (cursor && cursor.name !== "Tagged" && cursor.name !== "BlockMapping") {
    cursor = cursor.parent;
  }
  if (cursor && cursor.name === "Tagged") {
    const tagNode = cursor.firstChild;
    if (tagNode && tagNode.name === "Tag") {
      const tagText = input.read(tagNode.from, tagNode.to).trim();
      const parser = TAG_PARSERS[tagText];
      if (parser) return parser;
    }
  }

  // Heuristic fallback: inspect the first non-whitespace char of
  // the scalar's content. We only auto-dispatch HTML — its `<`
  // sigil is unambiguous; markdown / css / js have no single-char
  // marker that wouldn't false-positive on prose.
  const content = input.read(node.from, node.to);
  const firstNonWs = content.match(/\S/);
  if (firstNonWs && firstNonWs[0] === "<") {
    return htmlLanguage.parser;
  }
  return null;
}

/** lezer-yaml parser wrapped to dispatch embedded languages
 *  inside `BlockLiteralContent` ranges. */
const mixedYamlParser = yamlLanguage.parser.configure({
  wrap: parseMixed((node, input) => {
    if (node.name !== "BlockLiteralContent") return null;
    const parser = resolveEmbeddedParser(node, input);
    if (!parser) return null;
    return { parser };
  }),
});

/** dialog-yaml language: same data as `yamlLanguage` (style tags,
 *  indentation, folding) but with the mixed-parser wrap layered
 *  on. We reach across `LRLanguage`'s API by passing the wrapped
 *  parser to `LRLanguage.define` — that re-applies the existing
 *  parser configuration (style tags etc. survive because they're
 *  attached to the underlying parser via `parser.configure`,
 *  which `wrap` preserves). */
const dialogYamlLanguage = LRLanguage.define({
  name: "dialog-yaml",
  parser: mixedYamlParser,
});

/** Indent service that fixes the one ergonomic miss in
 *  `lang-yaml`: pressing Enter after a header line like `person:`
 *  (a key with no inline value) leaves the new line at column
 *  zero. The lezer-yaml indent prop walks up to the enclosing
 *  `BlockMapping` and returns the column of its first key, which
 *  for a top-level header is 0.
 *
 *  `insertNewlineAndIndent` calls this service with `pos` set to
 *  the cursor *before* the newline (it builds an `IndentContext`
 *  with `simulateBreak: pos`), so we look at the line containing
 *  `pos` — the line that's about to be split. If that line's
 *  text up to the cursor ends with `:` (header form, no inline
 *  value), the freshly opened body lives one indent unit deeper.
 *
 *  Returning `undefined` for any other shape defers to the next
 *  service and ultimately to the language's own indent prop, so
 *  list-item / nested-mapping indentation keeps working. */
const dialogYamlIndentService = indentService.of((cx, pos) => {
  const line = cx.state.doc.lineAt(pos);
  // Slice up to `pos` so we react to the *typed* prefix, not
  // anything that might trail the cursor on the same line. A
  // header line is "<text>:" — we accept `person:`, `with:`,
  // `foo!:`. `name: alice` ends in a value, not `:`, so this
  // won't match.
  const prefix = line.text.slice(0, pos - line.from).replace(/\s+$/, "");
  if (!prefix.endsWith(":")) return undefined;
  const baseIndent = line.text.length - line.text.trimStart().length;
  return baseIndent + getIndentUnit(cx.state);
});

export default [
  new LanguageSupport(dialogYamlLanguage),
  dialogYamlIndentService,
  dialectDecorations,
  dialectTheme,
];

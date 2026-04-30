// `dialog-yaml` language pack.
//
// Dialog notation is YAML at the syntax level — the difference is
// semantic (head → fields shape, query/assertion/retraction
// expression flavours, `?var` unification, `.name` bookmark
// references, etc.). Where dialog notation diverges visually from
// generic YAML is a small set of recognizable surface forms; we
// decorate them here so the editor communicates the dialect at a
// glance.
//
// Five decoration classes today:
//
// - **Variables** — `?<word>` and bare `_`. Both denote unbound
//   logic positions; coloring them identically signals their
//   shared role even though their syntax differs.
//
// - **Entities** — colon-bearing URI tokens (`did:key:zX`).
//   Underlining communicates "this is a reference to a thing"
//   without competing with the role-based color of the
//   surrounding token.
//
// - **Names** — the identifier portion of a `.<name>` bookmark
//   reference, plus the leading `.` rendered as a separate
//   sigil-colored mark. The split lets the eye read "reference"
//   without losing the name's identity.
//
// - **Effect marker** — the trailing `!` on a head (`person!:`,
//   `xyz.tonk!:`). Marks the expression as having an effect
//   (assertion or retraction) rather than being a query. Only the
//   `!` character itself is decorated, so the eye is drawn to the
//   one character that flips meaning.
//
// Future iterations can layer additional decorations by walking
// the syntax tree from `lang-yaml` (or, eventually, semantic
// tokens emitted by the language server) instead of regex-matching
// the raw text. For now the patterns are unambiguous enough that a
// regex pass over visible ranges is correct without false
// positives in any reasonable document.

import { yaml } from "@codemirror/lang-yaml";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";

/** Variable decoration — `?<word>` or bare `_`. The host's
 *  `--tonk-code-variable` color drives both. */
const variableMark = Decoration.mark({ class: "tonk-cm-variable" });

/** Entity decoration — colon-bearing URI tokens. Underlined to
 *  signal "identifier reference" without claiming a color slot
 *  in the role-based palette. */
const entityMark = Decoration.mark({ class: "tonk-cm-entity" });

/** Name-sigil decoration — the leading `.` of a `.name` bookmark
 *  reference. Distinct from the name itself so the eye reads
 *  "this token is a reference" without losing the name's
 *  identity. */
const nameSigilMark = Decoration.mark({ class: "tonk-cm-name-sigil" });

/** Name decoration — the identifier portion after the leading
 *  `.` of a bookmark reference. */
const nameMark = Decoration.mark({ class: "tonk-cm-name" });

/** Effect-marker decoration — the trailing `!` on a head. Only
 *  the `!` itself is colored, drawing the eye to the one
 *  character that flips an expression from query to mutation. */
const effectMark = Decoration.mark({ class: "tonk-cm-effect" });

/** Match a `?` followed by one or more word characters. */
const VARIABLE_NAMED = /\?\w+/g;

/** Match a bare `_` token: word-boundary on either side so we
 *  don't catch the underscore inside `did:key:zAlice` or
 *  `__init__`. JavaScript regex `\b` treats `_` as a word
 *  character, so we use lookarounds with explicit word-char
 *  classes instead. */
const VARIABLE_BLANK = /(?<![A-Za-z0-9_])_(?![A-Za-z0-9_])/g;

/** Match a colon-bearing identifier-ish token: a sequence of
 *  identifier characters (and dots, slashes) that contains at
 *  least one colon. Excludes the trailing colon that follows a
 *  YAML key (`name:` is grammar, not a token, so the `:` there
 *  isn't part of any text token the regex would see). */
const ENTITY_URI = /[A-Za-z][A-Za-z0-9._/-]*:[A-Za-z0-9._/:-]+/g;

/** Match `.<name>` bookmark references in value position. We
 *  capture the leading `.` and the name separately so the two can
 *  be decorated with distinct marks (sigil vs. name color). The
 *  lookbehind requires start-of-token or whitespace before the
 *  dot, so we don't catch `xyz.tonk` claim domains in head
 *  position, where the dot is mid-token. */
const NAME_REF = /(?<=^|[\s,[{])(\.)(\w[\w-]*)/gm;

/** Match a head's trailing `!` — the `!` immediately preceding
 *  either a `:` (start of body) or whitespace (preceding a
 *  binding token like `nick` or `?nick`). The lookbehind
 *  requires the `!` to follow a name character, so we don't
 *  catch a stray `!` elsewhere. */
const EFFECT_BANG = /(?<=[A-Za-z0-9._/-])!(?=:|\s)/g;

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
    collect(EFFECT_BANG, effectMark);
    // `.name` references — sigil and name decorated separately.
    NAME_REF.lastIndex = 0;
    for (let m; (m = NAME_REF.exec(text)); ) {
      const dotIndex = m.index;
      const nameIndex = dotIndex + m[1].length;
      hits.push({
        from: from + dotIndex,
        to: from + dotIndex + m[1].length,
        mark: nameSigilMark,
      });
      hits.push({
        from: from + nameIndex,
        to: from + nameIndex + m[2].length,
        mark: nameMark,
      });
    }
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
 *  retheme without touching this file. */
const dialectTheme = EditorView.theme({
  ".tonk-cm-variable": {
    color: "var(--tonk-code-variable)",
    fontStyle: "italic",
  },
  ".tonk-cm-entity": {
    textDecoration: "underline",
    textDecorationColor: "var(--tonk-code-entity)",
    textDecorationStyle: "solid",
    textDecorationThickness: "1px",
    textUnderlineOffset: "2px",
  },
  ".tonk-cm-name-sigil": {
    color: "var(--tonk-code-name-sigil)",
  },
  ".tonk-cm-name": {
    color: "var(--tonk-code-name)",
  },
  ".tonk-cm-effect": {
    color: "var(--tonk-code-effect)",
    fontWeight: "bold",
  },
});

export default [yaml(), dialectDecorations, dialectTheme];

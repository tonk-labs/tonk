// `dialog-yaml` language pack.
//
// Dialog notation is YAML at the syntax level — the difference is
// semantic (entity → context → fields hierarchy, reserved
// `dialog.*` namespace, `?var` unification, etc.). Where dialog
// notation diverges visually from generic YAML is a small set of
// recognizable surface forms; we decorate them here so the editor
// communicates the dialect at a glance.
//
// Two decoration classes today:
//
// - **Variables** — `?<word>` and bare `_`. Both denote a fresh
//   entity whose identity the runtime decides; coloring them
//   identically signals their shared role even though their syntax
//   differs.
//
// - **Entities** — any colon-bearing token. The RFC's level-1
//   entity rule is "contains `:` ⇒ global identifier" (DID, URI,
//   anything URI-shaped); decorating these with an underline
//   communicates "this is a reference to a thing" without
//   competing with the role-based color of the surrounding token
//   (key, string, value).
//
// Future iterations can layer additional decorations (reserved-
// prefix domains, bookmark names with grammar context) by walking
// the syntax tree from `lang-yaml` instead of regex-matching
// the raw text. For now the patterns are unambiguous enough that
// a regex pass over visible ranges is correct without false
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

/** Entity decoration — any colon-bearing token. Underlined to
 *  signal "identifier reference" without claiming a color slot
 *  in the role-based palette. */
const entityMark = Decoration.mark({ class: "tonk-cm-entity" });

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
const ENTITY = /[A-Za-z][A-Za-z0-9._/-]*:[A-Za-z0-9._/:-]+/g;

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
    collect(ENTITY, entityMark);
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
});

export default [yaml(), dialectDecorations, dialectTheme];

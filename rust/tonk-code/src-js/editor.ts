// The CodeMirror editor core of the library's `<code-editor>`
// (library.ts).
//
// Everything here is the editor itself: the state and view, the theme,
// the per-shadow stylesheet, and one compartment per concern a host
// toggles at runtime (language, read-only, placeholder, gutter, active
// line, LSP). What it is NOT: a custom element, a language loader, or
// an LSP client. The element owns those (library.ts and the methods in
// `library/code-editor/`).

import {
  EditorState,
  Compartment,
  type Extension,
} from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  placeholder as placeholderExt,
  type ViewUpdate,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  acceptCompletion,
  completionKeymap,
  moveCompletionSelection,
} from "@codemirror/autocomplete";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

/** Theme applied inside the editor's own DOM.
 *
 *  Every visual choice is routed through a `--tonk-code-*` CSS
 *  variable. The element ships GitHub-flavored defaults for those
 *  variables (see the per-shadow stylesheet in the constructor) that
 *  adapt to `prefers-color-scheme`. Consumers wire the element into
 *  their own design system by overriding the variables on the host:
 *
 *      code-editor {
 *        --tonk-code-bg: var(--wa-color-surface-default);
 *        --tonk-code-key: var(--wa-color-brand-on-quiet);
 *      }
 *
 *  The element does not reach into any consumer's tokens itself. */
const baseTheme = EditorView.theme({
  "&": {
    fontFamily: "var(--tonk-code-font)",
    fontSize: "var(--tonk-code-font-size)",
    lineHeight: "1.5",
    backgroundColor: "transparent",
    color: "var(--tonk-code-fg)",
    height: "100%",
  },
  ".cm-scroller": {
    fontFamily: "inherit",
  },
  ".cm-content": {
    caretColor: "var(--tonk-code-cursor)",
    padding: "0.5rem 0",
  },
  ".cm-gutters": {
    backgroundColor: "var(--tonk-code-gutter-bg)",
    color: "var(--tonk-code-fg-muted)",
    borderRight: "1px solid var(--tonk-code-border)",
    paddingRight: "0.25rem",
    userSelect: "none",
  },
  ".cm-activeLine": {
    backgroundColor: "var(--tonk-code-active-line)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "transparent",
    color: "var(--tonk-code-fg)",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeft: "2px solid var(--tonk-code-cursor)",
  },
  ".cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-content ::selection":
    {
      backgroundColor: "var(--tonk-code-selection)",
    },
  ".cm-placeholder": {
    color: "var(--tonk-code-fg-muted)",
    fontStyle: "italic",
    // Preserve newlines so multi-line ghost text (e.g. a small
    // example showing the document shape) renders as multiple
    // visual lines. CodeMirror's placeholder extension just sets
    // the string as `textContent` of one `<span>` — without this
    // the browser collapses every newline to a single space.
    whiteSpace: "pre-wrap",
  },
  ".cm-matchingBracket, .cm-nonmatchingBracket": {
    backgroundColor: "var(--tonk-code-bracket-match-bg)",
    outline: "1px solid var(--tonk-code-bracket-match-border)",
  },
  "&.cm-focused": { outline: "none" },

  // Lint / diagnostic tooltips. CodeMirror's base lint theme bakes
  // a light surface (`#f5f5f5`) into the tooltip, which renders
  // the message unreadable on dark host colors. Re-route every
  // tooltip surface through `--tonk-code-*` so it tracks the
  // editor's theme.
  ".cm-tooltip": {
    backgroundColor: "var(--tonk-code-tooltip-bg)",
    color: "var(--tonk-code-tooltip-fg)",
    border: "1px solid var(--tonk-code-tooltip-border)",
    borderRadius: "var(--tonk-code-radius)",
    boxShadow: "0 2px 8px rgba(0, 0, 0, 0.18)",
  },
  ".cm-tooltip-section": {
    borderBottomColor: "var(--tonk-code-tooltip-border)",
  },
  ".cm-tooltip-lint": {
    padding: "0",
    margin: "0",
  },
  ".cm-diagnostic": {
    padding: "0.35rem 0.6rem",
    color: "var(--tonk-code-tooltip-fg)",
    // Severity is conveyed by the left border color; override
    // each variant to use the matching syntax-role token. The
    // editor's wavy underline already shows severity in the
    // gutter so the bar can be subtle.
    borderLeft: "3px solid transparent",
  },
  ".cm-diagnostic-error": {
    borderLeftColor: "var(--tonk-code-error)",
  },
  ".cm-diagnostic-warning": {
    borderLeftColor: "var(--tonk-code-number)",
  },
  ".cm-diagnostic-info": {
    borderLeftColor: "var(--tonk-code-key)",
  },
  ".cm-diagnostic-hint": {
    borderLeftColor: "var(--tonk-code-comment)",
  },
  ".cm-diagnosticText": {
    color: "var(--tonk-code-tooltip-fg)",
  },
  // Wavy underlines for inline lint markers. CM's default
  // background-image SVG uses `stroke="currentColor"`, but
  // `currentColor` doesn't propagate into a `data:` SVG (it
  // resolves to the SVG document root, not the consumer
  // element). Replacing the painter with `text-decoration:
  // underline wavy <color>` lets the browser pick the actual
  // CSS color directly. The `text-decoration-skip-ink: none`
  // keeps the wave visible under descenders. Backgrounds get
  // cleared so they don't double up with the underline.
  ".cm-lintRange-error": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--tonk-code-error)",
    textDecorationSkipInk: "none",
  },
  ".cm-lintRange-warning": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--tonk-code-number)",
    textDecorationSkipInk: "none",
  },
  ".cm-lintRange-info": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--tonk-code-key)",
    textDecorationSkipInk: "none",
  },
  ".cm-lintRange-hint": {
    backgroundImage: "none",
    textDecoration: "underline dotted var(--tonk-code-comment)",
    textDecorationSkipInk: "none",
  },
});

/** Syntax highlight palette. Each tag bundle maps to one
 *  `--tonk-code-{role}` variable; the defaults follow GitHub's
 *  light/dark code palette and adapt to `prefers-color-scheme`. */
const highlightStyle = HighlightStyle.define([
  {
    tag: [t.atom, t.propertyName, t.tagName, t.keyword],
    color: "var(--tonk-code-key)",
    fontWeight: "600",
  },
  {
    tag: [t.string, t.special(t.string)],
    color: "var(--tonk-code-string)",
  },
  {
    tag: [t.number, t.bool, t.null],
    color: "var(--tonk-code-number)",
  },
  {
    tag: [t.comment, t.lineComment, t.blockComment, t.meta],
    color: "var(--tonk-code-comment)",
    fontStyle: "italic",
  },
  {
    tag: [t.operator, t.punctuation, t.separator],
    color: "var(--tonk-code-punctuation)",
  },
  {
    tag: [t.invalid],
    color: "var(--tonk-code-error)",
    textDecoration: "underline wavy",
  },
]);

/** Stable extensions present in every editor regardless of
 *  language. Anything *toggleable* (language pack, read-only,
 *  placeholder, gutter, active-line) lives in a `Compartment`
 *  instead — see the per-instance setup in `createEditor`. */
function baseExtensions(): Extension[] {
  return [
    history(),
    bracketMatching(),
    indentOnInput(),
    syntaxHighlighting(highlightStyle, { fallback: true }),
    // Tab accepts the active autocompletion popup (LSP-driven via
    // `@codemirror/lsp-client`). `acceptCompletion` returns false
    // when no completion is open, so the binding falls through to
    // `indentWithTab` and Tab still indents.
    //
    // `completionKeymap` is what makes the popup navigable at all:
    // ArrowUp/Down move the selection, Enter accepts,
    // Escape closes. Without it those keys fall through to
    // `defaultKeymap` and move the CURSOR instead — the popup stays
    // open showing a selection you cannot change, and Enter splits the
    // line under it. Every one of its bindings no-ops when no
    // completion is open, so ordinary editing is untouched.
    //
    // BEFORE `defaultKeymap`, since keymaps run in order and whichever
    // claims the key first wins.
    keymap.of([
      { key: "Tab", run: acceptCompletion },
      // Ctrl-n/p move through an open list too, as in most editors.
      // `completionKeymap` does not bind them, so the comment above
      // promised keys that did nothing; with no list open they return
      // false and fall through to the line motions they usually are.
      { key: "Ctrl-n", run: moveCompletionSelection(true) },
      { key: "Ctrl-p", run: moveCompletionSelection(false) },
      ...completionKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      indentWithTab,
    ]),
    baseTheme,
  ];
}

/** Line-number gutter. */
function gutterExtensions(): Extension[] {
  return [lineNumbers()];
}

/** Active-line highlight (both the line and its gutter row). The
 *  two are paired so the row decoration is consistent across the
 *  text and the gutter when both are enabled. */
function activeLineExtensions(): Extension[] {
  return [highlightActiveLine(), highlightActiveLineGutter()];
}

/** Build the read-only stack. `EditorState.readOnly` alone leaves
 *  the caret visible and the user can still navigate with arrow
 *  keys; pairing it with `EditorView.editable.of(false)` strips
 *  the contenteditable affordance entirely so sealed cells render
 *  as plain text (no caret, no insertion point on click). Text
 *  selection still works for copy. */
function readOnlyExtensions(readOnly: boolean): Extension[] {
  if (!readOnly) return [];
  return [EditorState.readOnly.of(true), EditorView.editable.of(false)];
}


/** The complete per-shadow stylesheet.
 *
 *  Defines two layers of variables on `:host`:
 *
 *  1. **GitHub-flavored color defaults** — one block for light, one
 *     overriding block under `@media (prefers-color-scheme: dark)`.
 *     The values are GitHub's official primer-css palette
 *     (`--color-prettylights-syntax-*`) condensed to the seven roles
 *     the highlight style needs.
 *
 *  2. **Host appearance defaults** — frame, radius, font, sizing.
 *     Also expressed as variables so consumers can override the
 *     whole field's look without touching the highlight palette.
 *
 *  Consumers theme the element by setting any of these variables on
 *  the host:
 *
 *      code-editor {
 *        --tonk-code-bg: var(--wa-color-surface-default);
 *        --tonk-code-key: var(--wa-color-brand-on-quiet);
 *      }
 *
 *  The element imports nothing from the consumer's design system;
 *  the variables are the contract. */
export const SHADOW_STYLESHEET = `
  :host {
    /* Host frame + sizing */
    --tonk-code-font: ui-monospace, SFMono-Regular, Menlo, Consolas,
                      "Liberation Mono", monospace;
    --tonk-code-font-size: 0.875rem;
    --tonk-code-radius: 6px;
    /* 'auto' lets the host size to the editor's content. The
       editor's own min-height comes from CodeMirror's line layout
       (one line plus content padding). Consumers that want a
       fixed-size field can set min-height on the host directly. */
    --tonk-code-min-height: auto;

    /* Surfaces & text — GitHub light defaults */
    --tonk-code-bg: #ffffff;
    --tonk-code-fg: #1f2328;
    --tonk-code-fg-muted: #59636e;
    --tonk-code-border: #d1d9e0;
    --tonk-code-active-line: #f6f8fa;
    --tonk-code-gutter-bg: #f6f8fa;
    --tonk-code-cursor: #0969da;
    --tonk-code-selection: #0969da33;
    --tonk-code-bracket-match-bg: #0969da26;
    --tonk-code-bracket-match-border: #0969da66;
    --tonk-code-focus-ring: #0969da66;

    /* Lint / hover tooltip surfaces. Default to a slightly
       raised neutral so the tooltip reads as a separate UI
       layer from the editor body. */
    --tonk-code-tooltip-bg: #ffffff;
    --tonk-code-tooltip-fg: #1f2328;
    --tonk-code-tooltip-border: #d1d9e0;

    /* Syntax — GitHub light primer values */
    --tonk-code-key: #cf222e;            /* keyword red */
    --tonk-code-string: #0a3069;         /* string blue */
    --tonk-code-number: #0550ae;         /* constant blue */
    --tonk-code-comment: #59636e;        /* comment grey */
    --tonk-code-punctuation: #1f2328;    /* foreground */
    --tonk-code-error: #cf222e;          /* error red */

    /* Dialect decorations (used by dialog-yaml and similar
       dialect packs). Defaults reuse adjacent slot colors so the
       dialect tokens always read in-palette without the consumer
       setting them explicitly. */
    --tonk-code-variable: var(--tonk-code-fg-muted);
    --tonk-code-entity: var(--tonk-code-fg-muted);
    --tonk-code-name: var(--tonk-code-key);
    --tonk-code-name-sigil: var(--tonk-code-fg-muted);
    --tonk-code-effect: var(--tonk-code-error);
    --tonk-code-keyword: var(--tonk-code-key);

    display: block;
    position: relative;
    box-sizing: border-box;
    min-height: var(--tonk-code-min-height);
    background: var(--tonk-code-bg);
    color: var(--tonk-code-fg);
    border: 1px solid var(--tonk-code-border);
    border-radius: var(--tonk-code-radius);
    overflow: hidden;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  @media (prefers-color-scheme: dark) {
    :host {
      /* GitHub dark defaults */
      --tonk-code-bg: #0d1117;
      --tonk-code-fg: #f0f6fc;
      --tonk-code-fg-muted: #9198a1;
      --tonk-code-border: #3d444d;
      --tonk-code-active-line: #151b23;
      --tonk-code-gutter-bg: #0d1117;
      --tonk-code-cursor: #1f6feb;
      --tonk-code-selection: #1f6feb59;
      --tonk-code-bracket-match-bg: #1f6feb33;
      --tonk-code-bracket-match-border: #1f6feb99;
      --tonk-code-focus-ring: #1f6feb99;

      --tonk-code-tooltip-bg: #151b23;
      --tonk-code-tooltip-fg: #f0f6fc;
      --tonk-code-tooltip-border: #3d444d;

      --tonk-code-key: #ff7b72;          /* GitHub dark keyword */
      --tonk-code-string: #a5d6ff;       /* string light-blue */
      --tonk-code-number: #79c0ff;       /* constant blue */
      --tonk-code-comment: #9198a1;
      --tonk-code-punctuation: #f0f6fc;
      --tonk-code-error: #ff7b72;
    }
  }

  :host([hidden]) { display: none; }

  /* No focus ring on the host. A cell takes focus whenever the caret
     enters it, and a blue border plus a 2px halo reads as a form field
     being validated rather than as a cursor. The editor already carries
     its own caret, and the notebook's block highlight says which block
     you are in; a third signal on top of those is noise. */
  :host(:focus-within) {
    border-color: var(--tonk-code-border);
  }

  .cm-editor { height: 100%; }
`;


/** What a host hands `createEditor`. Everything but `doc` defaults
 *  to off / empty. */
export type EditorOptions = {
  /** The initial document. */
  doc: string;
  readOnly?: boolean;
  placeholder?: string;
  lineNumbers?: boolean;
  activeLine?: boolean;
  /** After every edit that is not a `setValue`: the user's, or a
   *  transaction dispatched on `view` directly. */
  onChange?: (update: ViewUpdate) => void;
  /** After every update, edits or not (selection, effects). The LSP
   *  host reads fresh diagnostic frames off it. */
  onUpdate?: (update: ViewUpdate) => void;
  /** Shift+Enter or Mod+Enter. The key is consumed either way. */
  onRun?: () => void;
};

/** A mounted editor. Each setter reconfigures one compartment, so a
 *  toggle never rebuilds the state or loses undo history. */
export type Editor = {
  readonly view: EditorView;
  /** Replace the document, patching only the span that differs so the
   *  caret stays put. Does not call `onChange`. */
  setValue(next: string): void;
  setLanguage(extension: Extension): void;
  setReadOnly(readOnly: boolean): void;
  setPlaceholder(text: string): void;
  setLineNumbers(on: boolean): void;
  setActiveLine(on: boolean): void;
  setLsp(extension: Extension): void;
  destroy(): void;
};

/** Mount an editor into `root`, a shadow root the host owns.
 *
 *  A shadow root rather than any parent: CodeMirror mounts its styles
 *  on the root node of the editor's DOM, and only a root the editor
 *  renders into is the right context wherever the host is slotted. */
export function createEditor(root: ShadowRoot, options: EditorOptions): Editor {
  const language = new Compartment();
  const readOnly = new Compartment();
  const placeholder = new Compartment();
  const gutter = new Compartment();
  const activeLine = new Compartment();
  const lsp = new Compartment();
  let suppressChange = false;

  // Shift+Enter and Mod+Enter run and consume the key, so the editor
  // does not also insert a newline. First, so it wins against
  // `defaultKeymap`'s Enter. `preventDefault` too: inside a `<form>`
  // some browsers still let the key reach the implicit submit.
  const run = () => {
    options.onRun?.();
    return true;
  };
  const submitKeymap = keymap.of([
    { key: "Shift-Enter", run, preventDefault: true, stopPropagation: true },
    { key: "Mod-Enter", run, preventDefault: true, stopPropagation: true },
  ]);

  const state = EditorState.create({
    doc: options.doc,
    extensions: [
      submitKeymap,
      ...baseExtensions(),
      language.of([]),
      readOnly.of(readOnlyExtensions(options.readOnly ?? false)),
      placeholder.of(options.placeholder ? placeholderExt(options.placeholder) : []),
      gutter.of(options.lineNumbers ? gutterExtensions() : []),
      activeLine.of(options.activeLine ? activeLineExtensions() : []),
      lsp.of([]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged && !suppressChange) options.onChange?.(update);
        options.onUpdate?.(update);
      }),
    ],
  });

  const view = new EditorView({ state, root, parent: root });
  const reconfigure = (compartment: Compartment, extension: Extension) =>
    view.dispatch({ effects: compartment.reconfigure(extension) });

  return {
    view,
    setValue(next) {
      const prev = view.state.doc.toString();
      if (next === prev) return;
      // Replace only the span that differs. A whole-document replace
      // maps the caret to one end of the buffer, so an out-of-band write
      // (another replica editing this cell) would throw the author's
      // caret out of the line they were typing in.
      let start = 0;
      const shared = Math.min(prev.length, next.length);
      while (start < shared && prev.charCodeAt(start) === next.charCodeAt(start)) {
        start++;
      }
      let prevEnd = prev.length;
      let nextEnd = next.length;
      while (
        prevEnd > start &&
        nextEnd > start &&
        prev.charCodeAt(prevEnd - 1) === next.charCodeAt(nextEnd - 1)
      ) {
        prevEnd--;
        nextEnd--;
      }
      suppressChange = true;
      try {
        view.dispatch({
          changes: { from: start, to: prevEnd, insert: next.slice(start, nextEnd) },
        });
      } finally {
        suppressChange = false;
      }
    },
    setLanguage: (extension) => reconfigure(language, extension),
    setReadOnly: (on) => reconfigure(readOnly, readOnlyExtensions(on)),
    setPlaceholder: (text) => reconfigure(placeholder, text ? placeholderExt(text) : []),
    setLineNumbers: (on) => reconfigure(gutter, on ? gutterExtensions() : []),
    setActiveLine: (on) => reconfigure(activeLine, on ? activeLineExtensions() : []),
    setLsp: (extension) => reconfigure(lsp, extension),
    destroy: () => view.destroy(),
  };
}

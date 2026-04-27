// `<tonk-code>` — CodeMirror 6 packaged as a custom element.
//
// Attributes (all reflected on the host element):
//   value     — initial document text. Reactive: writes after
//               connect replace the editor buffer in-place.
//   mode      — language id (e.g. "yaml"). Loaded lazily as a
//               separate ESM chunk; missing chunks fail soft and
//               leave the editor in plain-text mode.
//   readonly  — boolean attribute. Presence locks the editor.
//   placeholder — ghost text shown while the document is empty.
//
// Properties (richer than the attribute surface):
//   .value : string                — round-trips with the buffer.
//                                    Setting moves the cursor to
//                                    end-of-document.
//
// Events:
//   change   — CustomEvent<{value, doc}>. Fires after every user
//              edit (programmatic `.value` writes do not refire).
//              `doc` is the live `Text` instance for callers that
//              want incremental access without re-stringifying.
//   ready    — fires once when the editor view has been mounted.
//              Useful for callers that want to call imperative
//              methods immediately after insertion.
//
// Theming: the element's host renders with the surrounding page
// font and color tokens. CodeMirror's syntax classes are styled
// by a tiny inline theme so the editor inherits the page's color
// scheme rather than baking in a CM theme.

import { EditorState, Compartment, type Extension } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  placeholder as placeholderExt,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { getClient as getLspClient } from "./lsp/client";

/** Detail object dispatched on the `change` event. */
export type ChangeDetail = {
  /** Full document text after the edit. */
  value: string;
  /** Live `Text` reference — caller can read line counts, slice
   *  ranges, etc. without paying for a full string materialization. */
  doc: import("@codemirror/state").Text;
};

/** Detail object dispatched on the `ready` event. */
export type ReadyDetail = {
  /** The CodeMirror view. Escape hatch for callers that want to
   *  reach in and dispatch transactions directly. Treat this as a
   *  power-user surface — most consumers should stick to attrs. */
  view: EditorView;
};

const OBSERVED = [
  "value",
  "mode",
  "readonly",
  "placeholder",
  "line-numbers",
  "language-server",
] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** Cache of in-flight / resolved language module loads, keyed by
 *  the `mode` attribute string. Shared across instances so two
 *  `<tonk-code mode="yaml">` elements don't fetch the chunk twice. */
const languageCache = new Map<string, Promise<Extension>>();

/** Resolve the URL of a sibling chunk relative to this module.
 *  `import.meta.url` is the URL `tonk-code.js` was loaded from,
 *  so language chunks live next to it regardless of where the
 *  consumer mounted the bundle (root, /assets/, CDN, …). */
function languageUrl(mode: string): string {
  return new URL(`./tonk-code-lang-${mode}.js`, import.meta.url).href;
}

async function loadLanguage(mode: string): Promise<Extension> {
  let pending = languageCache.get(mode);
  if (!pending) {
    pending = import(/* @vite-ignore */ languageUrl(mode))
      .then((mod) => mod.default as Extension)
      .catch((err) => {
        // Don't poison the cache on failure — a follow-up attempt
        // (e.g. after a network hiccup) deserves a fresh import.
        languageCache.delete(mode);
        throw err;
      });
    languageCache.set(mode, pending);
  }
  return pending;
}

/** Theme applied inside the editor's own DOM.
 *
 *  Every visual choice is routed through a `--tonk-code-*` CSS
 *  variable. The element ships GitHub-flavored defaults for those
 *  variables (see the per-shadow stylesheet in the constructor) that
 *  adapt to `prefers-color-scheme`. Consumers wire the element into
 *  their own design system by overriding the variables on the host:
 *
 *      tonk-code {
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

/** Stable extensions present in every editor regardless of mode.
 *
 *  Anything *toggleable* (language pack, read-only, placeholder,
 *  line-numbers gutter) lives in a `Compartment` instead — see the
 *  per-instance setup in `connectedCallback`. */
function baseExtensions(): Extension[] {
  return [
    highlightActiveLine(),
    highlightActiveLineGutter(),
    history(),
    bracketMatching(),
    indentOnInput(),
    syntaxHighlighting(highlightStyle, { fallback: true }),
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    baseTheme,
  ];
}

/** The gutter package: line numbers + the matching active-line
 *  highlighter inside the gutter. They go together — toggling line
 *  numbers off but leaving the active-line gutter on would be a
 *  vestigial column. */
function gutterExtensions(): Extension[] {
  return [lineNumbers()];
}

class TonkCodeElement extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return OBSERVED;
  }

  /** Shadow root we mount the editor into.
   *
   *  Why shadow DOM, even though the element doesn't expose a slot
   *  API: CodeMirror's stylesheet injection goes through `style-mod`,
   *  which mounts rules onto the element's *root node*. With a
   *  light-DOM custom element nested under another shadow host (in
   *  this app, every `<tonk-code>` lands inside `<wa-page>`'s shadow),
   *  the rendered CSS context is the surrounding shadow root rather
   *  than `document`. Styles mounted on `document.adoptedStyleSheets`
   *  never reach the element, the editor renders as un-styled DOM.
   *
   *  Owning a shadow root sidesteps the issue: CM mounts its rules on
   *  *our* shadow, which is the correct rendering context for the
   *  editor's DOM, no matter where in the document tree the host is
   *  slotted. The host element's own baseline style (border, sizing)
   *  still works because it targets the host from outside. */
  readonly #shadow: ShadowRoot;

  /** Created lazily in `connectedCallback` so disconnected hosts
   *  (e.g. constructed but not yet inserted) don't pay for a full
   *  view boot. */
  #view: EditorView | null = null;

  constructor() {
    super();
    // `delegatesFocus` so the host element forwards keyboard focus
    // into the editor's `contenteditable` content area without the
    // consumer having to know the editor lives inside a shadow root.
    this.#shadow = this.attachShadow({ mode: "open", delegatesFocus: true });

    const shadowStyle = document.createElement("style");
    shadowStyle.textContent = SHADOW_STYLESHEET;
    this.#shadow.append(shadowStyle);
  }

  /** Compartments let us swap a single extension (language pack,
   *  read-only flag, placeholder, gutter, language-server) at
   *  runtime without rebuilding the entire state. One per
   *  swappable concern. */
  readonly #language = new Compartment();
  readonly #readOnly = new Compartment();
  readonly #placeholder = new Compartment();
  readonly #gutter = new Compartment();
  readonly #lsp = new Compartment();

  /** Each editor instance gets a stable, unique LSP document URI.
   *  The server uses this to track the document's text across
   *  `didOpen`/`didChange`/`didClose`. We mint it lazily on first
   *  LSP connect so editors that never enable LSP don't burn a
   *  random number on the entropy pool. */
  #lspUri: string | null = null;

  /** Suppress the `change` event during programmatic `.value`
   *  writes so consumers don't see their own writes echoed back. */
  #suppressChange = false;

  /** Most-recently-requested mode — used to ignore stale resolves
   *  when the attribute changes again before a load completes. */
  #pendingMode: string | null = null;

  connectedCallback(): void {
    if (this.#view) return;

    const initialDoc = this.getAttribute("value") ?? "";
    const isReadOnly = this.hasAttribute("readonly");
    const placeholderText = this.getAttribute("placeholder") ?? "";
    const showLineNumbers = this.hasAttribute("line-numbers");
    const languageServer = this.getAttribute("language-server");

    const state = EditorState.create({
      doc: initialDoc,
      extensions: [
        ...baseExtensions(),
        this.#language.of([]),
        this.#readOnly.of(EditorState.readOnly.of(isReadOnly)),
        this.#placeholder.of(
          placeholderText ? placeholderExt(placeholderText) : []
        ),
        this.#gutter.of(showLineNumbers ? gutterExtensions() : []),
        this.#lsp.of(languageServer ? this.#buildLspExtension(languageServer) : []),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return;
          if (this.#suppressChange) return;
          this.dispatchEvent(
            new CustomEvent<ChangeDetail>("change", {
              detail: {
                value: update.state.doc.toString(),
                doc: update.state.doc,
              },
              bubbles: true,
              composed: true,
            })
          );
        }),
      ],
    });

    this.#view = new EditorView({ state, root: this.#shadow, parent: this.#shadow });

    // Apply mode after construction so the failure path (chunk
    // missing) doesn't block element insertion.
    const mode = this.getAttribute("mode");
    if (mode) {
      void this.#applyMode(mode);
    }

    this.dispatchEvent(
      new CustomEvent<ReadyDetail>("ready", {
        detail: { view: this.#view },
        bubbles: true,
        composed: true,
      })
    );
  }

  disconnectedCallback(): void {
    this.#view?.destroy();
    this.#view = null;
  }

  attributeChangedCallback(
    name: ObservedAttr,
    _old: string | null,
    next: string | null
  ): void {
    if (!this.#view) return;
    switch (name) {
      case "value":
        // Reflect attribute -> property only when they diverge,
        // otherwise we'd fight the user's keystrokes (input fires
        // an attribute write back from the consumer's side, etc.).
        if ((next ?? "") !== this.value) {
          this.value = next ?? "";
        }
        break;
      case "mode":
        void this.#applyMode(next);
        break;
      case "readonly":
        this.#view.dispatch({
          effects: this.#readOnly.reconfigure(
            EditorState.readOnly.of(next !== null)
          ),
        });
        break;
      case "placeholder":
        this.#view.dispatch({
          effects: this.#placeholder.reconfigure(
            next ? placeholderExt(next) : []
          ),
        });
        break;
      case "line-numbers":
        // Boolean attribute: presence (`next !== null`) toggles the
        // gutter on. Default is off so the element looks like a
        // form-field by default; full-page editors opt in.
        this.#view.dispatch({
          effects: this.#gutter.reconfigure(
            next !== null ? gutterExtensions() : []
          ),
        });
        break;
      case "language-server":
        // Reconfigure with a fresh LSP plugin for the new
        // languageId, or empty extension to detach. Note: the
        // document URI is sticky across changes — a single
        // `<tonk-code>` always represents one document, even if
        // the consumer hot-swaps language servers. The previous
        // server's `didClose` is the LSPClient's responsibility
        // when its plugin is removed from the editor state.
        this.#view.dispatch({
          effects: this.#lsp.reconfigure(
            next ? this.#buildLspExtension(next) : []
          ),
        });
        break;
    }
  }

  /** Build the CodeMirror extension that wires this editor into
   *  the page-wide LSP client.
   *
   *  Each editor instance owns one synthetic `tonk-buffer://`
   *  document URI minted on first call; the LSP server uses it as
   *  the key for tracking the document's text. The `languageId`
   *  comes from the `language-server` attribute and is what the
   *  server keys validators on (e.g. `"carry-asserted"` →
   *  asserted-notation validator). */
  #buildLspExtension(languageId: string): Extension {
    if (!this.#lspUri) {
      // Crypto-random URI so multiple editors on the same page
      // don't collide. The path doesn't carry meaning beyond
      // identity — the server only stores text under it.
      const id = crypto.randomUUID();
      this.#lspUri = `tonk-buffer:///${id}`;
    }
    const client = getLspClient();
    return client.plugin(this.#lspUri, languageId);
  }

  /** Current document text. */
  get value(): string {
    return this.#view ? this.#view.state.doc.toString() : (this.getAttribute("value") ?? "");
  }

  set value(next: string) {
    if (!this.#view) {
      // Buffer onto the attribute; `connectedCallback` will pick
      // it up. Don't go through `setAttribute` if we'd loop.
      if (this.getAttribute("value") !== next) {
        this.setAttribute("value", next);
      }
      return;
    }
    if (next === this.#view.state.doc.toString()) return;
    this.#suppressChange = true;
    try {
      this.#view.dispatch({
        changes: {
          from: 0,
          to: this.#view.state.doc.length,
          insert: next,
        },
      });
    } finally {
      this.#suppressChange = false;
    }
  }

  async #applyMode(mode: string | null): Promise<void> {
    this.#pendingMode = mode;
    if (!mode) {
      this.#view?.dispatch({
        effects: this.#language.reconfigure([]),
      });
      return;
    }
    try {
      const extension = await loadLanguage(mode);
      // Bail if the consumer flipped to a different mode while we
      // were loading; the second `applyMode` call will install
      // the right pack.
      if (this.#pendingMode !== mode) return;
      this.#view?.dispatch({
        effects: this.#language.reconfigure(extension),
      });
    } catch (err) {
      // Surface but don't throw — a missing language chunk shouldn't
      // crash the host. Editor stays in plain-text mode.
      console.warn(`[tonk-code] failed to load language "${mode}":`, err);
    }
  }
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
 *      tonk-code {
 *        --tonk-code-bg: var(--wa-color-surface-default);
 *        --tonk-code-key: var(--wa-color-brand-on-quiet);
 *      }
 *
 *  The element imports nothing from the consumer's design system;
 *  the variables are the contract. */
const SHADOW_STYLESHEET = `
  :host {
    /* Host frame + sizing */
    --tonk-code-font: ui-monospace, SFMono-Regular, Menlo, Consolas,
                      "Liberation Mono", monospace;
    --tonk-code-font-size: 0.875rem;
    --tonk-code-radius: 6px;
    --tonk-code-min-height: 6.5rem;

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

  :host(:focus-within) {
    border-color: var(--tonk-code-cursor);
    box-shadow: 0 0 0 2px var(--tonk-code-focus-ring);
  }

  .cm-editor { height: 100%; }
`;

if (!customElements.get("tonk-code")) {
  customElements.define("tonk-code", TonkCodeElement);
}

export type { TonkCodeElement };

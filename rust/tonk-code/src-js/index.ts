// `<tonk-code>` — CodeMirror 6 packaged as a custom element.
//
// Attributes (all reflected on the host element):
//   value       — initial document text. Reactive: writes after
//                 connect replace the editor buffer in-place.
//   language    — language identifier (e.g. "yaml", "dialog-yaml").
//                 Drives both syntax highlighting (loads
//                 `tonk-code-lang-<language>.js` lazily) and the
//                 LSP `languageId` reported in `didOpen`. When
//                 omitted the editor renders without highlighting
//                 and reports `plaintext` to the LSP server.
//   source      — document URI for LSP. Required to attach a
//                 language server; the editor announces this URI
//                 via the `tonk-code-connect` event and an
//                 ancestor `<tonk-diagnostics-provider>` opens it
//                 on its shared LSP client. No `source` ⇒ no
//                 LSP attachment, period — the editor stays in
//                 plain text-edit mode.
//   readonly    — boolean attribute. Presence locks the editor.
//   placeholder — ghost text shown while the document is empty.
//   line-numbers— boolean attribute. Presence shows the gutter.
//   active-line — boolean attribute. Presence highlights the row
//                 the cursor sits on. Off by default — short
//                 embedded fields don't need the visual cue.
//
// Dialects (e.g. `dialog-yaml`) ship as their own language packs.
// A dialect chunk typically re-exports the parent grammar plus
// dialect-specific decorations (DID tinting, variable
// highlighting, etc.) so dialect-aware highlighting lives next
// to the dialect's grammar rather than as an after-the-fact
// layer.
//
// Properties:
//   .value : string                — round-trips with the buffer.
//                                    Setting moves the cursor to
//                                    end-of-document.
//
// Methods:
//   .connect(client: LSPClient)    — attach an LSP client supplied
//                                    by an ancestor provider. Wires
//                                    diagnostics, hover, completion
//                                    etc. against the editor's
//                                    `source` URI.
//   .disconnect()                  — detach the LSP integration.
//                                    Editor goes back to plain
//                                    text-edit mode.
//
// Events:
//   change   — CustomEvent<{value, doc}>. Fires after every user
//              edit (programmatic `.value` writes do not refire).
//              `doc` is the live `Text` instance for callers that
//              want incremental access without re-stringifying.
//   ready    — fires once when the editor view has been mounted.
//   run      — CustomEvent<{value, doc}>. Fires when the user
//              presses Shift+Enter or Mod+Enter (Ctrl/Cmd+Enter)
//              in the editor. The keystroke is consumed — no
//              line break is inserted. Lets consumers wire a
//              "run / evaluate / submit" affordance without
//              having to reach into CodeMirror's keymap directly.
//              Named `run` rather than `submit` so it doesn't
//              collide with the native HTMLForm `submit` event
//              when the element sits inside a `<form>`.
//   diagnostics — CustomEvent<{value, doc, count, errorCount}>.
//              Fires every time a fresh diagnostic frame from the
//              LSP server lands. `count` is the total; `errorCount`
//              is the subset whose severity is `error`. `value`
//              and `doc` are the buffer state at the moment of
//              dispatch. The LSP server analyzes the version the
//              client `didChange`'d, so the verdict reflects the
//              text the user just typed (debounced through the
//              client's autoSync). Consumers gate auto-evaluation
//              on this — "no errors → eval the current buffer."
//   tonk-code-connect — CustomEvent<TonkCodeConnectDetail>. Bubbles
//              and composes. Fires on `connectedCallback` and on
//              `source`/`language` changes. Caught by an ancestor
//              `<tonk-diagnostics-provider>`, which calls
//              `event.target.connect(client)` to attach an LSP
//              client. No listener ⇒ no LSP integration.
//   tonk-code-disconnect — CustomEvent<TonkCodeDisconnectDetail>.
//              Bubbles and composes. Fires when the editor's
//              attachment should be torn down (element removed,
//              source/language re-announce). Provider closes the
//              workspace document for `source`.
//
// Theming runs through `--tonk-code-*` CSS custom properties so
// the element stays decoupled from any consumer's design system.

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
import { acceptCompletion, completionKeymap } from "@codemirror/autocomplete";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import { forEachDiagnostic, setDiagnosticsEffect } from "@codemirror/lint";
import { tags as t } from "@lezer/highlight";
import type { LSPClient } from "@codemirror/lsp-client";
// Side-effect import: registers the `<tonk-diagnostics-provider>`
// custom element alongside `<tonk-code>` so a single `<script>`
// tag wires up both elements.
import "./diagnostics-provider";
import { pushDiagnosticsAnnotation } from "./diagnostics-provider";
// Generated by scripts/build.mjs from @codemirror/language-data: a map
// from every language name/alias/extension (lower-cased) to its built
// chunk id. Statically bundled — small string table, no fetch — so
// resolving a `language` attribute to a chunk needs no round-trip.
import { LANGUAGE_CHUNKS } from "./lang/chunks.generated";

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

/** Detail object dispatched on the `run` event. Shape mirrors
 *  `ChangeDetail` so consumers can read the buffer without
 *  reaching back into the element. */
export type RunDetail = ChangeDetail;

/** Detail object dispatched on the `diagnostics` event. Fires
 *  every time a fresh server frame lands, carrying the buffer
 *  the server analyzed plus the live diagnostic counts. Consumers
 *  gate auto-evaluation on `errorCount === 0`. */
export type DiagnosticsDetail = ChangeDetail & {
  /** Total number of diagnostics on the document. */
  count: number;
  /** Subset whose severity is `error`. The other diagnostics
   *  are warnings, info, hints — those should not block submit. */
  errorCount: number;
};

/** Detail object dispatched on the `tonk-code-connect` event.
 *  Bubbling and composed; an ancestor
 *  `<tonk-diagnostics-provider>` catches it and calls
 *  `event.target.connect(client)` to attach its `LSPClient`. */
export type TonkCodeConnectDetail = {
  /** Document URI — what the LSP server sees as the document
   *  identity. Sourced from the editor's `source` attribute. */
  source: string;
  /** LSP `languageId` reported in `didOpen`. Derived from the
   *  `language` attribute. */
  language: string;
};

/** Detail object dispatched on the `tonk-code-disconnect` event.
 *  Bubbling and composed; provider tears down the workspace
 *  document for `source` and (optionally) calls
 *  `event.target.disconnect()`. */
export type TonkCodeDisconnectDetail = {
  /** Document URI being detached. */
  source: string;
};

const OBSERVED = [
  "value",
  "language",
  "readonly",
  "placeholder",
  "line-numbers",
  "active-line",
  "source",
  "auto-focus",
] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** Default LSP `languageId` reported when no `language`
 *  attribute is set. The LSP convention is `plaintext`; a server
 *  is free to refuse `didOpen` for it. */
const DEFAULT_LANGUAGE_ID = "plaintext";

/** Cache of in-flight / resolved language-pack module loads.
 *  Shared across instances so two `<tonk-code language="yaml">`
 *  elements don't fetch the chunk twice. */
const languageCache = new Map<string, Promise<Extension>>();

/** The directory prefix the tonk-code bundle is served from, used to
 *  resolve sibling language chunks. On a normal page this is
 *  `import.meta.url`'s directory (an absolute `http(s)://…/` URL). In the
 *  sealed guest `import.meta.url` is a `blob:` URL (no usable path), so
 *  fall back to the well-known host-relative path `/tonk-code/` — the
 *  guest's overridden `window.fetch` relays a host-relative URL to the
 *  host, which does the real fetch. Always ends in `/`, so a file name
 *  concatenates directly (a host-relative base can't go through `new
 *  URL`, which needs an absolute base). */
function bundleBase(): string {
  try {
    const url = new URL(import.meta.url);
    if (url.protocol === "http:" || url.protocol === "https:") {
      return url.href.replace(/[^/]*$/, "");
    }
  } catch {
    /* fall through */
  }
  return "/tonk-code/";
}

/** Blob-URL cache keyed by chunk file name, shared across language loads
 *  so a `chunk-*.js` fetched for one pack is reused by the next.
 *
 *  In the sealed guest it is SEEDED from `window.__tonkCodeChunks` — the
 *  blob map the runtime bootstrap minted for the element bundle at boot.
 *  Reusing those shared chunks (`@codemirror/state`/`view`/`language`) is
 *  essential: re-minting them for a language pack would create a second
 *  `@codemirror/state`, and CodeMirror's `instanceof` checks reject the
 *  pack ("Unrecognized extension value … multiple instances of
 *  @codemirror/state"). On a normal page the map is absent and real URLs
 *  already share one identity. */
const chunkBlobs = new Map<string, string>();
(() => {
  const seed = (globalThis as { __tonkCodeChunks?: Record<string, string> })
    .__tonkCodeChunks;
  if (seed) for (const name of Object.keys(seed)) chunkBlobs.set(name, seed[name]);
})();

/** Relative `./name.js` specifiers a chunk imports (both quote styles). */
function relativeImports(src: string): string[] {
  const out: string[] = [];
  const re = /['"]\.\/([^'"$]+)['"]/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src))) if (!out.includes(m[1])) out.push(m[1]);
  return out;
}

/** Fetch a code-split chunk graph starting at `entry` (a file name under
 *  `base`), minting a blob per file with its `./chunk-*.js` imports
 *  rewritten to the deps' blob URLs, and return the entry's blob URL.
 *  Walks the DAG by fetching each file and recursing into its unmet deps;
 *  shared chunks are cached in `chunkBlobs` so they're fetched once. Uses
 *  `window.fetch`, which the sealed guest overrides to relay host-relative
 *  URLs — so this works identically on a plain page and inside the guest,
 *  with no bridge protocol. */
async function mintChunkGraph(base: string, entry: string): Promise<string> {
  const srcByName = new Map<string, string>();

  async function fetchAll(name: string): Promise<void> {
    if (srcByName.has(name) || chunkBlobs.has(name)) return;
    // `base` ends in `/` and `name` is a bare file name; concatenate
    // directly. A host-relative base (`/tonk-code/`, the sealed guest)
    // can't go through `new URL` (no absolute base), and the guest's
    // fetch override needs the host-relative path anyway.
    const res = await fetch(base + name);
    if (!res.ok) throw new Error(`fetch ${name}: ${res.status}`);
    const src = await res.text();
    srcByName.set(name, src);
    await Promise.all(
      relativeImports(src).map((dep) => fetchAll(dep)),
    );
  }
  await fetchAll(entry);

  // Mint in dependency order: repeatedly mint any file whose deps are all
  // resolved (already-cached or minted this pass) until none remain.
  let pending = [...srcByName.keys()].filter((n) => !chunkBlobs.has(n));
  let guard = 0;
  while (pending.length && guard++ < 40) {
    const next: string[] = [];
    for (const name of pending) {
      const deps = relativeImports(srcByName.get(name)!).filter((d) =>
        srcByName.has(d) || chunkBlobs.has(d),
      );
      if (!deps.every((d) => chunkBlobs.has(d))) {
        next.push(name);
        continue;
      }
      let out = srcByName.get(name)!;
      for (const dep of deps) {
        const url = chunkBlobs.get(dep)!;
        out = out.split(`"./${dep}"`).join(`"${url}"`);
        out = out.split(`'./${dep}'`).join(`'${url}'`);
      }
      chunkBlobs.set(
        name,
        URL.createObjectURL(new Blob([out], { type: "text/javascript" })),
      );
    }
    pending = next;
  }
  const url = chunkBlobs.get(entry);
  if (!url) throw new Error(`could not mint ${entry}`);
  return url;
}

/** Load a language pack on demand. The `language` attribute (e.g. `lisp`,
 *  `python`, or an alias/extension like `js`/`rs`) resolves through the
 *  generated alias map (LANGUAGE_CHUNKS, statically bundled so it needs no
 *  fetch) to a built chunk id, then that chunk's graph is fetched and
 *  blob-imported — one fetch per file, only when a block actually uses the
 *  language, shared chunks reused across languages. Chunks are generated
 *  from the CodeMirror `@codemirror/language-data` catalog plus our custom
 *  `dialog-yaml` dialect (scripts/build.mjs), so common languages all
 *  resolve. An unknown id rejects; the caller logs and leaves the editor
 *  plain. */
async function loadLanguage(language: string): Promise<Extension> {
  let pending = languageCache.get(language);
  if (!pending) {
    const chunkId = LANGUAGE_CHUNKS[language.toLowerCase()];
    if (!chunkId) {
      return Promise.reject(new Error(`no language pack for "${language}"`));
    }
    pending = mintChunkGraph(bundleBase(), `tonk-code-lang-${chunkId}.js`)
      .then((url) => import(/* @vite-ignore */ url))
      .then((mod) => mod.default as Extension)
      .catch((err) => {
        // Don't poison the cache on failure — a follow-up attempt
        // (e.g. after a network hiccup) deserves a fresh import.
        languageCache.delete(language);
        throw err;
      });
    languageCache.set(language, pending);
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
 *  instead — see the per-instance setup in `connectedCallback`. */
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
    // ArrowUp/Down and Ctrl-n/p move the selection, Enter accepts,
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
   *  read-only flag, placeholder, gutter, active-line, lsp
   *  plugin) at runtime without rebuilding the entire state. One
   *  per swappable concern. */
  readonly #language = new Compartment();
  readonly #readOnly = new Compartment();
  readonly #placeholder = new Compartment();
  readonly #gutter = new Compartment();
  readonly #activeLine = new Compartment();
  readonly #lsp = new Compartment();

  /** Active LSP client supplied by an ancestor
   *  `<tonk-diagnostics-provider>` via `connect()`. The element
   *  itself never builds or owns a client — diagnostics, hover,
   *  completion etc. require a provider in scope. */
  #lspClient: LSPClient | null = null;

  /** Suppress the `change` event during programmatic `.value`
   *  writes so consumers don't see their own writes echoed back. */
  #suppressChange = false;

  /** Fire the `run` CustomEvent. Called from the
   *  Shift+Enter / Mod+Enter keymap. Returns nothing — the
   *  keymap binding consumes the key regardless of whether
   *  there's a listener. */
  #dispatchRun(): void {
    const view = this.#view;
    if (!view) return;
    this.dispatchEvent(
      new CustomEvent<RunDetail>("run", {
        detail: {
          value: view.state.doc.toString(),
          doc: view.state.doc,
        },
        bubbles: true,
        composed: true,
      })
    );
  }

  /** Latest diagnostic counts from the most recent server frame.
   *  Initialized to 0 — the server hasn't spoken yet, so by
   *  convention we treat the buffer as clean. */
  #lastDiagnosticTotal = 0;
  #lastDiagnosticErrors = 0;

  /** Public getter — total diagnostic count on the current
   *  document. Surfaces to consumers who'd rather poll than
   *  subscribe to the `diagnostics` event. */
  get errorCount(): number {
    return Math.max(0, this.#lastDiagnosticErrors);
  }

  /** The CodeMirror view, if mounted. Public so the diagnostics
   *  provider can dispatch lint-state effects directly when
   *  routing pushed diagnostics. Treat as a power-user surface —
   *  most consumers should stick to events and attributes. */
  get view(): EditorView | null {
    return this.#view;
  }


  /** Fire the `diagnostics` event when a fresh server frame
   *  lands. "Fresh" = the transaction carries `setDiagnosticsEffect`,
   *  which the LSP client only dispatches for `publishDiagnostics`
   *  frames whose `version` matches the file's current version
   *  (mismatched frames are dropped per LSP spec). So every fresh
   *  frame provably reflects a buffer the user actually typed —
   *  exactly the signal a consumer needs to auto-evaluate.
   *
   *  Doc-edit updates (no diagnostic effect, just range remaps as
   *  the user types) don't fire the event: those reflect the
   *  *previous* server verdict on the *new* buffer, which is
   *  exactly the staleness we want to avoid.
   *
   *  `forEachDiagnostic` is cheap — walks a small in-memory range
   *  tree. */
  #dispatchDiagnostics(): void {
    const view = this.#view;
    if (!view) return;
    let total = 0;
    let errors = 0;
    forEachDiagnostic(view.state, (d) => {
      total += 1;
      if (d.severity === "error") errors += 1;
    });
    this.#lastDiagnosticTotal = total;
    this.#lastDiagnosticErrors = errors;
    this.dispatchEvent(
      new CustomEvent<DiagnosticsDetail>("diagnostics", {
        detail: {
          value: view.state.doc.toString(),
          doc: view.state.doc,
          count: total,
          errorCount: errors,
        },
        bubbles: true,
        composed: true,
      }),
    );
  }

  /** Most-recently-requested language — used to ignore stale
   *  resolves when the attribute changes again before a load
   *  completes. */
  #pendingLanguage: string | null = null;

  /** Pending teardown scheduled by `disconnectedCallback`. The
   *  reactive frameworks we run under (Leptos, mostly) routinely
   *  detach and re-attach DOM nodes during rerenders. The custom-
   *  element spec fires `disconnectedCallback` immediately on
   *  detach — if we tore the editor down right then, every parent
   *  rerender would destroy and rebuild the view, drop the LSP
   *  session, and reset the document. We defer the teardown to
   *  the next microtask; if a `connectedCallback` runs first, we
   *  cancel. */
  #teardownScheduled = false;

  connectedCallback(): void {
    // If we were about to tear down (Leptos move), cancel — the
    // re-attach happened first.
    this.#teardownScheduled = false;
    if (this.#view) return;

    const initialDoc = this.getAttribute("value") ?? "";
    const isReadOnly = this.hasAttribute("readonly");
    const placeholderText = this.getAttribute("placeholder") ?? "";
    const showLineNumbers = this.hasAttribute("line-numbers");
    const showActiveLine = this.hasAttribute("active-line");

    // Submit keymap: Shift+Enter and Mod+Enter both fire the
    // `run` CustomEvent and consume the key (returning `true`
    // tells CodeMirror's keymap dispatcher to stop) so the
    // editor doesn't *also* insert a newline.
    //
    // Belt-and-braces `preventDefault` on the `keydown` itself:
    // when `<tonk-code>` sits inside a `<form>`, some browsers
    // can still bubble the key event up to the form's implicit
    // submit handler before CodeMirror's own preventDefault
    // runs — manifesting as a page-top scroll on Shift+Enter.
    // Catching keydown at the host's bubble phase (after CM's
    // capture-phase handler) lets us swallow it before the
    // form's submit logic kicks in.
    const submitKeymap = keymap.of([
      {
        key: "Shift-Enter",
        run: () => {
          this.#dispatchRun();
          return true;
        },
        preventDefault: true,
        stopPropagation: true,
      },
      {
        key: "Mod-Enter",
        run: () => {
          this.#dispatchRun();
          return true;
        },
        preventDefault: true,
        stopPropagation: true,
      },
    ]);

    const state = EditorState.create({
      doc: initialDoc,
      extensions: [
        // Submit keymap first so it wins against `defaultKeymap`'s
        // Enter binding (which `splitLine`s).
        submitKeymap,
        ...baseExtensions(),
        this.#language.of([]),
        this.#readOnly.of(readOnlyExtensions(isReadOnly)),
        this.#placeholder.of(
          placeholderText ? placeholderExt(placeholderText) : []
        ),
        this.#gutter.of(showLineNumbers ? gutterExtensions() : []),
        this.#activeLine.of(showActiveLine ? activeLineExtensions() : []),
        this.#lsp.of([]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && !this.#suppressChange) {
            this.dispatchEvent(
              new CustomEvent<ChangeDetail>("change", {
                detail: {
                  value: update.state.doc.toString(),
                  doc: update.state.doc,
                },
                bubbles: true,
                composed: true,
              }),
            );
          }
          // Fire `diagnostics` only when a fresh server frame
          // lands — a transaction carrying `setDiagnosticsEffect`
          // from `@codemirror/lsp-client`'s `publishDiagnostics`
          // handler. The client drops mismatched-version frames
          // per LSP spec, so every fresh frame provably reflects
          // a buffer version the server actually analyzed. The
          // user's intervening keystrokes haven't been seen by
          // the server yet (they're in `client.unsyncedChanges`,
          // waiting for the next `autoSync` flush) — the next
          // server roundtrip will produce a new frame, and we'll
          // fire `diagnostics` again then.
          //
          // Doc-edit updates (no diagnostic effect, just CodeMirror
          // remapping diagnostic ranges through the change) carry
          // the *previous* verdict on the *new* buffer — that's
          // staleness, so we don't fire on them.
          //
          // `<tonk-diagnostics-provider>`'s `tonk-push-diagnostics`
          // path *also* dispatches `setDiagnosticsEffect` (via
          // `@codemirror/lint`'s `setDiagnostics`) — those are our
          // own writes, not server frames. The provider tags those
          // transactions with `pushDiagnosticsAnnotation`; skip
          // them, otherwise the clear-on-success a consumer issues
          // after auto-evaluating would be misread as a fresh
          // frame and loop the auto-evaluate.
          const freshFrame = update.transactions.some(
            (tr) =>
              tr.annotation(pushDiagnosticsAnnotation) === undefined &&
              tr.effects.some((e) => e.is(setDiagnosticsEffect)),
          );
          if (freshFrame) {
            this.#dispatchDiagnostics();
          }
        }),
      ],
    });

    this.#view = new EditorView({
      state,
      root: this.#shadow,
      parent: this.#shadow,
    });

    // Bridge host focus → contenteditable focus. With
    // `delegatesFocus: true`, focusing the host normally
    // delegates to the *first focusable* shadow descendant —
    // which inside CodeMirror's DOM is `cm-scroller` (it has
    // tabindex), not the `cm-content` contenteditable. Listen
    // for `focus` on the host and explicitly call
    // `view.focus()`, so however the host gains focus (Tab key,
    // programmatic `host.focus()`, a click anywhere in the
    // editor frame), focus reliably lands on the contenteditable.
    //
    // Bonus: also recovers from any sync-mount blur (e.g. an LSP
    // plugin reconfigure later in the same `connectedCallback`
    // run) — the next time anything re-focuses the host, we end
    // up in the right place.
    const view = this.#view;
    this.addEventListener("focus", () => view.focus());

    // Apply language pack after construction so the failure path
    // (chunk missing, network blip) doesn't block element insertion.
    const language = this.getAttribute("language");
    if (language) {
      void this.#applyLanguage(language);
    }

    // `autofocus` mirrors the standard HTML attribute — focus
    // the editor on mount when present. Skipped for read-only
    // editors — they don't need a caret.
    //
    // Standard HTML autofocus only honors the *first* element
    // with the attribute on a page — later autofocus targets
    // Announce first so any ancestor `<tonk-diagnostics-provider>`
    // hands us its LSP client — `connect()` reconfigures the
    // editor's lsp compartment, which can clear focus on the
    // contenteditable. Firing `ready` *after* lets focus
    // handlers land focus that won't get clobbered by the
    // ensuing LSP attach.
    //
    // No provider in scope ⇒ no listener consumes the event ⇒
    // editor stays in plain text-edit mode. There is no
    // standalone-LSP fallback by design.
    this.#announceConnect();

    this.dispatchEvent(
      new CustomEvent<ReadyDetail>("ready", {
        detail: { view: this.#view },
        bubbles: true,
        composed: true,
      })
    );

    // `auto-focus` is a non-standard attribute (the standard
    // `autofocus` would be parsed by the browser on the host,
    // delegating focus through `delegatesFocus: true` to the
    // first focusable shadow descendant — `cm-scroller` rather
    // than the contenteditable, which is the wrong target).
    //
    // The setTimeout(0) defers focus to the next macrotask. Any
    // sibling editor's mount + sync LSP attach finishes before
    // our focus call lands, so the contenteditable focus
    // doesn't get clobbered by a sibling's reconfigure.
    if (!isReadOnly && this.hasAttribute("auto-focus")) {
      const view = this.#view;
      setTimeout(() => view.focus(), 0);
    }
  }

  disconnectedCallback(): void {
    // Defer the actual destruction to the next macrotask (not
    // microtask — Leptos's re-insert can happen synchronously
    // *or* across a microtask boundary). If the host gets
    // re-attached first, `connectedCallback` flips
    // `#teardownScheduled` off and the work is skipped.
    if (this.#teardownScheduled) return;
    this.#teardownScheduled = true;
    setTimeout(() => {
      if (!this.#teardownScheduled) return;
      this.#teardownScheduled = false;
      // Only tear down if we're still detached. `isConnected`
      // flips back to `true` if Leptos re-inserted us.
      if (this.isConnected) return;
      this.#announceDisconnect();
      this.#view?.destroy();
      this.#view = null;
    }, 0);
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
      case "language":
        // Apply the language pack (highlighting). LSP `languageId`
        // is also keyed off this attribute; if a provider has us
        // attached we re-announce so the provider can `didOpen`
        // the same source under the new languageId.
        void this.#applyLanguage(next);
        if (this.#lspClient) {
          this.#announceDisconnect();
          this.#announceConnect();
        }
        break;
      case "readonly":
        this.#view.dispatch({
          effects: this.#readOnly.reconfigure(readOnlyExtensions(next !== null)),
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
      case "active-line":
        this.#view.dispatch({
          effects: this.#activeLine.reconfigure(
            next !== null ? activeLineExtensions() : []
          ),
        });
        break;
      case "source":
        // The document's identity changed — tear down the
        // previous LSP attachment (if any) and re-announce so
        // the provider opens the new source. No-op if no
        // provider is in scope.
        if (this.#lspClient) {
          this.#announceDisconnect();
        }
        this.#announceConnect();
        break;
      case "auto-focus":
        // Applied at mount only. Post-mount writes are no-op —
        // adding it via JS later is uncommon enough that we'd
        // rather not steal focus from wherever the user has it.
        break;
    }
  }

  /** LSP languageId reported to the server in `didOpen`. Derived
   *  from the `language` attribute; falls back to `plaintext`. */
  #lspLanguageId(): string {
    return this.getAttribute("language") ?? DEFAULT_LANGUAGE_ID;
  }

  /** Bubbling, composed event the editor fires on connect (and
   *  on `source` / `language` changes mid-life). An ancestor
   *  `<tonk-diagnostics-provider>` catches the event and calls
   *  `connect()` on `event.target` with its `LSPClient`.
   *
   *  No provider in scope ⇒ no listener responds ⇒ the editor
   *  stays in plain text-edit mode. There is no fallback.  */
  #announceConnect(): void {
    const source = this.getAttribute("source");
    if (!source) return;
    this.dispatchEvent(
      new CustomEvent<TonkCodeConnectDetail>("tonk-code-connect", {
        detail: { source, language: this.#lspLanguageId() },
        bubbles: true,
        composed: true,
      }),
    );
  }

  /** Bubbling, composed event the editor fires when its current
   *  attachment should be torn down — on element removal, or
   *  before re-announcing under a different `source` /
   *  `language`. Provider closes the LSP document for `source`. */
  #announceDisconnect(): void {
    const source = this.getAttribute("source");
    if (!source) return;
    this.dispatchEvent(
      new CustomEvent<TonkCodeDisconnectDetail>("tonk-code-disconnect", {
        detail: { source },
        bubbles: true,
        composed: true,
      }),
    );
  }

  /** Public API used by an ancestor `<tonk-diagnostics-provider>`
   *  to attach an `LSPClient` to this editor. The provider supplies
   *  the client; the editor wires it into its CodeMirror extension
   *  set so diagnostics, hover, completion etc. start working
   *  against the editor's `source` URI.
   *
   *  Idempotent: calling `connect` twice with the same client just
   *  reconfigures the extension. Calling with a different client
   *  swaps the attachment.  */
  connect(client: LSPClient): void {
    if (!this.#view) return;
    const source = this.getAttribute("source");
    if (!source) return;
    this.#lspClient = client;
    this.#view.dispatch({
      effects: this.#lsp.reconfigure(
        client.plugin(source, this.#lspLanguageId()),
      ),
    });
  }

  /** Detach the LSP integration. The provider calls this in
   *  response to a `tonk-code-disconnect` event (when the editor
   *  goes away or moves to a new source). The editor doesn't
   *  destroy the client — that's the provider's lifecycle. */
  disconnect(): void {
    this.#lspClient = null;
    if (this.#view) {
      this.#view.dispatch({
        effects: this.#lsp.reconfigure([]),
      });
    }
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

  async #applyLanguage(language: string | null): Promise<void> {
    this.#pendingLanguage = language;
    if (!language) {
      this.#view?.dispatch({
        effects: this.#language.reconfigure([]),
      });
      return;
    }
    try {
      const extension = await loadLanguage(language);
      // Bail if the consumer flipped to a different language while
      // we were loading; the second `applyLanguage` call will
      // install the right pack.
      if (this.#pendingLanguage !== language) return;
      this.#view?.dispatch({
        effects: this.#language.reconfigure(extension),
      });
    } catch (err) {
      // Surface but don't throw — a missing language chunk shouldn't
      // crash the host. Editor stays without highlighting.
      console.warn(
        `[tonk-code] failed to load language "${language}":`,
        err,
      );
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

if (!customElements.get("tonk-code")) {
  customElements.define("tonk-code", TonkCodeElement);
}

export type { TonkCodeElement };

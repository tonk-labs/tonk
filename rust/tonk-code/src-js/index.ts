// `<tonk-code>` — CodeMirror 6 packaged as a custom element.
//
// Attributes (all reflected on the host element):
//   value       — initial document text. Reactive: writes after
//                 connect replace the editor buffer in-place.
//   language    — language identifier (e.g. "yaml", "dialog-yaml").
//                 Drives both syntax highlighting (loads
//                 `tonk-code-lang-<language>.js` lazily) and the
//                 LSP `languageId` reported to the server in
//                 `didOpen`. When omitted the editor renders
//                 without highlighting and reports `plaintext` to
//                 the LSP server.
//   language-server — opt-in to a language server. Boolean
//                 (presence alone connects to the default
//                 endpoint `/api/language-server`); a value
//                 (`language-server="/some/path"` or
//                 `language-server="https://host/path"`) is the
//                 server URL, resolved against `document.baseURI`
//                 the same way an `<a href>` would.
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
// Events:
//   change   — CustomEvent<{value, doc}>. Fires after every user
//              edit (programmatic `.value` writes do not refire).
//              `doc` is the live `Text` instance for callers that
//              want incremental access without re-stringifying.
//   ready    — fires once when the editor view has been mounted.
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
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { LSPClient } from "@codemirror/lsp-client";
import { connectLsp } from "./lsp/client";
import { httpTransport } from "./lsp/transport";

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
  "language",
  "readonly",
  "placeholder",
  "line-numbers",
  "active-line",
  "language-server",
] as const;
type ObservedAttr = (typeof OBSERVED)[number];

/** Default endpoint used when the `language-server` attribute is
 *  present with no value. Same-origin and relative — so the
 *  request goes through whatever fetch handler intercepts it
 *  (a service worker, a real backend, etc.). */
const DEFAULT_LANGUAGE_SERVER_URL = "/api/language-server";

/** Default LSP `languageId` reported when no `language`
 *  attribute is set. The LSP convention is `plaintext`; a server
 *  is free to refuse `didOpen` for it. */
const DEFAULT_LANGUAGE_ID = "plaintext";

/** How long to wait before rebuilding the LSP client after the
 *  current session ends. The transport is single-shot — when its
 *  inbound stream closes the element rebuilds rather than
 *  papering over the disconnect. We start with a polite 5s delay
 *  to give the underlying server (typically a service worker
 *  being upgraded) time to come back up before reconnecting,
 *  then exponentially back off if the rebuild keeps failing. */
const LSP_RECONNECT_INITIAL_MS = 5_000;
const LSP_RECONNECT_MAX_MS = 30_000;

/** Cache of in-flight / resolved language-pack module loads.
 *  Shared across instances so two `<tonk-code language="yaml">`
 *  elements don't fetch the chunk twice. */
const languageCache = new Map<string, Promise<Extension>>();

/** Resolve the URL of a sibling chunk relative to this module.
 *  `import.meta.url` is the URL `tonk-code.js` was loaded from,
 *  so language chunks live next to it regardless of where the
 *  consumer mounted the bundle (root, /assets/, CDN, …). */
function languageUrl(language: string): string {
  return new URL(`./tonk-code-lang-${language}.js`, import.meta.url).href;
}

async function loadLanguage(language: string): Promise<Extension> {
  let pending = languageCache.get(language);
  if (!pending) {
    pending = import(/* @vite-ignore */ languageUrl(language))
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
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
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

  /** Active LSP client, when one exists. The element manages its
   *  full lifecycle: build on `lsp` attribute set, destroy on
   *  attribute removal or transport drop, rebuild after the
   *  reconnect delay. */
  #lspClient: LSPClient | null = null;

  /** Pending reconnect timer, if any. Cleared on
   *  disconnect/destroy so a teardown doesn't leave a ghost
   *  rebuild scheduled. */
  #lspReconnectTimer: ReturnType<typeof setTimeout> | null = null;

  /** Current reconnect backoff in milliseconds. Resets to
   *  `LSP_RECONNECT_INITIAL_MS` on every successful connect;
   *  doubles, capped at `LSP_RECONNECT_MAX_MS`, on each failed
   *  rebuild attempt. */
  #lspReconnectDelay = LSP_RECONNECT_INITIAL_MS;

  /** Each editor instance gets a stable, unique LSP document URI.
   *  The server uses this to track the document's text across
   *  `didOpen`/`didChange`/`didClose`. We mint it lazily on first
   *  LSP connect so editors that never enable LSP don't burn a
   *  random number on the entropy pool. */
  #lspUri: string | null = null;

  /** Suppress the `change` event during programmatic `.value`
   *  writes so consumers don't see their own writes echoed back. */
  #suppressChange = false;

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
        this.#activeLine.of(showActiveLine ? activeLineExtensions() : []),
        this.#lsp.of([]),
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

    this.#view = new EditorView({
      state,
      root: this.#shadow,
      parent: this.#shadow,
    });

    // Apply language pack after construction so the failure path
    // (chunk missing, network blip) doesn't block element insertion.
    const language = this.getAttribute("language");
    if (language) {
      void this.#applyLanguage(language);
    }

    // Connect to the LSP server lazily — only on first user
    // interaction. Editors that the user never focuses (collapsed
    // sections, hidden tabs, etc.) don't open an SSE channel,
    // don't burn a `didOpen`, don't show up in the server's open-
    // documents map. The first focus is the moment the user
    // actually cares about diagnostics; before that they're
    // wasted bytes.
    if (this.hasAttribute("language-server")) {
      this.#armLazyLspConnect();
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
      this.#tearDownLsp();
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
        // is also keyed off this attribute; if an LSP session is
        // open we rebuild it so the server sees the right id.
        void this.#applyLanguage(next);
        if (this.#lspClient) this.#rebuildLspNow();
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
      case "active-line":
        this.#view.dispatch({
          effects: this.#activeLine.reconfigure(
            next !== null ? activeLineExtensions() : []
          ),
        });
        break;
      case "language-server":
        // Presence enables, absence disables. When *enabling*
        // for the first time, defer to first focus (same lazy
        // policy as the initial mount). When *changing* a URL on
        // an already-connected session, reconnect eagerly so the
        // swap takes effect.
        if (next !== null) {
          if (this.#lspClient) {
            this.#connectLsp(); // URL swap mid-session
          } else {
            this.#armLazyLspConnect();
          }
        } else {
          this.#tearDownLsp();
        }
        break;
    }
  }

  /** LSP languageId reported to the server in `didOpen`. Derived
   *  from the `language` attribute; falls back to `plaintext`. */
  #lspLanguageId(): string {
    return this.getAttribute("language") ?? DEFAULT_LANGUAGE_ID;
  }

  /** Generation counter for the LSP session. Each
   *  `#connectLsp()` increments it; transport callbacks capture
   *  the generation that owned them and ignore events after the
   *  generation has moved on. This prevents a torn-down
   *  transport's `onClose` from racing a fresh `#connectLsp` and
   *  scheduling a rebuild that fights the new session. */
  #lspGeneration = 0;

  /** URL + language id of the currently connected LSP session,
   *  if any. Used to short-circuit `#connectLsp()` when the
   *  desired session matches what's already running — keeps
   *  Leptos remounts from churning the connection. */
  #lspConnectedUrl: string | null = null;
  #lspConnectedLanguageId: string | null = null;

  /** First-focus listener for lazy LSP connect. Held so we can
   *  remove it after firing or when teardown happens before
   *  first focus. */
  #lazyLspListener: (() => void) | null = null;

  /** Arm a one-shot focus listener that connects the LSP on
   *  first user interaction. Editors the user never focuses
   *  don't open an SSE channel.
   *
   *  If `attributeChangedCallback` ends up calling `#connectLsp`
   *  directly (e.g. the consumer flips the `language-server`
   *  value at runtime, signalling explicit intent), the
   *  connect runs immediately and the focus listener is
   *  cancelled by `#tearDownLsp` → `#disarmLazyLspConnect`.  */
  #armLazyLspConnect(): void {
    if (this.#lazyLspListener) return; // already armed
    const fire = () => {
      this.#disarmLazyLspConnect();
      this.#connectLsp();
    };
    this.#lazyLspListener = fire;
    // `focus` with `delegatesFocus: true` on
    // the shadow root, the host receives the `focus` event when
    // focus delegates into the inner contenteditable. `focusin`
    // bubbles independently but the host listener path goes
    // through `focus` here. `once: true` removes the listener
    // after firing.
    this.addEventListener("focus", fire, { once: true });
  }

  #disarmLazyLspConnect(): void {
    if (!this.#lazyLspListener) return;
    this.removeEventListener("focus", this.#lazyLspListener);
    this.#lazyLspListener = null;
  }

  /** Build (or rebuild) the LSP client + transport pair and wire
   *  them into the editor. Idempotent: tears down any existing
   *  client before connecting. Safe to call from
   *  `attributeChangedCallback`. */
  #connectLsp(): void {
    if (!this.#view) return;
    // Short-circuit if we're already connected to the same
    // session. Belt-and-suspenders with the deferred teardown:
    // even if the deferred teardown ran (e.g. element was truly
    // removed and re-added later), this also catches duplicate
    // connect calls from any other path.
    if (this.#lspClient) {
      const desiredUrl = new URL(
        this.getAttribute("language-server") || DEFAULT_LANGUAGE_SERVER_URL,
        document.baseURI,
      ).href;
      if (
        desiredUrl === this.#lspConnectedUrl &&
        this.#lspLanguageId() === this.#lspConnectedLanguageId
      ) {
        return;
      }
    }
    this.#tearDownLsp();

    if (!this.#lspUri) {
      // Crypto-random URI so multiple editors on the same page
      // don't collide. The path doesn't carry meaning beyond
      // identity — the server only stores text under it.
      this.#lspUri = `tonk-buffer:///${crypto.randomUUID()}`;
    }

    // Resolve the URL the same way the browser would resolve an
    // `<a href>` — relative paths bind against `document.baseURI`,
    // absolute URLs (any scheme) pass through. The boolean form
    // (attribute present, empty value) maps to the default path.
    const raw =
      this.getAttribute("language-server") || DEFAULT_LANGUAGE_SERVER_URL;
    const url = new URL(raw, document.baseURI).href;

    const generation = ++this.#lspGeneration;
    const transport = httpTransport({
      url,
      onClose: () => {
        // Ignore the close event if a newer session has taken
        // over. Without this guard, intentional teardowns
        // (`#tearDownLsp`) cascade into a rebuild because the
        // disconnect hits the transport's `onClose` after the
        // new session has already started.
        if (this.#lspGeneration !== generation) return;
        this.#tearDownLsp();
        // The transport is single-shot; closing means the LSP
        // session is over. Schedule a rebuild — the server may
        // be a service worker that briefly disappeared during
        // an upgrade, so a polite delay before reconnecting
        // gives it time to come back.
        this.#scheduleLspReconnect();
      },
    });

    const client = connectLsp(transport);
    this.#lspClient = client;
    this.#lspReconnectDelay = LSP_RECONNECT_INITIAL_MS;
    this.#lspConnectedUrl = url;
    this.#lspConnectedLanguageId = this.#lspLanguageId();

    this.#view.dispatch({
      effects: this.#lsp.reconfigure(
        client.plugin(this.#lspUri, this.#lspConnectedLanguageId),
      ),
    });
  }

  /** Detach the LSP plugin from the editor, destroy the client,
   *  cancel any pending rebuild.
   *
   *  Bumps the generation so any `onClose` from the disconnect
   *  recognizes itself as stale and skips the reconnect path. */
  #tearDownLsp(): void {
    this.#lspGeneration++;
    this.#disarmLazyLspConnect();
    if (this.#lspReconnectTimer !== null) {
      clearTimeout(this.#lspReconnectTimer);
      this.#lspReconnectTimer = null;
    }
    if (this.#view) {
      this.#view.dispatch({
        effects: this.#lsp.reconfigure([]),
      });
    }
    if (this.#lspClient) {
      this.#lspClient.disconnect();
      this.#lspClient = null;
    }
    this.#lspConnectedUrl = null;
    this.#lspConnectedLanguageId = null;
  }

  /** Schedule a rebuild of the LSP client after the current
   *  reconnect delay, then double the delay (capped) for the
   *  next failure. The delay is reset on each successful
   *  rebuild via `#connectLsp`. */
  #scheduleLspReconnect(): void {
    if (!this.hasAttribute("language-server")) return; // attr removed; respect that
    if (this.#lspReconnectTimer !== null) return; // already pending

    const delay = this.#lspReconnectDelay;
    this.#lspReconnectDelay = Math.min(delay * 2, LSP_RECONNECT_MAX_MS);
    this.#lspReconnectTimer = setTimeout(() => {
      this.#lspReconnectTimer = null;
      this.#connectLsp();
    }, delay);
  }

  /** Force an immediate rebuild — used when an attribute change
   *  invalidates the current session (e.g. the `language` value
   *  changed and the server needs to see a new `languageId`). */
  #rebuildLspNow(): void {
    this.#connectLsp();
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

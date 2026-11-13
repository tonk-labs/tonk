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

import type { Extension } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { forEachDiagnostic, setDiagnosticsEffect } from "@codemirror/lint";
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
// The editor itself (state, view, theme, compartments), shared with the
// library's `<code-editor>`.
import { createEditor, SHADOW_STYLESHEET, type Editor } from "./editor";

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

/** Load a language pack on demand. The `language` attribute (e.g. `lisp`,
 *  `python`, or an alias/extension like `js`/`rs`) resolves through the
 *  generated alias map (LANGUAGE_CHUNKS, statically bundled so it needs no
 *  fetch) to a built chunk id, then that chunk is imported from beside this
 *  bundle, only when a block actually uses the language; its shared chunks
 *  resolve to the same URLs as this bundle's, so CodeMirror keeps one
 *  identity. Chunks are generated
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
    const url = new URL(`./tonk-code-lang-${chunkId}.js`, import.meta.url).href;
    pending = import(/* @vite-ignore */ url)
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
  #editor: Editor | null = null;

  get #view(): EditorView | null {
    return this.#editor?.view ?? null;
  }

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

  /** Active LSP client supplied by an ancestor
   *  `<tonk-diagnostics-provider>` via `connect()`. The element
   *  itself never builds or owns a client — diagnostics, hover,
   *  completion etc. require a provider in scope. */
  #lspClient: LSPClient | null = null;
  /** The `source` and language the attached client's plugin opened, so a
   *  repeat `connect` for the same attachment is a no-op. */
  #lspAttachment: string | null = null;

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

    // The keystroke `run` fires on is consumed inside the core, with
    // `preventDefault` as well: when `<tonk-code>` sits inside a
    // `<form>`, some browsers can still bubble the key up to the form's
    // implicit submit before CodeMirror's own handler runs.
    this.#editor = createEditor(this.#shadow, {
      doc: this.getAttribute("value") ?? "",
      readOnly: this.hasAttribute("readonly"),
      placeholder: this.getAttribute("placeholder") ?? "",
      lineNumbers: this.hasAttribute("line-numbers"),
      activeLine: this.hasAttribute("active-line"),
      onRun: () => this.#dispatchRun(),
      onChange: (update) => {
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
      },
      onUpdate: (update) => {
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
      },
    });
    const isReadOnly = this.hasAttribute("readonly");

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
    const view = this.#editor.view;
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
        detail: { view: this.#editor.view },
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
      const view = this.#editor.view;
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
      this.#editor?.destroy();
      this.#editor = null;
    }, 0);
  }

  attributeChangedCallback(
    name: ObservedAttr,
    _old: string | null,
    next: string | null
  ): void {
    const editor = this.#editor;
    if (!editor) return;
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
        editor.setReadOnly(next !== null);
        break;
      case "placeholder":
        editor.setPlaceholder(next ?? "");
        break;
      case "line-numbers":
        // Boolean attribute: presence (`next !== null`) toggles the
        // gutter on. Default is off so the element looks like a
        // form-field by default; full-page editors opt in.
        editor.setLineNumbers(next !== null);
        break;
      case "active-line":
        editor.setActiveLine(next !== null);
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
    const editor = this.#editor;
    if (!editor) return;
    const source = this.getAttribute("source");
    if (!source) return;
    const attachment = `${source}\n${this.#lspLanguageId()}`;
    // A second announcement of the same attachment (the `source`
    // attribute landing, then an explicit re-announce) must not build a
    // second plugin: the new one opens the file before the old one has
    // closed it, the workspace refuses two views on one file, and the
    // crashed plugin takes diagnostics (and everything that waits on
    // them) down with it.
    if (client === this.#lspClient && attachment === this.#lspAttachment) return;
    if (this.#lspClient) {
      // Close the old file first, for the same reason.
      editor.setLsp([]);
    }
    this.#lspClient = client;
    this.#lspAttachment = attachment;
    editor.setLsp(client.plugin(source, this.#lspLanguageId()));
  }

  /** Detach the LSP integration. The provider calls this in
   *  response to a `tonk-code-disconnect` event (when the editor
   *  goes away or moves to a new source). The editor doesn't
   *  destroy the client — that's the provider's lifecycle. */
  disconnect(): void {
    this.#lspClient = null;
    this.#lspAttachment = null;
    this.#editor?.setLsp([]);
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
    // Patches only the span that differs, so the caret stays put.
    this.#editor!.setValue(next);
  }

  async #applyLanguage(language: string | null): Promise<void> {
    this.#pendingLanguage = language;
    if (!language) {
      this.#editor?.setLanguage([]);
      return;
    }
    try {
      const extension = await loadLanguage(language);
      // Bail if the consumer flipped to a different language while
      // we were loading; the second `applyLanguage` call will
      // install the right pack.
      if (this.#pendingLanguage !== language) return;
      this.#editor?.setLanguage(extension);
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

if (!customElements.get("tonk-code")) {
  customElements.define("tonk-code", TonkCodeElement);
}

export type { TonkCodeElement };

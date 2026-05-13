// `<tonk-diagnostics-provider>` — owns the LSP client for the
// editors nested inside it.
//
// Mounting:
//
//   <tonk-diagnostics-provider language-server="/api/language-server">
//     <tonk-code language="dialog-yaml" source="tonk-buffer:///cell-1"></tonk-code>
//     <tonk-code language="dialog-yaml" source="tonk-buffer:///cell-2"></tonk-code>
//   </tonk-diagnostics-provider>
//
// Lifecycle:
//
// 1. The provider builds an `LSPClient` against the URL in its
//    `language-server` attribute (default `/api/language-server`).
// 2. Each descendant `<tonk-code>` dispatches a bubbling
//    `tonk-code-connect` event on `connectedCallback`. The
//    provider catches it and calls `event.target.connect(client)`
//    to attach the client. The editor uses the client to
//    `didOpen` its `source` and route incoming diagnostics into
//    its lint state.
// 3. Editors dispatch `tonk-code-disconnect` when they go away
//    (or before re-announcing under a different source/language).
//    The provider calls `event.target.disconnect()` to detach the
//    LSP integration.
//
// Worker-injected diagnostics (e.g. analyzer rejections from a
// `POST /evaluate`) are delivered via the
// `tonk-push-diagnostics` event — dispatched on the provider
// element with `{ source, diagnostics }`. The provider routes
// them to the LSP client's lint state for that source so they
// appear alongside server-pushed diagnostics on the same lint
// channel and follow the same replace-on-update semantics.
//
// Attributes:
//   language-server — URL of the LSP endpoint. Defaults to
//                     `/api/language-server`. Resolved against
//                     `document.baseURI` the same way an
//                     `<a href>` would.

import {
  forEachDiagnostic,
  setDiagnostics,
  type Diagnostic as CmDiagnostic,
} from "@codemirror/lint";
import { type LSPClient } from "@codemirror/lsp-client";
import type { EditorView } from "@codemirror/view";
import { connectLsp } from "./lsp/client";
import { httpTransport } from "./lsp/transport";
import type {
  TonkCodeConnectDetail,
  TonkCodeDisconnectDetail,
} from "./index";

const DEFAULT_LANGUAGE_SERVER_URL = "/api/language-server";

/** Diagnostic `source` we tag every pushed diagnostic with so a
 *  subsequent push (including an empty one for "clear") can
 *  identify and replace only the entries we own — leaving
 *  LSP-routed diagnostics alone. */
const PUSH_DIAGNOSTIC_SOURCE = "tonk-push";

/** Detail object accepted by the `tonk-push-diagnostics` event.
 *  Lets external code (e.g. a UI route handler) inject
 *  diagnostics for a specific document on the provider's LSP
 *  client. The provider routes the diagnostics into the
 *  matching editor's lint state. */
export type TonkPushDiagnosticsDetail = {
  /** Document URI whose lint state should be replaced. Must
   *  match an attached editor's `source` attribute. */
  source: string;
  /** LSP-shaped diagnostics. Severity numbers follow the LSP
   *  convention: 1 = Error, 2 = Warning, 3 = Info, 4 = Hint. */
  diagnostics: PushedDiagnostic[];
};

/** Subset of `lsp.Diagnostic` accepted by the provider. */
export type PushedDiagnostic = {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  severity?: number;
  code?: string;
  message: string;
  source?: string;
};

/** Minimal subset of `<tonk-code>` the provider depends on,
 *  declared structurally so the provider doesn't have to import
 *  the editor class (which would create a circular dependency
 *  with the index module). */
type TonkCodeElement = HTMLElement & {
  connect(client: LSPClient): void;
  disconnect(): void;
  // Internal escape: the provider needs the editor's CodeMirror
  // view to inject pushed diagnostics into its lint state. Today
  // the `ready` event exposes it; future-proof: we read it via
  // a documented property name agreed with `<tonk-code>`.
  view?: EditorView | null;
};

class TonkDiagnosticsProvider extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return ["language-server"];
  }

  /** Active LSP client, or `null` while we're between connects. */
  #client: LSPClient | null = null;

  /** URL the current client is connected to. Used to skip rebuilds
   *  when an attribute write doesn't actually change the endpoint
   *  (Leptos rerenders sometimes set the same value back). */
  #connectedUrl: string | null = null;

  /** source URI → editor element. Lets the provider re-attach
   *  on transport rebuild, and route pushed diagnostics back to
   *  the right CodeMirror view. */
  #attached = new Map<string, TonkCodeElement>();

  /** Generation counter for the LSP session. Each rebuild
   *  increments it; transport callbacks capture the generation
   *  that owned them and ignore events from older sessions. */
  #generation = 0;

  /** Pending reconnect timer, if any. */
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #reconnectDelay = 5_000;
  static readonly #RECONNECT_MAX_MS = 30_000;

  connectedCallback(): void {
    this.addEventListener("tonk-code-connect", this.#onConnect);
    this.addEventListener("tonk-code-disconnect", this.#onDisconnect);
    this.addEventListener("tonk-push-diagnostics", this.#onPushDiagnostics);
    this.#ensureClient();
  }

  disconnectedCallback(): void {
    this.removeEventListener("tonk-code-connect", this.#onConnect);
    this.removeEventListener("tonk-code-disconnect", this.#onDisconnect);
    this.removeEventListener("tonk-push-diagnostics", this.#onPushDiagnostics);
    this.#tearDown();
  }

  attributeChangedCallback(
    name: string,
    _old: string | null,
    next: string | null,
  ): void {
    if (name !== "language-server") return;
    const desired = this.#resolveUrl(next);
    if (desired === this.#connectedUrl) return;
    this.#tearDown();
    this.#ensureClient();
  }

  #onConnect = (event: Event): void => {
    const detail = (event as CustomEvent<TonkCodeConnectDetail>).detail;
    const target = event.target as TonkCodeElement | null;
    if (!target || typeof target.connect !== "function") return;
    if (!detail?.source) return;
    this.#attached.set(detail.source, target);
    if (this.#client) {
      target.connect(this.#client);
    }
    // No client yet: `#ensureClient` runs in `connectedCallback`,
    // but if the editor mounts first (depth-first ordering can
    // put a deeply nested child's connect before the provider's
    // own connectedCallback), the attach happens in `#ensureClient`
    // when it walks `#attached`.
  };

  #onDisconnect = (event: Event): void => {
    const detail = (event as CustomEvent<TonkCodeDisconnectDetail>).detail;
    const target = event.target as TonkCodeElement | null;
    if (!target) return;
    if (detail?.source) {
      this.#attached.delete(detail.source);
    }
    if (typeof target.disconnect === "function") {
      target.disconnect();
    }
  };

  #onPushDiagnostics = (event: Event): void => {
    const detail = (event as CustomEvent<TonkPushDiagnosticsDetail>).detail;
    if (!detail?.source) return;
    const editor = this.#attached.get(detail.source);
    if (!editor || !editor.view) return;
    const view = editor.view;
    // Preserve diagnostics from other sources (LSP, etc.) by
    // walking the current set and dropping only the ones we
    // ourselves pushed previously. Without this, pushing a new
    // (or empty) batch via `setDiagnostics` would wipe the
    // LSP-side warnings the user is actively reading.
    const merged: CmDiagnostic[] = [];
    forEachDiagnostic(view.state, (d) => {
      if (d.source === PUSH_DIAGNOSTIC_SOURCE) return;
      merged.push(d);
    });
    for (const d of detail.diagnostics) {
      const from = positionToOffset(view.state.doc, d.range.start);
      const to = positionToOffset(view.state.doc, d.range.end);
      if (from === null || to === null) continue;
      merged.push({
        from,
        to: Math.max(from, to),
        severity: lspToCmSeverity(d.severity),
        message: d.message,
        // Always tag with the sentinel so the next push can
        // identify and replace just our own contributions. The
        // caller-provided `d.source` is dropped intentionally —
        // mixing it would defeat the filter.
        source: PUSH_DIAGNOSTIC_SOURCE,
      });
    }
    view.dispatch(setDiagnostics(view.state, merged));
  };

  #ensureClient(): void {
    if (this.#client) return;
    const url = this.#resolveUrl(this.getAttribute("language-server"));
    const generation = ++this.#generation;
    const transport = httpTransport({
      url,
      onClose: () => {
        if (generation !== this.#generation) return;
        this.#tearDown();
        this.#scheduleReconnect();
      },
    });
    this.#client = connectLsp(transport);
    this.#connectedUrl = url;
    this.#reconnectDelay = 5_000;
    // Re-attach any editors that announced themselves before the
    // client existed.
    for (const editor of this.#attached.values()) {
      try {
        editor.connect(this.#client);
      } catch {
        // Don't let one bad editor block the rest.
      }
    }
  }

  #tearDown(): void {
    this.#generation++;
    if (this.#reconnectTimer !== null) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = null;
    }
    for (const editor of this.#attached.values()) {
      try {
        editor.disconnect();
      } catch {
        // ignore — editor may have torn itself down already
      }
    }
    if (this.#client) {
      this.#client.disconnect();
      this.#client = null;
    }
    this.#connectedUrl = null;
  }

  #scheduleReconnect(): void {
    if (!this.isConnected) return;
    if (this.#reconnectTimer !== null) return;
    const delay = this.#reconnectDelay;
    this.#reconnectDelay = Math.min(
      delay * 2,
      TonkDiagnosticsProvider.#RECONNECT_MAX_MS,
    );
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = null;
      this.#ensureClient();
    }, delay);
  }

  #resolveUrl(raw: string | null): string {
    return new URL(raw || DEFAULT_LANGUAGE_SERVER_URL, document.baseURI).href;
  }
}

if (!customElements.get("tonk-diagnostics-provider")) {
  customElements.define("tonk-diagnostics-provider", TonkDiagnosticsProvider);
}

/** Resolve an LSP `Position` (line + UTF-16 character) to a
 *  CodeMirror document offset. Returns `null` when the position
 *  is past the end of the buffer (the buffer may have been
 *  edited since the producer captured the range — drop those
 *  rather than crash). */
function positionToOffset(
  doc: import("@codemirror/state").Text,
  position: { line: number; character: number },
): number | null {
  const lineNumber = position.line + 1;
  if (lineNumber < 1 || lineNumber > doc.lines) return null;
  const line = doc.line(lineNumber);
  return Math.min(line.from + position.character, line.to);
}

function lspToCmSeverity(severity: number | undefined): CmDiagnostic["severity"] {
  switch (severity) {
    case 2:
      return "warning";
    case 3:
      return "info";
    case 4:
      return "hint";
    case 1:
    default:
      return "error";
  }
}

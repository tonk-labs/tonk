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

import { Annotation } from "@codemirror/state";
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

/** Marks a transaction as carrying a `tonk-push-diagnostics`
 *  write rather than an LSP server `publishDiagnostics` frame.
 *  Both paths dispatch `setDiagnosticsEffect`, so the effect
 *  alone can't tell them apart — `<tonk-code>` reads this
 *  annotation to skip firing its `diagnostics` event for our
 *  own pushes. Without it, a push (e.g. the empty clear after
 *  a successful auto-evaluate) is misread as a fresh server
 *  frame and re-triggers auto-evaluate, looping. */
export const pushDiagnosticsAnnotation = Annotation.define<true>();

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


/** The `<tonk-code>` an announcement came from.
 *
 *  NOT `event.target`: these events are `composed`, so when an editor lives
 *  inside a shadow root (a `<tonk-prose>` code-block node view, for one) the
 *  browser retargets them to that shadow HOST by the time they reach an
 *  ancestor listener. The host has no `connect`, so the announcement was
 *  silently dropped and the editor never received an LSP client — no
 *  diagnostics, no completion. `composedPath()[0]` is the element that
 *  actually dispatched. */
function editorOf(event: Event): TonkCodeElement | null {
  const path = event.composedPath();
  const first = (path && path.length > 0 ? path[0] : event.target) as
    | TonkCodeElement
    | null;
  return first ?? null;
}

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

  /** Set when the worker answered a subscribe with
   *  `{"control":"update-pending"}` — it is retiring, so the next
   *  reconnect holds for `controllerchange` rather than redialing it
   *  on a timer. Cleared once consumed. */
  #updatePending = false;

  /** Pending reconnect timer, if any. */
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #reconnectDelay = 5_000;
  static readonly #RECONNECT_MAX_MS = 30_000;
  /** Fallback for a `controllerchange` that never arrives while an
   *  update is pending. Long on purpose — see `#scheduleReconnect`. */
  static readonly #UPDATE_HOLD_MS = 30_000;

  connectedCallback(): void {
    this.addEventListener("tonk-code-connect", this.#onConnect);
    this.addEventListener("tonk-code-disconnect", this.#onDisconnect);
    this.addEventListener("tonk-push-diagnostics", this.#onPushDiagnostics);
    // Connect lazily: the LSP client is built the first time an editor
    // attaches (`tonk-code-connect`), not on mount. On pages with no
    // `<tonk-code>` (e.g. the Hub at `/`) nothing connects at all, and
    // by the time an editor mounts the service worker is settled — so
    // `initialize` never races the SW activation window.
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
    const target = editorOf(event);
    if (!target || typeof target.connect !== "function") return;
    if (!detail?.source) return;
    this.#attached.set(detail.source, target);
    if (this.#client) {
      target.connect(this.#client);
    } else {
      // First editor to attach — build the client now (lazily). If
      // the build is still waiting on the service worker, this editor
      // is already in `#attached`, so `#ensureClient` connects it when
      // the client comes up.
      this.#ensureClient();
    }
  };

  #onDisconnect = (event: Event): void => {
    const detail = (event as CustomEvent<TonkCodeDisconnectDetail>).detail;
    const target = editorOf(event);
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
    view.dispatch({
      ...setDiagnostics(view.state, merged),
      annotations: pushDiagnosticsAnnotation.of(true),
    });
  };

  #ensureClient(): void {
    if (this.#client) return;
    const url = this.#resolveUrl(this.getAttribute("language-server"));
    const generation = ++this.#generation;
    // The LSP transport rides on the service worker. On a first load
    // the SW may still be installing/activating, during which a POST
    // to `/api/language-server` falls through to the static server
    // (405) — and the LSP library's `initialize` then rejects on its
    // own timeout, an uncaught error we can't catch (we don't hold
    // that promise). Wait for the SW to be settled and controlling
    // before sending anything, so `initialize` never lands in the
    // activation window. The generation guard discards the build if a
    // teardown happened while we waited.
    void this.#whenWorkerReady().then(() => {
      if (generation !== this.#generation || this.#client) return;
      const transport = httpTransport({
        url,
        onClose: ({ updatePending }) => {
          if (generation !== this.#generation) return;
          // Remember BEFORE the teardown: `#scheduleReconnect` reads
          // it to decide whether to hold for `controllerchange`.
          this.#updatePending = updatePending;
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
    });
  }

  /** Resolve once the service worker is active and controlling the
   *  page, so requests reach the worker's router rather than the
   *  static fallback. Resolves immediately when there's no service
   *  worker (e.g. a bare page or a non-SW host) so the transport
   *  still gets a chance against whatever is serving. */
  async #whenWorkerReady(): Promise<void> {
    // Reading `navigator.serviceWorker` THROWS a SecurityError in a sealed
    // (sandboxed, opaque-origin) iframe that lacks `allow-same-origin` — not
    // just returns null. Guard the property access itself. In that context the
    // page has no service worker of its own, but `fetch` is proxied to the
    // host's SW through the portal bridge, so the LSP transport works anyway;
    // fall through and let it try.
    let swContainer: ServiceWorkerContainer | null = null;
    try {
      swContainer = navigator.serviceWorker;
    } catch {
      return;
    }
    if (!swContainer) return;
    try {
      // Bound the `ready` wait: even when a worker container exists,
      // `navigator.serviceWorker.ready` may never resolve (e.g. a context that
      // can't register one) — yet `fetch` may still be proxied to a host SW.
      // Without a timeout the LSP would hang here forever and never connect.
      // Lose the race after a beat and fall through; the transport's own
      // onClose handles a real failure on a normal page.
      const ready = Promise.race([
        swContainer.ready.then(() => true),
        new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 2_000)),
      ]);
      if (!(await ready)) return;
      if (swContainer.controller) return;
      // Registered but not yet controlling this client (first load
      // after install): wait for control, with a bounded fallback so
      // we never hang if `controllerchange` doesn't fire.
      await new Promise<void>((resolve) => {
        const onChange = () => {
          swContainer.removeEventListener("controllerchange", onChange);
          clearTimeout(timer);
          resolve();
        };
        const timer = setTimeout(() => {
          swContainer.removeEventListener("controllerchange", onChange);
          resolve();
        }, 5_000);
        swContainer.addEventListener("controllerchange", onChange);
      });
    } catch {
      // SW machinery unavailable/failed — fall through and let the
      // transport try anyway; its own onClose handles a failure.
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

  /** Reconnect, but HOLD for the controller change when the worker
   *  told us it is retiring.
   *
   *  A timed redial during an update lands back on the OUTGOING
   *  worker, and an SSE stream is a fetch event that never settles —
   *  so the redial re-pins the worker that is trying to retire and
   *  parks its replacement in `waiting`. From the user's side that is
   *  "the old version survives every reload", because the reloads keep
   *  landing on the old worker this reconnect is keeping alive.
   *
   *  The signal is the worker's own: `GET /api/language-server`
   *  answers `503` with `{"control":"update-pending"}` while a
   *  successor waits (see `router/lsp.rs`). On that we wait for
   *  `controllerchange` — the successor claiming the page — and
   *  reconnect onto the NEW worker. The long fallback only backstops a
   *  `controllerchange` that never arrives; it is deliberately far
   *  longer than the normal backoff so it can't become a disguised
   *  timed redial. This mirrors the query-subscription path in
   *  `tonk-host`, which holds on the same frame. */
  #scheduleReconnect(): void {
    if (!this.isConnected) return;
    if (this.#reconnectTimer !== null) return;

    if (this.#updatePending) {
      this.#updatePending = false;
      let swContainer: ServiceWorkerContainer | null = null;
      try {
        swContainer = navigator.serviceWorker;
      } catch {
        swContainer = null;
      }
      if (swContainer) {
        const generation = this.#generation;
        const resume = () => {
          swContainer.removeEventListener("controllerchange", onChange);
          if (this.#reconnectTimer !== null) {
            clearTimeout(this.#reconnectTimer);
            this.#reconnectTimer = null;
          }
          if (generation !== this.#generation || !this.isConnected) return;
          this.#reconnectDelay = 5_000;
          this.#ensureClient();
        };
        const onChange = () => resume();
        swContainer.addEventListener("controllerchange", onChange);
        this.#reconnectTimer = setTimeout(
          resume,
          TonkDiagnosticsProvider.#UPDATE_HOLD_MS,
        );
        return;
      }
    }

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
    const url = raw || DEFAULT_LANGUAGE_SERVER_URL;
    // A ROOT-RELATIVE `/api/…` URL must stay root-relative: inside a sealed
    // (opaque-origin) guest `document.baseURI` is `null`, so absolutizing
    // it yields `null/api/…` that the portal's `window.fetch` override
    // cannot recognize as host-relative and relay — the LSP then never
    // reaches the service worker (no diagnostics, no auto-eval). Only
    // resolve a genuinely relative URL (no leading `/`), where the caller
    // meant "relative to this document".
    if (url.startsWith("/")) return url;
    return new URL(url, document.baseURI).href;
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

// rust/tonk-worker/assets/bridge.ts
//
// Iframe-side bridge. Loaded as an ES module from /__tonk/bridge.js;
// the wrapper injected by host::wrap_html_body imports it before any
// iframe-authored script runs.
//
// Publishes globalThis.tonk with three methods that mirror today's
// `/api/.../query` and `/api/.../evaluate` HTTP shapes — same input
// payloads, same output payloads, just over postMessage instead of
// fetch. The SW dispatches each envelope to the existing axum
// handlers using the iframe's bound {repo, branch}.
//
// All envelopes carry a correlation `id` so the same port can multiplex
// many in-flight queries / subscriptions without ordering assumptions.
//
// Each public method internally awaits `this.ready` before posting, so
// authors call `tonk.query(...)` / `tonk.subscribe(...)` directly from
// a `<script type="module">` without writing handshake boilerplate.
// Failures (no SW controller, bad query, evaluate error) surface as
// Promise rejections.

type HelloEnvelope     = { v: 1; type: "hello" };
type QueryEnvelope     = { v: 1; type: "query";     id: string; body: unknown };
type SubscribeEnvelope = { v: 1; type: "subscribe"; id: string; body: unknown };
type UnsubscribeEnvelope = { v: 1; type: "unsubscribe"; id: string };
type EvaluateEnvelope  = {
  v: 1;
  type: "evaluate";
  id: string;
  body: string;
  transact: boolean;
};

type ReadyResponse           = { v: 1; type: "ready" };
type QueryResultResponse     = { v: 1; type: "query-result";     id: string; rows: unknown };
type QueryErrorResponse      = { v: 1; type: "query-error";      id: string; error: string };
type SubscribeEventResponse  = { v: 1; type: "subscribe-event";  id: string; rows: unknown };
type SubscribeErrorResponse  = { v: 1; type: "subscribe-error";  id: string; error: string };
type EvaluateResultResponse  = { v: 1; type: "evaluate-result";  id: string; result: unknown };
type EvaluateErrorResponse   = { v: 1; type: "evaluate-error";   id: string; error: string };
type Inbound =
  | ReadyResponse
  | QueryResultResponse | QueryErrorResponse
  | SubscribeEventResponse | SubscribeErrorResponse
  | EvaluateResultResponse | EvaluateErrorResponse;

class Bridge {
  private port: MessagePort;
  private nextId = 0;
  private pendingOnce = new Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
    kind: "query" | "evaluate";
  }>();
  private pendingStream = new Map<string, {
    onFrame: (rows: unknown) => void;
    onError?: (message: string) => void;
  }>();
  private resolveReady!: () => void;
  private rejectReady!: (err: Error) => void;
  ready: Promise<void>;

  constructor() {
    this.ready = new Promise<void>((resolve, reject) => {
      this.resolveReady = resolve;
      this.rejectReady = reject;
    });

    const channel = new MessageChannel();
    this.port = channel.port1;
    this.port.onmessage = (event: MessageEvent<Inbound>) => {
      this.dispatch(event.data);
    };

    const controller = navigator.serviceWorker?.controller;
    if (!controller) {
      // No SW controlling this page — every method will reject from
      // the awaited ready promise so callers see a clear error
      // instead of an indefinite hang.
      this.rejectReady(new Error("[tonk] bridge: no service worker controller"));
      return;
    }
    controller.postMessage(
      { v: 1, type: "hello" } satisfies HelloEnvelope,
      [channel.port2],
    );
  }

  async query(body: unknown): Promise<unknown> {
    await this.ready;
    const id = this.mintId();
    return new Promise<unknown>((resolve, reject) => {
      this.pendingOnce.set(id, { resolve, reject, kind: "query" });
      this.port.postMessage({ v: 1, type: "query", id, body } satisfies QueryEnvelope);
    });
  }

  subscribe(
    body: unknown,
    onFrame: (rows: unknown) => void,
    onError?: (message: string) => void,
  ): () => void {
    const id = this.mintId();
    this.pendingStream.set(id, { onFrame, onError });
    this.ready.then(
      () => {
        // Cancelled before ready resolved — skip the send entirely.
        if (this.pendingStream.has(id)) {
          this.port.postMessage({ v: 1, type: "subscribe", id, body } satisfies SubscribeEnvelope);
        }
      },
      (err: Error) => {
        // Bridge never came up. Surface to the caller's onError so a
        // failed subscribe looks like every other reportable failure
        // instead of silently dropping.
        if (this.pendingStream.delete(id)) {
          const message = err.message ?? String(err);
          if (onError) onError(message);
          else console.error("[tonk] subscribe failed:", message);
        }
      },
    );
    return () => {
      if (!this.pendingStream.delete(id)) return;
      // If the subscribe envelope never went out (cancelled before
      // ready), the SW will treat unsubscribe-for-unknown-id as a
      // no-op — see router::bridge::handle_unsubscribe.
      this.ready.then(
        () => {
          this.port.postMessage({ v: 1, type: "unsubscribe", id } satisfies UnsubscribeEnvelope);
        },
        () => { /* bridge never came up — nothing to unsubscribe */ },
      );
    };
  }

  async evaluate(body: string, transact = true): Promise<unknown> {
    await this.ready;
    const id = this.mintId();
    return new Promise<unknown>((resolve, reject) => {
      this.pendingOnce.set(id, { resolve, reject, kind: "evaluate" });
      this.port.postMessage({
        v: 1,
        type: "evaluate",
        id,
        body,
        transact,
      } satisfies EvaluateEnvelope);
    });
  }

  private mintId(): string {
    this.nextId += 1;
    return `r${this.nextId}`;
  }

  private dispatch(envelope: Inbound): void {
    switch (envelope.type) {
      case "ready":
        this.resolveReady();
        return;
      case "query-result":
      case "evaluate-result": {
        const handler = this.pendingOnce.get(envelope.id);
        if (!handler) {
          console.warn("[tonk] result for unknown id", envelope.id);
          return;
        }
        this.pendingOnce.delete(envelope.id);
        const payload = "rows" in envelope ? envelope.rows : envelope.result;
        handler.resolve(payload);
        return;
      }
      case "query-error":
      case "evaluate-error": {
        const handler = this.pendingOnce.get(envelope.id);
        if (!handler) {
          console.warn("[tonk] error for unknown id", envelope.id);
          return;
        }
        this.pendingOnce.delete(envelope.id);
        handler.reject(new Error(envelope.error));
        return;
      }
      case "subscribe-event": {
        const handler = this.pendingStream.get(envelope.id);
        if (!handler) {
          // Late frame after unsubscribe; drop silently.
          return;
        }
        try {
          handler.onFrame(envelope.rows);
        } catch (e) {
          console.error("[tonk] subscribe frame handler threw", e);
        }
        return;
      }
      case "subscribe-error": {
        const handler = this.pendingStream.get(envelope.id);
        if (!handler) return;
        if (handler.onError) {
          try {
            handler.onError(envelope.error);
          } catch (e) {
            console.error("[tonk] subscribe error handler threw", e);
          }
        } else {
          console.error("[tonk] subscribe error", envelope.error);
        }
        return;
      }
    }
  }
}

const bridge = new Bridge();
(globalThis as any).tonk = bridge;

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
// `subscribe` returns a `ReadableStream<unknown>` whose chunks are the
// `rows` payloads of `subscribe-event` envelopes. The stream's
// `cancel` posts an `unsubscribe` envelope, so dropping/cancelling
// the stream is the only teardown surface — no separate unsubscribe
// function is exposed.
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
  private streamControllers = new Map<
    string,
    ReadableStreamDefaultController<unknown>
  >();
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

  async subscribe(body: unknown): Promise<ReadableStream<unknown>> {
    await this.ready;
    const id = this.mintId();
    const port = this.port;
    const controllers = this.streamControllers;
    return new ReadableStream<unknown>({
      start(controller) {
        controllers.set(id, controller);
        port.postMessage(
          { v: 1, type: "subscribe", id, body } satisfies SubscribeEnvelope,
        );
      },
      cancel() {
        controllers.delete(id);
        port.postMessage(
          { v: 1, type: "unsubscribe", id } satisfies UnsubscribeEnvelope,
        );
      },
    });
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
        const controller = this.streamControllers.get(envelope.id);
        if (!controller) {
          // Late frame after cancel/error; drop silently.
          return;
        }
        try {
          controller.enqueue(envelope.rows);
        } catch (e) {
          // enqueue throws if the stream is already closed/errored —
          // drop the controller so subsequent frames are ignored.
          this.streamControllers.delete(envelope.id);
          console.error("[tonk] subscribe enqueue failed", e);
        }
        return;
      }
      case "subscribe-error": {
        const controller = this.streamControllers.get(envelope.id);
        if (!controller) return;
        this.streamControllers.delete(envelope.id);
        controller.error(new Error(envelope.error));
        return;
      }
    }
  }
}

const bridge = new Bridge();
(globalThis as any).tonk = bridge;

// HTTP transport for `@codemirror/lsp-client`.
//
// `Transport` is an LSP-shaped string pipe with three methods —
// `send`, `subscribe`, `unsubscribe` — and we implement it over
// plain HTTP:
//
// - **Outbound** (`send`): `POST` the message body to the
//   configured endpoint. The response body, if non-empty, is
//   the JSON-RPC reply for the original request and gets fed
//   back through the dispatch path so the LSP client's request
//   bookkeeping resolves the matching promise.
//
// - **Inbound** (`subscribe`): `GET` the same endpoint with
//   `Accept: text/event-stream`. Server-pushed JSON-RPC
//   notifications arrive as SSE `data:` lines.
//
// The transport is *single-shot*: one connect on construction,
// one inbound stream. When the SSE body ends — for any reason —
// the transport calls its `onClose` callback and goes idle. It
// does **not** auto-reconnect; the consumer (the `<tonk-code>`
// element) is responsible for tearing down the LSP client and
// building a fresh one. That keeps every LSP session in a
// known-good state — `initialize` re-runs against whichever
// server is on the other side, document state is re-synced —
// rather than papering over silent server resets with stale
// in-memory bookkeeping.

import type { Transport } from "@codemirror/lsp-client";

export type Handler = (message: string) => void;

export interface HttpTransportOptions {
  /** Endpoint URL. Both `POST` (outbound) and `GET` (SSE) hit
   *  the same URL; method + `Accept` header distinguish the two
   *  operations. */
  url: string;
  /** Called once when the inbound SSE channel ends. Indicates
   *  that the LSP session this transport was carrying is over
   *  and the consumer should rebuild from scratch. Idempotent —
   *  fired at most once per transport.
   *
   *  `updatePending` is true when the worker refused the stream
   *  because a newer service worker is waiting to take over. The
   *  consumer must then HOLD its reconnect for `controllerchange`
   *  rather than redialing on a timer: a redial lands on the
   *  outgoing worker and its never-settling SSE fetch re-pins it,
   *  parking the successor in `waiting`. */
  onClose: (reason: { updatePending: boolean }) => void;
}

/** Construct an LSP transport over plain HTTP. */
export function httpTransport(opts: HttpTransportOptions): Transport {
  const handlers = new Set<Handler>();
  let closed = false;

  /** Fan out a server message to every subscriber. Per-handler
   *  errors are isolated: one broken subscriber doesn't take
   *  down the others — the LSPClient assumes its handler runs
   *  in isolation from any other code that might have
   *  subscribed for its own reasons. */
  function dispatch(message: string): void {
    for (const handler of handlers) {
      try {
        handler(message);
      } catch (err) {
        console.error("[tonk-code/lsp] subscriber threw", err);
      }
    }
  }

  /** Set when the worker answered the subscribe with
   *  `update-pending`, so `close` can pass it to the consumer. */
  let updatePending = false;

  function close(): void {
    if (closed) return;
    closed = true;
    handlers.clear();
    try {
      opts.onClose({ updatePending });
    } catch (err) {
      console.error("[tonk-code/lsp] onClose threw", err);
    }
  }

  // Kick off the inbound stream. When it ends we close the whole
  // transport; the LSP client's reads on `subscribe` simply stop
  // receiving messages, and the consumer (notified via `onClose`)
  // builds a fresh transport.
  void readEvents(opts.url, dispatch)
    .catch((err: unknown) => {
      if (err instanceof UpdatePendingError) updatePending = true;
      else console.warn("[tonk-code/lsp] SSE ended:", err);
    })
    .finally(close);

  return {
    send(message: string): void {
      if (closed) return;
      void postMessage(opts.url, message, dispatch).catch((err) => {
        console.warn("[tonk-code/lsp] POST failed:", err);
        // A failed POST is a sign the channel is gone (worker
        // restart, network blip). Tear down so the consumer can
        // rebuild.
        close();
      });
    },
    subscribe(handler: Handler): void {
      if (!closed) handlers.add(handler);
    },
    unsubscribe(handler: Handler): void {
      handlers.delete(handler);
    },
  };
}

async function postMessage(
  url: string,
  message: string,
  dispatch: Handler,
): Promise<void> {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: message,
  });
  if (!response.ok) {
    throw new Error(`POST ${url} -> ${response.status}`);
  }
  // Empty body == notification echo (server has nothing to say
  // back). For requests, the server's reply lives in the body.
  const text = await response.text();
  if (text.length > 0) dispatch(text);
}

/** The worker is retiring and declined to open a stream. Carries no
 *  message of its own: it is a control signal, not an error to report. */
class UpdatePendingError extends Error {}

/** Open the inbound SSE stream and dispatch each `data:` event. */
async function readEvents(url: string, dispatch: Handler): Promise<void> {
  const response = await fetch(url, {
    headers: { accept: "text/event-stream" },
  });
  // `503` + `{"control":"update-pending"}` is the worker declining to
  // open a long-lived stream because it is retiring — not a failure.
  // Distinguished so the consumer holds its reconnect for the
  // controller change instead of redialing the outgoing worker.
  if (response.status === 503) {
    const body = await response.text().catch(() => "");
    if (body.includes("update-pending")) throw new UpdatePendingError();
  }
  if (!response.ok || !response.body) {
    throw new Error(`GET ${url} -> ${response.status}`);
  }
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    buf += decoder.decode(value, { stream: true });
    // SSE events are separated by a blank line; each event has
    // one or more `field: value` lines. We only emit `data:`
    // lines today (the server doesn't send `event:`/`id:`). If
    // a single event spans multiple `data:` lines they get
    // concatenated with `\n` per the SSE spec.
    let sep: number;
    while ((sep = buf.indexOf("\n\n")) !== -1) {
      const block = buf.slice(0, sep);
      buf = buf.slice(sep + 2);
      const lines = block.split("\n");
      const dataLines: string[] = [];
      for (const line of lines) {
        if (line.startsWith("data: ")) dataLines.push(line.slice(6));
        else if (line.startsWith("data:")) dataLines.push(line.slice(5));
      }
      if (dataLines.length > 0) dispatch(dataLines.join("\n"));
    }
  }
}

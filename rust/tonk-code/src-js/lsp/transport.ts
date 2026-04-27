// Transport adapter for `@codemirror/lsp-client` over the
// in-process LSP server hosted in our service worker.
//
// `Transport` is just a string pipe — three methods: `send`,
// `subscribe`, `unsubscribe`. We split that pipe in two:
//
// - **Outbound** (`send`): `POST /api/lsp` with the message body.
//   Server replies (responses to client requests) come back in
//   the response body and are immediately fed to subscribers as
//   if they had arrived over the inbound channel — that's how
//   `@codemirror/lsp-client` expects responses regardless of
//   whether the transport is bidirectional or split.
//
// - **Inbound** (`subscribe`): `GET /api/lsp/events` returning
//   `text/event-stream`. Server-initiated notifications
//   (especially `textDocument/publishDiagnostics`) arrive here.
//
// If either channel fails the adapter retries with exponential
// backoff bounded at 30s, since service worker restarts are
// frequent during development and we want diagnostics to recover
// without a page reload.

import type { Transport } from "@codemirror/lsp-client";

const LSP_ENDPOINT = "/api/lsp";
const LSP_EVENTS_ENDPOINT = "/api/lsp/events";

type Handler = (message: string) => void;

/** Construct a Transport that talks to the SW-hosted LSP server.
 *  The returned transport is connected on construction; the SSE
 *  reader reconnects on failure. */
export function serviceWorkerTransport(): Transport {
  const handlers = new Set<Handler>();

  /** Fan out a server message to every subscriber. We catch
   *  per-handler errors so one broken subscriber doesn't take
   *  down the others — the LSPClient assumes its handler is
   *  isolated from any other code that might also subscribe. */
  function dispatch(message: string): void {
    for (const h of handlers) {
      try {
        h(message);
      } catch (err) {
        console.error("[tonk-code/lsp] subscriber threw", err);
      }
    }
  }

  // Long-lived SSE reader. Auto-reconnects with backoff.
  void runEventStream(dispatch);

  return {
    send(message: string): void {
      // Fire and forget — the response (if any) is dispatched
      // back through the same `handlers` set, so the LSPClient's
      // bookkeeping of in-flight request IDs matches what it
      // expects from a bidirectional transport.
      void postMessage(message, dispatch);
    },
    subscribe(handler: Handler): void {
      handlers.add(handler);
    },
    unsubscribe(handler: Handler): void {
      handlers.delete(handler);
    },
  };
}

async function postMessage(
  message: string,
  dispatch: (message: string) => void
): Promise<void> {
  let response: Response;
  try {
    response = await fetch(LSP_ENDPOINT, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: message,
    });
  } catch (err) {
    console.warn("[tonk-code/lsp] POST failed:", err);
    return;
  }
  if (!response.ok) {
    console.warn("[tonk-code/lsp] POST status", response.status);
    return;
  }
  // Empty body == notification echo (server has nothing to say
  // back). For requests, the server's reply lives in the body.
  const text = await response.text();
  if (text.length > 0) {
    dispatch(text);
  }
}

async function runEventStream(
  dispatch: (message: string) => void
): Promise<void> {
  let backoffMs = 250;
  const maxBackoffMs = 30_000;

  // Subscribe once to `controllerchange`. When a new service
  // worker takes control mid-session — typical during dev when
  // trunk rebuilds the wasm and the new worker calls
  // `skipWaiting()` from its install handler — we abort the
  // current SSE fetch so the loop reconnects against the new
  // worker. Without this the long-lived fetch keeps the *old*
  // worker alive serving diagnostics from stale wasm, even
  // though new POSTs already route through the new worker.
  let activeAbort: AbortController | null = null;
  const onControllerChange = () => {
    activeAbort?.abort();
  };
  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.addEventListener(
      "controllerchange",
      onControllerChange,
    );
  }

  for (;;) {
    activeAbort = new AbortController();
    try {
      const response = await fetch(LSP_EVENTS_ENDPOINT, {
        // Defensive: some browsers honor this on SSE, helps
        // with intermediate proxies that might buffer.
        headers: { accept: "text/event-stream" },
        signal: activeAbort.signal,
      });
      if (!response.ok || !response.body) {
        throw new Error(`SSE response status ${response.status}`);
      }
      // Successful connect resets the backoff for the next loop.
      backoffMs = 250;
      await readEventStream(response.body, dispatch);
      // Stream ended cleanly (server closed broadcast). Reconnect
      // immediately — the channel is intended to be long-lived,
      // a clean close is a worker restart.
    } catch (err) {
      console.warn("[tonk-code/lsp] events stream lost:", err);
    }
    await sleep(backoffMs);
    backoffMs = Math.min(backoffMs * 2, maxBackoffMs);
  }
}

/** Parse an SSE response body and dispatch each `data:` event.
 *
 *  We don't use the browser's `EventSource` API because the SW
 *  intercepts `fetch` but not `EventSource` — the latter wouldn't
 *  reach our handler. Reading the response body manually keeps
 *  the request inside the fetch-routed pipeline. */
async function readEventStream(
  body: ReadableStream<Uint8Array>,
  dispatch: (message: string) => void
): Promise<void> {
  const reader = body.getReader();
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
      if (dataLines.length > 0) {
        dispatch(dataLines.join("\n"));
      }
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

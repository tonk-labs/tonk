// rust/tonk-worker/assets/bridge.ts
//
// Iframe-side bridge module. Served by the SW at
// /__tonk/bridge.js (the .ts is compiled by the Trunk hook in
// build.rs / Trunk.toml; see Task 14).
//
// On load:
//   1. Open a MessageChannel.
//   2. Send port2 to navigator.serviceWorker.controller with a
//      `hello` envelope.
//   3. Wait for `ready` over port1. Materialise tonk.subscriptions
//      with one Signal per declared name.
//   4. Apply each subsequent `data` envelope to its matching
//      signal; surface `error` envelopes via signal error listeners.
//
// The agent's body only sees `globalThis.tonk` — never postMessage,
// MessageChannel, or navigator.serviceWorker.

type Row = { this: string; [field: string]: unknown };

type ReadyEnvelope = {
  v: 1;
  type: "ready";
  subscriptions: string[];
};

type DataEnvelope = {
  v: 1;
  type: "data";
  name: string;
  rows: Row[];
};

type ErrorEnvelope = {
  v: 1;
  type: "error";
  name?: string;
  error: string;
};

type InboundEnvelope = ReadyEnvelope | DataEnvelope | ErrorEnvelope;

class Signal<T> {
  private value: T | undefined = undefined;
  private listeners = new Set<(value: T) => void>();
  private errorListeners = new Set<(message: string) => void>();

  get(): T | undefined {
    return this.value;
  }

  set(value: T): void {
    this.value = value;
    for (const listener of this.listeners) {
      try {
        listener(value);
      } catch (e) {
        console.error("[tonk] signal listener threw", e);
      }
    }
  }

  fireError(message: string): void {
    for (const listener of this.errorListeners) {
      try {
        listener(message);
      } catch (e) {
        console.error("[tonk] error listener threw", e);
      }
    }
  }

  subscribe(listener: (value: T) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  subscribeError(listener: (message: string) => void): () => void {
    this.errorListeners.add(listener);
    return () => {
      this.errorListeners.delete(listener);
    };
  }
}

const subscriptions: Record<string, Signal<Row[]>> = {};

let resolveReady!: () => void;
const ready = new Promise<void>(resolve => {
  resolveReady = resolve;
});

(globalThis as any).tonk = { ready, subscriptions };

function handle(envelope: InboundEnvelope): void {
  switch (envelope.type) {
    case "ready":
      for (const name of envelope.subscriptions) {
        subscriptions[name] = new Signal<Row[]>();
      }
      resolveReady();
      return;
    case "data": {
      const signal = subscriptions[envelope.name];
      if (!signal) {
        console.warn("[tonk] data for unknown subscription", envelope.name);
        return;
      }
      signal.set(envelope.rows);
      return;
    }
    case "error": {
      if (envelope.name) {
        const signal = subscriptions[envelope.name];
        if (signal) {
          signal.fireError(envelope.error);
          return;
        }
      }
      console.error("[tonk] bridge error", envelope.error);
      return;
    }
  }
}

const channel = new MessageChannel();
channel.port1.onmessage = (event: MessageEvent<InboundEnvelope>) => {
  handle(event.data);
};

const controller = navigator.serviceWorker?.controller;
if (!controller) {
  console.error("[tonk] no service worker controller; bridge inactive");
} else {
  controller.postMessage({ v: 1, type: "hello" }, [channel.port2]);
}

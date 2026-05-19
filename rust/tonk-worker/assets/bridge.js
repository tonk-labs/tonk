// rust/tonk-worker/assets/bridge.ts
var Signal = class {
  value = void 0;
  listeners = /* @__PURE__ */ new Set();
  errorListeners = /* @__PURE__ */ new Set();
  get() {
    return this.value;
  }
  set(value) {
    this.value = value;
    for (const listener of this.listeners) {
      try {
        listener(value);
      } catch (e) {
        console.error("[tonk] signal listener threw", e);
      }
    }
  }
  fireError(message) {
    for (const listener of this.errorListeners) {
      try {
        listener(message);
      } catch (e) {
        console.error("[tonk] error listener threw", e);
      }
    }
  }
  subscribe(listener) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  subscribeError(listener) {
    this.errorListeners.add(listener);
    return () => {
      this.errorListeners.delete(listener);
    };
  }
};
var subscriptions = {};
var resolveReady;
var ready = new Promise((resolve) => {
  resolveReady = resolve;
});
globalThis.tonk = { ready, subscriptions };
function handle(envelope) {
  switch (envelope.type) {
    case "ready":
      for (const name of envelope.subscriptions) {
        subscriptions[name] = new Signal();
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
var channel = new MessageChannel();
channel.port1.onmessage = (event) => {
  handle(event.data);
};
var controller = navigator.serviceWorker?.controller;
if (!controller) {
  console.error("[tonk] no service worker controller; bridge inactive");
} else {
  controller.postMessage({ v: 1, type: "hello" }, [channel.port2]);
}

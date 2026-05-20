// rust/tonk-worker/assets/bridge.ts
var Bridge = class {
  port;
  nextId = 0;
  pendingOnce = /* @__PURE__ */ new Map();
  pendingStream = /* @__PURE__ */ new Map();
  resolveReady;
  ready;
  constructor() {
    this.ready = new Promise((resolve) => {
      this.resolveReady = resolve;
    });
    const channel = new MessageChannel();
    this.port = channel.port1;
    this.port.onmessage = (event) => {
      this.dispatch(event.data);
    };
    const controller = navigator.serviceWorker?.controller;
    if (!controller) {
      console.error("[tonk] bridge: no service worker controller; calls will hang");
      return;
    }
    controller.postMessage(
      { v: 1, type: "hello" },
      [channel.port2]
    );
  }
  query(body) {
    const id = this.mintId();
    return new Promise((resolve, reject) => {
      this.pendingOnce.set(id, { resolve, reject, kind: "query" });
      this.port.postMessage({ v: 1, type: "query", id, body });
    });
  }
  subscribe(body, onFrame, onError) {
    const id = this.mintId();
    this.pendingStream.set(id, { onFrame, onError });
    this.port.postMessage({ v: 1, type: "subscribe", id, body });
    return () => {
      if (!this.pendingStream.delete(id)) return;
      this.port.postMessage({ v: 1, type: "unsubscribe", id });
    };
  }
  evaluate(body, transact = true) {
    const id = this.mintId();
    return new Promise((resolve, reject) => {
      this.pendingOnce.set(id, { resolve, reject, kind: "evaluate" });
      this.port.postMessage({
        v: 1,
        type: "evaluate",
        id,
        body,
        transact
      });
    });
  }
  mintId() {
    this.nextId += 1;
    return `r${this.nextId}`;
  }
  dispatch(envelope) {
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
};
var bridge = new Bridge();
globalThis.tonk = bridge;

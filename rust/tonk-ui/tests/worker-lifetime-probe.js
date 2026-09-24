// Injected only into persisted-upgrade test generations, before the worker shim.
(() => {
    const pending = new Map();
    const recent = [];
    let sequence = 0;
    const describe = event => event.request
        ? `${event.type} ${event.request.method} ${new URL(event.request.url).pathname}`
        : `${event.type} ${event.data?.type ?? ""}`;
    const track = (kind, label, promise) => {
        const id = ++sequence;
        const entry = { kind, label, started: Date.now() };
        pending.set(id, entry);
        const settle = outcome => {
            pending.delete(id);
            recent.push({ ...entry, duration: Date.now() - entry.started, outcome });
            if (recent.length > 30) recent.shift();
        };
        Promise.resolve(promise).then(() => settle("resolved"), error => settle(String(error)));
    };
    const waitUntil = ExtendableEvent.prototype.waitUntil;
    ExtendableEvent.prototype.waitUntil = function (promise) {
        const result = waitUntil.call(this, promise);
        track("waitUntil", describe(this), promise);
        return result;
    };
    const respondWith = FetchEvent.prototype.respondWith;
    FetchEvent.prototype.respondWith = function (promise) {
        const result = respondWith.call(this, promise);
        track("response-headers", describe(this), promise);
        return result;
    };
    const skipWaiting = self.skipWaiting.bind(self);
    self.skipWaiting = () => {
        const promise = skipWaiting();
        track("skipWaiting", "", promise);
        return promise;
    };
    self.addEventListener("message", event => {
        if (event.data?.type !== "tonk-test-lifetimes") return;
        event.stopImmediatePropagation();
        event.ports[0].postMessage({
            pending: [...pending.values()].map(entry => ({ ...entry, age: Date.now() - entry.started })),
            recent,
        });
    });
})();

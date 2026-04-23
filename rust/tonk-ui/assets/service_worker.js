import init, { activate } from "./worker.js";

const log = (...args) => console.log("[Tonk Service Worker]", ...args);

let tonkServiceWorkerResolves;

async function activateWorker() {
    if (tonkServiceWorkerResolves == null) {
        tonkServiceWorkerResolves = init().then(() => activate());
        log("Worker initialized");
    }

    return tonkServiceWorkerResolves;
}

self.skipWaiting();

self.oninstall = _event => {
    log("Installed");
};

self.onactivate = event => {
    event.waitUntil(self.clients.claim());
    log("Activated");
};

// Hand *every* fetch to the Rust side. The Rust worker decides
// whether to route through its own handlers or pass the request
// through to the network (which it does itself via `self.fetch`).
// SPA-style 404 → index.html fallback and /api/* routing all
// live in Rust now, so this shim is pure forwarding.
self.onfetch = event => {
    event.respondWith(
        (async () => (await activateWorker()).onfetch(event))(),
    );
};

// Background Sync API event handler
self.onsync = event => {
    log("Background sync event:", event.tag);
    event.waitUntil(
        (async () => {
            let worker = await activateWorker();
            return worker.sync();
        })()
    );
};

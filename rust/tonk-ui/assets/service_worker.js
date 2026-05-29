import init, { activate } from "./worker.js";

const log = (...args) => console.log("[Tonk Service Worker]", ...args);

let tonkServiceWorkerResolves;

// Static import at top-level is forced on us: dynamic `import()`
// is disallowed inside a ServiceWorkerGlobalScope per the HTML
// spec, and `importScripts()` is incompatible with module SWs.
// So the worker bundle costs ~1 s of `workerStart` time on cold
// navigations until we find a better way to defer it.
async function activateWorker() {
    if (tonkServiceWorkerResolves == null) {
        tonkServiceWorkerResolves = init().then(() => activate());
        log("Worker initialized");
    }

    return tonkServiceWorkerResolves;
}

self.oninstall = event => {
    // Promote this worker straight from `installing` to `activating`
    // without parking in `waiting`. Earlier this call sat at the top
    // of the script, where it's a no-op — the browser only honors
    // `skipWaiting()` while the worker is in `waiting`, and at
    // top-level eval time the lifecycle hasn't reached install yet.
    event.waitUntil(self.skipWaiting());
    log("Installed");
};

self.onactivate = event => {
    event.waitUntil((async () => {
        await self.clients.claim();
        try {
            const worker = await activateWorker();
            await worker.onactivate?.();
        } catch (err) {
            log("onactivate dispatch failed:", err);
        }
    })());
    log("Activated");
};

// When a *newer* version of this worker enters the `installing`
// state, this script (the currently active worker) is on the way
// out. Forward the lifecycle event to the Rust side so it can
// release every long-lived response we're serving — chiefly the
// `/api/lsp/events` SSE stream. With those streams hung up the
// in-flight fetch events settle, the spec releases this worker,
// and the new one activates.
//
// `worker.onupdatefound()` is exported by `tonk_worker::worker`
// and drops the LSP push channel's sender. Receivers see EOF,
// browser side resumes via the existing reconnect loop against
// the new worker.
self.registration.addEventListener?.("updatefound", async () => {
    log("Update found — forwarding to wasm worker");
    try {
      const worker = await activateWorker();
      // let worker know there is an update
      await worker.onupdatefound?.();
    } catch (err) {
        log("Failed to forward updatefound:", err);
    }
});

// Document navigations bypass the Rust worker entirely. The
// shell HTML lives in the SW cache and never crosses the data
// plane — there's no reason a navigation should wait on
// `dialog-repository` / axum / IndexedDB initialisation just to
// hand back a cached `index.html`. Skipping the worker boot
// here cuts the navigation TTFB on a warm load from ~1 s to
// ~10 ms.
//
// Every other fetch (`/api/*`, static assets that may need the
// guest-iframe rewrite, etc.) goes through the Rust worker —
// it's the only place that knows whether a request is from a
// view client and how the rewrite should map. The Rust side
// itself runs static cacheable GETs through the same shell
// cache so this isn't a separate cache from the navigation
// case, just a different entry point.
const SHELL_CACHE = "TONK_SHELL_v1";

async function serveNavigation(request) {
    const cache = await caches.open(SHELL_CACHE);
    try {
        const response = await fetch(request);
        if (response.status === 404) {
            return (await cache.match("/index.html")) || (await fetchAndCacheIndex(cache));
        }
        if (response.ok && response.type !== "opaque") {
            cache.put(request, response.clone()).catch(() => {});
        }
        return response;
    } catch {
        return (
            (await cache.match(request)) ||
            (await cache.match("/index.html")) ||
            fetchAndCacheIndex(cache)
        );
    }
}

async function fetchAndCacheIndex(cache) {
    const response = await fetch("/index.html");
    if (response.ok && response.type !== "opaque") {
        cache.put("/index.html", response.clone()).catch(() => {});
    }
    return response;
}

self.onfetch = event => {
    if (event.request.mode === "navigate") {
        event.respondWith(serveNavigation(event.request));
        return;
    }
    event.respondWith(
        (async () => (await activateWorker()).onfetch(event))(),
    );
};

// Iframe-side bridge messages. The iframe sends `{v:1,type:"hello"}`
// at boot (with a transferred MessagePort) and then dispatches
// query/subscribe/evaluate envelopes over the port. The Rust
// worker's `onmessage` stashes the port against the client id and
// routes per-envelope dispatch from there.
//
// One synchronous early-out: a `{type:"claim"}` message asks the
// SW to take control of every client in scope. The page sends
// this on cold-start when it lands on a SW that was activated in
// a previous session — `onactivate` doesn't refire, so without
// this nudge the page would stay uncontrolled (and every /api/*
// fetch would land on the static-asset server as a 405). The
// claim raises `controllerchange` on the page side, which the
// shell's `serviceWorkerActivates()` Promise awaits.
self.onmessage = event => {
    if (event.data && event.data.type === "claim") {
        event.waitUntil?.(self.clients.claim());
        return;
    }
    event.waitUntil?.(
        (async () => {
            try {
                const worker = await activateWorker();
                await worker.onmessage(event);
            } catch (err) {
                log("onmessage dispatch failed:", err);
            }
        })(),
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

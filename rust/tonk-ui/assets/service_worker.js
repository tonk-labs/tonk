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
    event.waitUntil(self.clients.claim());
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

self.onfetch = event => {
    let request = event.request;
    let url = new URL(request.url);

    if (url.pathname.match(/^\/api/)) {
        log(request.method, url.pathname);
        event.respondWith(
            (async () => {
                return (await activateWorker()).onfetch(request);
            })(),
        );
    // NOTE: Only intercept navigate as candidates for serving
    // the index.html (in order to provide SPA-style routing)
    } else if (request.mode === 'navigate') {
        event.respondWith(
            fetch(request).then(response => {
                if (response.status === 404) {
                    return fetch("/index.html");
                }
                return response;
            }),
        );
    }
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

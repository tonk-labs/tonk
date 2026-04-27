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
    // without parking in `waiting`. The previous version of this
    // file called `skipWaiting()` at the top of the script, which
    // is a no-op: the call is only honored while the worker is in
    // the *waiting* state, and at top-level evaluation time the
    // lifecycle hasn't yet reached `install`. Moving the call into
    // the install handler matches the browser's contract — the new
    // worker takes over as soon as it finishes installing, so a
    // simple page reload after `cargo build` actually picks up the
    // new wasm rather than serving the previous version forever.
    event.waitUntil(self.skipWaiting());
    log("Installed");
};

self.onactivate = event => {
    event.waitUntil(self.clients.claim());
    log("Activated");
};

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

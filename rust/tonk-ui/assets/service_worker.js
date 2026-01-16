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

self.onfetch = event => {
    let request = event.request;
    let url = new URL(request.url);

    if (url.pathname.match(/^\/api/)) {
        log(request.method, url.pathname);

        // Check if this is a fact assertion request - we need to push after
        const shouldPushAfter = url.pathname.match(/^\/api\/fact\/assert\//);

        event.respondWith(
            (async () => {
                const worker = await activateWorker();
                const response = await worker.onfetch(request);

                // Trigger background push for fact assertions
                // Don't block the response - use waitUntil to run in background
                if (shouldPushAfter && response.ok) {
                    event.waitUntil(
                        worker.push().catch(err => {
                            log("Background push failed:", err);
                        }),
                    );
                }

                return response;
            })(),
        );
    }
};
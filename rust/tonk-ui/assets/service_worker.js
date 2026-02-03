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
        event.respondWith(
            (async () => {
                return (await activateWorker()).onfetch(request);
            })(),
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

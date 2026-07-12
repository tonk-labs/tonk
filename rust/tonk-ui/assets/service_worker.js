import init, { activate } from "./worker.js";

const log = (...args) => console.log("[Tonk Service Worker]", ...args);

// Shell cache name. Kept in step with `cache.rs`'s `SHELL_CACHE`.
// Declared up here (not beside `serveNavigation`) because
// `oninstall` precaches the shell into it.
const SHELL_CACHE = "TONK_SHELL_v1";

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
    //
    // Precache the shell under `/` — the origin serves the shell
    // HTML there (200), whereas `/index.html` is a 307 redirect to
    // `/`. `cache.add` refuses to store a redirect, so seeding
    // `/index.html` silently fails and every reload then serves a
    // dead key. `serveNavigation` reads and writes the same `/`
    // key, so a first-visit-online install populates the shell
    // every later navigation falls back to.
    event.waitUntil((async () => {
        try {
            const cache = await caches.open(SHELL_CACHE);
            await cache.add("/");
        } catch (err) {
            log("Shell precache failed:", err);
        }
        await self.skipWaiting();
    })());
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

// Connectivity transitions, observed by the worker itself. An offline
// page stops polling, so no fetch would ever wake the Rust side to
// stamp `sync:offline` — the SW's own `offline` event is the reliable
// signal; `online` runs a drain so statuses reconcile the moment
// connectivity returns. The wasm boot (`activateWorker`) can outlive a
// flapping transition, so dispatch on the CURRENT `navigator.onLine`,
// not on which event happened to fire.
const onConnectivityChange = async () => {
    try {
        const worker = await activateWorker();
        if (navigator.onLine) {
            log("Online — reconciling");
            await worker.ononline?.();
        } else {
            log("Offline — stamping sync:offline");
            await worker.onoffline?.();
        }
    } catch (err) {
        log("Failed to handle connectivity change:", err);
    }
};
self.addEventListener("offline", onConnectivityChange);
self.addEventListener("online", onConnectivityChange);

// Serve the precached app shell for every route navigation. The
// SPA router resolves the actual path client-side, so there's
// nothing URL-specific to serve — just hand back the shell from
// the cache, which means a navigation never waits on the network
// (a network-first shell blocked slow loads on a full round-trip
// even though the shell was already on disk).
//
// Keyed on `/`, where the origin serves the shell HTML (200).
// `/index.html` is a 307 redirect to `/`, so it can't be cached
// or served as the shell.
//
// Stale-while-revalidate: serve the cached shell IMMEDIATELY (a
// navigation never waits on the network, so a repeat load on a slow
// or flaky connection is instant and works offline), and in the
// background fetch `/` to refresh the cached copy for NEXT time. But
// a plain SWR on the shell alone poisons the cache: the shell names
// content-hashed assets, so refreshing `/` to a newer build leaves
// the cache holding the new shell beside the OLD build's assets (and
// missing the new ones) — exactly the mix that makes a later load
// reference assets that aren't cached and hard-fail. So when the
// refreshed shell DIFFERS from the cached one (a new build), drop
// every cached asset: the new shell then re-populates its own assets
// on the next load, and the two never cross builds. The current
// load keeps serving the coherent build it already had.
async function serveNavigation(event) {
    const cache = await caches.open(SHELL_CACHE);
    const cached = await cache.match("/");

    const revalidate = (async () => {
        try {
            const fresh = await fetch("/");
            if (!fresh.ok || fresh.type === "opaque") return;
            // Compare against the copy we served to detect a build change.
            const freshText = await fresh.clone().text();
            const cachedText = cached ? await cached.clone().text() : null;
            if (cachedText !== null && cachedText !== freshText) {
                // New build: flush the whole shell cache so the old
                // build's hashed assets can't mix with the new shell,
                // then seed the fresh shell into the clean cache.
                await caches.delete(SHELL_CACHE);
                const clean = await caches.open(SHELL_CACHE);
                await clean.put("/", fresh);
            } else {
                await cache.put("/", fresh);
            }
        } catch {
            // Offline / fetch failed — keep the cached shell as-is.
        }
    })();

    if (cached) {
        // Don't block the response on the refresh, but keep the worker
        // alive until it settles so the cache updates for next time.
        event?.waitUntil?.(revalidate);
        return cached;
    }
    // Cold cache (first visit, or just flushed): the refresh IS the
    // response — wait for it and serve the fetched shell.
    await revalidate;
    return (await cache.match("/")) || fetch("/");
}

// Route navigations straight to the cached shell (bypassing the
// Rust worker boot, so TTFB doesn't wait on dialog-repository /
// axum / IndexedDB init); everything else — `/api/*` and static
// assets — goes through the Rust worker, which owns the guest
// rewrite and the resource cache.
self.onfetch = event => {
    if (event.request.mode === "navigate") {
        // `/api/*` navigations are real data-plane requests, not SPA
        // routes — a user visiting e.g. `/api/migrate/...` directly, or
        // a server-issued redirect landing there. Route them through
        // the Rust worker so axum answers (and its redirects resolve),
        // instead of handing back the shell HTML (which would boot the
        // SPA and 404 in the client router). Everything else is a
        // navigation to an app route: serve the cached shell.
        const path = new URL(event.request.url).pathname;
        if (!path.startsWith("/api/")) {
            event.respondWith(serveNavigation(event));
            return;
        }
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

// Background Sync API event handler. A single bare `sync` tag: the worker
// drains the whole queue regardless, so the tag carries no identity. A
// rejected promise here tells the user agent to retry the sync with backoff.
self.onsync = event => {
    log("Background sync event:", event.tag);
    event.waitUntil(
        (async () => {
            let worker = await activateWorker();
            return worker.sync(event.tag);
        })()
    );
};

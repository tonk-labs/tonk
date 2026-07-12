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

// Connectivity transitions. The Rust side reads `navigator.onLine` itself
// (reliable in the SW scope) and reconciles — an offline reading stamps
// `sync:offline`, an online one runs a drain. We only need to POKE it on a
// transition; the decision is the worker's. The SW's own `offline`/`online`
// events fire the poke, and so does a `{type:"connectivity"}` message from the
// active page (belt-and-suspenders — the page's events are the ones guaranteed
// to fire, and it forwards them here).
const onConnectivityChange = async () => {
    try {
        const worker = await activateWorker();
        await worker.onconnectivity?.();
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

    // Background refresh. Only ever mutates the cache with a fresh shell
    // ALREADY IN HAND — never deletes before it can replace, so an offline
    // or failed fetch leaves the cached shell untouched (the app must stay
    // loadable offline). On a build change it replaces `/` and prunes the
    // OLD build's hashed assets so the shell and its assets never cross
    // builds; `/` itself is rewritten in the same pass, so the cache is
    // never without a shell.
    // Snapshot the cached shell's body NOW, while we still own it. Below we
    // hand `cached` itself to the browser, which consumes its body to render
    // the page — and a Response whose body is being consumed can no longer be
    // cloned. Cloning inside `revalidate` (which runs concurrently under
    // `waitUntil`) therefore raced the page and threw
    // "Response body is already used", so the `cache.put` below it never ran:
    // the shell cache was never refreshed, and every navigation paid a `/`
    // fetch whose result was thrown away on the exception.
    const cachedTextPromise = cached ? cached.clone().text() : null;

    const revalidate = async () => {
        let fresh;
        try {
            fresh = await fetch("/");
        } catch {
            return; // offline / network error — keep what we have
        }
        if (!fresh.ok || fresh.type === "opaque") return;

        const freshText = await fresh.clone().text();
        const cachedText = cachedTextPromise ? await cachedTextPromise : null;

        if (cachedText !== null && cachedText !== freshText) {
            // New build. Prune every OTHER entry (the previous build's
            // hashed assets) but keep `/` continuously populated by
            // overwriting it with the fresh shell in the same cache.
            const keys = await cache.keys();
            await Promise.all(
                keys
                    .filter((req) => new URL(req.url).pathname !== "/")
                    .map((req) => cache.delete(req)),
            );
            await cache.put("/", fresh);
        } else {
            await cache.put("/", fresh);
        }
    };

    if (cached) {
        // Serve the cached shell immediately; refresh in the background.
        event?.waitUntil?.(revalidate());
        return cached;
    }
    // Cold cache: no shell to serve yet. Fetch it (this is the only path
    // that can hard-fail offline, and only when nothing was ever cached —
    // `oninstall` precaches `/`, so this is the rare first-visit race).
    try {
        const fresh = await fetch("/");
        if (fresh.ok && fresh.type !== "opaque") {
            cache.put("/", fresh.clone()).catch(() => {});
        }
        return fresh;
    } catch {
        // Offline with an empty cache — nothing we can do but surface it.
        return Response.error();
    }
}

// Whether a request can be served from the shell cache by the JS shim
// WITHOUT booting the wasm worker. Same-origin GETs that aren't the data
// plane (`/api/*`) and don't ask for fresh content. Mirrors `cache.rs`'s
// `is_cacheable`, but lives here so a cached asset is served straight from
// the Cache API — the wasm worker's own bundle can be multiple MB, and
// booting it to serve a static file made every subresource wait on that
// download (fatal on 3G). The Rust worker still owns the SAME cache for
// misses and for guest-iframe path rewrites.
function isShellCacheable(request, path) {
    if (request.method !== "GET") return false;
    if (request.mode === "navigate") return false;
    if (path.startsWith("/api/")) return false;
    // A guest-iframe subresource is rewritten to a branch-scoped `/api/...`
    // path by the Rust worker; those must not be served as top-level assets.
    // They carry a client id the shim can't see, so exclude by the one
    // shared-asset exception the worker passes through unchanged.
    const cache = request.cache;
    if (cache === "no-store" || cache === "reload" || cache === "no-cache") {
        return false;
    }
    return true;
}

// Serve a static asset cache-first, revalidating in the background. Returns
// the cached response immediately when present (no network, no worker boot —
// instant on any connection); on a miss, falls through to the wasm worker,
// which fetches, caches, and applies any guest rewrite.
async function serveAsset(event) {
    const cache = await caches.open(SHELL_CACHE);
    // Match ONLY what the Rust worker itself cached (it writes the top-level
    // resource graph under the request URL). A guest-iframe subresource is
    // rewritten to a branch-scoped `/api/...` path by the worker and cached
    // under THAT key, so it can never match here — no cross-serving a guest
    // asset from the top-level cache.
    const cached = await cache.match(event.request);
    if (cached) {
        // Background refresh for next time; never blocks this response.
        event.waitUntil?.(
            (async () => {
                try {
                    const fresh = await fetch(event.request);
                    if (fresh.ok && fresh.type !== "opaque") {
                        await cache.put(event.request, fresh.clone());
                    }
                } catch {
                    // offline / blip — keep the cached copy
                }
            })(),
        );
        return cached;
    }
    // Cache miss: let the wasm worker fetch + cache (it owns the guest
    // rewrite and the resource cache write). This is the only path that
    // pays the worker-boot cost, and only for assets not yet cached.
    return (await activateWorker()).onfetch(event);
}

// Route navigations straight to the cached shell (bypassing the
// Rust worker boot, so TTFB doesn't wait on dialog-repository /
// axum / IndexedDB init); cached static assets are served from the
// Cache API directly (also bypassing the boot); everything else —
// `/api/*` and cache-missed assets — goes through the Rust worker,
// which owns the guest rewrite and the resource cache.
self.onfetch = event => {
    const path = new URL(event.request.url).pathname;
    // Shortcut-service routes (`PUT /@`, `GET /@/{hash}`) belong to
    // the edge worker, not this one. Not intercepting at all is the
    // point: the edge answers `GET /@/{hash}` with a relative 301
    // that the browser must follow natively so the short link's
    // `#fragment` (an invite's seed) carries over to the target via
    // RFC 7231 fragment inheritance. Serving the shell here would
    // swallow the redirect for any user who already has this worker
    // installed.
    if (path === "/@" || path.startsWith("/@/")) {
        return;
    }
    if (event.request.mode === "navigate") {
        // `/api/*` navigations are real data-plane requests, not SPA
        // routes — a user visiting e.g. `/api/migrate/...` directly, or
        // a server-issued redirect landing there. Route them through
        // the Rust worker so axum answers (and its redirects resolve),
        // instead of handing back the shell HTML (which would boot the
        // SPA and 404 in the client router). Everything else is a
        // navigation to an app route: serve the cached shell.
        if (!path.startsWith("/api/")) {
            event.respondWith(serveNavigation(event));
            return;
        }
    } else if (isShellCacheable(event.request, path)) {
        // A cached static asset is served WITHOUT booting the wasm worker,
        // so a slow connection never waits on the worker's own bundle.
        event.respondWith(serveAsset(event));
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
    // Connectivity nudge from the active page on an `online`/`offline`
    // transition. The worker re-reads `navigator.onLine` itself and reconciles;
    // this just wakes it so the overlay updates without waiting for a fetch.
    if (event.data && event.data.type === "connectivity") {
        event.waitUntil?.(
            (async () => {
                try {
                    const worker = await activateWorker();
                    await worker.onconnectivity?.();
                } catch (err) {
                    log("connectivity dispatch failed:", err);
                }
            })(),
        );
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

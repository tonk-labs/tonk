import init, { activate } from "./worker.js";

const log = (...args) => console.log("[Tonk Service Worker]", ...args);

// ---- Introspection -------------------------------------------------
//
// Everything the worker logs is captured into a bounded ring, and
// `GET /api/health` answers from this glue WITHOUT the wasm worker —
// so a worker that fails to initialize is still diagnosable with one
// fetch from any page, instead of spelunking serviceworker-internals.
const LOG_RING_CAPACITY = 400;
const logRing = [];
const ringify = level => {
    const original = console[level].bind(console);
    console[level] = (...args) => {
        const message = args
            .map(arg => {
                if (typeof arg === "string") return arg;
                try {
                    return arg instanceof Error ? (arg.stack || String(arg)) : JSON.stringify(arg);
                } catch {
                    return String(arg);
                }
            })
            .join(" ");
        logRing.push({ t: Date.now(), level, message: message.slice(0, 2000) });
        if (logRing.length > LOG_RING_CAPACITY) logRing.shift();
        original(...args);
    };
};
["log", "warn", "error"].forEach(ringify);
self.addEventListener("unhandledrejection", event => {
    logRing.push({
        t: Date.now(),
        level: "error",
        message: `unhandledrejection: ${event.reason?.stack || event.reason}`.slice(0, 2000),
    });
    if (logRing.length > LOG_RING_CAPACITY) logRing.shift();
});

// Worker-init observability: state, the last failure, and how many
// times initialization has been attempted.
const workerHealth = {
    state: "idle", // idle | initializing | ok | failed
    error: null,
    attempts: 0,
    lastAttemptAt: null,
    startedAt: Date.now(),
};

function healthResponse() {
    return new Response(
        JSON.stringify({
            worker: workerHealth.state,
            error: workerHealth.error,
            attempts: workerHealth.attempts,
            lastAttemptAt: workerHealth.lastAttemptAt,
            startedAt: workerHealth.startedAt,
            log: logRing.slice(-200),
        }),
        { status: 200, headers: { "content-type": "application/json" } },
    );
}

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
// How long a failed initialization parks further attempts. A failed
// init used to be memoized forever (the rejected promise was cached),
// so one transient failure — storage still settling, a race at boot —
// bricked the worker until the browser discarded it. Now a failure is
// recorded for `/api/health`, and the next fetch after the hold-off
// retries from scratch.
const INIT_RETRY_HOLDOFF_MS = 5000;

async function activateWorker() {
    if (tonkServiceWorkerResolves == null) {
        const now = Date.now();
        if (
            workerHealth.state === "failed" &&
            now - workerHealth.lastAttemptAt < INIT_RETRY_HOLDOFF_MS
        ) {
            throw new Error(`worker initialization failed: ${workerHealth.error}`);
        }
        workerHealth.state = "initializing";
        workerHealth.attempts += 1;
        workerHealth.lastAttemptAt = now;
        tonkServiceWorkerResolves = init()
            .then(() => activate())
            .then(worker => {
                workerHealth.state = "ok";
                workerHealth.error = null;
                return worker;
            })
            .catch(error => {
                workerHealth.state = "failed";
                workerHealth.error = String(error?.message || error).slice(0, 2000);
                // Un-memoize so a later fetch retries instead of
                // replaying this rejection for the worker's lifetime.
                tonkServiceWorkerResolves = null;
                log("Worker initialization failed:", error);
                throw error;
            });
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
    // Do not claim every open page merely because this worker activated.
    // A page from before the page-directed update protocol cannot align or
    // reload itself safely, so it stays on its current controller until its
    // next navigation. A compatible page sends `{type:"claim"}` below after
    // observing this worker reach `activated`; first-install pages use the
    // same explicit message.
    //
    // The wasm worker is still poked outside waitUntil: activateWorker()
    // waits for the active-worker lock, and gating ACTIVATION on that lock
    // deadlocks the swap — the outgoing worker cannot die while its
    // in-flight fetches hang, the lock never frees, and this worker pins in
    // `activating` while every page waits on it.
    (async () => {
        try {
            const worker = await activateWorker();
            await worker.onactivate?.();
        } catch (err) {
            log("onactivate dispatch failed:", err);
        }
    })();
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
    // The event fires on the REGISTRATION, so every worker running this script
    // hears it — including the newly-installing worker, about its own arrival.
    // Only the OUTGOING worker may act on it: `onupdatefound` stops all sync
    // work, and that flag is never cleared (a retiring worker must not re-arm
    // `waitUntil`, or it pins itself in `waiting`). A new worker that ran it
    // would latch itself off and then go on to serve the page while refusing
    // every sync drain for the rest of its life.
    //
    // `registration.installing` is the incoming worker. If that is us, this is
    // our own birth announcement, not our eviction notice.
    if (self.serviceWorker && self.registration.installing === self.serviceWorker) {
        log("Update found — that is us installing; staying live");
        return;
    }
    log("Update found — stopping background work; the successor runs independently");
    try {
      const worker = await activateWorker();
      // Stop the sync loop and release long-lived streams so this
      // instance winds down. Serving continues until the successor
      // claims the pages — the two may overlap briefly, which storage
      // is designed to tolerate (CAS commits over content-addressed
      // blocks; transaction settling is race-armed). Nothing here may
      // couple the successor's startup to this worker's death: worker
      // lifecycles belong to the browser.
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
// ---- Graceful upgrade -----------------------------------------------
//
// A deploy must never brick a page, and worker lifecycles belong to
// the browser: nothing here may couple the successor's startup to the
// outgoing worker's death. On updatefound the outgoing worker stops
// background work and releases streams; it keeps serving until the
// successor activates and claims the pages (a brief overlap storage
// tolerates by design: CAS commits over content-addressed blocks,
// race-armed transaction settling). Teardown is then the browser's
// default path — it works because every response is fast or bounded
// by the storage settle watchdog, so activation always finds a quiet
// gap and the reaped worker never has a reason to linger.

/// A real error page for a worker that cannot start: the actual error,
/// a retry, and a pointer at /api/health — never an endless spinner.
function failurePage() {
    const detail = String(workerHealth.error || "unknown error");
    const escaped = detail
        .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    return new Response(
        `<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tonk failed to start</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0; display: grid;
         place-items: center; min-height: 100vh; min-height: 100dvh;
         background: light-dark(#e8e6e4, #161313);
         color: light-dark(#38182a, #e2dfdd); }
  main { max-width: 42rem; padding: 2rem; }
  h1 { font-size: 1.3rem; }
  pre { background: light-dark(#fcfbfb, #261f20);
        border: 1px solid light-dark(rgb(56 24 42 / 28%), rgb(226 223 221 / 28%));
        padding: 1rem; white-space: pre-wrap; word-break: break-word;
        font-size: 0.85rem; }
  button { font: inherit; padding: 0.5rem 1.25rem;
           border: 0; background: light-dark(#38182a, #e2dfdd);
           color: light-dark(#f7f6f5, #221c1d); cursor: pointer; }
  p.hint { color: light-dark(#5b4953, #c8c3bf); font-size: 0.85rem; }
</style></head><body><main>
<h1>Tonk failed to start</h1>
<p>The storage worker could not initialize. Attempt ${workerHealth.attempts}.</p>
<pre>${escaped}</pre>
<button onclick="location.reload()">Try again</button>
<p class="hint">Diagnostics: <code>/api/health</code> has the full log ring.</p>
</main></body></html>`,
        { status: 503, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

self.onfetch = event => {
    const path = new URL(event.request.url).pathname;
    // Answered from this glue, never the wasm worker: health must be
    // readable precisely when the worker cannot answer for itself.
    if (path === "/api/health") {
        event.respondWith(healthResponse());
        return;
    }
    // A failed worker must fail LOUD: navigations get an error page with
    // the actual reason and a retry; data-plane requests get a clean 503
    // carrying the error. An endless spinner is never acceptable. The
    // init-retry holdoff still applies — a later reload attempts a fresh
    // initialization, and success clears this state.
    if (workerHealth.state === "failed") {
        if (event.request.mode === "navigate" && !path.startsWith("/api/")) {
            // Kick a background re-initialization (the holdoff inside
            // activateWorker paces it) so "Try again" can actually
            // succeed once the cause has healed.
            activateWorker().catch(() => {});
            event.respondWith(failurePage());
            return;
        }
        if (path.startsWith("/api/")) {
            event.respondWith(new Response(
                JSON.stringify({ error: {
                    kind: "worker-failed",
                    message: workerHealth.error || "worker initialization failed",
                } }),
                { status: 503, headers: { "content-type": "application/json" } },
            ));
            return;
        }
    }
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
    // A page became visible again — wake the worker so sync resumes the
    // active cadence immediately instead of waiting out a hidden interval.
    if (event.data && event.data.type === "visibility") {
        event.waitUntil?.(
            (async () => {
                try {
                    const worker = await activateWorker();
                    await worker.onvisibility?.();
                } catch (err) {
                    log("visibility dispatch failed:", err);
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

// Some engines report a structured-clone failure only to the receiver.
// Never inspect `event.data` here: it may be unavailable, and custody
// messages can contain transient PRF bytes.
self.onmessageerror = () => {
    console.error("messageerror: service-worker message could not be deserialized");
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

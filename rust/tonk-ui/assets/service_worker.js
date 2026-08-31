import init, { activate } from "./worker.js";

// ---- Build identity --------------------------------------------------
//
// Both constants are REWRITTEN IN PLACE by `scripts/hash-guest.sh` at
// post-build. The values below are the dev placeholders; a built dist
// always carries real hashes.
//
// `BUILD_ID` covers the whole worker artifact set (glue + wasm), so this
// script's bytes change whenever either half does — which is what makes
// the browser's byte-comparison update check fire. It also names the
// per-version caches, so two builds can never share cache state.
//
// `WORKER_WASM_HASH` is the sha256 prefix of `worker_bg.wasm` as built
// ALONGSIDE this exact glue. `oninstall` verifies the wasm it precaches
// against it, which is what keeps the two halves from drifting apart.
const BUILD_ID = "dev";
const WORKER_WASM_HASH = "dev";

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

// ---- Caches ----------------------------------------------------------
//
// Both caches are named per BUILD, not per schema version. Two workers
// from different builds therefore never read or write the same cache:
// an install populates its OWN cache, and `onactivate`'s purge drops
// everyone else's. That makes an install atomic — a half-populated
// incoming cache can't be observed by the still-serving old worker,
// which previously could hand out the new shell beside the old build's
// hashed assets during the swap window.
//
// The Rust side derives the same names from the build id handed to it
// at activate time (see `cache.rs`), so the name is injected once here
// rather than hand-synced across two languages.
const SHELL_CACHE = `TONK_SHELL_${BUILD_ID}`;

// Where this worker's own wasm lives. Separate from the shell cache so
// the shell's build-change prune can't evict the bytes this worker
// needs to boot.
const WORKER_CACHE = `TONK_WORKER_${BUILD_ID}`;
const WORKER_WASM_URL = new URL("./worker_bg.wasm", self.location.href).href;

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

// ---- Worker wasm: one atomic artifact set ----------------------------
//
// `service_worker.js` and its static import `worker.js` are pinned in
// the browser's SW script resource map at install time: the browser
// re-runs those exact bytes for the registration's lifetime. But
// `worker_bg.wasm` used to be fetched by `init()` at RUNTIME from a
// fixed URL, through the ordinary HTTP cache — which always answers
// with the newest deployed bytes.
//
// So after every deploy, any not-yet-updated worker that cold-started
// ran OLD GLUE AGAINST NEW WASM. Glue and wasm are tightly coupled
// (export indices, shim names), so that is an init failure at best and
// silent miswiring at worst. Safari terminates idle workers within
// seconds, which made the cold boot — and the skew — near-certain
// there: "stuck on a broken old version no matter what".
//
// The fix is to make the worker self-contained. At install we fetch the
// wasm, verify it against the hash stamped alongside THIS glue, and
// store it in a per-build cache. `init()` then instantiates from those
// cached bytes and never touches the network, so glue and wasm are the
// pair that was built together for as long as this worker lives.

/// sha256 prefix of `bytes`, in the same 16-hex-char form `hash_of` in
/// `hash-guest.sh` produces.
async function digestOf(bytes) {
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return Array.from(new Uint8Array(digest))
        .map(b => b.toString(16).padStart(2, "0"))
        .join("")
        .slice(0, 16);
}

/// Fetch this build's wasm and store it in the per-build worker cache,
/// verified against the stamped hash. Throws if the fetch fails or the
/// bytes don't match — `oninstall` propagates that, so a worker that
/// cannot assemble a coherent artifact set never installs and the old
/// (internally consistent) worker keeps running.
async function precacheWorkerWasm() {
    const cache = await caches.open(WORKER_CACHE);
    if (await cache.match(WORKER_WASM_URL)) return;

    // `no-store`: this must be the bytes on the origin right now, not
    // whatever an intermediate cache is holding.
    const response = await fetch(WORKER_WASM_URL, { cache: "no-store" });
    if (!response.ok) {
        throw new Error(`worker wasm fetch failed: ${response.status}`);
    }
    const bytes = await response.arrayBuffer();

    // A dev build carries the placeholder and has nothing to verify
    // against; a stamped build must match exactly.
    if (WORKER_WASM_HASH !== "dev") {
        const actual = await digestOf(bytes);
        if (actual !== WORKER_WASM_HASH) {
            throw new Error(
                `worker wasm hash mismatch: expected ${WORKER_WASM_HASH}, got ${actual}`,
            );
        }
    }

    await cache.put(
        WORKER_WASM_URL,
        new Response(bytes, {
            headers: { "content-type": "application/wasm" },
        }),
    );
}

/// The wasm bytes this worker boots from: the copy precached at
/// install. Falls back to a direct fetch only when the cache entry is
/// gone (storage pressure can evict it), which is still correct — a
/// worker whose install succeeded has already proven the deployed wasm
/// matched its glue at that moment.
async function workerWasmModule() {
    const cache = await caches.open(WORKER_CACHE);
    const cached = await cache.match(WORKER_WASM_URL);
    if (cached) return cached.arrayBuffer();
    log("Worker wasm missing from cache — refetching");
    await precacheWorkerWasm();
    const refreshed = await (await caches.open(WORKER_CACHE)).match(WORKER_WASM_URL);
    if (!refreshed) throw new Error("worker wasm unavailable");
    return refreshed.arrayBuffer();
}

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
        tonkServiceWorkerResolves = workerWasmModule()
            .then(module_or_path => init({ module_or_path }))
            .then(() => activate(BUILD_ID))
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

// ---- Remote kill switch ----------------------------------------------
//
// A worker that is broken in a way no page can recover from is the case
// nothing else here covers: the escape hatch on the failure page needs
// the user to reach that page and press a button, and a worker broken
// in a subtler way (serving, but wrong) never shows it at all.
//
// So: a tiny `no-store` flag file the worker checks at install and
// activate. When it names this build, the worker unregisters itself and
// clears its caches, and pages fall back to the network on their next
// load. Publishing a one-line JSON file is then enough to pull a bad
// deploy back out of every browser that already installed it — no user
// action, and no waiting for a normal update to be detected.
//
// Absent, unreachable, or malformed, it does nothing at all: the check
// is best-effort by construction, and a network blip must never
// unregister a healthy worker.
const KILL_SWITCH_URL = "/kill-switch.json";

async function killSwitchEngaged() {
    try {
        const response = await fetch(KILL_SWITCH_URL, { cache: "no-store" });
        if (!response.ok) return false;
        // An SPA host answers an unknown path with the shell HTML and a
        // 200, so "absent" arrives looking like success. Parse
        // defensively and treat anything that isn't the expected shape
        // as "no flag" — never as a reason to unregister.
        const text = await response.text();
        let revoked;
        try {
            ({ revoked } = JSON.parse(text));
        } catch {
            return false;
        }
        if (!Array.isArray(revoked)) return false;
        return revoked.includes(BUILD_ID);
    } catch {
        return false;
    }
}

/// Unregister this worker and drop every cache it owns. Pages already
/// open keep being served until they go away; their next navigation is
/// uncontrolled and goes straight to the network.
async function selfDestruct() {
    log(`Kill switch engaged for build ${BUILD_ID} — unregistering`);
    try {
        const names = await caches.keys();
        await Promise.all(
            names
                .filter(name => name.startsWith("TONK_SHELL_") || name.startsWith("TONK_WORKER_"))
                .map(name => caches.delete(name)),
        );
    } catch (err) {
        log("Kill-switch cache purge failed:", err);
    }
    try {
        await self.registration.unregister();
    } catch (err) {
        log("Kill-switch unregister failed:", err);
    }
    // Deliberately NOT navigating the open clients.
    //
    // Reloading them here looks helpful and is a trap: the fresh page
    // runs the registration script, installs this same revoked build
    // again, which activates, re-reads the flag, unregisters, and
    // reloads — a navigation loop that is worse than the bad worker.
    //
    // Unregistering is enough. This worker keeps serving the pages it
    // already controls until they go away, and their NEXT navigation
    // is uncontrolled and goes straight to the network. The page's own
    // update probe (`version.json`) is what tells the user to reload.
    log("Kill switch complete — this worker is unregistered");
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
        // The worker's own wasm FIRST, and un-caught: an install that
        // cannot assemble a coherent glue+wasm pair must fail, leaving
        // the old worker (which has a coherent pair of its own) in
        // place. Installing anyway is what produced a bricked worker
        // that no reload could clear.
        await precacheWorkerWasm();

        // The shell is best-effort by contrast — a worker with no
        // precached shell still serves; `serveNavigation` fetches on
        // the cold-cache path.
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
        // Before doing any work as the new controller: has this build
        // been revoked? Checked here rather than only at install so a
        // build already installed everywhere can still be pulled.
        if (await killSwitchEngaged()) {
            await selfDestruct();
            return;
        }
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
/// Whether this worker has already released its streams for a waiting
/// successor. Per worker instance, which is what we want: a restarted
/// worker re-checks. Declared here, above every use — a `let` read
/// before its declaration is a runtime TDZ error, not a hoist.
let retired = false;

/// Release this worker's long-lived streams because a successor is
/// waiting. Idempotent: safe to run from the `updatefound` event and
/// again at startup.
async function retire(reason) {
    log(`Retiring — ${reason}`);
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
        log("Failed to release streams:", err);
    }
}

// Catch-up check, run at every script evaluation.
//
// `updatefound` is an EVENT, and a service worker is killed and
// restarted constantly — so the event fires into a dead worker whenever
// the successor installs while this one is asleep. The listener below
// is then re-registered on restart having already missed its only
// notice, and this worker goes on holding streams open forever: the
// successor sits in `waiting`, reloads keep landing on the old active
// worker, and nothing ever breaks the deadlock. That is precisely the
// "waiting to activate" state that never clears.
//
// The registration itself is durable where the event is not, so ask it
// directly on every startup instead of trusting that we were awake.
if (self.registration.waiting) {
    retired = true;
    retire("a successor was already waiting at startup");
}

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
    await retire("a newer worker is installing");
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

    // Stale-while-revalidate leaves the page structurally ONE BUILD
    // BEHIND: it serves the cached shell and only refreshes it for next
    // time, so converging on a new build takes two reloads even when
    // everything else works. That's tolerable as a steady state and
    // wrong at the one moment the user is actively trying to update.
    //
    // So when a successor is already waiting, go network-first: the
    // reload the user just performed (very likely from the "update
    // ready" prompt) then lands on the new shell immediately. Falls
    // back to the cached shell if the network doesn't answer, because a
    // navigation must never hard-fail on a shell we already hold.
    if (self.registration.waiting) {
        try {
            const fresh = await fetch("/");
            if (fresh.ok && fresh.type !== "opaque") {
                event?.waitUntil?.(cache.put("/", fresh.clone()).catch(() => {}));
                return fresh;
            }
        } catch {
            // offline — fall through to the cached shell below
        }
    }

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
    // Same-origin only. Excluding opaque responses isn't enough: a
    // CORS-enabled cross-origin GET yields a perfectly ordinary
    // `basic`-looking success that would be stored in — and later
    // served from — the app's own shell cache, which has no business
    // holding another origin's resources.
    if (new URL(request.url).origin !== self.location.origin) return false;
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
/// After this many consecutive failed initializations, stop offering
/// only a retry — the cause is not transient, and a user reloading into
/// the same failure indefinitely is the reported "no matter what"
/// experience. Offer the reset ladder alongside it.
const STUCK_AFTER_ATTEMPTS = 3;

function failurePage() {
    const stuck = workerHealth.attempts >= STUCK_AFTER_ATTEMPTS;
    const detail = String(workerHealth.error || "unknown error");
    const escaped = detail
        .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    return new Response(
        `<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tonk failed to start</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0; display: grid;
         place-items: center; min-height: 100vh; min-height: 100dvh;
         background: #111; color: #eee; }
  main { max-width: 42rem; padding: 2rem; }
  h1 { font-size: 1.3rem; }
  pre { background: #1d1d1f; border: 1px solid #333; border-radius: 8px;
        padding: 1rem; white-space: pre-wrap; word-break: break-word;
        color: #f0b0b0; font-size: 0.85rem; }
  button { font: inherit; padding: 0.5rem 1.25rem; border-radius: 8px;
           border: 1px solid #555; background: #2a2a2e; color: #eee;
           cursor: pointer; }
  p.hint { color: #999; font-size: 0.85rem; }
</style></head><body><main>
<h1>Tonk failed to start</h1>
<p>The storage worker could not initialize. Attempt ${workerHealth.attempts}.</p>
<pre>${escaped}</pre>
<button onclick="location.reload()">Try again</button>
${stuck ? `<button id="reset">Reset and reload</button>` : ""}
<p class="hint">Diagnostics: <code>/api/health</code> has the full log ring.</p>
${stuck ? `<p class="hint">Repeated failures usually mean a bad cached worker.
Resetting clears Tonk's caches and unregisters the worker, then reloads.
Your data is stored separately and is not affected.</p>` : ""}
</main>
<script>
  // The recovery ladder, reachable from the page that actually needs
  // it. This page is NOT the boot shell, so the shell's own stall
  // watchdog (which does the same clear-and-unregister) never runs
  // here — "Try again" just reloads into the same failing init, with
  // the same pinned glue, forever. That made the one mechanism able
  // to heal a wedged worker unreachable from the only state where it
  // mattered.
  document.getElementById("reset")?.addEventListener("click", async event => {
    const button = event.currentTarget;
    button.disabled = true;
    button.textContent = "Resetting…";
    try {
      const registrations =
        await navigator.serviceWorker.getRegistrations();
      await Promise.all(registrations.map(r => r.unregister()));
      const names = await caches.keys();
      await Promise.all(names.map(name => caches.delete(name)));
    } catch (err) {
      console.error("reset failed", err);
    }
    // Bypass the HTTP cache too, so the reload refetches the worker
    // script rather than replaying whatever put us here.
    location.replace(location.pathname + "?reset=" + Date.now());
  });
</script>
</body></html>`,
        { status: 503, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

/// How often a running worker re-checks the kill switch. The check at
/// activate only covers a worker that newly activates — but the worker
/// that most needs revoking is one already installed and activated
/// everywhere, which may not activate again for days. So re-check
/// periodically off the fetch path (which is free: it piggybacks on
/// traffic the worker is already serving).
const KILL_SWITCH_INTERVAL_MS = 30 * 60 * 1000;
let killSwitchCheckedAt = 0;

/// Release streams if a successor is waiting and we have not already.
///
/// Called from the fetch path because that is the one thing guaranteed
/// to run. A worker holding an open SSE stream is never killed — the
/// stream keeps it alive — so it never restarts, never re-evaluates
/// this script, and never reaches the startup catch-up above. Its only
/// remaining contact with the outside world is the fetches it serves,
/// so the check has to live here too.
function retireIfSuperseded(event) {
    if (retired || !self.registration.waiting) return;
    retired = true;
    event.waitUntil?.(retire("a successor is waiting"));
}

function maybeCheckKillSwitch(event) {
    const now = Date.now();
    if (now - killSwitchCheckedAt < KILL_SWITCH_INTERVAL_MS) return;
    killSwitchCheckedAt = now;
    event.waitUntil?.(
        (async () => {
            if (await killSwitchEngaged()) await selfDestruct();
        })(),
    );
}

self.onfetch = event => {
    const path = new URL(event.request.url).pathname;
    // A waiting successor means this worker is retiring. Checked on the
    // fetch path because a worker pinned by its own open streams never
    // restarts to run the startup check.
    retireIfSuperseded(event);
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
    // Control files: the update probe and the kill switch. NOT
    // intercepted at all, deliberately.
    //
    // Both exist to be readable when the worker is the thing that is
    // wrong, so routing them through the worker defeats their purpose.
    // Worse, an SPA host answers an unknown path with the shell, so a
    // missing `kill-switch.json` came back as HTML — which parsed as
    // neither JSON nor a valid absence. Letting the browser fetch them
    // directly keeps a real 404 a real 404.
    if (path === "/version.json" || path === KILL_SWITCH_URL) {
        return;
    }
    // Piggyback the periodic revocation check on a navigation — the
    // moment a self-unregister is cheapest, since the page is loading
    // anyway. Placed AFTER the control-file early-out so the probe can
    // never trigger itself.
    if (event.request.mode === "navigate") {
        maybeCheckKillSwitch(event);
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

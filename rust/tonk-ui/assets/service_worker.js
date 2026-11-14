// The app origin's service worker.
//
// Every address on this origin is the same static page. It frames the
// profile's site, tells that site where the address bar is, and runs the
// passkey ceremonies the profile's worker asks for. This worker answers each
// navigation with that page and keeps what the page loads, so the page opens
// with no network. It holds no database and speaks to no other worker: the
// profile and each space have origins, and workers, of their own.

// The identity of the build this worker serves, stamped after the build.
// Unstamped under the dev server, where nothing is kept: every build there
// carries the same name, and a kept file would never be replaced.
const BUILD_ID = "dev";
const CACHE = `TONK_APP_${BUILD_ID}`;
const KEEPS = BUILD_ID !== "dev";
const PAGE = new URL("/", self.location.origin).href;
// Where the deployment says its sites are. The page cannot frame the
// profile's site without it, so it is kept like the page is.
const CONFIG = new URL("/.well-known/tonk", self.location.origin).href;

// What the server answers itself: the access service's routes, a short
// link, the agent's static pages, the OAuth server an LLM host opens in this
// browser for `/mcp`, and what names the latest build.
const SERVED = [
    "/.well-known/",
    "/ucan",
    "/customer/",
    "/object/",
    "/connection/",
    "/agent/",
    "/api/",
    "/version.json",
];

// Addresses the server answers exactly, with their own paths beneath them.
const SERVED_EXACTLY = ["/@", "/authorize", "/token", "/register"];

function served(path) {
    return SERVED_EXACTLY.some(exact => path === exact || path.startsWith(`${exact}/`)) ||
        SERVED.some(prefix => path.startsWith(prefix));
}

// A navigation is for the page unless the server answers that address
// itself, or it names a file.
function isPage(url) {
    return !served(url.pathname) && !url.pathname.split("/").pop().includes(".");
}

self.oninstall = event => {
    event.waitUntil((async () => {
        // This worker never installs on a site's origin: it would take the
        // scope the site's own worker serves.
        const label = new URL(self.location.href).hostname.split(".")[0];
        if (/^(profile|b[a-z2-7]{40,})(-[a-z0-9]+)?$/.test(label)) {
            throw new Error("the app's worker does not install on a site's origin");
        }
        if (KEEPS) {
            const cache = await caches.open(CACHE);
            await cache.add(new Request(PAGE, { cache: "reload" }));
            await cache.add(new Request(CONFIG, { cache: "reload" })).catch(() => {});
        }
        await self.skipWaiting();
    })());
};

self.onactivate = event => {
    event.waitUntil((async () => {
        // What an earlier build kept is no use to this one.
        for (const name of await caches.keys()) {
            if (name !== CACHE) await caches.delete(name);
        }
        await self.clients.claim();
    })());
};

// How long the page waits on the server before it opens from what is kept.
const NETWORK_MS = 15_000;

// The page or the configuration at `url`, from the server when it answers
// and from what is kept when it does not. A server that does not answer at
// all is waited on only so long when something is kept: the fetch goes on
// and keeps what it gets for the next load.
async function latest(url) {
    const fetched = (async () => {
        try {
            const fresh = await fetch(url, { cache: "no-store" });
            if (fresh.ok) {
                if (KEEPS) await (await caches.open(CACHE)).put(url, fresh.clone());
                return fresh;
            }
        } catch {
            // Offline: what is kept answers below.
        }
        return null;
    })();
    const kept = await caches.match(url, { cacheName: CACHE });
    let timer;
    const waited = kept
        ? new Promise(resolve => { timer = setTimeout(() => resolve(null), NETWORK_MS); })
        : new Promise(() => {});
    const fresh = await Promise.race([fetched, waited]);
    clearTimeout(timer);
    return fresh ?? kept ?? Response.error();
}

// A file the page loads: kept once fetched, and answered from what is kept.
// A build's files never change under its name.
async function file(request) {
    const cache = await caches.open(CACHE);
    const kept = await cache.match(request);
    if (kept) return kept;
    const fresh = await fetch(request);
    if (fresh.ok && fresh.type === "basic") cache.put(request, fresh.clone());
    return fresh;
}

// What the page loaded before this worker was there to keep it. The page
// says what that was once the worker is, and it is kept now: the browser
// still holds each, so asking again costs nothing. Without it a first
// visit could not be opened again with no network.
self.onmessage = event => {
    const urls = event.data?.type === "keep" ? event.data.urls : null;
    if (!KEEPS || !Array.isArray(urls)) return;
    event.waitUntil((async () => {
        const cache = await caches.open(CACHE);
        for (const url of urls) {
            const at = new URL(url, self.location.origin);
            if (at.origin !== self.location.origin || served(at.pathname)) continue;
            if (await cache.match(at.href)) continue;
            try {
                const fresh = await fetch(at.href);
                if (fresh.ok) await cache.put(at.href, fresh);
            } catch {
                // Not to be had now: kept when the page next loads it.
            }
        }
    })());
};

self.onfetch = event => {
    const request = event.request;
    const url = new URL(request.url);
    if (request.method !== "GET" || url.origin !== self.location.origin) return;
    if (request.mode === "navigate") {
        if (isPage(url)) event.respondWith(latest(PAGE));
        return;
    }
    if (KEEPS && url.href === CONFIG) {
        event.respondWith(latest(CONFIG));
        return;
    }
    if (KEEPS && !served(url.pathname)) event.respondWith(file(request));
};

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

// What the server answers itself: the access service's routes, a short
// link, the agent's static pages, and what names the latest build.
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

function served(path) {
    return path === "/@" || path.startsWith("/@/") || SERVED.some(prefix => path.startsWith(prefix));
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

// The page, from the server when it answers and from what is kept when it
// does not.
async function page() {
    try {
        const fresh = await fetch(PAGE, { cache: "no-store" });
        if (fresh.ok) {
            if (KEEPS) (await caches.open(CACHE)).put(PAGE, fresh.clone());
            return fresh;
        }
    } catch {
        // Offline: what is kept answers below.
    }
    return (await caches.match(PAGE, { cacheName: CACHE })) ?? Response.error();
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
        if (isPage(url)) event.respondWith(page());
        return;
    }
    if (KEEPS && !served(url.pathname)) event.respondWith(file(request));
};

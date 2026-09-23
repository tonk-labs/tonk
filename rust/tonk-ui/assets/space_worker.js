// The service worker of a site origin (`{label}.{host}`), proof of concept.
//
// Each space renders at its own origin, and this worker controls that
// origin. The profile renders on one too (`profile.{host}`), with no blobs.
// It answers three kinds of request and refuses the rest:
//
// - Navigations get the static shell, carrying the space's CSP. The server
//   hands out the same shell for any path, so a deep link with no worker yet
//   still boots one.
// - `/blob/{hash}` is read from the space and served natively, so `<img>`,
//   `<video>` and `<link>` just work. The bytes come from the host worker
//   over a `MessagePort`, since the space database still lives on the host
//   origin. The host binds that port to this space, so nothing here ever
//   names a repository.
// - The app's own static assets (`/images/`, `/fonts/`) pass through to the
//   server, which serves them on every host. A sealed frame used to reach
//   them on the host origin; this origin is where relative URLs land now.
// - Everything else is a 404. Author code has no network through this worker.
//
// The port does not survive a restart of either worker. A restarted space
// worker has none and asks its clients to broker one from the host; a
// restarted host worker stops acknowledging, and the space worker drops the
// dead port and asks again.

const SHELL_PATH = "/space-origin.html";
const SHELL_CACHE = "tonk-space-shell";
// The app's static assets, served by the server on every host.
const STATIC_PREFIXES = ["/images/", "/fonts/"];
// A base58btc blob hash: the only thing a `/blob/` path may carry.
const BLOB_PATH = /^\/blob\/([1-9A-HJ-NP-Za-km-z]+)$/;
// How long a client may take to broker a port, and the host to acknowledge a
// request. A silent host is presumed restarted, not slow: it acknowledges
// before reading anything, so a large blob does not trip this.
const PORT_TIMEOUT_MS = 5_000;
const ACK_TIMEOUT_MS = 3_000;

const log = (...args) => console.log("[Space Worker]", ...args);

self.addEventListener("install", event => {
    self.skipWaiting();
    event.waitUntil(caches.open(SHELL_CACHE).then(cache => cache.add(SHELL_PATH)));
});

// Claim right away: the shell waits for control before it asks the host for
// its document, so the first load is served blobs too.
self.addEventListener("activate", event => {
    event.waitUntil(self.clients.claim());
});

// ---- The port to the host worker ----------------------------------------

let hostPort = null;
let portWaiters = [];
let nextId = 1;
const pending = new Map();

self.addEventListener("message", event => {
    if (event.data?.type !== "port") return;
    const [port] = event.ports;
    if (port) adopt(port);
});

function adopt(port) {
    // A replaced port is left open: replies to requests already sent on it
    // still arrive, and every reply is matched by id, not by port.
    port.onmessage = onReply;
    hostPort = port;
    for (const resolve of portWaiters.splice(0)) resolve(port);
    log("adopted a port to the host worker");
}

function onReply({ data }) {
    const entry = pending.get(data?.id);
    if (!entry) return;
    if (data.ack) {
        entry.acked();
        return;
    }
    pending.delete(data.id);
    if (data.error) entry.reject(new Error(data.error));
    else entry.resolve(data);
}

async function portToHost() {
    if (hostPort) return hostPort;
    const brokered = new Promise(resolve => portWaiters.push(resolve));
    // Any open client can broker a port; ask them all and take the first.
    const clients = await self.clients.matchAll({ type: "window" });
    if (clients.length === 0) {
        throw new Error("no client to broker a port to the host worker");
    }
    for (const client of clients) client.postMessage({ type: "need-port" });
    return within(brokered, PORT_TIMEOUT_MS, "no client brokered a port in time");
}

// Ask the host worker, re-brokering once if the port has gone dead.
async function askHost(request) {
    for (let attempt = 0; ; attempt++) {
        const port = await portToHost();
        const id = nextId++;
        let acked;
        const ack = new Promise(resolve => (acked = resolve));
        const reply = new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject, acked });
        });
        port.postMessage({ ...request, id });
        try {
            await within(ack, ACK_TIMEOUT_MS, "the host worker did not answer");
        } catch (error) {
            pending.delete(id);
            if (hostPort === port) hostPort = null;
            if (attempt > 0) throw error;
            log("port to the host went quiet; asking for a new one");
            continue;
        }
        return reply;
    }
}

function within(promise, ms, message) {
    let timer;
    const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), ms);
    });
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

// ---- Responses ----------------------------------------------------------

// The site origin's policy. The host frames the profile, and the profile
// frames the spaces, so both may be ancestors and sites may frame sites. Author
// code gets no network beyond this origin, which this worker answers alone;
// `worker-src blob:` stops `register()` of any other service worker, since a
// service worker script must be same-origin. `'unsafe-inline'` and
// `'wasm-unsafe-eval'` carry the injected runtime until it loads from this
// origin instead. `'unsafe-eval'` is for author views and element shims,
// which are compiled from strings; space code is untrusted by design, so the
// boundary is this origin and its lack of network, not `script-src`.
function spacePolicy() {
    const host = self.location.host.split(".").slice(1).join(".");
    const parent = `${self.location.protocol}//${host}`;
    const profile = `${self.location.protocol}//profile.${host}`;
    const sites = `${self.location.protocol}//*.${host}`;
    return [
        "default-src 'none'",
        "script-src 'self' blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'",
        "style-src 'self' blob: 'unsafe-inline'",
        "img-src 'self' blob: data:",
        "media-src 'self' blob:",
        "font-src 'self' data:",
        "connect-src 'self' blob: data:",
        `frame-src 'self' blob: ${sites}`,
        "worker-src blob:",
        "form-action 'none'",
        "base-uri 'self'",
        `frame-ancestors ${parent} ${profile}`,
    ].join("; ");
}

async function serveShell() {
    let response;
    try {
        response = await fetch(SHELL_PATH, { cache: "no-cache" });
        if (response.ok) {
            const cache = await caches.open(SHELL_CACHE);
            await cache.put(SHELL_PATH, response.clone());
        }
    } catch {
        response = await caches.match(SHELL_PATH);
    }
    if (!response) return new Response("offline", { status: 503 });
    const headers = new Headers(response.headers);
    headers.set("content-security-policy", spacePolicy());
    headers.set("x-content-type-options", "nosniff");
    return new Response(response.body, { status: response.status, headers });
}

async function serveBlob(hash, request) {
    const reply = await askHost({ blob: hash });
    const headers = new Headers(reply.headers);
    headers.set("x-content-type-options", "nosniff");
    if (reply.status !== 200) {
        return new Response(reply.body, { status: reply.status, headers });
    }
    headers.set("accept-ranges", "bytes");
    const range = parseRange(request.headers.get("range"), reply.body.byteLength);
    if (!range) {
        headers.set("content-length", String(reply.body.byteLength));
        return new Response(request.method === "HEAD" ? null : reply.body, { headers });
    }
    const [start, end] = range;
    headers.set("content-range", `bytes ${start}-${end}/${reply.body.byteLength}`);
    headers.set("content-length", String(end - start + 1));
    const body = request.method === "HEAD" ? null : reply.body.slice(start, end + 1);
    return new Response(body, { status: 206, headers });
}

// A single `bytes=` range, clamped to the body. Anything else reads whole.
function parseRange(header, size) {
    const match = /^bytes=(\d*)-(\d*)$/.exec(header ?? "");
    if (!match || size === 0) return null;
    const [, from, to] = match;
    if (from === "" && to === "") return null;
    if (from === "") {
        const suffix = Math.min(Number(to), size);
        return [size - suffix, size - 1];
    }
    const start = Number(from);
    if (start >= size) return null;
    const end = to === "" ? size - 1 : Math.min(Number(to), size - 1);
    return end < start ? null : [start, end];
}

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (event.request.mode === "navigate") {
        event.respondWith(serveShell());
        return;
    }
    const blob = BLOB_PATH.exec(url.pathname);
    if (blob && (event.request.method === "GET" || event.request.method === "HEAD")) {
        event.respondWith(
            serveBlob(blob[1], event.request).catch(error => {
                log("blob read failed:", error);
                return new Response(String(error.message), { status: 502 });
            }),
        );
        return;
    }
    if (url.pathname === SHELL_PATH) return;
    if (
        event.request.method === "GET" &&
        STATIC_PREFIXES.some(prefix => url.pathname.startsWith(prefix))
    ) {
        return;
    }
    event.respondWith(new Response("not found", { status: 404 }));
});

// The service worker of a space origin: the host's bootloader.
//
// The host owns this script and the embedder it loads (the Wasm Component
// Model Polyfill, built into `embedder_bg.wasm`); both follow the deployment.
// The space owns only the pin: the BLAKE3 hash of the guest kernel it runs,
// named in this worker's own address as `?kernel={hash}`. A kernel is data,
// not a script, so it is fetched, checked against the pin, and booted.
importScripts("/embedder.js");

const PINNED = new URL(self.location.href).searchParams.get("kernel");
const HOST_FILES = new Set(["/", "/index.html", "/boot.js", "/embedder.js", "/embedder_bg.wasm"]);

// The bytes of the pinned kernel: from the cache when it has them, since a
// hash names them for ever, and from the origin otherwise.
async function kernelBytes(pin) {
    const cache = await caches.open("kernels");
    const address = `/_kernel/${pin}.wasm`;
    let response = await cache.match(address);
    const cached = !!response;
    if (!response) {
        response = await fetch(address);
        if (!response.ok) throw new Error(`no kernel ${pin}: ${response.status}`);
        await cache.put(address, response.clone());
    }
    return { bytes: new Uint8Array(await response.arrayBuffer()), cached };
}

// The space's store, kept across worker restarts: each record the embedder
// drains, under a key that names what it writes, so a later write replaces it.
const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
const keyOf = record =>
    record.kind === "block" ? `block|${record.subject}|${record.catalog}|${hex(record.key)}`
    : record.kind === "blob" ? `blob|${record.subject}|${hex(record.key)}`
    : `cell|${record.subject}|${record.namespace}|${record.cell}`;
const database = new Promise((resolve, reject) => {
    const request = indexedDB.open("space", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("records");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
});
async function kept() {
    const db = await database;
    return new Promise((resolve, reject) => {
        const request = db.transaction("records").objectStore("records").getAll();
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
    });
}
async function keep(records) {
    if (!records.length) return;
    const db = await database;
    await new Promise((resolve, reject) => {
        const transaction = db.transaction("records", "readwrite");
        const store = transaction.objectStore("records");
        for (const record of records) store.put(record, keyOf(record));
        transaction.oncomplete = resolve;
        transaction.onerror = () => reject(transaction.error);
    });
}

const started = performance.now();
const booting = (async () => {
    if (!PINNED) throw new Error("this worker pins no kernel");
    const stages = {};
    let mark = performance.now();
    await wasm_bindgen({ module_or_path: "/embedder_bg.wasm" });
    stages.embedder = performance.now() - mark;
    mark = performance.now();
    const { bytes, cached } = await kernelBytes(PINNED);
    stages.kernelFetch = performance.now() - mark;
    mark = performance.now();
    const records = await kept();
    stages.read = performance.now() - mark;
    const embedder = await wasm_bindgen.boot(bytes, PINNED, 2, 3, records);
    mark = performance.now();
    const written = await embedder.drain();
    await keep(written);
    stages.write = performance.now() - mark;
    return { embedder, stages: { ...stages, ...embedder.timings, total: performance.now() - started }, cached, restored: records.length, wrote: written.length };
})();

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin || HOST_FILES.has(url.pathname) || url.pathname.startsWith("/_kernel/")) return;
    if (url.pathname === "/_boot") {
        event.respondWith(
            booting.then(
                ({ stages, cached, restored, wrote }) => Response.json({ pin: PINNED, cached, restored, wrote, stages }),
                error => Response.json({ pin: PINNED, error: String(error) }, { status: 500 }),
            ),
        );
        return;
    }
    event.respondWith(
        (async () => {
            const { embedder } = await booting;
            const body = new Uint8Array(await event.request.arrayBuffer());
            const reply = await embedder.handle(event.request.method, url.pathname + url.search, body);
            event.waitUntil(embedder.drain().then(keep));
            return new Response(reply.body, { status: reply.status });
        })().catch(error => new Response(String(error), { status: 502 })),
    );
});

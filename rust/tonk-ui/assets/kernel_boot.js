// The service worker of a space that runs a kernel: the host's bootloader for
// a space on host version 2 (see `plan/space-kernel.md`).
//
// The space holds its kernel: the kernel's wasm is a blob of the space, and
// the space's boot cell pins it by its content identifier. This script and
// the embedder it loads are the host's, and follow the deployment. The
// worker's address names the kernel the space pins, `?kernel={id}`, so a
// space that moves its pin is a space with another worker.
//
// What this worker does:
//
// - Keeps the space's store: each record the embedder writes, in IndexedDB.
// - Asks the person's profile, over the port a page of this origin hands it,
//   for a delegation for the space, which says which space this origin is.
// - Installs into a space that holds no kernel the one this deployment
//   ships (`/_host/kernel/{id}.wasm`), as the space's own blob and boot cell.
//   From then on the kernel comes from the space's store, not from here.
// - Boots the kernel the space pins and hands it every request but the
//   shell's: a page load gets the shell, unless the shell asked for the
//   address's content, and anything else is the kernel's to answer. A space
//   nothing has been installed into answers every page with a 404.

importScripts("/_host/embedder.js");

const PINNED = new URL(self.location.href).searchParams.get("kernel");
// The contract between this worker and its shell, as for a release.
const PROTOCOL = 2;
const SHELL_PATH = "/space.html";
const SHELL_MARK = '<meta name="tonk-shell" content="worker" />';
// What this origin serves from the deployment, never from the kernel.
const HOST_FILES = new Set([SHELL_PATH, "/kernel_boot.js"]);
const HOST_PREFIXES = ["/_host/", "/.well-known/"];

const log = (...args) => console.log("[Space Kernel]", ...args);

// ---- The space's store --------------------------------------------------
//
// Each record the embedder drains, under a key naming what it writes, so a
// later write replaces it. `meta` keeps what is this origin's own: its
// session key and the delegation it holds.

const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
const keyOf = record =>
    record.kind === "block" ? `block|${record.subject}|${record.catalog}|${hex(record.key)}`
    : record.kind === "blob" ? `blob|${record.subject}|${hex(record.key)}`
    : `cell|${record.subject}|${record.namespace}|${record.cell}`;

const database = new Promise((resolve, reject) => {
    const request = indexedDB.open("space-kernel", 1);
    request.onupgradeneeded = () => {
        request.result.createObjectStore("records");
        request.result.createObjectStore("meta");
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
});

async function transact(store, mode, act) {
    const db = await database;
    return new Promise((resolve, reject) => {
        const transaction = db.transaction(store, mode);
        const result = act(transaction.objectStore(store));
        transaction.oncomplete = () => resolve(result?.result);
        transaction.onerror = () => reject(transaction.error);
    });
}

const kept = () => transact("records", "readonly", store => store.getAll());
async function keep(records) {
    if (!records.length) return;
    await transact("records", "readwrite", store => {
        for (const record of records) store.put(record, keyOf(record));
    });
}
const readMeta = key => transact("meta", "readonly", store => store.get(key));
const writeMeta = (key, value) => transact("meta", "readwrite", store => store.put(value, key));

// The key this origin signs with: made on its first start, and never sent
// anywhere. The profile delegates the space to it.
async function sessionSeed() {
    let seed = await readMeta("session");
    if (!seed) {
        seed = crypto.getRandomValues(new Uint8Array(32));
        await writeMeta("session", seed);
    }
    return seed;
}

// ---- The port up --------------------------------------------------------
//
// A page of this origin hands this worker a port to the person's profile,
// and hands it again after either worker restarts. Requests go up it with an
// id, are acknowledged, and are answered by id.

let hostPort = null;
let portWaiters = [];
let nextId = 1;
const pending = new Map();

function adopt(port) {
    port.onmessage = ({ data }) => {
        if (typeof data?.ping === "number") {
            port.postMessage({ pong: data.ping });
            return;
        }
        const waiter = pending.get(data?.id);
        if (!waiter || data.ack) return;
        pending.delete(data.id);
        if (data.error) waiter.reject(new Error(data.error));
        else waiter.resolve(data);
    };
    hostPort = port;
    for (const resolve of portWaiters.splice(0)) resolve(port);
    log("adopted a port up");
}

async function askPages() {
    const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const page of pages) page.postMessage({ type: "need-port" });
}

function portUp() {
    if (hostPort) return Promise.resolve(hostPort);
    askPages();
    return new Promise(resolve => portWaiters.push(resolve));
}

async function askHost(request) {
    const port = await portUp();
    const id = nextId++;
    return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        port.postMessage({ id, ...request });
    });
}

// ---- The space ----------------------------------------------------------

// The delegation this origin holds for its space, asked of the profile once.
// It says which space this origin is: the label of an origin does not.
async function grant(seed) {
    const held = await readMeta("grant");
    if (held) return held;
    const audience = wasm_bindgen.session(seed);
    const answer = await askHost({ delegate: audience });
    const taken = { space: answer.space, chain: new Uint8Array(answer.chain), expires: answer.expires };
    await writeMeta("grant", taken);
    // A space that runs a kernel starts blank: nothing of what the profile
    // would seed it with is taken. Saying it holds the space lets the
    // profile stop waiting for it to.
    await askHost({ seeded: true, from: "kernel" }).catch(error => log("the profile was not told:", error));
    return taken;
}

// The kernel this deployment ships, checked against the id that names it.
async function shippedKernel(id) {
    const response = await fetch(`/_host/kernel/${id}.wasm`);
    if (!response.ok) throw new Error(`the deployment ships no kernel ${id}: ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const named = wasm_bindgen.kernelId(bytes);
    if (named !== id) throw new Error(`the kernel at ${id} is ${named}`);
    return bytes;
}

const started = performance.now();
const booting = (async () => {
    if (!PINNED) throw new Error("this worker pins no kernel");
    await wasm_bindgen({ module_or_path: "/_host/embedder_bg.wasm" });
    const seed = await sessionSeed();
    const { space } = await grant(seed);
    let records = await kept();
    // A space with no kernel of its own yet is one being created: install the
    // one its worker was registered for.
    if (!wasm_bindgen.pinned(space, records)) {
        const installed = wasm_bindgen.install(space, await shippedKernel(PINNED));
        await keep(installed);
        records = await kept();
        log("installed the kernel", PINNED);
    }
    const pinned = wasm_bindgen.pinned(space, records);
    if (pinned !== PINNED) throw new Error(`the space pins ${pinned}, not ${PINNED}`);
    const embedder = await wasm_bindgen.boot(space, seed, records);
    await keep(await embedder.drain());
    log("booted in", Math.round(performance.now() - started), "ms", embedder.timings);
    return embedder;
})();
booting.catch(error => log("the kernel did not boot:", error));

// ---- Requests -----------------------------------------------------------

// Addresses whose next load is answered with their content, as a release
// does: the shell that lands in a frame of this origin asks for it.
const admitted = new Map();
function takeAdmission(url) {
    const until = admitted.get(url);
    admitted.delete(url);
    return until !== undefined && until > Date.now();
}

async function serveShell() {
    const response = await fetch(SHELL_PATH, { cache: "no-cache" });
    const headers = new Headers(response.headers);
    headers.delete("content-length");
    const shell = (await response.text()).replace("<head>", `<head>\n        ${SHELL_MARK}`);
    return new Response(shell, { status: response.status, headers });
}

async function serveKernel(event) {
    const embedder = await booting;
    const url = new URL(event.request.url);
    const body = new Uint8Array(await event.request.arrayBuffer());
    const reply = await embedder.handle(event.request.method, url.pathname + url.search, body);
    event.waitUntil(embedder.drain().then(keep));
    return new Response(reply.body, { status: reply.status });
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

self.addEventListener("message", event => {
    const type = event.data?.type;
    if (type === "protocol") event.ports[0]?.postMessage({ protocol: PROTOCOL });
    else if (type === "claim") event.waitUntil(self.clients.claim());
    else if (type === "content") {
        admitted.set(event.data.url, Date.now() + 5_000);
        event.ports[0]?.postMessage({ admitted: true });
    } else if (type === "port" && event.ports[0]) {
        adopt(event.ports[0]);
        event.waitUntil(booting.catch(() => {}));
    }
});

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (HOST_FILES.has(url.pathname) || HOST_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) return;
    if (event.request.mode === "navigate" && !takeAdmission(url.href)) {
        event.respondWith(serveShell());
        return;
    }
    event.respondWith(
        serveKernel(event).catch(error => new Response(String(error?.message ?? error), { status: 502 })),
    );
});

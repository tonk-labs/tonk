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
// - Installs into a space that holds no kernel the one it was made for, which
//   the person's profile hands it, as the space's own blob and boot cell.
//   From then on the kernel comes from the space's store, not from anywhere
//   else. The kernel founds the space on it: its first commit records the
//   kernel and the pin to it.
// - Keeps the host's own files it serves, so a space that started once
//   starts again offline.
// - Boots the kernel the space pins and hands it every request of this
//   origin but the host's own files. Each page it answers with carries the
//   host's broker (`kernel_broker.js`), which carries the port up to the
//   person's profile; a page load the kernel answers with no page gets one of
//   the host's that says what the kernel answered. A space nothing has been
//   installed into answers every page with a 404.

importScripts("/_host/embedder.js");

const PINNED = new URL(self.location.href).searchParams.get("kernel");
// The contract between this worker and its shell, as for a release.
const PROTOCOL = 2;
const BROKER_PATH = "/kernel_broker.js";
// What this origin serves from the deployment, never from the kernel.
const HOST_FILES = new Set(["/space.html", "/kernel_boot.js", BROKER_PATH]);
const HOST_PREFIXES = ["/_host/", "/.well-known/"];

const log = (...args) => console.log("[Space Kernel]", ...args);

// What the kernel wrote since the last time, kept a moment after each write
// so a burst is kept at once.
let draining = null;
function drainSoon(embedder) {
    draining ??= new Promise(resolve => setTimeout(resolve, 50))
        .then(() => embedder.drain())
        .then(keep)
        .catch(error => log("what the kernel wrote was not kept:", error))
        .finally(() => {
            draining = null;
        });
    return draining;
}

// The body of an answer of the kernel, as it writes it.
function bodyOf(embedder, reply) {
    return new ReadableStream({
        async pull(controller) {
            const piece = await reply.body.next();
            drainSoon(embedder);
            if (piece) controller.enqueue(piece);
            else controller.close();
        },
        cancel() {
            reply.body.cancel();
        },
    });
}

// A whole answer of the kernel, for an answer that ends.
async function whole(embedder, reply) {
    return new Uint8Array(await new Response(bodyOf(embedder, reply)).arrayBuffer());
}

// ---- The space's store --------------------------------------------------
//
// Each record the embedder drains, under a key naming what it writes, so a
// later write replaces it. `meta` keeps what is this origin's own: its
// session key and the delegation it holds.

const hex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
const keyOf = record =>
    record.kind === "block" ? `block|${record.subject}|${record.catalog}|${hex(record.key)}`
    : record.kind === "blob" ? `blob|${record.subject}|${hex(record.key)}`
    : record.kind === "key" ? `key|${record.subject}|${record.address}`
    : record.kind === "secret" ? `secret|${record.subject}|${record.site}`
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
        // A request the profile passes down about this space, answered by
        // the kernel as a page's would be.
        if (typeof data?.call === "number") {
            answerCall(port, data);
            return;
        }
        if (typeof data?.cancel === "number") {
            cancelCall(data.cancel);
            return;
        }
        // The profile asks for a copy of the space to make another from. A
        // kernel space has no such copy to give yet.
        if (typeof data?.ask === "number") {
            port.postMessage({ answer: data.ask, empty: true });
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

// Ask for a new delegation once the one the kernel holds has less than this
// left.
const RENEW_MARGIN_SECONDS = 60 * 60;

// The kernel acts for the space as a profile of its own, which it makes on
// its first start: the profile delegates the space to that, and the kernel
// takes the delegation up. Asked again when the kernel's profile is another
// or the one held is about to lapse.
async function takeUpSpace(embedder) {
    const held = await readMeta("adopted");
    const identified = await embedder.handle("GET", "/api/identify", [], new Uint8Array());
    const who = await whole(embedder, identified);
    if (identified.status !== 200) throw new Error(`the kernel did not say who it is: ${identified.status}`);
    const { did } = JSON.parse(new TextDecoder().decode(who));
    if (held?.did === did && held.expires - Date.now() / 1000 > RENEW_MARGIN_SECONDS) return;
    const answer = await askHost({ delegate: did });
    const taken = await embedder.handle("POST", "/.kernel/adopt", [], new Uint8Array(answer.chain));
    const said = await whole(embedder, taken);
    await drainSoon(embedder);
    if (taken.status !== 204) {
        throw new Error(`the kernel did not take up its space: ${new TextDecoder().decode(said)}`);
    }
    await writeMeta("adopted", { did, expires: answer.expires });
    log("took up", answer.space, "until", new Date(answer.expires * 1000).toISOString());
}

// The host's files this origin serves, from the network while it answers,
// and from what this worker kept of them when it does not: a space that
// started once starts again offline.
const HOST_CACHE = "tonk-host";
async function hostFile(request) {
    try {
        const response = await fetch(request);
        if (response.ok) await (await caches.open(HOST_CACHE)).put(request, response.clone());
        return response;
    } catch (error) {
        const kept = await caches.match(request, { cacheName: HOST_CACHE });
        if (kept) return kept;
        throw error;
    }
}

// The kernel this space was made for: from the person's profile, which holds
// the deployment's copy, so a space made on this device takes nothing from
// anywhere else. Checked against the id that names it.
async function shippedKernel(id) {
    const answer = await askHost({ kernel: id });
    const bytes = new Uint8Array(answer.bytes);
    const named = wasm_bindgen.kernelId(bytes);
    if (named !== id) throw new Error(`the kernel at ${id} is ${named}`);
    return bytes;
}

async function start() {
    const started = performance.now();
    if (!PINNED) throw new Error("this worker pins no kernel");
    await wasm_bindgen({ module_or_path: await hostFile(new Request(new URL("/_host/embedder_bg.wasm", self.location.href))) });
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
}

// Boot the kernel, then have it take up its space. A page load waits on the
// boot alone: after a space's first start, booting needs nothing of the
// person's profile, and a delegation renewed alongside never holds a page up.
// What the profile asks of the space waits on both. A kernel that has not
// taken up its space still answers, with what it holds: nothing of the space.
// The next start asks again.
function run() {
    booting = start();
    booting.catch(error => log("the kernel did not boot:", error));
    takenUp = booting.then(embedder =>
        takeUpSpace(embedder)
            .catch(error => log("the kernel did not take up its space:", error))
            .then(() => embedder),
    );
    // Whoever waits on it hears a boot that failed; nothing else need.
    takenUp.catch(() => {});
}
let booting;
let takenUp;
run();

// A kernel that trapped answers nothing again: boot it anew, from what the
// space's store kept. At most once in a while, so a kernel that traps as it
// starts does not loop.
const RESTART_EVERY_MS = 10_000;
let restarted = Date.now();
function restartIfStopped(error) {
    if (!/the kernel has stopped/.test(String(error?.message ?? error))) return;
    if (Date.now() - restarted < RESTART_EVERY_MS) return;
    restarted = Date.now();
    log("the kernel stopped; booting it again");
    run();
}

// ---- Requests -----------------------------------------------------------

// The broker, as the first thing in a page's head: before any script of the
// page's own.
const BROKER = `<script src="${BROKER_PATH}"></script>`;
// How far into a page its head is looked for.
const HEAD_WITHIN = 64 * 1024;

// `page`, a stream of HTML, with the broker put in at the top of its head.
// A page with no head in its start gets the broker after its doctype, or
// first.
function withBroker(page) {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    let seen = "";
    let placed = false;
    const place = controller => {
        const head = /<head(\s[^>]*)?>/i.exec(seen);
        const doctype = /^\s*<!doctype[^>]*>/i.exec(seen);
        const at = head ? head.index + head[0].length : doctype ? doctype[0].length : 0;
        controller.enqueue(encoder.encode(seen.slice(0, at) + BROKER + seen.slice(at)));
        placed = true;
        seen = "";
    };
    return page.pipeThrough(
        new TransformStream({
            transform(piece, controller) {
                if (placed) return controller.enqueue(piece);
                seen += decoder.decode(piece, { stream: true });
                if (/<head(\s[^>]*)?>/i.test(seen) || seen.length > HEAD_WITHIN) place(controller);
            },
            flush(controller) {
                if (placed) return;
                seen += decoder.decode();
                place(controller);
            },
        }),
    );
}

// The host's page for a page load the kernel answered with something other
// than a page: what it answered, with the broker, so the space is still
// reached.
function hostPage(status) {
    const said =
        status === 404 ? "Nothing is installed here yet."
        : status >= 500 ? "This space could not answer."
        : `This space answered ${status}.`;
    return new Response(
        `<!doctype html><html><head>${BROKER}<meta charset="utf-8"><title>${status}</title>` +
        `<style>body{font:14px/1.5 system-ui,sans-serif;margin:0;display:grid;place-items:center;` +
        `min-height:100vh;opacity:.7}</style></head><body><p>${said}</p></body></html>`,
        { status, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

// What the kernel answers a page load with, for a frame: its page with the
// broker in it, or the host's page.
function framed(reply) {
    const type = reply.headers.get("content-type") ?? "";
    if (!/^text\/html/i.test(type) || !reply.body) return hostPage(reply.status);
    const headers = new Headers(reply.headers);
    headers.delete("content-length");
    return new Response(withBroker(reply.body), { status: reply.status, headers });
}

async function serveKernel(event) {
    const embedder = await booting;
    const url = new URL(event.request.url);
    const body = new Uint8Array(await event.request.arrayBuffer());
    const reply = await embedder.handle(event.request.method, url.pathname + url.search, [...event.request.headers], body);
    drainSoon(embedder);
    const bodiless = reply.status === 204 || reply.status === 304 || event.request.method === "HEAD";
    const response = new Response(bodiless ? null : bodyOf(embedder, reply), {
        status: reply.status,
        headers: reply.headers,
    });
    return event.request.mode === "navigate" ? framed(response) : response;
}

// Answer a request passed down the port: its status, then its body as the
// kernel writes it, then that it has ended, as a release answers one. The
// profile cancels a call whose answer no one reads any more.
const calls = new Map();
async function answerCall(port, { call, request }) {
    try {
        const embedder = await takenUp;
        const body = new Uint8Array(await new Response(request.body ?? null).arrayBuffer());
        const reply = await embedder.handle(request.method, request.path, request.headers ?? [], body);
        drainSoon(embedder);
        calls.set(call, reply.body);
        port.postMessage({ call, head: { status: reply.status, headers: reply.headers } });
        for (;;) {
            const piece = await reply.body.next();
            drainSoon(embedder);
            if (!piece) break;
            if (!calls.has(call)) return;
            const chunk = piece.buffer.slice(piece.byteOffset, piece.byteOffset + piece.byteLength);
            port.postMessage({ call, chunk }, [chunk]);
        }
        port.postMessage({ call, end: true });
    } catch (error) {
        restartIfStopped(error);
        port.postMessage({ call, error: String(error?.message ?? error) });
    } finally {
        calls.delete(call);
    }
}

function cancelCall(call) {
    calls.get(call)?.cancel();
    calls.delete(call);
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

self.addEventListener("message", event => {
    const type = event.data?.type;
    if (type === "protocol") event.ports[0]?.postMessage({ protocol: PROTOCOL });
    else if (type === "claim") event.waitUntil(self.clients.claim());
    // The shell that installed this worker waits for the kernel to boot and
    // take up its space before it goes to the space's address.
    else if (type === "booted") {
        const reply = event.ports[0];
        event.waitUntil(
            takenUp.then(
                () => reply?.postMessage({ booted: true }),
                error => reply?.postMessage({ error: String(error?.message ?? error) }),
            ),
        );
    }
    else if (type === "port" && event.ports[0]) {
        adopt(event.ports[0]);
        event.waitUntil(booting.catch(() => {}));
    }
});

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (HOST_FILES.has(url.pathname) || HOST_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        event.respondWith(hostFile(event.request));
        return;
    }
    event.respondWith(
        serveKernel(event).catch(error => {
            restartIfStopped(error);
            log("could not answer", url.pathname, error);
            return event.request.mode === "navigate"
                ? hostPage(502)
                : new Response(String(error?.message ?? error), { status: 502 });
        }),
    );
});

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
// - Boots the kernel over the space where Dialog DB keeps it, as the
//   embedder opens each layer: its IndexedDB database, with its blobs in the
//   origin's file system.
// - Opens the key of this origin's session from Dialog DB's credential
//   store, where nothing reads its seed, and asks the person's profile, over
//   the port a page of this origin hands it, to delegate the space to it,
//   which says which space this origin is. The kernel acts as that session,
//   claims what it needs delegated, and keeps it.
// - Installs into a space that holds no kernel the one it was made for, which
//   the person's profile hands it, as the space's own blob and boot cell.
//
// On the profile's origin (`profile.{host}`) the space is the person's
// profile: the session's own, so it needs no one's delegation, and its
// kernel is the deployment's. There this worker answers every page load
// with the profile's shell, and serves the ports a space's host asks the
// profile over.
//   From then on the kernel comes from the space's store, not from anywhere
//   else. The kernel founds the space on it: its first commit records the
//   kernel and the pin to it.
// - Keeps the host's own files it serves, so a space that started once
//   starts again offline.
// - Boots the kernel the space pins and hands it every request of this
//   origin but the host's own files. Each page it answers with carries the
//   host's relay (`kernel/relay.js`), which carries the port up to the
//   person's profile; a page load the kernel answers with no page gets one of
//   the host's that says what the kernel answered. A space nothing has been
//   installed into answers every page with a 404.

importScripts("/_host/embedder.js");

const PINNED = new URL(self.location.href).searchParams.get("kernel");
// Which site this origin is. A profile's label starts with `profile`; a
// space's is its key in base32, which never does.
const PROFILE = self.location.hostname.split(".")[0].startsWith("profile");
// The contract between this worker and its shell, as for a release.
const PROTOCOL = 2;
const RELAY_PATH = "/kernel/relay.js";
// What this origin serves from the deployment, never from the kernel.
const HOST_FILES = new Set(["/space.html", "/profile.html", "/kernel_boot.js", RELAY_PATH]);
const HOST_PREFIXES = ["/_host/", "/.well-known/"];

const log = (...args) => console.log(PROFILE ? "[Profile Kernel]" : "[Space Kernel]", ...args);

// The body of an answer of the kernel, as it writes it. The worker stays up
// until it ends.
function bodyOf(reply, event) {
    let ended;
    event.waitUntil(new Promise(resolve => (ended = resolve)));
    return new ReadableStream({
        async pull(controller) {
            try {
                const piece = await reply.body.next();
                if (piece) return controller.enqueue(piece);
                controller.close();
            } catch (error) {
                controller.error(error);
            }
            ended();
        },
        cancel() {
            reply.body.cancel();
            ended();
        },
    });
}

// The name the key of this origin's session is kept under in Dialog DB's
// credential store: the one a profile's worker before the kernel kept the
// profile's key under, so the profile goes on as who it was.
const SESSION = "tonk";

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

// The delegation of the space to this origin's session, from the person's
// profile. It says which space this origin is: the label of an origin does
// not. Asked once: the kernel keeps what it is delegated.
async function delegateSession(session) {
    const answer = await askHost({ delegate: session.did });
    // A space that runs a kernel starts blank: nothing of what the profile
    // would seed it with is taken. Saying it holds the space lets the
    // profile stop waiting for it to.
    await askHost({ seeded: true, from: "kernel" }).catch(error => log("the profile was not told:", error));
    return { space: answer.space, session, proofs: [new Uint8Array(answer.chain)] };
}

// What answers the kernel's claims: the delegations the person's profile
// handed over with the session, once, and after them the ones it delegates
// anew to the same key, once a page of this origin hands this worker a port
// up to it.
function claims(session, handed) {
    return async claim => {
        if (handed.length) return handed.splice(0);
        log("the kernel claims", claim.command, "for", claim.reason);
        const answer = await askHost({ delegate: session.did });
        log("delegated", answer.space, "until", new Date(answer.expires * 1000).toISOString());
        return [new Uint8Array(answer.chain)];
    };
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
    return checkedKernel(id, new Uint8Array(answer.bytes));
}

// The kernel `id` this deployment ships, from this origin's copy of it.
async function deployedKernel(id) {
    if (!/^b[a-z2-7]+$/.test(id)) throw new Error(`${id} names no kernel`);
    const response = await hostFile(new Request(new URL(`/_host/kernel/${id}.wasm`, self.location.href)));
    if (!response.ok) throw new Error(`the deployment ships no kernel ${id}: ${response.status}`);
    return checkedKernel(id, new Uint8Array(await response.arrayBuffer()));
}

function checkedKernel(id, bytes) {
    const named = wasm_bindgen.kernelId(bytes);
    if (named !== id) throw new Error(`the kernel at ${id} is ${named}`);
    return bytes;
}

async function start() {
    const started = performance.now();
    if (!PINNED) throw new Error("this worker pins no kernel");
    await wasm_bindgen({ module_or_path: await hostFile(new Request(new URL("/_host/embedder_bg.wasm", self.location.href))) });
    const session = await wasm_bindgen.openSession(SESSION);
    // The profile's space is its session's own: the profile acts under its
    // own authority, so it claims nothing.
    let space = PROFILE ? session.did : await wasm_bindgen.heldSpace();
    let proofs = [];
    if (!space) {
        ({ space, proofs } = await delegateSession(session));
        await wasm_bindgen.holdSpace(space);
    }
    // A space with no kernel of its own yet is one being created: install the
    // one its worker was registered for. The profile runs the deployment's,
    // and takes up a new one as the deployment does.
    let pinned = await wasm_bindgen.pinned(space);
    if (!pinned || (PROFILE && pinned !== PINNED)) {
        const kernel = PROFILE ? await deployedKernel(PINNED) : await shippedKernel(PINNED);
        pinned = await wasm_bindgen.install(space, kernel);
        log("installed the kernel", pinned);
    }
    if (pinned !== PINNED) throw new Error(`the space pins ${pinned}, not ${PINNED}`);
    const answer = PROFILE ? async () => [] : claims(session, proofs);
    const embedder = await wasm_bindgen.boot(space, session, answer);
    log("booted in", Math.round(performance.now() - started), "ms", embedder.timings);
    return embedder;
}

// Boot the kernel, which takes up its space as it boots. After a space's
// first start, booting needs nothing of the person's profile, and a
// delegation the kernel claims alongside never holds a page up.
function run() {
    booting = start();
    booting.catch(error => {
        log("the kernel did not boot:", error);
        // The embedder's script, which this worker imported as it installed,
        // and its wasm, fetched as it starts, are of two builds: a newer
        // worker imports the script of the newer one.
        if (error instanceof WebAssembly.LinkError) self.registration?.update();
    });
}
let booting;
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

// The relay, as the first thing in a page: a classic script, so the parser
// runs it before anything after it. It takes its own script out of the page
// as it runs.
const RELAY = `<script src="${RELAY_PATH}"></script>`;
// What may come before the relay without leaving the page in quirks mode:
// whitespace, comments and the doctype.
const PROLOGUE = /^(?:\s+|<!--[\s\S]*?-->|<!doctype[^>]*>)*/i;
// How far into a page its prologue is waited for.
const PROLOGUE_WITHIN = 64 * 1024;

// Where the relay goes in `text`, the start of a page: right after its
// prologue, or `null` while a piece of the prologue may still be arriving.
function relayAt(text, whole) {
    const at = PROLOGUE.exec(text)[0].length;
    const rest = text.slice(at);
    if (whole || text.length > PROLOGUE_WITHIN) return at;
    if (rest === "" || "<!doctype".startsWith(rest.slice(0, 9).toLowerCase()) || "<!--".startsWith(rest.slice(0, 4))) {
        return null;
    }
    if (/^<!(doctype|--)/i.test(rest)) return null;
    return at;
}

// `page`, a stream of HTML, with the relay put in before anything of the
// page's own.
function withRelay(page) {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    let seen = "";
    let placed = false;
    const place = (controller, at) => {
        controller.enqueue(encoder.encode(seen.slice(0, at) + RELAY + seen.slice(at)));
        placed = true;
        seen = "";
    };
    return page.pipeThrough(
        new TransformStream({
            transform(piece, controller) {
                if (placed) return controller.enqueue(piece);
                seen += decoder.decode(piece, { stream: true });
                const at = relayAt(seen, false);
                if (at !== null) place(controller, at);
            },
            flush(controller) {
                if (placed) return;
                seen += decoder.decode();
                place(controller, relayAt(seen, true));
            },
        }),
    );
}

// The host's page for a page load the kernel answered with something other
// than a page: what it answered, with the relay, so the space is still
// reached.
function hostPage(status) {
    const said =
        status === 404 ? "Nothing is installed here yet."
        : status >= 500 ? "This space could not answer."
        : `This space answered ${status}.`;
    return new Response(
        `<!doctype html><html><head>${RELAY}<meta charset="utf-8"><title>${status}</title>` +
        `<style>body{font:14px/1.5 system-ui,sans-serif;margin:0;display:grid;place-items:center;` +
        `min-height:100vh;opacity:.7}</style></head><body><p>${said}</p></body></html>`,
        { status, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

// What the kernel answers a page load with, for a frame: its page with the
// relay in it, or the host's page.
function framed(reply) {
    const type = reply.headers.get("content-type") ?? "";
    if (!/^text\/html/i.test(type) || !reply.body) {
        reply.body?.cancel();
        return hostPage(reply.status);
    }
    const headers = new Headers(reply.headers);
    headers.delete("content-length");
    return new Response(withRelay(reply.body), { status: reply.status, headers });
}

async function serveKernel(event) {
    const embedder = await booting;
    const url = new URL(event.request.url);
    const body = new Uint8Array(await event.request.arrayBuffer());
    const reply = await embedder.handle(event.request.method, url.pathname + url.search, [...event.request.headers], body);
    const bodiless = reply.status === 204 || reply.status === 304 || event.request.method === "HEAD";
    const response = new Response(bodiless ? null : bodyOf(reply, event), {
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
        const embedder = await booting;
        const body = new Uint8Array(await new Response(request.body ?? null).arrayBuffer());
        const reply = await embedder.handle(request.method, request.path, request.headers ?? [], body);
        calls.set(call, reply.body);
        port.postMessage({ call, head: { status: reply.status, headers: reply.headers } });
        for (;;) {
            const piece = await reply.body.next();
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

// ---- The profile -------------------------------------------------------
//
// The profile's pages are the deployment's: every page load gets the
// profile's shell, which brings up the hub from this origin's static files
// and asks the kernel through `/api/`. A space's host reaches the profile
// over a port a page of the space's origin hands up, bound to that space.

const SHELL_PATH = "/profile.html";
// What the shell this worker serves carries and the server's copy does not.
const SHELL_MARK = '<meta name="tonk-shell" content="worker" />';
// The app's static files, which the shell loads from this origin.
const STATIC_PREFIXES = ["/guest/", "/styles-", "/images/", "/fonts/", "/tonk-code/", "/tonk-prose/", "/tonk-table/"];
// No document on a site's origin may use the app's passkeys.
const NO_PASSKEYS = "publickey-credentials-get=(), publickey-credentials-create=()";

// Where the deployment says its sites are.
async function siteOrigins() {
    try {
        const response = await hostFile(new Request(new URL("/.well-known/tonk", self.location.href)));
        return (await response.json()).sites;
    } catch {
        return null;
    }
}

// The profile's policy: framed by the app, and framing itself and the
// spaces. It renders the app's own hub, which reads the template catalog
// from where it is published.
function profilePolicy(sites) {
    const scheme = sites ? new URL(sites.app).protocol : null;
    const profile = sites ? `${scheme}//profile${sites.suffix ?? ""}.${sites.host}` : null;
    const ancestors = sites ? `'self' ${sites.app} ${profile}` : "'none'";
    const framed = sites ? ` ${scheme}//*.${sites.host}` : "";
    const published = " https: http://localhost:* http://127.0.0.1:*";
    return [
        "default-src 'none'",
        "script-src 'self' blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'",
        "style-src 'self' blob: 'unsafe-inline'",
        `img-src 'self' blob: data:${published}`,
        "media-src 'self' blob:",
        "font-src 'self' data:",
        `connect-src 'self' blob: data:${published}`,
        `frame-src 'self' blob:${framed}`,
        "worker-src blob:",
        "form-action 'none'",
        "base-uri 'self'",
        `frame-ancestors ${ancestors}`,
    ].join("; ");
}

async function serveShell() {
    const response = await hostFile(new Request(new URL(SHELL_PATH, self.location.href), { cache: "no-cache" }));
    const headers = new Headers(response.headers);
    headers.set("content-security-policy", profilePolicy(await siteOrigins()));
    headers.set("permissions-policy", NO_PASSKEYS);
    headers.set("x-content-type-options", "nosniff");
    headers.delete("content-length");
    // Marked as this worker's: the shell renders the hub only in a document
    // that carries the policy above, and reads the mark to know it does.
    const shell = (await response.text()).replace("<head>", `<head>\n        ${SHELL_MARK}`);
    return new Response(shell, { status: response.status, headers });
}

// The whole body of what the kernel answers `method` at `path` with.
async function askKernel(method, path, json) {
    const embedder = await booting;
    const body = new TextEncoder().encode(JSON.stringify(json ?? null));
    const reply = await embedder.handle(method, path, [["content-type", "application/json"]], body);
    const pieces = [];
    for (let piece; (piece = await reply.body.next()); ) pieces.push(piece);
    const text = new TextDecoder().decode(await new Blob(pieces).arrayBuffer());
    if (reply.status !== 200) throw new Error(`${method} ${path}: ${reply.status} ${text}`);
    return JSON.parse(text);
}

// Serve the host of the space `repo` over `port`. What it asks is for the
// space the port is bound to, never another, and is acknowledged before it
// is answered, so the host can tell a restarted worker from a slow one.
function bindSpacePort(port, { repo }) {
    if (!port || typeof repo !== "string") return;
    port.onmessage = async ({ data }) => {
        const id = data?.id;
        if (typeof id !== "number") return;
        port.postMessage({ id, ack: true });
        try {
            // The delegation of the space to the key its host acts under.
            if (typeof data.delegate === "string") {
                const delegated = await askKernel("POST", `/api/repository/${repo}/delegate`, { audience: data.delegate });
                const chain = new Uint8Array(delegated.chain).buffer;
                port.postMessage({ id, space: delegated.space, chain, expires: delegated.expires }, [chain]);
                return;
            }
            // The kernel the space was made for, from the deployment's copy:
            // a space made on this device takes nothing from anywhere else.
            if (typeof data.kernel === "string") {
                const bytes = (await deployedKernel(data.kernel)).buffer;
                port.postMessage({ id, bytes }, [bytes]);
                return;
            }
            // A space that runs a kernel starts blank, so there is nothing
            // of it here to settle.
            if (data.seeded === true) {
                port.postMessage({ id, settled: true });
                return;
            }
            throw new Error("unknown request");
        } catch (error) {
            port.postMessage({ id, error: String(error?.message ?? error) });
        }
    };
}

// What the profile's origin answers itself, never the kernel: its shell for
// every page load, and the app's static files.
function serveProfile(event, url) {
    if (event.request.mode === "navigate") return serveShell();
    if (event.request.method === "GET" && STATIC_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        return hostFile(event.request);
    }
    return null;
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

self.addEventListener("message", event => {
    const type = event.data?.type;
    if (type === "protocol") event.ports[0]?.postMessage({ protocol: PROTOCOL });
    else if (type === "claim") event.waitUntil(self.clients.claim());
    // The shell that installed this worker waits for the kernel to boot, and
    // so take up its space, before it goes to the space's address.
    else if (type === "booted") {
        const reply = event.ports[0];
        event.waitUntil(
            booting.then(
                () => reply?.postMessage({ booted: true }),
                error => reply?.postMessage({ error: String(error?.message ?? error) }),
            ),
        );
    }
    else if (type === "port" && event.ports[0]) {
        adopt(event.ports[0]);
        event.waitUntil(booting.catch(() => {}));
    }
    // The page of the app hands the profile one end of a port to a space's
    // host.
    else if (type === "space-port" && PROFILE) bindSpacePort(event.ports[0], event.data);
    // A shell asks for the content at its address: the profile keeps none.
    else if (type === "content") event.ports[0]?.postMessage({ admitted: true });
});

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (HOST_FILES.has(url.pathname) || HOST_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        event.respondWith(hostFile(event.request));
        return;
    }
    const profiled = PROFILE && serveProfile(event, url);
    if (profiled) {
        event.respondWith(profiled);
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

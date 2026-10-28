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
//   which says which space this origin is, and who the person's
//   representative and persona are. The kernel acts as that session, asks
//   its representative for what it needs delegated, and keeps it.
// - Installs into a space that holds no kernel the one it was made for, which
//   the person's profile hands it, as the space's own blob and boot cell.
//
// On the profile's origin (`profile.{host}`) the space is the person's
// profile: the session's own, so it needs no one's delegation, and its
// kernel is the persona it pins. Until the person makes themselves known,
// the session's key represents the persona and is it. There this worker answers every page load
// with the profile's shell, and serves the ports a space's host asks the
// profile over.
//   From then on the kernel comes from the space's store, not from anywhere
//   else. The kernel founds the space on it: its first commit records the
//   kernel and the pin to it.
// - Keeps the host's own files it serves, so a space that started once
//   starts again offline.
// - Answers every page load with the site's shell, the deployment's, which
//   brings the space up from this origin's static files, and answers those
//   files itself. Boots the kernel the space pins and hands it every other
//   request of this origin, and the loads the shell asks to be answered with
//   the content at their address. Each page the kernel answers with carries
//   the host's relay (`kernel/relay.js`); a page load the kernel answers with
//   nothing gets one of the host's that says what the kernel answered.

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

// What the kernel asks for as it takes its space up for the first time: the
// space, which the profile names, since the label of an origin does not.
const FIRST_ACCESS = { command: "/", arguments: [], reason: "to serve the space" };

// Who this space's kernel acts for, and through: the person's persona and
// its representative, as the profile said when it first delegated the space.
// Kept with the host's files, and asked of the profile again when they are
// gone.
const STANDING_REQUEST = new Request(new URL("/_host/standing.json", self.location.href));
async function keptStanding() {
    const kept = await caches.match(STANDING_REQUEST, { cacheName: HOST_CACHE });
    return kept ? kept.json() : null;
}
async function keepStanding({ representative, persona }) {
    const standing = { representative, persona };
    const cache = await caches.open(HOST_CACHE);
    await cache.put(STANDING_REQUEST, new Response(JSON.stringify(standing), { headers: { "content-type": "application/json" } }));
    return standing;
}
async function standing() {
    return (await keptStanding()) ?? keepStanding(await askHost({ standing: true }));
}

// The delegation of the space to this origin's session, which the person's
// persona makes. It says which space this origin is. Asked once: the kernel
// keeps what it is delegated.
async function delegateSession(session) {
    const answer = await askHost({ delegate: session.did, access: FIRST_ACCESS });
    await keepStanding(answer);
    return { space: answer.space, handed: new Uint8Array(answer.chain) };
}

// What answers the kernel's requests to its representative: the delegation
// the person's profile handed over with the session, once, and after it the
// ones it delegates anew, once a page of this origin hands this worker a port
// up to it.
function delegates(handed) {
    return async (audience, access) => {
        if (handed) {
            const chain = handed;
            handed = null;
            return chain;
        }
        log("the kernel asks for", access.command, "for", access.reason);
        const answer = await askHost({ delegate: audience, access });
        log("delegated", answer.space);
        return new Uint8Array(answer.chain);
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
    // own authority, so it asks for nothing.
    let space = PROFILE ? session.did : await wasm_bindgen.heldSpace();
    let handed = null;
    if (!space) {
        ({ space, handed } = await delegateSession(session));
        awaitCreation();
    }
    // A space with no kernel of its own yet is one being created, and a
    // profile with none a new one: install the one its worker was registered
    // for.
    let pinned = await wasm_bindgen.pinned(space);
    if (!pinned) {
        const kernel = PROFILE ? await deployedKernel(PINNED) : await shippedKernel(PINNED);
        pinned = await wasm_bindgen.install(space, kernel);
        log("installed the kernel", pinned);
    }
    if (pinned !== PINNED) throw new Error(`the space pins ${pinned}, not ${PINNED}`);
    let embedder;
    if (PROFILE) {
        profileDid = session.did;
        embedder = await wasm_bindgen.bootPersona(space, session, kernelSent);
    } else {
        const { representative, persona } = await standing();
        embedder = await wasm_bindgen.bootSpace(space, session, representative, persona, delegates(handed), kernelSent);
    }
    log("booted in", Math.round(performance.now() - started), "ms", embedder.timings);
    return embedder;
}

// Boot the kernel, which takes up its space as it boots. After a space's
// first start, booting needs nothing of the person's profile, and a
// delegation the kernel asks for alongside never holds a page up.
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
// Whether the space taken up for the first time has been created: its kernel
// creates it from what the person's profile kept for it once it serves, and
// the shell goes to its address only after. Resolved at once for any other
// boot, and after a while however the creation went, so no page waits on it
// for good.
let created = Promise.resolve();
let settleCreation = () => {};
const CREATION_WAIT_MS = 30_000;
function awaitCreation() {
    created = new Promise(resolve => {
        settleCreation = resolve;
        setTimeout(resolve, CREATION_WAIT_MS);
    });
}
// The profile's DID, on the profile's origin: its session's, which
// represents the persona and is it, until the person makes themselves known.
let profileDid = null;
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

// What the kernel answers a page load with, the content at an address, for
// a frame: its page with the relay in it and the site's policy on it, an
// asset as it is with the policy on it, or the host's page.
async function framed(reply) {
    const type = reply.headers.get("content-type") ?? "";
    const headers = await withPolicy(new Headers(reply.headers));
    if (/^text\/html/i.test(type) && reply.body) {
        return new Response(withRelay(reply.body), { status: reply.status, headers });
    }
    if (reply.ok && reply.body) return new Response(reply.body, { status: reply.status, headers });
    reply.body?.cancel();
    return hostPage(reply.status);
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

// ---- The site's pages ---------------------------------------------------
//
// A site's pages are the deployment's: every page load gets the site's
// shell, the profile's or a space's, which brings up the hub or the space
// from this origin's static files and asks the kernel through `/api/`. On a
// space's origin, a load the shell asked to be answered with content, and
// an asset opened as a page, are the kernel's to answer. A space's host
// reaches the profile over a port a page of the space's origin hands up,
// bound to that space.

const SHELL_PATH = PROFILE ? "/profile.html" : "/space.html";
// What the shell this worker serves carries and the server's copy does not.
const SHELL_MARK = '<meta name="tonk-shell" content="worker" />';
// The app's static files, which the shell loads from this origin.
const STATIC_PREFIXES = ["/guest/", "/styles-", "/images/", "/fonts/", "/tonk-code/", "/tonk-prose/", "/tonk-table/"];
// The icons `<wa-icon>` reads, which the app ships.
const ICON_PATH = /^\/webawesome\/icons\/[a-z0-9-]+\/[a-z0-9-]+\.svg$/;
// An asset of a space, opened as a page.
const ASSET_PATH = /^\/asset:[1-9A-HJ-NP-Za-km-z]+$/;
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

// The site's policy: framed by the app, by the profile, and by the site
// itself, and framing the sites. The profile renders the app's own hub,
// which reads the template catalog from where it is published. A space gets
// no network at all: its code is its author's.
function sitePolicy(sites) {
    const scheme = sites ? new URL(sites.app).protocol : null;
    const profile = sites ? `${scheme}//profile${sites.suffix ?? ""}.${sites.host}` : null;
    const ancestors = sites ? `'self' ${sites.app} ${profile}` : "'none'";
    const framed = sites ? ` ${scheme}//*.${sites.host}` : "";
    const published = PROFILE ? " https: http://localhost:* http://127.0.0.1:*" : "";
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

// `headers`, with what every page of the site carries.
async function withPolicy(headers) {
    headers.set("content-security-policy", sitePolicy(await siteOrigins()));
    headers.set("permissions-policy", NO_PASSKEYS);
    headers.set("x-content-type-options", "nosniff");
    headers.delete("content-length");
    return headers;
}

// Addresses whose next load is answered with their content, and until when.
// A shell that lands in a frame inside a page of this origin asks for the
// content at its address here, and loads it once more.
const admitted = new Map();
const ADMIT_MS = 5_000;

function admit(url) {
    const now = Date.now();
    for (const [held, until] of admitted) if (until <= now) admitted.delete(held);
    admitted.set(url, now + ADMIT_MS);
}

// Whether the load of `url` is one a shell asked to be answered with
// content. Asking uses it up.
function takeAdmission(url) {
    const until = admitted.get(url);
    admitted.delete(url);
    return until !== undefined && until > Date.now();
}

async function serveShell() {
    const response = await hostFile(new Request(new URL(SHELL_PATH, self.location.href), { cache: "no-cache" }));
    const headers = await withPolicy(new Headers(response.headers));
    // Marked as this worker's: the shell renders the site only in a document
    // that carries the policy above, and reads the mark to know it does.
    const shell = (await response.text()).replace("<head>", `<head>\n        ${SHELL_MARK}`);
    return new Response(shell, { status: response.status, headers });
}

// ---- What the kernel asks of other kernels, and of its host --------------
//
// The kernel sends a message with `application.send`: `{ ask, to, method,
// path, body }` asks the kernel of the space `to`, and `{ ask, read }` asks
// this host for the text at the address `read`, which anyone may read, such
// as a seed. The answer goes back with `receive` as `{ answer, status, body
// }`, or `{ answer, error }`. Only the persona asks other kernels.
async function kernelSent(bytes) {
    let message;
    try {
        message = JSON.parse(new TextDecoder().decode(bytes));
    } catch (error) {
        log("the kernel sent what is not JSON:", error);
        return;
    }
    if (typeof message?.ask !== "number") return;
    let answer;
    try {
        answer = { answer: message.ask, ...(await answerOf(message)) };
    } catch (error) {
        answer = { answer: message.ask, error: String(error?.message ?? error) };
    }
    try {
        await (await booting).receive(new TextEncoder().encode(JSON.stringify(answer)));
    } catch (error) {
        log("the kernel did not take an answer:", error);
    }
}

// What answers `message`: `{ status, body }`.
async function answerOf(message) {
    if (typeof message.read === "string") return readPublic(message.read);
    // What the space is to be created from, which the person's profile kept
    // as it created it, and that it has been.
    if (message.seed === true && !PROFILE) {
        const answer = await askHost({ seed: true });
        log(answer.fresh ? "the profile kept a seed for the space" : "the profile kept no seed for the space");
        if (!answer.fresh) settleCreation();
        return { status: 200, body: answer.fresh ?? "null" };
    }
    if (message.seeded === true && !PROFILE) {
        settleCreation();
        await askHost({ seeded: true, from: "seed" });
        return { status: 200, body: "" };
    }
    if (!PROFILE) throw new Error("a space's kernel asks no other kernel");
    const body = message.body == null ? null : new TextEncoder().encode(message.body).buffer;
    const headers = body ? [["content-type", "application/json"]] : [];
    const reply = await callSpace(message.to, { method: message.method, path: message.path, headers, body });
    return { status: reply.status, body: new TextDecoder().decode(reply.body) };
}

// The text at `url`, read as anyone would read it: no credentials, no
// redirects, nothing kept.
async function readPublic(url) {
    const response = await fetch(url, { mode: "cors", credentials: "omit", redirect: "error", cache: "no-store" });
    return { status: response.status, body: await response.text() };
}

// The port to each space's host, by the space's DID, as the profile's pages
// open them.
const spacePorts = new Map();
const spacePortWaiters = new Map();
const SPACE_PORT_WAIT_MS = 20_000;
const SPACE_PORT_ASK_EVERY_MS = 1_000;
const SPACE_ANSWER_WAIT_MS = 30_000;
let nextSpaceCall = 1;

// The pages of this origin that render the profile. The frame the app keeps
// only to reach this worker renders nothing, and opens no ports.
async function profilePages() {
    const pages = await self.clients.matchAll({ type: "window" });
    return pages.filter(page => !page.url.endsWith("#connector"));
}

async function askForSpacePort(repo) {
    for (const page of await profilePages()) page.postMessage({ type: "need-space-port", repo, branch: "main" });
}

// The port to the host of the space `repo`, asking the pages to open one
// when there is none. `null` when none is opened in time.
function spacePort(repo) {
    const held = spacePorts.get(repo);
    if (held) return Promise.resolve(held);
    return new Promise(resolve => {
        const asking = setInterval(() => askForSpacePort(repo), SPACE_PORT_ASK_EVERY_MS);
        const settle = value => {
            clearInterval(asking);
            resolve(value);
        };
        const waiters = spacePortWaiters.get(repo) ?? [];
        waiters.push(settle);
        spacePortWaiters.set(repo, waiters);
        setTimeout(() => settle(null), SPACE_PORT_WAIT_MS);
        askForSpacePort(repo);
    });
}

// Ask the kernel of the space `repo` `request`, as a page of its origin
// would: answers `{ status, headers, body }`, the body whole.
async function callSpace(repo, request) {
    const held = await spacePort(repo);
    if (!held) throw new Error(`the host of ${repo} could not be reached`);
    const call = nextSpaceCall++;
    const pieces = [];
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            held.calls.delete(call);
            reject(new Error(`the kernel of ${repo} did not answer`));
        }, SPACE_ANSWER_WAIT_MS);
        let head = null;
        held.calls.set(call, data => {
            if (data.head) head = data.head;
            else if (data.chunk) pieces.push(new Uint8Array(data.chunk));
            else if (data.end || data.error) {
                clearTimeout(timer);
                held.calls.delete(call);
                if (data.error) return reject(new Error(data.error));
                const body = new Uint8Array(pieces.reduce((length, piece) => length + piece.length, 0));
                let at = 0;
                for (const piece of pieces) {
                    body.set(piece, at);
                    at += piece.length;
                }
                resolve({ status: head?.status ?? 502, headers: head?.headers ?? [], body });
            }
        });
        held.port.postMessage({ call, request });
    });
}

// Serve the host of the space `repo` over `port`. What it asks is for the
// space the port is bound to, never another, and is acknowledged before it
// is answered, so the host can tell a restarted worker from a slow one. The
// answers to what the persona asks of it come back over the same port.
function bindSpacePort(port, { repo }) {
    if (!port || typeof repo !== "string") return;
    const held = { port, calls: new Map() };
    // A port replaces the last: the space's host restarted, and whatever it
    // was still answering will never finish.
    for (const answered of spacePorts.get(repo)?.calls.values() ?? []) answered({ error: "the space's host restarted" });
    spacePorts.set(repo, held);
    for (const resolve of spacePortWaiters.get(repo) ?? []) resolve(held);
    spacePortWaiters.delete(repo);
    port.onmessage = async ({ data }) => {
        if (typeof data?.call === "number") {
            held.calls.get(data.call)?.(data);
            return;
        }
        const id = data?.id;
        if (typeof id !== "number") return;
        port.postMessage({ id, ack: true });
        try {
            // What the space's kernel asks its representative for, for its
            // session, which this profile's persona delegates. The access is
            // to the space this port is bound to, whatever it names.
            if (typeof data.delegate === "string") {
                const access = { ...(data.access ?? FIRST_ACCESS), subject: repo };
                const chain = (await (await booting).delegate(data.delegate, access)).buffer;
                const representative = profileDid;
                port.postMessage({ id, space: repo, chain, representative, persona: profileDid }, [chain]);
                return;
            }
            // Who the space's kernel acts for, and through.
            if (data.standing === true) {
                await booting;
                port.postMessage({ id, representative: profileDid, persona: profileDid });
                return;
            }
            // The kernel the space was made for, from the deployment's copy:
            // a space made on this device takes nothing from anywhere else.
            if (typeof data.kernel === "string") {
                const bytes = (await deployedKernel(data.kernel)).buffer;
                port.postMessage({ id, bytes }, [bytes]);
                return;
            }
            // What the space is to be created from, which the persona kept as
            // it created it, and that it has been.
            if (data.seed === true) {
                const reply = await askPersona("GET", `/_kernel/seed/${repo}`);
                if (reply.status !== 200) throw new Error(reply.text);
                port.postMessage({ id, fresh: reply.text === "null" ? null : reply.text });
                return;
            }
            if (data.seeded === true) {
                const reply = await askPersona("POST", `/_kernel/seed/${repo}/settled`);
                if (reply.status !== 200) throw new Error(reply.text);
                port.postMessage({ id, settled: true });
                return;
            }
            throw new Error("unknown request");
        } catch (error) {
            port.postMessage({ id, error: String(error?.message ?? error) });
        }
    };
}

// Ask the persona's kernel, as a page of this origin would: `{ status, text
// }`, the body whole.
async function askPersona(method, path) {
    const reply = await (await booting).handle(method, path, [], new Uint8Array());
    const pieces = [];
    for (let piece; (piece = await reply.body.next()); ) pieces.push(piece);
    return { status: reply.status, text: await new Blob(pieces).text() };
}

// What a site's origin answers itself, never the kernel: its shell for
// every page load but a space's content, and the app's static files.
function serveSite(event, url) {
    if (event.request.mode === "navigate") {
        if (!PROFILE && (ASSET_PATH.test(url.pathname) || takeAdmission(url.href))) return null;
        return serveShell();
    }
    const shipped = STATIC_PREFIXES.some(prefix => url.pathname.startsWith(prefix)) || ICON_PATH.test(url.pathname);
    if (event.request.method === "GET" && shipped) return hostFile(event.request);
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
            booting.then(() => created).then(
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
    // A shell that loaded inside a page of this origin asks to be replaced
    // by the content at its address: the next load of that address is the
    // kernel's to answer, once. The profile keeps no content.
    else if (type === "content") {
        if (!PROFILE && typeof event.data.url === "string") admit(event.data.url);
        event.ports[0]?.postMessage({ admitted: true });
    }
});

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (HOST_FILES.has(url.pathname) || HOST_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        event.respondWith(hostFile(event.request));
        return;
    }
    const served = serveSite(event, url);
    if (served) {
        event.respondWith(served);
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

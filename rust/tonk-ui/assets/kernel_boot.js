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
// Where the kernel a site pins is a persona (it exports
// `sapling:kernel/guest`), the space is the person's profile: the session's
// own, so it needs no one's delegation. Until the person makes themselves
// known, the session's key represents the persona and is it. There this
// worker serves the ports a space's host asks the profile over, and the
// person's account. The worker is the same on every site: what a site can do
// is what its kernel is, never where it is.
//   From then on the kernel comes from the space's store, not from anywhere
//   else. The kernel founds the space on it: its first commit records the
//   kernel and the pin to it.
// - Keeps the host's own files it serves, so a space that started once
//   starts again offline.
// - Boots the kernel the space pins and hands it every request of this
//   origin, page loads included, but for the app's static files, which the
//   kernel's page loads and this worker answers itself. Each page the kernel
//   answers with carries
//   the host's relay (`kernel/relay.js`); a page load the kernel answers with
//   nothing gets one of the host's that says what the kernel answered.

importScripts("/_host/embedder.js");

const PINNED = new URL(self.location.href).searchParams.get("kernel");
// Whether the kernel this site runs is a persona: it holds the person's
// account and hosts their spaces. Known once the kernel is installed; what
// only a persona does waits for the boot.
let personaKernel = false;
// The contract between this worker and its shell, as for a release.
const PROTOCOL = 3;
const RELAY_PATH = "/kernel/relay.js";
// What this origin serves from the deployment, never from the kernel.
const HOST_FILES = new Set(["/site.html", "/kernel_boot.js", RELAY_PATH]);
const HOST_PREFIXES = ["/_host/", "/.well-known/"];

const log = (...args) => console.log(`[Kernel ${self.location.hostname.split(".")[0]}]`, ...args);

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
        // What the kernel takes up of the space changed, such as where it
        // syncs: it asks for its delegation again.
        if (data?.signal === "grant") {
            booting.then(embedder => embedder.receive(new TextEncoder().encode(JSON.stringify({ tell: "renew" })))).catch(error => log("the kernel was not told to renew:", error));
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
// started once starts again offline. A network that stalls never says it
// does not answer, so where a copy is kept, the network has NETWORK_MS to
// begin answering before the copy does. The fetch goes on, and keeps what it
// gets for the next load.
const HOST_CACHE = "tonk-host";
const NETWORK_MS = 1_000;
async function hostFile(request) {
    const fetched = fetch(request).then(response => {
        if (response.ok) {
            const copy = response.clone();
            caches.open(HOST_CACHE).then(cache => cache.put(request, copy)).catch(() => {});
        }
        return response;
    });
    const kept = await caches.match(request, { cacheName: HOST_CACHE });
    if (!kept) return fetched;
    let timer;
    const waited = new Promise(resolve => (timer = setTimeout(() => resolve(kept), NETWORK_MS)));
    try {
        return await Promise.race([fetched, waited]);
    } catch {
        return kept;
    } finally {
        clearTimeout(timer);
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
    // A server answers an address it has no file for with the app's page.
    const type = response.headers.get("content-type") ?? "";
    if (!response.ok || !type.startsWith("application/wasm")) {
        throw new Error(`this site runs the kernel ${id}, which this deployment no longer serves`);
    }
    return checkedKernel(id, new Uint8Array(await response.arrayBuffer()));
}

// The kernel a site taken up for the first time installs: the one its
// worker was registered for. A site whose framer hands it a port up to a
// host, as the person's profile does a space, has it from that host, which
// holds the deployment's copy, so a space made on this device takes nothing
// from anywhere else. A site with no host, or a host without it, has it from
// this origin's copy of the deployment's.
async function firstKernel() {
    if (await handedHost()) {
        try {
            return await shippedKernel(PINNED);
        } catch (error) {
            log(`the host has no kernel ${PINNED}:`, String(error?.message ?? error));
        }
    }
    return deployedKernel(PINNED);
}

// Whether the page that framed this site said it hands the site a port up to
// a host, when it named the kernel the site runs (see `site.html`).
async function handedHost() {
    const held = await (await caches.open(BOOT_CACHE)).match(BOOT_KEY);
    const record = held ? await held.json() : null;
    return record?.v === 2 && record.host === true;
}

// The kernel the shell keeps that the site pins: where the site moved to
// another kernel than the one its worker was registered for, the shell
// registers the worker for it on its next load.
const BOOT_CACHE = "tonk-boot";
const BOOT_KEY = "/__boot";
async function recordedKernel() {
    const held = await (await caches.open(BOOT_CACHE)).match(BOOT_KEY);
    const record = held ? await held.json() : null;
    return record?.v === 2 ? record.kernel : null;
}

function checkedKernel(id, bytes) {
    const named = wasm_bindgen.kernelId(bytes);
    if (named !== id) throw new Error(`what was served as the kernel ${id} is another file (${named})`);
    return bytes;
}

async function start() {
    const started = performance.now();
    if (!PINNED) throw new Error("this worker pins no kernel");
    await wasm_bindgen({ module_or_path: await hostFile(new Request(new URL("/_host/embedder_bg.wasm", self.location.href))) });
    const session = await wasm_bindgen.openSession(SESSION);
    let space = await wasm_bindgen.heldSpace();
    let handed = null;
    // A site with no kernel of its own yet is a new one: it installs the one
    // its worker was registered for. A persona's space is its session's own:
    // it acts under its own authority, so it asks for nothing. Any other is
    // the space the person's profile delegates to this session, which says
    // which space it is.
    if (!space) {
        const kernel = await firstKernel();
        if (wasm_bindgen.isPersona(kernel)) {
            space = session.did;
        } else {
            ({ space, handed } = await delegateSession(session));
            awaitCreation();
        }
        log("installed the kernel", await wasm_bindgen.install(space, kernel));
    }
    const pinned = await wasm_bindgen.pinned(space);
    if (pinned !== PINNED && pinned !== (await recordedKernel())) {
        throw new Error(`the space pins ${pinned}, not ${PINNED}`);
    }
    personaKernel = await wasm_bindgen.pinsPersona(space);
    let embedder;
    if (personaKernel) {
        profileDid = session.did;
        embedder = await wasm_bindgen.bootPersona(space, session, kernelSent, askedToMerge, askedToAuthorize, await keptPersona());
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
    bootFailed = false;
    booting.catch(error => {
        bootFailed = true;
        log("the kernel did not boot:", error);
        // The embedder's script, which this worker imported as it installed,
        // and its wasm, fetched as it starts, are of two builds: a newer
        // worker imports the script of the newer one.
        if (error instanceof WebAssembly.LinkError) self.registration?.update();
    });
}
let booting;
// Whether the last boot failed, such as for a space the person's profile
// did not hold yet: the next request boots it again.
let bootFailed = false;
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
// starts does not loop. A kernel that did not boot is booted again on a
// request after it, sooner: what it lacked, such as the person's profile
// holding its space, may have come since.
const RESTART_EVERY_MS = 10_000;
const REBOOT_EVERY_MS = 2_000;
let restarted = Date.now();
function restartIfStopped(error) {
    const stopped = /the kernel has stopped/.test(String(error?.message ?? error));
    if (!stopped && !bootFailed) return false;
    if (Date.now() - restarted < (stopped ? RESTART_EVERY_MS : REBOOT_EVERY_MS)) return false;
    restarted = Date.now();
    log(stopped ? "the kernel stopped; booting it again" : "the kernel did not boot; booting it again");
    run();
    return true;
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

// The host's page for a page load the kernel could not answer, saying why.
// While the kernel did not boot, the page tries again in a while, as the
// next load boots it again: what it lacked, such as the person's profile
// holding its space, may have come.
const RETRY_PAGE_MS = 3_000;
function hostPage(status, reason) {
    const retry = bootFailed ? `<script>setTimeout(() => location.reload(), ${RETRY_PAGE_MS})</script>` : "";
    const why = String(reason ?? "").replace(/[&<>"]/g, c => `&#${c.charCodeAt(0)};`);
    return new Response(
        `<!doctype html><html><head>${RELAY}${retry}<meta charset="utf-8"><title>${status}</title>` +
        `<style>body{font:14px/1.5 system-ui,sans-serif;margin:0;display:grid;place-items:center;` +
        `min-height:100vh;opacity:.7}small{display:block;opacity:.7}</style></head>` +
        `<body><p>This site could not answer.<small>${why}</small></p></body></html>`,
        { status, headers: { "content-type": "text/html; charset=utf-8" } },
    );
}

// What the kernel answers a page load with, for a frame: its page with the
// relay in it, or what it answered as it is, with the site's policy on it.
async function framed(reply) {
    const type = reply.headers.get("content-type") ?? "";
    const headers = await withPolicy(new Headers(reply.headers));
    const body = /^text\/html/i.test(type) && reply.body ? withRelay(reply.body) : reply.body;
    return new Response(body, { status: reply.status, headers });
}

async function serveKernel(event) {
    const embedder = await booting;
    const url = new URL(event.request.url);
    const body = new Uint8Array(await event.request.arrayBuffer());
    // The page that asked, so the kernel can send it somewhere.
    const client = event.clientId || event.resultingClientId;
    // What a load is for, as the browser would say in `sec-fetch-dest`,
    // which a worker's request does not carry: a whole tab or a frame.
    const dest = event.request.destination ? [["sec-fetch-dest", event.request.destination]] : [];
    const headers = [...event.request.headers, ...dest, ...(client ? [["x-tonk-client-id", client]] : [])];
    const reply = await embedder.handle(event.request.method, url.pathname + url.search, headers, body);
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
// A site's pages are its kernel's: it answers a page load with the content
// its routes keep at the address, or with its page, which brings up the hub
// or the space, and asks the kernel through `/api/`. A space keeps its page
// and the renderer it loads, which its kernel answers; the profile's are the
// app's static files this worker serves. A space's host reaches the profile
// over a port a page of the space's origin hands up, bound to that space.

// The app's static files, which the shell loads from this origin.
const STATIC_PREFIXES = ["/images/", "/fonts/", "/tonk-code/", "/tonk-prose/", "/tonk-table/"];
// The renderer the shell loads: the one the site's kernel keeps, as a space
// keeps the one it was created with, or, where it keeps none, the
// deployment's.
const RENDERER_PREFIXES = ["/guest/", "/styles-"];
// The icons `<wa-icon>` reads, which the app ships.
const ICON_PATH = /^\/webawesome\/icons\/[a-z0-9-]+\/[a-z0-9-]+\.svg$/;
// No document on a site's origin may use the app's passkeys.
const NO_PASSKEYS = "publickey-credentials-get=(), publickey-credentials-create=()";

// Where the deployment says its sites are: read once in a worker's life,
// since every page load's policy names them, and read again only where that
// read had no answer.
let sitesRead = null;
async function siteOrigins() {
    sitesRead ??= hostFile(new Request(new URL("/.well-known/tonk", self.location.href)))
        .then(response => {
            if (!response.ok) throw new Error(`/.well-known/tonk answered ${response.status}`);
            return response.json();
        })
        .then(config => config.sites);
    try {
        return await sitesRead;
    } catch {
        sitesRead = null;
        return null;
    }
}

// The site's policy: framed by the app, by the profile, and by the site
// itself, and framing the sites. A persona's site renders the person's hub,
// which reads the template catalog from where it is published. Any other
// gets no network at all: its code is its author's.
function sitePolicy(sites) {
    const scheme = sites ? new URL(sites.app).protocol : null;
    const profile = sites ? `${scheme}//profile${sites.suffix ?? ""}.${sites.host}` : null;
    const ancestors = sites ? `'self' ${sites.app} ${profile}` : "'none'";
    const framed = sites ? ` ${scheme}//*.${sites.host}` : "";
    const published = personaKernel ? " https: http://localhost:* http://127.0.0.1:*" : "";
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

// ---- What the kernel asks of other kernels, and of its host --------------
//
// The kernel tells this host where the page that asked something is to go,
// `{ tell: "navigate", client, href, replace }`; a persona, what to post to
// its pages, `{ tell: "post", client, except, message }`, and the delegation
// a page asked for, `{ tell: "delegated", chain }` or `{ tell: "delegated",
// error }`. A persona asks to act for another persona from now on,
// `{ ask, standing: { persona, link64 } }`, or for its provisional one,
// `{ ask, standing: null }`.
// It asks with
// `application.send`: `{ ask, to, method, path, body }` asks the kernel of
// the space `to`; `{ ask, read }` asks this host for the text at the
// address `read`, which anyone may read, such as a seed; and `{ ask, fetch,
// method, type, body64 }` asks it to deliver a request, such as an
// invocation the kernel signed, to the address `fetch`. The answer goes back
// with `receive` as `{ answer, status, body }` (`body64` for a delivery), or
// `{ answer, error }`. Only the persona asks other kernels and has requests
// delivered: a space's code is its author's, and gets no network.
async function kernelSent(bytes) {
    let message;
    try {
        message = JSON.parse(new TextDecoder().decode(bytes));
    } catch (error) {
        log("the kernel sent what is not JSON:", error);
        return;
    }
    if (message?.tell === "navigate") return navigatePage(message);
    if (message?.tell === "cancel") return cancelStream(message.ask);
    if (message?.tell === "changed") return signalSpaces(message.space);
    if (message?.tell === "post" && personaKernel) return postPages(message);
    if (message?.tell === "delegated" && personaKernel) return delegated(message);
    if (typeof message?.ask !== "number") return;
    if (message.stream === true) return streamToKernel(message);
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

// What the persona asked a space's kernel for in pieces, by its ask: the
// port the call went over, and the call.
const kernelStreams = new Map();

// Ask the kernel of the space `to` what `message` asks, and hand the persona
// the answer as it comes: its status, each piece, and its end.
async function streamToKernel(message) {
    const tell = async answer => {
        try {
            await (await booting).receive(new TextEncoder().encode(JSON.stringify(answer)));
        } catch (error) {
            log("the kernel did not take a piece:", error);
        }
    };
    const id = message.ask;
    try {
        if (!personaKernel) throw new Error("a kernel that is no persona asks no other kernel");
        const held = await spacePort(message.to);
        if (!held) throw new Error(`the host of ${message.to} could not be reached`);
        const call = nextSpaceCall++;
        kernelStreams.set(id, { held, call });
        held.calls.set(call, async data => {
            if (data.head) await tell({ answer: id, status: data.head.status });
            else if (data.chunk) await tell({ piece: id, body64: toBase64(new Uint8Array(data.chunk)) });
            else if (data.end || data.error) {
                held.calls.delete(call);
                kernelStreams.delete(id);
                await tell(data.error ? { answer: id, error: data.error } : { end: id });
            }
        });
        const body = message.body == null ? null : new TextEncoder().encode(message.body).buffer;
        const headers = body ? [["content-type", "application/json"], ["accept", "text/event-stream"]] : [["accept", "text/event-stream"]];
        held.port.postMessage({ call, request: { method: message.method, path: message.path, headers, body } }, body ? [body] : []);
    } catch (error) {
        await tell({ answer: id, error: String(error?.message ?? error) });
    }
}

// What the kernel of `space`, or of every space for `null`, takes up of it
// changed, such as where it syncs: signal its host to have it ask for its
// delegation again, as the profile's worker signalled a space's.
function signalSpaces(space) {
    const named = space == null ? null : space.startsWith("did:key:") ? space : `did:key:${space}`;
    for (const [repo, held] of spacePorts) {
        if (named === null || repo === named) held.port.postMessage({ signal: "grant" });
    }
}

// The persona stopped reading what it asked in pieces.
function cancelStream(id) {
    const streaming = kernelStreams.get(id);
    if (!streaming) return;
    kernelStreams.delete(id);
    streaming.held.calls.delete(streaming.call);
    streaming.held.port.postMessage({ cancel: streaming.call });
}

// Send the page `client` to `href`, as the kernel told: its `<tonk-host>`
// goes there.
async function navigatePage({ client, href, replace }) {
    const page = typeof client === "string" ? await self.clients.get(client) : null;
    if (!page) return log("navigate: the page that asked is gone");
    page.postMessage({ type: "navigate", href, ...(replace ? { replace: true } : {}) });
}

// Post `message` to the page `client`, or else to each page of this origin
// but `except`: the top-level ones, or, where none is, as on a site's origin,
// every frame, which passes what it hears up to the page around it.
async function postPages({ client, except, message }) {
    if (typeof client === "string") {
        const page = await self.clients.get(client);
        if (!page) return log("post: the page is gone");
        return page.postMessage(message);
    }
    const pages = await self.clients.matchAll({ type: "window" });
    const framed = !pages.some(page => page.frameType === "top-level");
    for (const page of pages) {
        if ((!framed && page.frameType !== "top-level") || page.id === except) continue;
        page.postMessage(message);
    }
}

// What answers `message`: `{ status, body }`.
async function answerOf(message) {
    if (typeof message.read === "string") return readPublic(message.read, message.bytes === true);
    if (typeof message.ceremony === "string" && personaKernel) {
        const link = Uint8Array.from(atob(message.ceremony), char => char.charCodeAt(0));
        return { status: 200, body: await (await booting).ceremony(link) };
    }
    if (message.enroll === true && personaKernel) return enroll();
    if ("standing" in message && personaKernel) {
        await stand(message.standing);
        return { status: 200, body: "" };
    }
    if (typeof message.fetch === "string") {
        if (!personaKernel) throw new Error("a kernel that is no persona has nothing delivered");
        return deliver(message);
    }
    // What the space is to be created from, which the person's profile kept
    // as it created it, and that it has been.
    if (message.seed === true && !personaKernel) {
        const answer = await within(askHost({ seed: true }), CREATION_WAIT_MS, "the profile did not say what the space is made from");
        log(answer.fresh ? "the profile kept a seed for the space" : "the profile kept no seed for the space");
        if (!answer.fresh) settleCreation();
        const copy = answer.copy
            ? { content64: toBase64(new Uint8Array(answer.copy.content)), revision64: answer.copy.revision64 }
            : null;
        const seed = answer.fresh ? JSON.parse(answer.fresh) : null;
        return { status: 200, body: JSON.stringify({ seed, copy }) };
    }
    // What only the person's persona can do for the space, passed up to it.
    if (message.profile && !personaKernel) {
        const { id: _, ...answer } = await askHost(message.profile);
        return { status: 200, body: JSON.stringify(answer) };
    }
    if (message.seeded === true && !personaKernel) {
        settleCreation();
        await askHost({ seeded: true, from: "seed" });
        return { status: 200, body: "" };
    }
    if (!personaKernel) throw new Error("a kernel that is no persona asks no other kernel");
    const body = message.body == null ? null : new TextEncoder().encode(message.body).buffer;
    const headers = body ? [["content-type", "application/json"]] : [];
    const reply = await callSpace(message.to, { method: message.method, path: message.path, headers, body });
    return { status: reply.status, body: new TextDecoder().decode(reply.body) };
}

// Deliver the request `message` asks for, with no credentials: what it
// carries is its own authority. The body goes both ways as base64.
async function deliver({ fetch: url, method, type, body64 }) {
    const headers = type ? { "content-type": type } : {};
    const body = body64 == null ? undefined : Uint8Array.from(atob(body64), char => char.charCodeAt(0));
    const response = await fetch(url, { method, headers, body, credentials: "omit", redirect: "error", cache: "no-store" });
    return { status: response.status, body64: toBase64(new Uint8Array(await response.arrayBuffer())) };
}

// `promise`, or a failure saying `what` once `ms` passed.
function within(promise, ms, what) {
    let timer;
    const late = new Promise((_, reject) => (timer = setTimeout(() => reject(new Error(what)), ms)));
    return Promise.race([promise, late]).finally(() => clearTimeout(timer));
}

function toBase64(bytes) {
    let binary = "";
    for (let at = 0; at < bytes.length; at += 0x8000) binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    return btoa(binary);
}

// The text at `url`, or its bytes in base64, read as anyone would read
// it: no credentials, no redirects, nothing kept.
async function readPublic(url, bytes) {
    const response = await fetch(url, { mode: "cors", credentials: "omit", redirect: "error", cache: "no-store" });
    if (bytes) return { status: response.status, body64: toBase64(new Uint8Array(await response.arrayBuffer())) };
    return { status: response.status, body: await response.text() };
}

// ---- Signing in ---------------------------------------------------------
//
// The persona's provisional persona asks to merge (`provisional-persona
// .merge`), and this host asks the page that asked this origin something
// to sign the person in: `{ type: "sign-in" }`. The page asks who they are,
// looks the address up, and runs the passkey ceremony: a new passkey for an
// address with no account, the account's own for one with. It hands over
// what the passkey derived, `{ type: "custody", credentialId, key, kek,
// request }` with a port to answer on, the request naming the ceremony,
// address, account name, device and remote. The page may sign the person in
// without the persona asking, as it does for a space it shows itself; the
// hand-off is the same. A person who puts the page's panel away, or whose
// browser refuses the ceremony, is told as `{ type: "ceremony-refused",
// kind, name }`. The embedder creates or opens the account with it and
// merges the persona into it; inside that merge the kernel asks what the
// ceremony made of the account (`{ ask, ceremony }`) and, for one it
// created, to enroll it (`{ ask, enroll }`). The persona then boots as the
// account, through its delegation to this device, which this host keeps
// with its files.

const PERSONA_REQUEST = new Request(new URL("/_host/persona.json", self.location.href));
async function keptPersona() {
    if (!personaKernel) return null;
    const kept = await caches.match(PERSONA_REQUEST, { cacheName: HOST_CACHE });
    if (!kept) return null;
    const { persona, link64 } = await kept.json();
    return { persona, link: Uint8Array.from(atob(link64), char => char.charCodeAt(0)) };
}
async function forgetPersona() {
    const cache = await caches.open(HOST_CACHE);
    await cache.delete(PERSONA_REQUEST);
}
async function keepPersona({ persona, link }) {
    const body = JSON.stringify({ persona, link64: toBase64(new Uint8Array(link)) });
    const cache = await caches.open(HOST_CACHE);
    await cache.put(PERSONA_REQUEST, new Response(body, { headers: { "content-type": "application/json" } }));
}

// The page that last asked this origin something: the one to ask to sign
// the person in.
let lastPage = null;

// The page to ask to sign the person in: the one they are looking at, of
// the pages open on this origin, and else the one that last asked something.
async function pageInView() {
    const pages = await self.clients.matchAll({ type: "window" });
    return pages.find(page => page.focused)
        ?? pages.find(page => page.visibilityState === "visible")
        ?? (lastPage && pages.find(page => page.id === lastPage))
        ?? pages[0];
}

async function askedToMerge() {
    try {
        const page = await pageInView();
        if (!page) throw new Error("no page is open to sign anyone in");
        page.postMessage({ type: "sign-in" });
    } catch (error) {
        log("the sign-in could not be asked for:", error);
        await tellKernel({ tell: "refused", kind: "log-in", name: "NotSupportedError" });
    }
}

// What a sign-in hand-off asks for, off its request: the ceremony, the
// address, what to call an account it creates, this device's name, and
// where the account syncs. Null for a hand-off that is no sign-in.
function signInAsked(request) {
    if (request?.kind === "create-account") {
        return { kind: "create", email: request.email ?? "", name: request.displayName ?? null, deviceName: request.deviceName, remote: request.remote };
    }
    if (request?.kind === "login") {
        return { kind: "log-in", email: request.email ?? "", name: null, deviceName: request.deviceName, remote: request.endpoint };
    }
    return null;
}

// What the page derived with a passkey for the sign-in `asked`: sign the
// person in with it, and answer the page on `port`.
async function signIn(handoff, port, asked) {
    try {
        const merged = await (await booting).signIn(handoff, asked.kind, asked.email, asked.name, asked.deviceName, asked.remote);
        await keepPersona(merged);
        port?.postMessage({ ok: {} });
    } catch (error) {
        const message = String(error?.message ?? error);
        log("the sign-in failed:", message);
        port?.postMessage({ error: message, ...(error?.code ? { code: error.code } : {}) });
        await tellKernel({ tell: "failed", kind: asked.kind, code: error?.code ?? null, message });
        if (error?.code === "awaiting-activation") resumeSignIn();
    }
}

// A log-in refused until the person follows the emailed link is parked in
// the embedder with what the passkey derived: try it again until the
// account is served, then finish it as the sign-in would have, with no
// second ceremony. An hour of tries at most.
const RESUME_EVERY_MS = 4_000;
const RESUME_TRIES = 900;
async function resumeSignIn() {
    for (let tries = 0; tries < RESUME_TRIES; tries++) {
        await new Promise(resolve => setTimeout(resolve, RESUME_EVERY_MS));
        try {
            const merged = await (await booting).resumeSignIn();
            if (!merged) continue;
            await keepPersona(merged);
            log("the parked log-in finished once its account was served");
        } catch (error) {
            log("the parked log-in could not finish:", String(error?.message ?? error));
        }
        return;
    }
}

// ---- Acting as the account ---------------------------------------------
//
// Once signed in, the kernel asks to act as the account
// (`permanent-persona.authorize`), with the custody intent the page runs
// the ceremony for as its context. This host asks the page that last asked
// this origin something for the passkey, as for a sign-in, and lends the
// kernel the account the passkey opens for the one call of
// `lifecycle.authorize`, which is refused unless it is the account the
// kernel acts for. A page that asks the account to delegate, as the bar
// does to promote a member, runs the ceremony first: its hand-off waits
// here while the kernel is told, `{ tell: "delegate" }`, and asks for the
// authorization, and the delegation the kernel minted answers the page.

// The authorization the persona asked for, while the page runs its ceremony.
let authorizing = null;
// A page's hand-off for a delegation it asked for, until the kernel asks to
// be authorized for it, and the page's port, until the delegation is made.
let delegating = null;

async function askedToAuthorize(reason, context) {
    if (delegating?.handoff) {
        const { handoff } = delegating;
        delegating.handoff = null;
        return authorize(handoff, context, delegating.port, false);
    }
    try {
        const intent = JSON.parse(new TextDecoder().decode(context));
        authorizing = { context, reason };
        const page = (lastPage && (await self.clients.get(lastPage))) ?? (await self.clients.matchAll({ type: "window" }))[0];
        if (!page) throw new Error("no page is open to ask for a passkey");
        page.postMessage({ type: "webauthn", request: "custody", intent });
    } catch (error) {
        log("the authorization could not be asked for:", error);
        authorizing = null;
        await tellKernel({ tell: "unauthorized", message: "This page could not be asked for a passkey." });
    }
}

// What the page derived with a passkey for the authorization `context`: lend
// the kernel the account it opens, and answer the page on `port`, unless the
// answer is the delegation the kernel mints.
async function authorize(handoff, context, port, answers = true) {
    try {
        const app = (await siteOrigins())?.app ?? self.location.origin;
        await (await booting).authorize(handoff, new URL("/ucan/", app).href, context);
        if (answers) port?.postMessage({ ok: {} });
    } catch (error) {
        const message = String(error?.message ?? error);
        log("the authorization failed:", message);
        port?.postMessage({ error: message });
        if (!answers) delegating = null;
        await tellKernel({ tell: "unauthorized", message });
    }
}

// The custody hand-off a page posted, with `port` to answer it on: for the
// authorization asked for, a sign-in, or a page's own ask to delegate.
function custodyFrom(handoff, port) {
    if (authorizing) {
        const { context } = authorizing;
        authorizing = null;
        return authorize(handoff, context, port);
    }
    const request = handoff?.request;
    const asked = signInAsked(request);
    if (asked) return signIn(handoff, port, asked);
    if (request?.kind === "delegate") {
        delegating = { handoff, port };
        return tellKernel({ tell: "delegate", subject: request.subject, command: request.command, audience: request.audience });
    }
    port?.postMessage({ error: "no passkey was asked for" });
}

// The delegation the kernel minted for the page that asked, or why there is
// none.
function delegated({ chain, error }) {
    const asked = delegating;
    delegating = null;
    if (!asked) return log("a delegation was made that no page waits for");
    asked.port?.postMessage(typeof chain === "string" ? { ok: { delegation: chain } } : { error: error ?? "no delegation was made" });
}

// The kernel acts for another persona now, or for its provisional one, when
// `standing` is null: boot it as that from now on.
async function stand(standing) {
    const now = standing ? { persona: standing.persona, link: Uint8Array.from(atob(standing.link64), char => char.charCodeAt(0)) } : null;
    await (await booting).stand(now);
    if (now) await keepPersona(now);
    else await forgetPersona();
}

async function tellKernel(message) {
    try {
        await (await booting).receive(new TextEncoder().encode(JSON.stringify(message)));
    } catch (error) {
        log("the kernel was not told:", error);
    }
}

// The account the sign-in created, enrolled at the access service: its
// receipt.
async function enroll() {
    const app = (await siteOrigins())?.app ?? self.location.origin;
    const body = await (await booting).enrollment();
    const answer = await fetch(new URL("/ucan/", app), {
        method: "POST",
        headers: { "content-type": "application/cbor" },
        body,
        credentials: "omit",
        cache: "no-store",
    });
    return { status: answer.status, body: await answer.text() };
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

// What a page of the profile asks about a space's content is that space's
// kernel's to answer: everything under one of its branches, its remotes,
// and what the inspector reads of it. The profile's own repository is
// reached by the same routes, by its DID, and that one is the persona's.
const SPACE_PATH =
    /^\/api\/(?:inspect\/)?repository\/((?:did:key:)?z[1-9A-HJ-NP-Za-km-z]+)\/(?:branch|remote|archive)\//;

// The space whose kernel answers `url`, or `null` for the persona.
function spaceAsked(url) {
    // A page may write the space's DID percent-encoded, as a path segment.
    let path;
    try {
        path = decodeURIComponent(url.pathname);
    } catch {
        return null;
    }
    const named = SPACE_PATH.exec(path)?.[1];
    if (!named) return null;
    const space = named.startsWith("did:key:") ? named : `did:key:${named}`;
    return space === profileDid ? null : space;
}

// What the persona answered of a space whose kernel did not answer, while
// it goes on, by the space: each answer's stream and what it reads from.
const answeredForSpaces = new Map();

// `response`, the persona's answer of `space`, which ends once the space's
// kernel answers something: a subscription the page then opens again
// reaches that kernel.
function untilSpaceAnswers(space, response) {
    if (!response.body) return response;
    const reader = response.body.getReader();
    const answered = answeredForSpaces.get(space) ?? new Set();
    answeredForSpaces.set(space, answered);
    let held;
    const body = new ReadableStream({
        start(controller) {
            held = { controller, reader };
            answered.add(held);
        },
        async pull(controller) {
            const { done, value } = await reader.read();
            if (!done) return controller.enqueue(value);
            answered.delete(held);
            controller.close();
        },
        cancel(reason) {
            answered.delete(held);
            return reader.cancel(reason);
        },
    });
    return new Response(body, { status: response.status, headers: response.headers });
}

// The kernel of `space` answers now: end what the persona answers of it.
function spaceAnswers(space) {
    const answered = answeredForSpaces.get(space);
    if (!answered) return;
    answeredForSpaces.delete(space);
    for (const { controller, reader } of answered) {
        try {
            controller.error(new Error(`the kernel of ${space} answers now`));
        } catch {}
        reader.cancel().catch(() => {});
    }
}

// Pass `event`'s request to the kernel of `repo`, over the port to its host,
// and answer as it answers, as it writes: a subscription goes on flowing
// until the page stops reading it.
async function passToSpace(event, repo) {
    const held = await spacePort(repo);
    if (!held) return new Response(`the host of ${repo} could not be reached`, { status: 502 });
    const url = new URL(event.request.url);
    const method = event.request.method;
    const body = method === "GET" || method === "HEAD" ? null : await event.request.arrayBuffer();
    const request = { method, path: url.pathname + url.search, headers: [...event.request.headers], body };
    const call = nextSpaceCall++;
    return new Promise((resolve, reject) => {
        let writing = null;
        let answered = false;
        const timer = setTimeout(() => {
            if (answered) return;
            held.calls.delete(call);
            reject(new Error(`the kernel of ${repo} did not answer`));
        }, SPACE_ANSWER_WAIT_MS);
        const stream = new ReadableStream({
            start(controller) {
                writing = controller;
            },
            cancel() {
                held.calls.delete(call);
                held.port.postMessage({ cancel: call });
            },
        });
        held.calls.set(call, data => {
            if (data.head) {
                answered = true;
                clearTimeout(timer);
                spaceAnswers(repo);
                resolve(new Response(stream, { status: data.head.status, headers: data.head.headers }));
            } else if (data.chunk) writing.enqueue(new Uint8Array(data.chunk));
            else if (data.end) {
                held.calls.delete(call);
                writing.close();
            } else if (data.error) {
                held.calls.delete(call);
                clearTimeout(timer);
                if (answered) writing.error(new Error(data.error));
                else reject(new Error(data.error));
            }
        });
        held.port.postMessage({ call, request }, body ? [body] : []);
    });
}

// Serve the host of the space `repo` over `port`. What it asks is for the
// space the port is bound to, never another, and is acknowledged before it
// is answered, so the host can tell a restarted worker from a slow one. The
// answers to what the persona asks of it come back over the same port.
function bindSpacePort(port, { repo }) {
    if (!port || typeof repo !== "string") return;
    const held = { port, calls: new Map() };
    // A port replaces the last for what is asked from now on. What was asked
    // over the last is answered over it, if its host is still there: a
    // space's page hands a port as it loads and again when the host asks
    // for one, and a host that did restart is never heard from again, which
    // the calls on its port wait out.
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
                // The space's kernel takes the space up with it: what the
                // persona answered of the space meanwhile is the kernel's
                // to answer now.
                spaceAnswers(repo);
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
                const fresh = reply.text === "null" ? null : reply.text;
                // A space made as a copy starts from the space it copies, as
                // that space's own kernel holds it now, or as an earlier
                // release kept it, which this host keeps until the copy is.
                const source = fresh ? JSON.parse(fresh).copy_from : null;
                if (!source) {
                    port.postMessage({ id, fresh });
                    return;
                }
                const snapshot = source.startsWith(MIGRATED)
                    ? await migratedCopy(source, repo)
                    : await callSpace(source, { method: "GET", path: "/_kernel/snapshot", headers: [], body: null });
                if (snapshot.status !== 200) throw new Error(`the space to copy answered ${snapshot.status}`);
                const revision64 = new Headers(snapshot.headers).get("x-tonk-revision");
                const content = snapshot.body.buffer;
                port.postMessage({ id, fresh, copy: { content, revision64 } }, [content]);
                return;
            }
            if (data.seeded === true) {
                const reply = await askPersona("POST", `/_kernel/seed/${repo}/settled`);
                if (reply.status !== 200) throw new Error(reply.text);
                await forgetMigrated(repo);
                port.postMessage({ id, settled: true });
                return;
            }
            // What only the persona can do for the space: mint an invite to
            // it, answer the account's authority over it, or revoke a grant
            // on it, for the space's kernel, which asked.
            const asked =
                typeof data.invite === "number" ? ["POST", "invite", { time: data.invite }]
                : data.authority === true ? ["GET", "authority", null]
                : data.revoke ? ["POST", "revoke", data.revoke]
                : typeof data.withdraw === "string" ? ["POST", "withdraw", { audience: data.withdraw }]
                : null;
            if (asked) {
                const [method, what, body] = asked;
                const reply = await askPersona(method, `/_kernel/space/${repo}/${what}`, body);
                if (reply.status !== 200) throw new Error(reply.text);
                port.postMessage({ id, ...JSON.parse(reply.text) });
                return;
            }
            throw new Error("unknown request");
        } catch (error) {
            port.postMessage({ id, error: String(error?.message ?? error) });
        }
    };
}

// A space an earlier release kept in the app's origin, which the app's page
// read and handed over, is made again as a copy: this host keeps what it was
// handed, under `migrated:{id}`, until the new space's kernel is created from
// it.
const MIGRATED = "migrated:";
const MIGRATION_CACHE = "tonk-migration";
const migratedPath = source => `/_host/migrated/${encodeURIComponent(source.slice(MIGRATED.length))}`;

// Keep each space the app's page handed over, and have the persona make a
// space as a copy of it. Answers the spaces made, and those that were not.
async function bringOver({ spaces }, reply) {
    const made = [];
    const failed = [];
    const cache = await caches.open(MIGRATION_CACHE);
    log("bringing over", spaces?.length ?? 0, "space(s) an earlier release kept");
    for (const { name, content, revision } of spaces ?? []) {
        const copy = `${MIGRATED}${crypto.randomUUID()}`;
        try {
            const headers = { "x-tonk-revision": toBase64(new Uint8Array(revision)) };
            await cache.put(migratedPath(copy), new Response(content, { headers }));
            const answer = await askPersona("POST", "/_kernel/migrated", { name, copy });
            if (answer.status !== 200) throw new Error(answer.text);
            made.push({ name, space: JSON.parse(answer.text).space });
            log("brought over", name, "as", made.at(-1).space);
        } catch (error) {
            log("could not bring over", name, String(error?.message ?? error));
            await cache.delete(migratedPath(copy));
            failed.push({ name, error: String(error?.message ?? error) });
        }
    }
    reply?.postMessage({ made, failed });
}

// Ask for another activation email as the hub's notice does, with the
// persona's `account/resend-activation`, and answer `{ said }`: what the
// persona said of it, once it ran.
async function resendActivation(reply) {
    try {
        const profiles = await askPersona("GET", "/api/profiles");
        if (profiles.status !== 200) throw new Error(profiles.text);
        const at = `/api/repository/profile:tonk/branch/${JSON.parse(profiles.text).active}`;
        const asked = await askPersona("POST", `${at}/transact`, { claims: [{ op: "assert", application: {
            predicate: { kind: "transient", concept: { with: {
                at: { the: "xyz.tonk.resend-activation/at", as: "Float" },
            } } },
            parameters: { this: `urn:uuid:${crypto.randomUUID()}`, at: Date.now() },
        } }] });
        if (asked.status !== 200) throw new Error(asked.text);
        const answer = await askPersona("POST", `${at}/query`, {
            predicate: { with: { said: { the: "xyz.tonk.account-resend/said", as: "Text", cardinality: "one" } } },
            terms: { this: "tonk:account/resend", said: { "?": { name: "said" } } },
        });
        if (answer.status !== 200) throw new Error(answer.text);
        const rows = JSON.parse(answer.text);
        reply?.postMessage({ said: (Array.isArray(rows) ? rows : rows.conclusions ?? [])[0]?.fields?.said ?? null });
    } catch (error) {
        log("could not ask for another activation email", String(error?.message ?? error));
        reply?.postMessage({ said: null });
    }
}

// What the app's page handed over for `source`, as a space's snapshot
// answers, for the space `repo` is created from.
async function migratedCopy(source, repo) {
    const kept = await caches.match(migratedPath(source), { cacheName: MIGRATION_CACHE });
    if (!kept) return { status: 404, headers: [], body: new Uint8Array() };
    migratedFor.set(repo, source);
    return { status: 200, headers: [...kept.headers], body: new Uint8Array(await kept.arrayBuffer()) };
}

// Which handed-over copy a space was created from, until it settles.
const migratedFor = new Map();

async function forgetMigrated(repo) {
    const source = migratedFor.get(repo);
    if (!source) return;
    migratedFor.delete(repo);
    await (await caches.open(MIGRATION_CACHE)).delete(migratedPath(source));
}

// Ask the persona's kernel, as a page of this origin would: `{ status, text
// }`, the body whole.
async function askPersona(method, path, body = null) {
    const bytes = body == null ? new Uint8Array() : new TextEncoder().encode(JSON.stringify(body));
    const headers = body == null ? [] : [["content-type", "application/json"]];
    const reply = await (await booting).handle(method, path, headers, bytes);
    const pieces = [];
    for (let piece; (piece = await reply.body.next()); ) pieces.push(piece);
    return { status: reply.status, text: await new Blob(pieces).text() };
}

// What a site's origin answers itself, never the kernel: the app's static
// files.
function serveSite(event, url) {
    const shipped = STATIC_PREFIXES.some(prefix => url.pathname.startsWith(prefix)) || ICON_PATH.test(url.pathname);
    if (event.request.method === "GET" && shipped) return hostFile(event.request);
    return null;
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", event => event.waitUntil(self.clients.claim()));

// What only a persona does for the page that asked, once the kernel booted:
// `act`, or, where the kernel is no persona, that it is not done.
async function asPersona(type, reply, act) {
    let persona = false;
    try {
        await booting;
        persona = personaKernel;
    } catch {}
    if (persona) return act();
    log(`a kernel that is no persona does not take ${type}`);
    reply?.postMessage({ error: `this site's kernel does not take ${type}` });
}

// What the pages of a persona's site hand its worker for the person's
// account: what a passkey ceremony derived, or that the person refused it;
// spaces an earlier release kept in the app's origin, to be made again
// here; and asking for another activation email.
const FOR_THE_ACCOUNT = {
    custody: (data, reply) => custodyFrom(data, reply),
    migrate: (data, reply) => bringOver(data, reply),
    resend: (_data, reply) => resendActivation(reply),
    "ceremony-refused": data => tellKernel({ tell: "refused", kind: data.kind, name: data.name }),
};

self.addEventListener("message", event => {
    const type = event.data?.type;
    if (Object.hasOwn(FOR_THE_ACCOUNT, type)) {
        const reply = event.ports[0];
        event.waitUntil(asPersona(type, reply, () => FOR_THE_ACCOUNT[type](event.data, reply)));
        return;
    }
    if (type === "protocol") event.ports[0]?.postMessage({ protocol: PROTOCOL });
    else if (type === "claim") event.waitUntil(self.clients.claim());
    // The shell that installed this worker waits for the kernel to boot, and
    // so take up its space, before it goes to the space's address. One that
    // did not boot boots again: what it lacked, such as the person's profile
    // holding its space, may have come since. The shell asks again in a
    // while, so it paces this.
    else if (type === "booted") {
        const reply = event.ports[0];
        if (bootFailed) {
            log("the kernel did not boot; booting it again for the shell");
            run();
        }
        event.waitUntil(
            booting.then(() => created).then(
                () => reply?.postMessage({ booted: true }),
                error => reply?.postMessage({ error: String(error?.message ?? error) }),
            ),
        );
    }
    // A page asks what this site can do: whether its kernel hosts the
    // person's spaces, as a persona does. Answered once it booted.
    else if (type === "abilities") {
        const reply = event.ports[0];
        event.waitUntil(booting.then(() => true, () => false).then(() => reply?.postMessage({ hostsSpaces: personaKernel })));
    }
    else if (type === "port" && event.ports[0]) {
        adopt(event.ports[0]);
        event.waitUntil(booting.catch(() => {}));
    }
    // A page hands a persona's site one end of a port to a space's host.
    else if (type === "space-port") {
        const port = event.ports[0];
        event.waitUntil(asPersona(type, null, () => bindSpacePort(port, event.data)));
    }
});

self.addEventListener("fetch", event => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (event.clientId) lastPage = event.clientId;
    if (HOST_FILES.has(url.pathname) || HOST_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        event.respondWith(hostFile(event.request));
        return;
    }
    const served = serveSite(event, url);
    if (served) {
        event.respondWith(served);
        return;
    }
    if (event.request.method === "GET" && RENDERER_PREFIXES.some(prefix => url.pathname.startsWith(prefix))) {
        event.respondWith(serveRenderer(event));
        return;
    }
    const space = spaceAsked(url);
    if (space) {
        event.respondWith(answerAboutSpace(event, space));
        return;
    }
    event.respondWith(answer(event));
});

// The renderer's file `event` asks for: the kernel's, where it keeps the
// renderer, else the deployment's.
async function serveRenderer(event) {
    try {
        const kept = await serveKernel(event);
        if (kept.ok) return kept;
        await kept.body?.cancel();
    } catch (error) {
        restartIfStopped(error);
    }
    return hostFile(event.request);
}

// What a page of a persona's site asks about a space's content is that
// space's kernel's to answer. A space whose kernel does not answer, such as
// one the person does not hold, is answered by the persona: of one it does
// not hold, it knows nothing. Any other site's kernel answers for its own
// space.
async function answerAboutSpace(event, space) {
    try {
        await booting;
    } catch {}
    if (!personaKernel) return answer(event);
    const unanswered = { request: event.request.clone(), clientId: event.clientId, resultingClientId: event.resultingClientId, waitUntil: promise => event.waitUntil(promise) };
    return passToSpace(event, space).catch(error => {
        log("the kernel of", space, "did not answer; the persona does:", String(error?.message ?? error));
        return serveKernel(unanswered).then(
            response => untilSpaceAnswers(space, response),
            error => new Response(String(error?.message ?? error), { status: 502 }),
        );
    });
}

// What the site's kernel answers `event` with.
function answer(event) {
    const url = new URL(event.request.url);
    // A request that had the kernel booted again is answered by the kernel
    // it booted, when it reads nothing it could not read twice.
    const notAnswered = error => {
        log("could not answer", url.pathname, error);
        return event.request.mode === "navigate"
            ? hostPage(502, error?.message ?? error)
            : new Response(String(error?.message ?? error), { status: 502 });
    };
    const again = event.request.method === "GET" || event.request.method === "HEAD";
    return serveKernel(event).catch(error =>
        restartIfStopped(error) && again ? serveKernel(event).catch(notAnswered) : notAnswered(error),
    );
}

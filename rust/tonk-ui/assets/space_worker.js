// The service worker of a site origin (`{label}.{host}`).
//
// Every site renders at an origin of its own, and this worker controls that
// origin and holds its database: the same Rust worker throughout, opened on
// this origin's storage. There are two kinds of site.
//
// - A PROFILE's (`profile….{host}`). Its worker holds the person's profile:
//   their key, their account, the spaces they have. The app's page reaches it
//   through the app's own worker, which holds nothing and passes every
//   request on over a port. It hands each space its delegation.
// - A SPACE's (`{label}.{host}`). Its worker runs under a profile made for
//   this origin and holds nothing but a delegation for its one space, which
//   it asks the person's profile for.
//
// It answers these requests and refuses the rest:
//
// - Navigations get the static shell, carrying the site's CSP, except one
//   to an asset. The server hands out the same shell for any path, so a deep
//   link with no worker yet still boots one.
// - `/api/*` is answered by this origin's database. What a profile is asked
//   about a space's content it passes on to that space's worker.
// - `/asset:{hash}`, an asset's own URI as a path, is read from a space's
//   database and served with the media type and size recorded for it,
//   natively, so `<img>`, `<video>` and `<link>` just work.
// - `PUT /` stores the request's body as an asset of the space and answers
//   with its location, `/asset:{hash}`.
// - The app's own static assets (the guest runtime, stylesheet, images and
//   fonts) come from the server, which serves them on every host, and are
//   kept so the site loads offline (see "This origin's runtime, offline").
// - Everything else is a 404. Author code has no network through this worker.
//
// Each site has a port UP, to the worker it answers to: a space's to its
// profile's worker, a profile's to the app's. The other end passes requests
// down it and gets answers back; a space also asks up it for its delegation.
// A port does not survive a restart of either worker. A restarted worker has
// none and asks its pages to broker one; a restarted worker above stops
// acknowledging, and this one drops the dead port and asks again.

import init, { activate } from "./worker.js";

// The hash of the `worker_bg.wasm` built alongside this script, written in
// by `scripts/stamp-service-worker.sh`. It names the copy this worker runs,
// and makes a change to the wasm alone a change to this script, which is
// what the browser compares when it looks for an update.
const WORKER_WASM_HASH = "dev";

const SHELL_PATH = "/space-origin.html";
const SHELL_CACHE = "tonk-space-shell";
// What this origin runs: this worker's wasm, and the app's static assets as
// they are loaded. Kept so the site works offline once it has loaded.
const RUNTIME_CACHE = "tonk-space-runtime";
const WORKER_WASM_URL = new URL("./worker_bg.wasm", self.location.href).href;
const WORKER_WASM_KEY = `${WORKER_WASM_URL}?${WORKER_WASM_HASH}`;
// The app's static assets, served by the server on every host: the runtime a
// guest loads from its own origin (`/guest/`), the app stylesheet, images and
// fonts.
const STATIC_PREFIXES = ["/guest/", "/styles-", "/images/", "/fonts/"];
// A name that carries its content's hash never changes what it serves.
const HASHED_NAME = /-[0-9a-f]{16}(?=\.)/;
// An asset's entity, `asset:{hash}`, as a path: the hash is base58btc.
const ASSET_PATH = /^\/asset:([1-9A-HJ-NP-Za-km-z]+)$/;
// How long a client may take to broker a port, and the host to acknowledge a
// request. A silent host is presumed restarted, not slow: it acknowledges
// before reading anything, so a large reply does not trip this.
const PORT_TIMEOUT_MS = 5_000;
const ACK_TIMEOUT_MS = 3_000;
// The contract between this worker and the shell (`space-origin.html`). The
// shell replaces a worker that does not answer with the same number.
const PROTOCOL = 1;

// Which kind of site this origin is. A profile's label starts with
// `profile`; a space's is its key in base32, which never does.
const PROFILE = self.location.hostname.split(".")[0].startsWith("profile");

const log = (...args) => console.log(PROFILE ? "[Profile Worker]" : "[Space Worker]", ...args);

self.addEventListener("install", event => {
    self.skipWaiting();
    event.waitUntil(
        Promise.all([
            caches.open(SHELL_CACHE).then(cache => Promise.all([cache.add(SHELL_PATH), siteOrigins()])),
            pinWorkerWasm(),
        ]),
    );
});

// Claim right away: the shell waits for control before it asks the host for
// its document, so the first load is served assets too.
self.addEventListener("activate", event => {
    event.waitUntil(Promise.all([self.clients.claim(), dropOtherWorkerWasm()]));
});

// ---- This origin's runtime, offline -------------------------------------
//
// A worker the browser restarts loads its wasm again, and a page that reloads
// loads the guest runtime again. Offline neither can come from the server, so
// both are kept here as they are first loaded.

// Keep the wasm built alongside this script. An install that gets another
// build's wasm (a deploy landed in between) fails, and the browser tries the
// update again later.
async function pinWorkerWasm() {
    const cache = await caches.open(RUNTIME_CACHE);
    if (await cache.match(WORKER_WASM_KEY)) return;
    const response = await fetch(WORKER_WASM_URL, { cache: "no-cache" });
    if (!response.ok) throw new Error(`worker wasm: ${response.status}`);
    const bytes = await response.arrayBuffer();
    if (WORKER_WASM_HASH !== "dev") {
        const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
        const hex = [...digest].map(byte => byte.toString(16).padStart(2, "0")).join("");
        if (!hex.startsWith(WORKER_WASM_HASH)) {
            throw new Error(`worker wasm is ${hex.slice(0, 16)}, expected ${WORKER_WASM_HASH}`);
        }
    }
    await cache.put(
        WORKER_WASM_KEY,
        new Response(bytes, { headers: { "content-type": "application/wasm" } }),
    );
}

// Once this worker is the active one, no other build's wasm is needed.
async function dropOtherWorkerWasm() {
    const cache = await caches.open(RUNTIME_CACHE);
    for (const request of await cache.keys()) {
        if (request.url.startsWith(`${WORKER_WASM_URL}?`) && request.url !== WORKER_WASM_KEY) {
            await cache.delete(request);
        }
    }
}

async function workerWasm() {
    const held = await caches.match(WORKER_WASM_KEY, { cacheName: RUNTIME_CACHE });
    return held ?? fetch(WORKER_WASM_URL);
}

// Serve one of the app's static assets. A name with its content's hash in it
// is served from the cache once held; any other is asked of the server
// first, and the cache answers only when the server cannot.
async function serveStatic(request) {
    const url = new URL(request.url);
    const cache = await caches.open(RUNTIME_CACHE);
    const hashed = HASHED_NAME.test(url.pathname);
    if (hashed) {
        const held = await cache.match(url.href);
        if (held) return held;
    }
    let response;
    try {
        response = await fetch(request);
    } catch (error) {
        const held = await cache.match(url.href);
        if (held) return held;
        throw error;
    }
    if (response.ok) {
        await cache.put(url.href, response.clone());
        if (hashed) await dropSuperseded(cache, url);
    }
    return response;
}

// A newer build's asset replaces the older one of the same name.
async function dropSuperseded(cache, url) {
    const stem = url.pathname.replace(HASHED_NAME, "");
    for (const request of await cache.keys()) {
        const other = new URL(request.url);
        if (other.href !== url.href && other.pathname.replace(HASHED_NAME, "") === stem) {
            await cache.delete(request);
        }
    }
}

// ---- The port up ---------------------------------------------------------

let hostPort = null;
let portWaiters = [];
let nextId = 1;
const pending = new Map();

self.addEventListener("message", event => {
    const type = event.data?.type;
    if (type === "protocol") {
        event.ports[0]?.postMessage({ protocol: PROTOCOL });
        return;
    }
    if (type === "flush") {
        event.waitUntil(flushSession());
        return;
    }
    // A page says it is still open. Hearing it is all there is to do: an
    // event is what keeps the browser from stopping this worker as idle,
    // and a request passed down a port is not one.
    if (type === "keepalive") return;
    // The profile's page frames a space and hands this worker one end of a
    // port to that space's worker.
    if (type === "space-port" && PROFILE) {
        bindSpacePort(event.ports[0], event.data);
        return;
    }
    if (type !== "port") return;
    const [port] = event.ports;
    if (!port) return;
    adopt(port, event.source?.id ?? "");
    // A space can now ask for its delegation, and a profile be asked of:
    // bring the database up.
    event.waitUntil(
        siteWorker().catch(error => log("the database failed to start:", error)),
    );
});

// `frame` is the page that handed the port over, a frame of this origin. What
// comes down the port was asked by the page around that frame, on another
// origin, which this worker cannot name or reach: it answers as though the
// frame had asked, and what it has to tell the asker it tells the frame.
function adopt(port, frame) {
    // A replaced port is left open: replies to requests already sent on it
    // still arrive, and every reply is matched by id, not by port.
    port.onmessage = event => {
        const { data } = event;
        if (typeof data?.ping === "number") port.postMessage({ pong: data.ping });
        else if (typeof data?.call === "number") answer(port, data, frame);
        else if (typeof data?.cancel === "number") answering.get(data.cancel)?.cancel().catch(() => {});
        else if (typeof data?.signal === "string") signalled(data.signal);
        else if (data?.message !== undefined) messaged(data.message, event.ports, frame);
        else onReply(event);
    };
    hostPort = port;
    for (const resolve of portWaiters.splice(0)) resolve(port);
    log("adopted a port up");
}

// ---- Requests passed down the port ---------------------------------------
//
// The worker above this one passes requests down the port: the app's passes
// a profile every request its page makes, and a profile passes a space
// whatever it is asked about that space's content. The answer goes back the
// same way: its status and headers, then its body in pieces as
// it is produced, so a subscription keeps flowing.

// The body readers of the requests still being answered, to cancel.
const answering = new Map();

async function answer(port, { call, request }, frame) {
    try {
        const response = await api({
            request: new Request(new URL(request.path, self.location.origin), {
                method: request.method,
                headers: request.headers,
                body: request.body ?? undefined,
            }),
            clientId: frame,
            resultingClientId: "",
            waitUntil() {},
        });
        port.postMessage({ call, head: { status: response.status, headers: [...response.headers] } });
        if (response.body) {
            const reader = response.body.getReader();
            answering.set(call, reader);
            for (;;) {
                const { done, value } = await reader.read();
                if (done) break;
                const chunk = value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
                port.postMessage({ call, chunk }, [chunk]);
            }
        }
        port.postMessage({ call, end: true });
    } catch (error) {
        port.postMessage({ call, error: String(error?.message ?? error) });
    } finally {
        answering.delete(call);
        // Only a write can stamp a site.
        if (request.method !== "GET") sessionChanged();
    }
}

// Something the page above told its worker, passed down for this worker to
// hear as well: that the network came or went, or the page became visible.
// A worker's sync loop paces itself on both, and a profile's spaces want to
// hear them too.
async function signalled(signal) {
    try {
        const worker = await siteWorker();
        // The profile changed what it told this space's worker.
        if (signal === "grant") {
            await ensureGrant(worker, { renew: true });
            return;
        }
        // The person removed this space from the device.
        if (signal === "forget" && !PROFILE) {
            await forgetSite();
            return;
        }
        for (const space of spacePorts.values()) space.port.postMessage({ signal });
        if (signal === "connectivity") await worker.onconnectivity?.();
        else if (signal === "visibility") await worker.onvisibility?.();
    } catch (error) {
        log(`could not act on ${signal}:`, error);
    }
}

// A message the app's page sent its worker, passed down with its ports: the
// Rust worker reads these (a passkey's result, on its way to the account).
async function messaged(data, ports, frame) {
    try {
        const worker = await siteWorker();
        const source = frame ? ((await self.clients.get(frame)) ?? null) : null;
        await worker.onmessage({ data, ports: [...ports], source });
    } catch (error) {
        log("could not deliver a message:", error);
    }
}

// Answer an `/api/` request. What a profile is asked about a space's
// content is that space's worker's to answer; anything else is this origin's
// database's.
async function api(event) {
    if (PROFILE) {
        const space = spaceOf(event.request);
        if (space) return askSpace(event.request, space);
    }
    return (await siteWorker()).onfetch(event);
}

// ---- A profile's spaces ---------------------------------------------------
//
// A profile's worker holds none of a space's content: the worker on the
// space's own origin does. So whatever is asked of this worker about a
// space's content (a read, a write, a command) it passes to that worker over
// a port, and answers with what it answers.
//
// The profile's page opens the port. Around a space on screen it hands this
// worker one end as the space's frame loads. For a space that is not on
// screen this worker asks, and the page frames the space's origin unseen,
// only to reach its worker. Over the same port the space's worker asks for
// its delegation and for what its content starts from.
//
// A message over a port does not wake a stopped worker, and a worker stops
// without a word. So while a space's worker is answering something this
// worker asks it, every so often, whether it is there. One that is silent has
// stopped: the page is asked for a new port, and handing one over is what
// starts it again. A space nothing has been asked of for a while is let go,
// and the page drops the frame it kept for it.

// What belongs to a space's own worker: everything under one of its
// branches, and what the inspector reads of it.
const SPACE_PATH =
    /^\/api\/(?:inspect\/)?repository\/((?:did:key:)?z[1-9A-HJ-NP-Za-km-z]+)\/(?:branch|remote|archive)\//;
const SPACE_PORT_WAIT_MS = 10_000;
const SPACE_PORT_ASK_EVERY_MS = 1_500;
const SPACE_ANSWER_WAIT_MS = 30_000;
const SPACE_PROBE_WAIT_MS = 3_000;
const SPACE_PROBE_EVERY_MS = 10_000;
const SPACE_IDLE_MS = 60_000;

// The port to each space's worker, by the space's key, with what has been
// passed over it and is still being answered.
const spacePorts = new Map();
const spacePortWaiters = new Map();
let nextSpaceCall = 1;
let nextSpaceProbe = 1;

const spaceKey = repo => repo.replace(/^did:key:/, "");

// Serve a space's worker over `port`, bound to the space `repo`: the
// delegation it holds for that one space, and what its content starts from
// (the seed of a space made for it to fill, or a copy of one this worker
// held before). Every request is acknowledged before it is answered, so the
// space's worker can tell a restarted (silent) worker from a slow one.
function bindSpacePort(port, { repo, branch }) {
    if (!port || typeof repo !== "string" || typeof branch !== "string") return;
    const space = holdSpacePort(repo, port);
    port.onmessage = async ({ data }) => {
        if (typeof data?.pong === "number") {
            space.probes.get(data.pong)?.();
            return;
        }
        // An answer to something this worker passed on to the space's worker.
        if (typeof data?.call === "number") {
            spaceAnswered(space, data);
            return;
        }
        const id = data?.id;
        if (typeof id !== "number") return;
        port.postMessage({ id, ack: true });
        try {
            const worker = await siteWorker();
            // The space's worker asks for a delegation for its own profile.
            // It is issued for the space this port is bound to, never another.
            if (typeof data.delegate === "string") {
                const grant = await worker.delegateSpace(repo, data.delegate);
                const chain = grant.chain.buffer;
                port.postMessage(
                    {
                        id,
                        space: repo,
                        chain,
                        expires: grant.expires,
                        remote: grant.remote,
                        account: grant.account,
                    },
                    [chain],
                );
                return;
            }
            // Where the space syncs and which account this profile acts
            // for, for the space's worker to compare with what it took up.
            if (data.terms === true) {
                const terms = await worker.spaceTerms(repo);
                port.postMessage({ id, remote: terms.remote, account: terms.account });
                return;
            }
            // The space's worker has created the content it was handed the
            // seed for.
            if (data.seeded === true) {
                await worker.settleSeed(repo);
                port.postMessage({ id, settled: true });
                return;
            }
            if (data.snapshot === true) {
                // A space this worker created and left for its own worker to
                // fill comes with the seed to fill it from, whatever this
                // worker's branch holds: a page's own writes may have put a
                // revision on it since, with none of the space's content.
                const fresh = await worker.pendingSeed(repo);
                if (fresh) {
                    port.postMessage({ id, fresh });
                    return;
                }
                const snapshot = await worker.snapshotSpace(repo);
                if (!snapshot) {
                    port.postMessage({ id, empty: true });
                    return;
                }
                const content = snapshot.content.buffer;
                const revision = snapshot.revision.buffer;
                port.postMessage({ id, content, revision }, [content, revision]);
                return;
            }
            throw new Error("unknown request");
        } catch (error) {
            port.postMessage({ id, error: String(error?.message ?? error) });
        }
    };
}

function holdSpacePort(repo, port) {
    const key = spaceKey(repo);
    const space = { key, port, calls: new Map(), probes: new Map(), probing: false, lost: false, idle: null };
    // A port replaces the last: the space's worker restarted, and whatever
    // it was still answering will never finish. Ask the new worker the same
    // things. A subscription carries on in the response already open: its
    // next piece is the whole result again, as on first asking.
    const stale = spacePorts.get(key);
    if (stale) {
        clearTimeout(stale.idle);
        for (const [call, passed] of stale.calls) {
            passed.space = space;
            space.calls.set(call, passed);
            port.postMessage({ call, request: passed.request });
        }
        if (stale.calls.size > 0) {
            log(`asking ${stale.calls.size} thing(s) of ${key} again of its new worker`);
        }
        stale.calls.clear();
    }
    spacePorts.set(key, space);
    for (const resolve of spacePortWaiters.get(key) ?? []) resolve(space);
    spacePortWaiters.delete(key);
    watchSpace(space);
    restSpace(space);
    return space;
}

// The port to the worker of the space `key`, asking the pages to open one
// when there is none or the one held has gone quiet. `null` when none opens
// one in time: the space's origin could not be loaded.
function spacePort(key) {
    const held = spacePorts.get(key);
    if (held && !held.lost) return Promise.resolve(held);
    return new Promise(resolve => {
        // A page that is still loading cannot answer yet: keep asking.
        const asking = setInterval(() => askForSpacePort(key), SPACE_PORT_ASK_EVERY_MS);
        const settle = value => {
            clearInterval(asking);
            resolve(value);
        };
        const waiters = spacePortWaiters.get(key) ?? [];
        waiters.push(settle);
        spacePortWaiters.set(key, waiters);
        setTimeout(() => settle(null), SPACE_PORT_WAIT_MS);
        askForSpacePort(key);
    });
}

// The pages of this origin that render the profile. The frame the app keeps
// only to reach this worker renders nothing, and opens no ports.
async function profilePages() {
    const pages = await self.clients.matchAll({ type: "window" });
    return pages.filter(page => !page.url.endsWith("#connector"));
}

async function askForSpacePort(key) {
    for (const page of await profilePages()) {
        page.postMessage({ type: "need-space-port", repo: `did:key:${key}`, branch: "main" });
    }
}

// Whether the space's worker answers on `space`'s port.
function probeSpace(space) {
    const probe = nextSpaceProbe++;
    return new Promise(resolve => {
        const timer = setTimeout(() => {
            space.probes.delete(probe);
            resolve(false);
        }, SPACE_PROBE_WAIT_MS);
        space.probes.set(probe, () => {
            clearTimeout(timer);
            space.probes.delete(probe);
            resolve(true);
        });
        space.port.postMessage({ ping: probe });
    });
}

// Probe a space's worker for as long as it is answering something. Once it
// is silent its port is given up for lost and the pages are asked for
// another; what was being answered moves to the new one.
async function watchSpace(space) {
    if (space.probing) return;
    space.probing = true;
    try {
        while (spacePorts.get(space.key) === space && space.calls.size > 0) {
            if (!(await probeSpace(space))) {
                if (spacePorts.get(space.key) !== space) return;
                log(`the worker of ${space.key} went quiet; asking for a new port`);
                space.lost = true;
                await askForSpacePort(space.key);
                return;
            }
            await delay(SPACE_PROBE_EVERY_MS);
        }
    } finally {
        space.probing = false;
    }
}

// Let a space go once nothing has been asked of it for a while: the page
// drops the frame it kept only to reach it.
function restSpace(space) {
    clearTimeout(space.idle);
    if (space.calls.size > 0) return;
    space.idle = setTimeout(async () => {
        if (spacePorts.get(space.key) !== space || space.calls.size > 0) return;
        spacePorts.delete(space.key);
        for (const page of await profilePages()) {
            page.postMessage({ type: "release-space-port", repo: `did:key:${space.key}`, branch: "main" });
        }
    }, SPACE_IDLE_MS);
}

// The space a request is about, when that space's own worker answers it.
function spaceOf(request) {
    const match = SPACE_PATH.exec(new URL(request.url).pathname);
    return match ? spaceKey(match[1]) : null;
}

function spaceUnreachable(message) {
    return new Response(JSON.stringify({ error: { kind: "space-unreachable", message } }), {
        status: 503,
        headers: { "content-type": "application/json", "retry-after": "2" },
    });
}

// Pass `request` to the worker of the space `key`, and answer with what it
// answers.
async function askSpace(request, key) {
    const held = await spacePort(key);
    if (!held) return spaceUnreachable("the space's worker could not be reached");
    const url = new URL(request.url);
    const body = request.method === "GET" || request.method === "HEAD" ? null : await request.arrayBuffer();
    const call = nextSpaceCall++;
    let controller;
    let answered;
    const head = new Promise((resolve, reject) => {
        answered = { resolve, reject };
    });
    const settled = () => {
        passed.space.calls.delete(call);
        restSpace(passed.space);
    };
    const passed = {
        // The port may be replaced while this is answered: `space` follows
        // the request to wherever it has moved.
        space: held,
        request: {
            method: request.method,
            path: url.pathname + url.search,
            headers: [...request.headers],
            body,
        },
        // Asked again of a new worker, a request already answering keeps
        // the status it gave.
        head: answered.resolve,
        chunk: bytes => controller.enqueue(new Uint8Array(bytes)),
        end() {
            settled();
            controller.close();
        },
        fail(error) {
            settled();
            answered.reject(error);
            try {
                controller.error(error);
            } catch {}
        },
    };
    const drop = () => {
        if (!passed.space.calls.has(call)) return;
        passed.space.port.postMessage({ cancel: call });
        settled();
    };
    const stream = new ReadableStream({
        start(started) {
            controller = started;
        },
        cancel: drop,
    });
    clearTimeout(held.idle);
    held.calls.set(call, passed);
    request.signal?.addEventListener("abort", drop);
    // Not transferred: the body is kept to ask a new worker with.
    held.port.postMessage({ call, request: passed.request });
    watchSpace(held);
    const timer = setTimeout(
        () => answered.reject(new Error("the space's worker did not answer")),
        SPACE_ANSWER_WAIT_MS,
    );
    try {
        const { status, headers } = await head;
        const bodiless = status === 204 || status === 304;
        if (bodiless) settled();
        return new Response(bodiless ? null : stream, { status, headers });
    } catch (error) {
        log(`${key} did not answer:`, error);
        drop();
        return spaceUnreachable(String(error?.message ?? error));
    } finally {
        clearTimeout(timer);
    }
}

// The Rust worker asks a space's own worker through this: what one of the
// profile's commands does to a space's content, that worker does.
if (PROFILE) {
    self.tonkAskSpace = async (space, method, path, body) => {
        const response = await askSpace(
            new Request(new URL(path, self.location.origin), {
                method,
                headers: body == null ? {} : { "content-type": "application/json" },
                body: body ?? undefined,
            }),
            spaceKey(space),
        );
        return { status: response.status, body: await response.text() };
    };

    // The Rust worker says through this that what a space's worker was told has
    // changed (where the space syncs, which account this profile acts for):
    // of one space, or of all of them.
    // The Rust worker says through this that the person removed a space
    // from this device: its own worker forgets it, origin and all.
    self.tonkForgetSpace = async space => {
        const key = spaceKey(space);
        const held = await spacePort(key);
        if (!held) throw new Error("the space's worker could not be reached");
        held.port.postMessage({ signal: "forget" });
        // Nothing more is asked of it, so the page drops its frame.
        clearTimeout(held.idle);
        spacePorts.delete(key);
        for (const page of await profilePages()) {
            page.postMessage({ type: "release-space-port", repo: `did:key:${key}`, branch: "main" });
        }
    };

    self.tonkSpaceChanged = space => {
        for (const held of spacePorts.values()) {
            if (space == null || held.key === spaceKey(space)) held.port.postMessage({ signal: "grant" });
        }
    };
}

function spaceAnswered(space, data) {
    const call = space.calls.get(data.call);
    if (!call) return;
    if (data.head) call.head(data.head);
    else if (data.chunk) call.chunk(data.chunk);
    else if (data.end) call.end();
    else if (data.error) call.fail(new Error(data.error));
}

// End everything passed on to a space's worker, so that nothing this worker
// streams outlives its retirement. Whoever was asking asks its successor.
function releaseSpaceReads(reason) {
    for (const space of spacePorts.values()) {
        for (const [call, passed] of [...space.calls]) {
            space.port.postMessage({ cancel: call });
            passed.fail(new Error(reason));
        }
    }
}

// ---- This origin's database ----------------------------------------------
//
// The same Rust worker on every site, opening this origin's storage. On a
// profile's origin it is the person's profile. On a space's it generates a
// profile of its own on first boot, whose key never leaves this origin and
// which holds nothing but a delegation for this one space: it asks the
// person's profile for that, and asks again before it lapses.

// The Rust worker fills a missing starter image from the app's bundled
// library through this hook, which the host's worker defines too. The server
// serves `/library/` on every host.
self.tonkBundledAsset = async path => {
    if (typeof path !== "string" || !path.startsWith("/library/") ||
        path.includes("..") || path.includes("?") || path.includes("#")) {
        throw new Error("invalid bundled library path");
    }
    return fetch(new URL(path, self.location.origin));
};

const GRANT_KEY = "/__space/grant";
// What taking up a grant does. A grant taken up by an earlier version is
// taken up again, so the space gets what that version left out.
const GRANT_VERSION = 4;
// Ask for a new delegation once the held one has less than this left.
const RENEW_MARGIN_SECONDS = 60 * 60;

// ---- Forgetting a space ---------------------------------------------------
//
// A space the person removes from the device is removed by its profile, and
// its content is here. Told so, this worker removes everything its origin
// stored and unregisters itself. A database this worker still has open is
// deleted once it stops, which is when its last page goes.
async function forgetSite() {
    log("forgetting this space");
    retired = true;
    for (const name of await caches.keys()) await caches.delete(name);
    for (const { name } of await indexedDB.databases()) indexedDB.deleteDatabase(name);
    const root = await navigator.storage.getDirectory();
    for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
    await self.registration.unregister();
}

// ---- A profile's move in from the app's origin ---------------------------
//
// Before sites had origins of their own, the app's worker held the person's
// profile, in the app's origin. A person who was here before has theirs
// there still. So the first time a profile's worker starts, before it opens
// anything, it asks the app's worker for what that origin stored and copies
// it in: the databases as they are, record by record, and the files beside
// them. It holds the spaces too, each of which then moves on to its own origin the first time it is
// opened.
//
// Once, and only into an origin that holds no profile yet: one made here is
// never replaced. A copy that was cut short is thrown away and made again.

const MOVED_KEY = "/__profile/moved";
// The databases a profile is kept in: its own, its credentials', and one for
// each space it holds.
const PROFILE_DATABASE = /^(tonk[.-]|did:)/;
const MOVED_BATCH = 128;
const MOVED_CHUNK = 8 * 1024 * 1024;

function settled(request) {
    return new Promise((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
    });
}

async function moveIn() {
    const cache = await caches.open(SHELL_CACHE);
    const marker = await (await cache.match(MOVED_KEY))?.json();
    if (marker && marker.state !== "moving") return;
    const mark = state =>
        cache.put(MOVED_KEY, new Response(JSON.stringify({ state, at: new Date().toISOString() })));
    const own = (await indexedDB.databases()).filter(({ name }) => PROFILE_DATABASE.test(name));
    if (!marker && own.some(({ name }) => name === "tonk.profile")) {
        await mark("kept");
        return;
    }
    const { databases, files } = await askHost({ stored: "list" });
    if (!databases.some(({ name }) => name === "tonk.profile")) {
        await mark("none");
        return;
    }
    await mark("moving");
    const root = await navigator.storage.getDirectory();
    for (const { name } of own) await settled(indexedDB.deleteDatabase(name));
    for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
    let records = 0;
    for (const database of databases) records += await copyDatabase(database);
    for (const file of files) await copyFile(root, file);
    await mark("moved");
    log(
        `moved in from the app's origin: ${databases.length} database(s), ` +
            `${records} record(s), ${files.filter(file => file.size !== undefined).length} file(s)`,
    );
}

// Make the directory or file at `path` here as the app's origin has it. What
// a record is too large for is kept in a file, in the origin's private file
// system.
async function copyFile(root, { path, size }) {
    let directory = root;
    const parents = size === undefined ? path : path.slice(0, -1);
    for (const name of parents) directory = await directory.getDirectoryHandle(name, { create: true });
    if (size === undefined) return;
    const handle = await directory.getFileHandle(path[path.length - 1], { create: true });
    const writable = await handle.createWritable();
    try {
        for (let offset = 0; offset < size; offset += MOVED_CHUNK) {
            const { bytes } = await askHost({
                stored: { file: path, offset, length: Math.min(MOVED_CHUNK, size - offset) },
            });
            await writable.write(bytes);
        }
    } finally {
        await writable.close();
    }
}

// Make `database` here as the app's origin has it, and fill it.
async function copyDatabase({ name, version, stores }) {
    const opening = indexedDB.open(name, version);
    opening.onupgradeneeded = () => {
        for (const { name: storeName, keyPath, autoIncrement, indexes } of stores) {
            const store = opening.result.createObjectStore(storeName, { keyPath, autoIncrement });
            for (const index of indexes) {
                store.createIndex(index.name, index.keyPath, {
                    unique: index.unique,
                    multiEntry: index.multiEntry,
                });
            }
        }
    };
    const database = await settled(opening);
    let copied = 0;
    try {
        for (const { name: storeName, keyPath } of stores) {
            let after;
            for (;;) {
                const page = await askHost({
                    stored: { database: name, store: storeName, after, limit: MOVED_BATCH },
                });
                if (page.records.length > 0) {
                    const transaction = database.transaction(storeName, "readwrite");
                    const store = transaction.objectStore(storeName);
                    for (const [key, value] of page.records) {
                        if (keyPath === null) store.put(value, key);
                        else store.put(value);
                    }
                    await new Promise((resolve, reject) => {
                        transaction.oncomplete = resolve;
                        transaction.onerror = () => reject(transaction.error);
                        transaction.onabort = () => reject(transaction.error);
                    });
                    copied += page.records.length;
                    after = page.records[page.records.length - 1][0];
                }
                if (page.done) break;
            }
        }
    } finally {
        database.close();
    }
    return copied;
}

let rust;

function siteWorker() {
    rust ??= (PROFILE ? moveIn() : Promise.resolve())
        .then(() => init({ module_or_path: workerWasm() }))
        .then(() => activate(PROFILE ? "profile" : "space", []))
        .then(async worker => {
            if (PROFILE) {
                // A profile's spaces each have a worker of their own, which
                // holds their content: this one creates a space's identity
                // and leaves the rest to that worker.
                await worker.setSiteOrigins(true);
            } else {
                const grant = await ensureGrant(worker);
                // Who this worker acts for is kept, but where a view reads it
                // (the session overlay) lasts only as long as the worker.
                await worker.resumeSpace(grant.space);
                // The profile may have changed what it told this worker
                // while it was not running to hear it.
                keepTerms(worker).catch(error => log("could not check the space's terms:", error));
            }
            await restoreSession(worker);
            // The worker above passes requests down the port, and a port
            // does not outlive the worker it was handed to. Open one now, so
            // the worker above learns this is a new worker and asks again
            // what it was still being answered.
            if (!hostPort) portToHost().catch(() => {});
            return worker;
        })
        .catch(error => {
            rust = null;
            throw error;
        });
    return rust;
}

// The delegation this worker holds: which space, and until when. Kept in the
// shell cache so a restart does not ask again while the grant still holds.
async function heldGrant() {
    const held = await caches.match(GRANT_KEY, { cacheName: SHELL_CACHE });
    return held ? held.json() : null;
}

// The space this origin holds. An origin that holds none (the profile's) has
// no assets, and is told so without bringing a database up.
async function heldSpace() {
    return (await heldGrant())?.space ?? null;
}

let renewing = null;

// Ask again for a delegation that is close to lapsing, once at a time. Only
// reads the held grant's expiry until it is due.
function renewIfDue() {
    if (PROFILE) return Promise.resolve();
    renewing ??= siteWorker()
        .then(ensureGrant)
        .catch(error => log("could not renew the space's delegation:", error))
        .finally(() => {
            renewing = null;
        });
    return renewing;
}

// Take up a new delegation when what the profile holds for this space is no
// longer what this worker took up: where it syncs, or which account it is
// for.
async function keepTerms(worker) {
    const held = await heldGrant();
    if (!held) return;
    const terms = await askHost({ terms: true });
    if ((terms.remote ?? null) === (held.remote ?? null) && terms.account === held.account) return;
    log("what the profile holds for this space changed; taking up a new delegation");
    await ensureGrant(worker, { renew: true });
}

async function ensureGrant(worker, { renew = false } = {}) {
    const held = await heldGrant();
    const now = Date.now() / 1000;
    if (!renew && held?.version === GRANT_VERSION && held.expires - now > RENEW_MARGIN_SECONDS) {
        return held;
    }
    const audience = await worker.profileDid();
    const grant = await askHost({ delegate: audience });
    await worker.adoptSpace(
        grant.space,
        new Uint8Array(grant.chain),
        grant.remote ?? undefined,
        grant.account,
    );
    // A freshly mounted replica is empty. A space made where spaces have
    // origins of their own has no content anywhere yet: the host hands over
    // what to create it from, and it is created here, the one place it is
    // kept. A space from before that is copied from the host's, once. Either
    // leaves a replica that already has content alone.
    if (!held?.seeded) {
        const snapshot = await askHost({ snapshot: true }).catch(error => {
            // A space that syncs fills from where it syncs. The profile
            // holds only part of one it joined, which is no snapshot.
            if (!grant.remote) throw error;
            log("the profile has no copy to start from; the space will fill as it syncs:", error);
            return { empty: true };
        });
        if (snapshot.fresh) {
            await worker.createContent(grant.space, snapshot.fresh);
            await askHost({ seeded: true });
        } else if (!snapshot.empty) {
            await worker.seedSpace(
                grant.space,
                new Uint8Array(snapshot.content),
                new Uint8Array(snapshot.revision),
            );
        }
    }
    const record = {
        space: grant.space,
        expires: grant.expires,
        remote: grant.remote,
        account: grant.account,
        seeded: true,
        version: GRANT_VERSION,
    };
    const cache = await caches.open(SHELL_CACHE);
    await cache.put(GRANT_KEY, new Response(JSON.stringify(record)));
    log(`holding a delegation for ${grant.space} until ${new Date(grant.expires * 1000).toISOString()}`);
    return record;
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
        throw new Error("no page to broker a port up");
    }
    for (const client of clients) client.postMessage({ type: "need-port" });
    return within(brokered, PORT_TIMEOUT_MS, "no client brokered a port in time");
}

// Ask the worker above, re-brokering once if the port has gone dead.
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
            await within(ack, ACK_TIMEOUT_MS, "the worker above did not answer");
        } catch (error) {
            pending.delete(id);
            if (hostPort === port) hostPort = null;
            if (attempt > 0) throw error;
            log("the port up went quiet; asking for a new one");
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

// ---- The session, across restarts ---------------------------------------
//
// The browser stops an idle worker without telling it, and an update replaces
// it. Either way the next instance starts with no sites stamped, and each page
// would have to notice and claim its site again. So this worker saves what its
// stamps were made from, and the next one makes them again before it serves
// anything.
//
// Saving on every change would cost a cache write per request. A change marks
// the session dirty instead, and one write follows once changes pause. The
// request that made the change holds this worker alive until that write
// lands, so an idle stop never loses it. A page that is hidden or leaving asks
// for the write at once, and so does handing over to a successor.

const SESSION_KEY = "/__space/session";
const SAVE_DELAY_MS = 1_000;

let dirty = false;
let saving = null;
// The bytes last saved, so an unchanged session is not written again.
let saved = null;

async function restoreSession(worker) {
    const held = await caches.match(SESSION_KEY, { cacheName: SHELL_CACHE });
    if (!held) return;
    const bytes = new Uint8Array(await held.arrayBuffer());
    const live = (await self.clients.matchAll({ type: "window" })).map(client => client.id);
    if (await worker.restoreSession(bytes, live)) {
        saved = bytes;
        log("restored the saved session");
    }
}

// Mark the session changed; resolves once the change is saved.
function sessionChanged() {
    dirty = true;
    saving ??= delay(SAVE_DELAY_MS)
        .then(saveWhileDirty)
        .finally(() => {
            saving = null;
        });
    return saving;
}

// Save now, and wait for any save already under way.
async function flushSession() {
    if (!rust) return;
    dirty = true;
    await Promise.all([saving, saveWhileDirty()]);
}

async function saveWhileDirty() {
    while (dirty && rust) {
        dirty = false;
        try {
            const worker = await rust;
            const bytes = await worker.savedSession();
            if (saved && sameBytes(bytes, saved)) continue;
            const cache = await caches.open(SHELL_CACHE);
            await cache.put(SESSION_KEY, new Response(bytes));
            saved = bytes;
        } catch (error) {
            log("could not save the session:", error);
        }
    }
}

function sameBytes(a, b) {
    return a.length === b.length && a.every((byte, index) => byte === b[index]);
}

function delay(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// Serve a request that may change the session, and save once it has: the
// response and every task it left running (a `tonk:load` stamps its site
// after the commit answers) have settled.
function serveChanging(event) {
    const work = [];
    const view = {
        request: event.request,
        clientId: event.clientId,
        resultingClientId: event.resultingClientId,
        waitUntil(promise) {
            work.push(promise);
            event.waitUntil(promise);
        },
    };
    const response = api(view);
    event.waitUntil(
        response
            .then(() => Promise.allSettled(work))
            .then(sessionChanged, () => {}),
    );
    return response;
}

// ---- Handing over to a successor ----------------------------------------
//
// A subscription is a fetch whose response never ends. While one is open this
// worker counts as busy, and a successor that installed waits for it to
// finish, which it never does. So once a successor has installed, this worker
// releases its streams (the Rust worker's `onupdatefound`): the page's
// subscriptions reconnect and land on the successor. The host's worker hands
// over the same way.

let retired = false;

// Only the worker the browser holds as active serves the pages, and only it
// may release their streams. An installing worker hears the same events about
// its own arrival.
function isActiveIncumbent() {
    return self.registration.active === self.serviceWorker;
}

async function retire(reason) {
    // Nothing to release until the database has come up.
    if (retired || !rust || !isActiveIncumbent()) return;
    retired = true;
    log(`handing over: ${reason}`);
    try {
        const worker = await rust;
        // The successor makes its stamps from what is saved: save first.
        await flushSession();
        await worker.onupdatefound?.();
        releaseSpaceReads("this worker is retiring");
    } catch (error) {
        retired = false;
        log("failed to release streams:", error);
    }
}

// An installing worker is not a successor yet: its install can still fail.
// Streams are released only once it has installed.
function watchSuccessor(candidate) {
    if (!candidate || !isActiveIncumbent()) return;
    const observe = () => {
        if (["installed", "activating", "activated"].includes(candidate.state)) {
            candidate.removeEventListener("statechange", observe);
            retire("a newer worker installed");
        } else if (candidate.state === "redundant") {
            candidate.removeEventListener("statechange", observe);
        }
    };
    candidate.addEventListener("statechange", observe);
    observe();
}

// `updatefound` fires into a sleeping worker and is lost, so a restarted
// worker asks the registration whether a successor is already waiting.
if (self.registration.waiting) {
    Promise.resolve().then(() => retire("a successor was already waiting at startup"));
}
watchSuccessor(self.registration.installing);
self.registration.addEventListener("updatefound", () => {
    watchSuccessor(self.registration.installing);
});

// ---- Responses ----------------------------------------------------------

// Where this deployment renders sites, from its `/.well-known/tonk`: the
// authority sites sit under and the app that frames them. The app is not
// derivable from this origin (staging's app is staging.tonk.xyz, its sites
// `{label}.tonk.spot`), so the server says. Kept in the shell cache so a
// restart does not wait on the network.
const CONFIG_PATH = "/.well-known/tonk";
let sites;

async function siteOrigins() {
    if (sites) return sites;
    const cache = await caches.open(SHELL_CACHE);
    try {
        const response = await fetch(CONFIG_PATH, { cache: "no-cache" });
        if (response.ok) await cache.put(CONFIG_PATH, response.clone());
        sites = (await response.json()).sites;
    } catch {
        sites = (await (await cache.match(CONFIG_PATH))?.json())?.sites;
    }
    return sites;
}

// The site origin's policy. The app frames the profile, and the profile frames
// the spaces, so both may be ancestors and sites may frame sites. Author
// code gets no network beyond this origin, which this worker answers alone;
// `worker-src blob:` stops `register()` of any other service worker, since a
// service worker script must be same-origin. `'unsafe-inline'` and
// `'wasm-unsafe-eval'` carry the injected runtime until it loads from this
// origin instead. `'unsafe-eval'` is for author views and element shims,
// which are compiled from strings; space code is untrusted by design, so the
// boundary is this origin and its lack of network, not `script-src`. Without
// the deployment's site origins nothing may frame this origin at all.
// A site's origin may sit under the app's own domain (`{label}.tonk.foundation`
// under `tonk.foundation`), where WebAuthn would let it ask for the app's
// passkeys. No document on a site origin may use them.
const NO_PASSKEYS = "publickey-credentials-get=(), publickey-credentials-create=()";

function spacePolicy(sites, { framedBySelf = false } = {}) {
    const scheme = sites ? new URL(sites.app).protocol : null;
    const profile = sites ? `${scheme}//profile${sites.suffix ?? ""}.${sites.host}` : null;
    const outer = sites ? `${sites.app} ${profile}` : "'none'";
    // An asset opened in a frame is framed by the space that holds it.
    const ancestors = framedBySelf && sites ? `'self' ${outer}` : outer;
    const framed = sites ? ` ${scheme}//*.${sites.host}` : "";
    // A profile renders the app's own hub, not author code, and the hub
    // reads the template catalog and its pictures from where they are
    // published. A space gets no network at all.
    const published = PROFILE ? " https:" : "";
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
    headers.set("content-security-policy", spacePolicy(await siteOrigins()));
    headers.set("permissions-policy", NO_PASSKEYS);
    headers.set("x-content-type-options", "nosniff");
    return new Response(response.body, { status: response.status, headers });
}

// Read an asset from the space's own database, through the same route a
// page's `/api/.../blob/...` read takes.
async function readAsset(hash) {
    const space = await heldSpace();
    if (!space) return { status: 404, headers: [], body: new TextEncoder().encode("not found").buffer };
    const worker = await siteWorker();
    const request = new Request(
        new URL(`/api/repository/${space}/branch/main/blob/asset:${hash}`, self.location.origin),
    );
    const response = await worker.onfetch({
        request,
        clientId: "",
        resultingClientId: "",
        waitUntil() {},
    });
    return { status: response.status, headers: [...response.headers], body: await response.arrayBuffer() };
}

// Store a request's body as an asset of the space, through the same route a
// page's upload takes: the bytes go into the space's store and the commit
// records the asset, with the request's media type. Answers where it now is.
async function storeAsset(event) {
    const space = await heldSpace();
    if (!space) return new Response("not found", { status: 404 });
    const worker = await siteWorker();
    const headers = new Headers();
    for (const name of ["content-type", "x-tonk-blob-name"]) {
        const value = event.request.headers.get(name);
        if (value) headers.set(name, value);
    }
    const request = new Request(
        new URL(`/api/repository/${space}/branch/main/blob`, self.location.origin),
        { method: "POST", headers, body: await event.request.arrayBuffer() },
    );
    const response = await worker.onfetch({
        request,
        clientId: event.clientId,
        resultingClientId: "",
        waitUntil(work) {
            event.waitUntil(work);
        },
    });
    if (!response.ok) return response;
    const stored = await response.json();
    return new Response(JSON.stringify(stored), {
        status: 201,
        headers: { "content-type": "application/json", location: `/${stored.entity}` },
    });
}

// Serve an asset. One opened as a document (a frame or a tab navigated to
// it) is author content like any the space renders, so it carries the
// site's policy as the shell does.
async function serveAsset(hash, request) {
    const reply = await readAsset(hash);
    const headers = new Headers(reply.headers);
    headers.set("x-content-type-options", "nosniff");
    if (request.mode === "navigate") {
        // No such asset: the path is the page's to route, like any other.
        if (reply.status === 404) return serveShell();
        headers.set(
            "content-security-policy",
            spacePolicy(await siteOrigins(), { framedBySelf: true }),
        );
        headers.set("permissions-policy", NO_PASSKEYS);
    }
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
    // Every navigation gets the shell, whatever its path, except one to an
    // asset, which gets the asset.
    const asset = ASSET_PATH.exec(url.pathname);
    if (event.request.mode === "navigate" && !asset) {
        event.respondWith(serveShell());
        return;
    }
    if (asset && (event.request.method === "GET" || event.request.method === "HEAD")) {
        event.respondWith(
            serveAsset(asset[1], event.request).catch(error => {
                log("asset read failed:", error);
                return new Response(String(error.message), { status: 502 });
            }),
        );
        return;
    }
    if (url.pathname.startsWith("/api/")) {
        // Only a write can stamp a site.
        event.respondWith(event.request.method === "GET" ? api(event) : serveChanging(event));
        // A worker kept alive past its delegation's window would otherwise
        // hold a lapsed one: check it as it serves, and ask again when due.
        event.waitUntil(renewIfDue());
        // A worker woken by a page while a successor waits never heard it
        // install: hand over now, before this request opens a stream.
        if (self.registration.waiting) {
            event.waitUntil(retire("a successor is waiting"));
        }
        return;
    }
    if (url.pathname === "/" && event.request.method === "PUT") {
        event.respondWith(
            storeAsset(event).catch(error => {
                log("asset store failed:", error);
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
        event.respondWith(serveStatic(event.request));
        return;
    }
    event.respondWith(new Response("not found", { status: 404 }));
});

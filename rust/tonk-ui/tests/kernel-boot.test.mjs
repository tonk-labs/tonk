import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = readFileSync(join(HERE, "..", "assets", "kernel_boot.js"), "utf8");

const ORIGIN = "https://bspace.tonk.test";
const SPACE = "did:key:zSpace";
const KERNEL = "bafkr4ikernel";
const SESSION = "did:key:zSession";
const REPRESENTATIVE = "did:key:zDevice";
const PERSONA = "did:key:zPersona";
// What a kernel asks its representative to delegate.
const ACCESS = { subject: SPACE, command: "/", arguments: [], reason: "serve the space" };

// Run the kernel bootloader as the worker of a space pinning `pinned`,
// against stand-ins for a service worker's globals and for the embedder.
// `holds` is the kernel the space already holds, if any; `shipped` is what
// the deployment serves under `/_host/kernel/`, and `ships` what its
// `/_host/kernel.json` says it ships; `record` is what the site's shell keeps
// of the kernel it pins. `persona` is whether the kernel is a persona, as
// the embedder reads it from the kernel's exports. `sites` is where the
// deployment says its sites are, at `/.well-known/tonk`, if it says.
function space({
  pinned = KERNEL, holds = null, shipped = { [KERNEL]: "WASM" }, ships = null, origin = ORIGIN, boots = true, record = null,
  answers = {}, persona = false, sites = null,
} = {}) {
  const listeners = {};
  const told = [];
  const booted = [];
  const handled = [];
  const sentHeaders = [];
  // The body of each request the kernel was handed, as text.
  const bodies = [];
  // What the host handed the kernel with `receive`.
  const received = [];
  const imported = [];
  const fetched = [];
  const pages = [{ postMessage: (message) => void told.push(message) }];
  // What the host lent the kernel for an authorization, and whom it had the
  // kernel act for.
  const authorized = [];
  const stood = [];
  const embedder = {
    timings: {},
    authorize: async (handoff, remote, context) =>
      void authorized.push({ handoff, remote, context: JSON.parse(new TextDecoder().decode(context)) }),
    stand: async (standing) => void stood.push(standing),
    receive: async (bytes) => void received.push(JSON.parse(new TextDecoder().decode(bytes))),
    handle: async (method, path, headers = [], body = new Uint8Array()) => {
      handled.push(`${method} ${path}`);
      sentHeaders.push(headers);
      bodies.push(new TextDecoder().decode(body));
      const answer = answers[`${method} ${path}`];
      const [status, text, type = "text/plain"] =
        answer ? answer(JSON.parse(bodies.at(-1) || "null"))
        : path === "/api/identify" ? [200, JSON.stringify({ did: "did:key:zKernel" })]
        : path === "/.kernel/adopt" ? [204, ""]
        : path === "/page" ? [200, '<!doctype html><html><head><title>A page</title></head><body>hi</body></html>', "text/html"]
        : path === "/early" ? [200, '<!-- made by hand -->\n<!DOCTYPE html><script>first()</script><html><head></head></html>', "text/html"]
        : [404, "not found"];
      // The body as the kernel writes it: in pieces of a few bytes, so a
      // page's start arrives split, then its end.
      const bytes = new TextEncoder().encode(text);
      const pieces = [];
      for (let at = 0; at < bytes.length; at += 3) pieces.push(bytes.slice(at, at + 3));
      // A subscription stays open once it said what it had.
      const open = path.endsWith("/subscribe");
      return {
        status,
        headers: [["content-type", type]],
        body: { next: () => (pieces.length || !open ? Promise.resolve(pieces.shift()) : new Promise(() => {})), cancel() {} },
      };
    },
  };
  // What the embedder keeps where Dialog DB keeps it: the space this origin
  // serves, and the kernel each space pins.
  const kept = { pins: new Map(holds ? [[SPACE, holds]] : []) };
  const opened = [];
  // Whether the space's kernel boots: the person's profile came to hold the
  // space once `letBoot` is called.
  let canBoot = boots;
  const letBoot = () => void (canBoot = true);
  const bindgen = Object.assign(async () => {}, {
    openSession: async (name) => (opened.push(name), { did: SESSION }),
    heldSpace: async () => kept.pins.keys().next().value,
    kernelId: (bytes) => new TextDecoder().decode(bytes).replace("WASM", KERNEL),
    install: async (subject, bytes) => {
      const id = new TextDecoder().decode(bytes).replace("WASM", KERNEL);
      kept.pins.set(subject, id);
      return id;
    },
    pinned: async (subject) => kept.pins.get(subject),
    isPersona: () => persona,
    pinsPersona: async (subject) => persona && kept.pins.has(subject),
    // A kernel creates a space it takes up for the first time, and says so.
    bootSpace: async (subject, session, representative, persona, delegate, sent) => {
      if (!canBoot) throw new Error("the person has no such space");
      booted.push({ subject, session: session.did, representative, persona, delegate, sent });
      if (!holds) queueMicrotask(() => sent(new TextEncoder().encode(JSON.stringify({ ask: 1, seeded: true }))));
      return embedder;
    },
    bootPersona: async (subject, session, sent, merge, authorize) => {
      booted.push({ subject, session: session.did, sent, merge, authorize });
      return embedder;
    },
  });
  let offline = false;
  // A network that stalls takes every request and answers none, as a link
  // that stopped carrying anything does: nothing says it is down.
  let stalled = false;
  const network = async (address) => {
    if (offline) throw new TypeError("Failed to fetch");
    const path = new URL(typeof address === "string" ? address : address.url, origin).pathname;
    fetched.push(path);
    if (stalled) return new Promise(() => {});
    if (path === "/site.html") return new Response("<head></head>SHELL");
    if (path === "/kernel/relay.js") return new Response("RELAY");
    if (path === "/.well-known/tonk" && sites) return Response.json({ sites });
    if (path === "/library/media.webp") return new Response(new Uint8Array([0, 159, 255]));
    if (path === "/_host/kernel.json" && ships) return Response.json(ships);
    const id = /^\/_host\/kernel\/(.+)\.wasm$/.exec(path)?.[1];
    if (id && shipped[id]) return new Response(shipped[id], { headers: { "content-type": "application/wasm" } });
    return new Response("", { status: 404 });
  };
  const self = {
    location: new URL(`/kernel_boot.js?kernel=${pinned}`, origin),
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => pages, get: async (id) => (id === "page-1" ? pages[0] : undefined) },
  };
  // The caches this worker keeps, as the test reads them.
  let hostCaches;
  const context = vm.createContext({
    self,
    importScripts: (...urls) => void imported.push(...urls),
    wasm_bindgen: bindgen,
    fetch: network,
    performance,
    crypto,
    URL, Request, Response, Headers, TextEncoder, TextDecoder, Uint8Array, Promise, ReadableStream, TransformStream, Blob,
    setTimeout, clearTimeout, queueMicrotask, btoa, atob,
    // The host's files this worker keeps; for a space that holds its
    // kernel, who it acts for, as its first start kept.
    caches: (hostCaches = (() => {
      const kept = new Map([
        ...(holds
          ? [[new URL("/_host/standing.json", origin).href, Response.json({ representative: REPRESENTATIVE, persona: PERSONA })]]
          : []),
        // What the shell keeps of the kernel the site pins.
        ...(record ? [["/__boot", Response.json(record)]] : []),
      ]);
      const key = (request) => (typeof request === "string" ? request : request.url);
      const cache = {
        match: async (request) => kept.get(key(request))?.clone(),
        put: async (request, response) => void kept.set(key(request), response),
        delete: async (request) => kept.delete(key(request)),
        keys: async () => [...kept.keys()],
      };
      return { open: async () => cache, match: async (request) => cache.match(request) };
    })()),
    console: { log() {} },
  });
  vm.runInContext(SOURCE, context);

  // The profile, at the other end of the port a page hands this worker.
  const asked = [];
  const port = {
    postMessage: (message) => {
      asked.push(message);
      const standing = { representative: REPRESENTATIVE, persona: PERSONA };
      const answer = message.delegate
        ? { id: message.id, space: SPACE, chain: new ArrayBuffer(1), ...standing }
        : message.standing
          ? { id: message.id, ...standing }
          : message.kernel
          ? { id: message.id, bytes: new TextEncoder().encode(shipped[message.kernel] ?? "").buffer }
            : { id: message.id, settled: true };
      queueMicrotask(() => port.onmessage({ data: answer }));
    },
  };
  const handPort = () =>
    listeners.message({ data: { type: "port" }, ports: [port], waitUntil() {} });

  // Hand the worker `request` as a fetch event; answers what it responded
  // with, or `undefined` where it left the request to the network.
  const request = async (path, { mode = "cors", method = "GET", clientId = "" } = {}) => {
    let responded;
    const waits = [];
    const made = {
      url: new URL(path, origin).href,
      method,
      mode,
      headers: new Headers(),
      arrayBuffer: async () => new ArrayBuffer(0),
      clone: () => ({ ...made }),
    };
    listeners.fetch({
      request: made,
      clientId,
      respondWith: (response) => void (responded = response),
      waitUntil: (promise) => void waits.push(promise),
    });
    // A body is read as a page reads it, and the worker's work waited for
    // after, since some of it lasts as long as the body does.
    const response = await responded;
    if (!response?.body) {
      await Promise.all(waits);
      return response;
    }
    const body = await response.arrayBuffer();
    await Promise.all(waits);
    return new Response(body, response);
  };
  const message = (data) => {
    let answered;
    listeners.message({ data, ports: [{ postMessage: (reply) => void (answered = reply) }], waitUntil() {} });
    return answered;
  };
  // Post `data` as a page does, answered later on its port: resolves to the
  // answer.
  const post = (data) =>
    new Promise((resolve) => listeners.message({ data, ports: [{ postMessage: resolve }], waitUntil() {} }));
  // The profile passes a request down the port, as it does one a page of
  // the app asks about this space.
  const passDown = (call, path) => port.onmessage({ data: { call, request: { method: "GET", path } } });
  const goOffline = () => void (offline = true);
  const stall = () => void (stalled = true);
  // Ask, as the shell that installed the worker does, for the kernel to boot
  // and take up its space.
  const started = () =>
    new Promise((resolve) =>
      listeners.message({ data: { type: "booted" }, ports: [{ postMessage: resolve }], waitUntil() {} }),
    );
  return { hostCaches, bodies, received, sentHeaders, listeners, imported, fetched, booted, handled, asked, told, kept, opened, authorized, stood, handPort, passDown, request, message, post, goOffline, stall, started, letBoot };
}

test("the bootloader loads the host's embedder", () => {
  assert.deepEqual(space().imported, ["/_host/embedder.js"]);
});

test("a page the kernel answers with carries the host's relay, right after its doctype", async () => {
  const site = space();
  site.handPort();
  const page = await (await site.request("/page", { mode: "navigate" })).text();
  assert.equal(
    page,
    '<!doctype html><script src="/kernel/relay.js"></script><html><head><title>A page</title></head><body>hi</body></html>',
  );
  assert.deepEqual(site.handled.slice(-1), ["GET /page"]);
});

test("the relay runs before a script a page puts ahead of its head", async () => {
  const site = space();
  site.handPort();
  const page = await (await site.request("/early", { mode: "navigate" })).text();
  assert.equal(
    page,
    '<!-- made by hand -->\n<!DOCTYPE html><script src="/kernel/relay.js"></script><script>first()</script><html><head></head></html>',
  );
});

test("a page load is the kernel's to answer, with the site's policy on what it answers", async () => {
  const site = space();
  site.handPort();
  const response = await site.request("/", { mode: "navigate" });
  assert.equal(response.status, 404);
  assert.equal(await response.text(), "not found", "what the kernel answered, as it is");
  assert.match(response.headers.get("content-security-policy"), /default-src 'none'/);
  assert.deepEqual(site.handled.slice(-1), ["GET /"]);
});

test("a page load the kernel cannot answer gets the host's page, with the relay", async () => {
  const site = space({ pinned: "bafkr4iother", holds: KERNEL });
  site.handPort();
  const response = await site.request("/", { mode: "navigate" });
  assert.equal(response.status, 502);
  assert.match(await response.text(), /^<!doctype html><html><head><script src="\/kernel\/relay.js"><\/script>/);
});

test("a request that is not a page load is answered as the kernel answers it", async () => {
  const site = space();
  site.handPort();
  const response = await site.request("/page");
  assert.match(await response.text(), /^<!doctype html><html><head><title>/, "no relay outside a page load");
});

// What the shell of a space keeps when the page that framed it named its
// kernel and said it hands it a host.
const HOSTED = { v: 2, kernel: KERNEL, host: true };

test("a space with no kernel of its own installs the one its worker names, and boots it", async () => {
  const site = space({ record: HOSTED });
  site.handPort();
  assert.equal((await site.started()).booted, true, "the shell is told the kernel has started");
  const response = await site.request("/api/anything");
  assert.equal(response.status, 404);
  assert.deepEqual(
    site.asked.map((asked) => Object.keys(asked).filter((key) => key !== "id")),
    [["kernel"], ["delegate", "access"], ["seeded", "from"]],
    "it takes its kernel, which is no persona, asks for the space for its session, and says it created the space",
  );
  assert.equal(site.asked.find((asked) => asked.delegate)?.delegate, SESSION, "the space is delegated to the key made here");
  assert.equal(site.asked.find((asked) => asked.kernel)?.kernel, KERNEL);
  assert.ok(!site.fetched.some((path) => path.startsWith("/_host/kernel/")), "nothing of the kernel is fetched");
  assert.deepEqual(
    site.booted.map(({ subject, session }) => ({ subject, session })),
    [{ subject: SPACE, session: SESSION }],
  );
  assert.deepEqual(site.handled, ["GET /api/anything"], "the kernel takes its space up as it boots");
});

test("the kernel's first request to its representative is answered with the delegation handed over with its session", async () => {
  const site = space();
  site.handPort();
  await site.started();
  const asked = site.asked.length;
  const chain = await site.booted[0].delegate(SESSION, ACCESS);
  assert.equal(chain.length, 1);
  assert.equal(site.asked.length, asked, "the profile is not asked again");
  assert.deepEqual(
    [site.booted[0].representative, site.booted[0].persona],
    [REPRESENTATIVE, PERSONA],
    "the kernel acts through the representative, for the persona, the profile named",
  );
});

test("the session's key is the one Dialog DB's credential store keeps under the profile's name", async () => {
  const site = space();
  site.handPort();
  await site.started();
  assert.deepEqual(site.opened, ["tonk"]);
});

test("the origin remembers the space it was delegated", async () => {
  const site = space();
  site.handPort();
  await site.started();
  assert.equal(site.kept.pins.keys().next().value, SPACE, "the space the profile delegated is the one this origin serves");
});

test("a space that holds its kernel boots it from where it is kept, with no port up", async () => {
  const site = space({ holds: KERNEL, shipped: {} });
  await site.started();
  assert.ok(!site.fetched.some((path) => path.startsWith("/_host/kernel/")), "nothing is fetched to boot");
  assert.deepEqual(
    site.booted.map(({ subject, session }) => ({ subject, session })),
    [{ subject: SPACE, session: SESSION }],
    "it boots with no port up",
  );
});

test("a kernel that boots on what it kept has its request answered by the profile once a port is up", async () => {
  const site = space({ holds: KERNEL, shipped: {} });
  await site.started();
  let claimed;
  site.booted[0].delegate(SESSION, ACCESS).then((chain) => (claimed = chain));
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(claimed, undefined, "nothing answers before a port is up");
  site.handPort();
  for (let i = 0; i < 50 && !claimed; i++) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(
    site.asked.map((asked) => asked.delegate).filter(Boolean),
    [SESSION],
    "the profile delegates to the key the kernel acts with",
  );
  assert.equal(claimed.length, 1);
});

test("a space the shell says pins another kernel than its worker's boots it", async () => {
  const site = space({ pinned: "bafkr4iother", holds: KERNEL, record: { v: 2, kernel: KERNEL } });
  site.handPort();
  const response = await site.request("/api/anything");
  assert.notEqual(response.status, 502, "the kernel answers, not the host");
  assert.equal(site.booted.length, 1);
});

test("a worker for another kernel than the space pins does not boot it", async () => {
  const site = space({ pinned: "bafkr4iother", holds: KERNEL });
  site.handPort();
  const response = await site.request("/api/anything");
  assert.equal(response.status, 502);
  assert.match(await response.text(), /pins bafkr4ikernel, not bafkr4iother/);
  assert.deepEqual(site.booted, []);
});

test("the host's own files come from the network, and are kept for when it does not answer", async () => {
  const site = space();
  const shell = await site.request("/site.html", { mode: "navigate" });
  assert.match(await shell.text(), /SHELL/);
  assert.deepEqual(site.handled, [], "the kernel is not asked for the host's files");
  site.goOffline();
  const kept = await site.request("/site.html", { mode: "navigate" });
  assert.match(await kept.text(), /SHELL/, "offline, the kept copy answers");
});

test("a page load does not wait on a network that stalls for what the worker keeps", async () => {
  const site = space({ sites: { app: "https://tonk.test", host: "tonk.test" } });
  site.handPort();
  // A page loaded once: its policy read where the deployment says its
  // sites are, and its relay fetched.
  await (await site.request("/page", { mode: "navigate" })).text();
  assert.equal(await (await site.request("/kernel/relay.js")).text(), "RELAY");
  site.stall();
  let timer;
  const stalled = new Promise((resolve) => (timer = setTimeout(() => resolve("stalled"), 5_000)));
  const page = await Promise.race([site.request("/page", { mode: "navigate" }), stalled]);
  assert.notEqual(page, "stalled", "the page load is answered while the network stalls");
  assert.match(page.headers.get("content-security-policy"), /frame-ancestors 'self' https:\/\/tonk\.test/);
  assert.match(await page.text(), /<title>A page<\/title>/);
  const relay = await Promise.race([site.request("/kernel/relay.js"), stalled]);
  assert.notEqual(relay, "stalled", "the relay the page loads is answered while the network stalls");
  clearTimeout(timer);
  assert.equal(await relay.text(), "RELAY");
});

test("the worker speaks the shell's protocol", () => {
  assert.equal(space().message({ type: "protocol" })?.protocol, 3);
});

test("a request the profile passes down is answered by the kernel", async () => {
  const site = space();
  site.handPort();
  site.passDown(7, "/api/roster");
  for (let i = 0; i < 50 && !site.asked.some((message) => message.end); i++) await new Promise((r) => setTimeout(r, 5));
  const answer = site.asked.filter((message) => message.call === 7);
  assert.equal(answer[0].head.status, 404);
  const body = answer.filter((message) => message.chunk).map((message) => new TextDecoder().decode(message.chunk));
  assert.equal(body.join(""), "not found");
  assert.equal(answer.at(-1).end, true);
  assert.deepEqual(site.handled.slice(-1), ["GET /api/roster"]);
});

test("the kernel reads public bytes through its host, in base64", async () => {
  const site = space();
  site.handPort();
  await site.request("/api/anything");
  await site.booted[0].sent(new TextEncoder().encode(JSON.stringify({ ask: 5, read: "/library/media.webp", bytes: true })));
  for (let i = 0; i < 50 && !site.received.some((message) => message.answer === 5); i++) await new Promise((r) => setTimeout(r, 5));
  const answer = site.received.find((message) => message.answer === 5);
  assert.equal(answer.status, 200);
  assert.equal(answer.body64, btoa(String.fromCharCode(0, 159, 255)), "the bytes are as served");
});

test("the kernel sends the page that asked somewhere, through its host", async () => {
  const site = space();
  site.handPort();
  await site.request("/api/anything", { clientId: "page-1" });
  assert.deepEqual(
    [...site.sentHeaders.at(-1).find(([name]) => name === "x-tonk-client-id")],
    ["x-tonk-client-id", "page-1"],
    "the kernel hears which page asked",
  );
  await site.booted[0].sent(new TextEncoder().encode(JSON.stringify({ tell: "navigate", client: "page-1", href: "/" })));
  assert.deepEqual(JSON.parse(JSON.stringify(site.told.at(-1))), { type: "navigate", href: "/" });
});

test("the profile passes what a page asks about a space's content to that space's kernel, as it answers", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  const asked = [];
  const host = {
    postMessage: ({ call, request }) => {
      asked.push(request);
      queueMicrotask(() => {
        host.onmessage({ data: { call, head: { status: 200, headers: [["content-type", "text/event-stream"]] } } });
        host.onmessage({ data: { call, chunk: new TextEncoder().encode("data: 1\n\n").buffer } });
        host.onmessage({ data: { call, end: true } });
      });
    },
  };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  const path = `/api/repository/${SPACE}/branch/main/query`;
  const response = await profile.request(path, { method: "POST" });
  assert.equal(response.status, 200);
  assert.equal(await response.text(), "data: 1\n\n");
  assert.deepEqual(
    asked.map(({ method, path }) => ({ method, path })),
    [{ method: "POST", path }],
    "the space's kernel is asked, not the persona",
  );
  assert.ok(!profile.handled.includes(`POST ${path}`), "the persona is not asked about the space");
});

test("a space's DID written percent-encoded still reaches that space's kernel", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  const asked = [];
  const host = {
    postMessage: ({ call, request }) => {
      asked.push(request);
      queueMicrotask(() => {
        host.onmessage({ data: { call, head: { status: 200, headers: [["content-type", "application/json"]] } } });
        host.onmessage({ data: { call, end: true } });
      });
    },
  };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  const path = `/api/repository/${encodeURIComponent(SPACE)}/branch/main/sync/status`;
  const response = await profile.request(path);
  assert.equal(response.status, 200);
  assert.equal(asked.length, 1, "the space's kernel is asked, not the persona");
  assert.ok(!profile.handled.some((line) => line.includes("sync/status")), "the persona is not asked about the space");
});

test("the persona's subscription to a space's kernel reaches it in pieces, and stops when the persona stops reading", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  const asked = [];
  const host = {
    postMessage: (message) => {
      asked.push(message);
      if (!message.request) return;
      const { call } = message;
      queueMicrotask(() => {
        host.onmessage({ data: { call, head: { status: 200, headers: [] } } });
        host.onmessage({ data: { call, chunk: new TextEncoder().encode("data: 1\n\n").buffer } });
      });
    },
  };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  await profile.request("/api/identify");
  const ask = { ask: 9, to: SPACE, method: "POST", path: "/api/repository/x/branch/main/query", body: "{}", stream: true };
  await profile.booted[0].sent(new TextEncoder().encode(JSON.stringify(ask)));
  for (let i = 0; i < 50 && profile.received.length < 2; i++) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(JSON.parse(JSON.stringify(profile.received.slice(0, 2))), [
    { answer: 9, status: 200 },
    { piece: 9, body64: btoa("data: 1\n\n") },
  ]);
  assert.equal(asked[0].request.path, ask.path, "the space's kernel is asked");

  await profile.booted[0].sent(new TextEncoder().encode(JSON.stringify({ tell: "cancel", ask: 9 })));
  assert.deepEqual(JSON.parse(JSON.stringify(asked.at(-1))), { cancel: asked[0].call }, "the call to the space is cancelled");
});

// What the kernel sends its host, as bytes.
const sending = (message) => new TextEncoder().encode(JSON.stringify(message));
// What a page derived with a passkey, for `request`.
const handoff = (request) => ({ type: "custody", credentialId: "aa", key: new Uint8Array(32), kek: new Uint8Array(32), request });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("the persona's ask to act as the account asks the page for the passkey, and lends the account it opens", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  await profile.started();
  const intent = { kind: "purge-account" };
  await profile.booted[0].authorize("to delete your account", sending(intent));
  assert.deepEqual(JSON.parse(JSON.stringify(profile.told.at(-1))), { type: "webauthn", request: "custody", intent });

  const answer = await profile.post(handoff(intent));
  assert.deepEqual(JSON.parse(JSON.stringify(answer)), { ok: {} });
  assert.equal(profile.authorized.length, 1);
  assert.deepEqual(profile.authorized[0].context, intent, "the kernel gets back the context it asked in");
  assert.equal(profile.authorized[0].remote, "https://profile.tonk.test/ucan/");
});

test("a page's ask for a delegation is the kernel's to make, under the passkey the page already ran", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  await profile.started();
  const request = { kind: "delegate", subject: SPACE, command: "/", audience: PERSONA };
  const answered = profile.post(handoff(request));
  await settle();
  assert.deepEqual(profile.received.at(-1), { tell: "delegate", subject: SPACE, command: "/", audience: PERSONA });

  // The kernel asks to act as the account for it: the page is not asked
  // again, and its hand-off is lent.
  const asked = profile.told.length;
  await profile.booted[0].authorize("to share a space", sending(request));
  assert.equal(profile.told.length, asked, "the page is not asked for the passkey again");
  assert.equal(profile.authorized.length, 1);

  await profile.booted[0].sent(sending({ tell: "delegated", chain: "zChain" }));
  assert.deepEqual(JSON.parse(JSON.stringify(await answered)), { ok: { delegation: "zChain" } });
});

test("the persona acts for another persona once it asks to, from its next boot too", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  await profile.started();
  await profile.booted[0].sent(sending({ ask: 7, standing: { persona: PERSONA, link64: btoa("link") } }));
  await settle();
  assert.equal(profile.stood.length, 1);
  assert.equal(profile.stood[0].persona, PERSONA);
  assert.equal(new TextDecoder().decode(profile.stood[0].link), "link");
  assert.deepEqual(profile.received.at(-1), { answer: 7, status: 200, body: "" });

  await profile.booted[0].sent(sending({ ask: 8, standing: null }));
  await settle();
  assert.equal(profile.stood.at(-1), null, "signed out, it acts for its provisional persona");
});

test("a new site whose kernel is a persona is its session's own, and asks no one for it", async () => {
  const profile = space({ origin: "https://bwhatever.tonk.test", persona: true });
  await profile.started();
  assert.deepEqual(profile.asked, [], "no host is asked for the space or the kernel");
  assert.equal(profile.booted[0].subject, SESSION, "the persona's space is its session's");
  assert.ok(profile.booted[0].merge, "and it boots as a persona, wherever it is");
});

test("a new site whose kernel is no persona is the space its host delegates, on the profile's origin too", async () => {
  const site = space({ origin: "https://profile.tonk.test" });
  site.handPort();
  await site.started();
  assert.equal(site.booted[0].subject, SPACE, "the space the host named");
  assert.equal(site.booted[0].representative, REPRESENTATIVE, "booted as a space");
});

test("a profile whose kernel the deployment no longer serves says so on its page", async () => {
  const profile = space({ origin: "https://profile.tonk.test", shipped: {}, persona: true });
  const response = await profile.request("/", { mode: "navigate" });
  assert.equal(response.status, 502);
  assert.match(await response.text(), /runs the kernel bafkr4ikernel, which this deployment no longer serves/);
  assert.deepEqual(profile.booted, []);
});

test("a page load while the kernel did not boot gets the host's page, which tries again", async () => {
  const site = space({ holds: KERNEL, boots: false });
  site.handPort();
  const response = await site.request("/", { mode: "navigate" });
  assert.equal(response.status, 502);
  const page = await response.text();
  assert.match(page, /location\.reload\(\)/, "the page loads again in a while");
});

test("a shell asking again for a kernel that did not boot has it boot again", async () => {
  const site = space({ holds: KERNEL, boots: false });
  site.handPort();
  assert.match((await site.started()).error, /no such space/);

  site.letBoot();
  assert.equal((await site.started()).booted, true, "what it lacked has come");
  assert.equal(site.booted.length, 1);
});

test("what a page asks of a space whose kernel does not answer, the persona answers", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  const host = {
    postMessage: ({ call }) => {
      queueMicrotask(() => host.onmessage({ data: { call, error: "the person has no such space" } }));
    },
  };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  const path = `/api/repository/${SPACE}/branch/main/query`;
  const response = await profile.request(path, { method: "POST" });
  assert.equal(response.status, 404, "the persona's answer, of a space it does not hold");
  assert.ok(profile.handled.includes(`POST ${path}`), "the persona is asked");
});

test("what the persona answered of a space ends once the space's kernel answers", async () => {
  const profile = space({ origin: "https://profile.tonk.test", persona: true });
  let answers = false;
  const host = {
    postMessage: ({ call }) => {
      queueMicrotask(() => {
        if (!answers) return host.onmessage({ data: { call, error: "the kernel did not boot" } });
        host.onmessage({ data: { call, head: { status: 200, headers: [] } } });
        host.onmessage({ data: { call, end: true } });
      });
    },
  };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  const path = `/api/repository/${SPACE}/branch/main/subscribe`;
  let responded;
  profile.listeners.fetch({
    request: { url: new URL(path, "https://profile.tonk.test").href, method: "POST", mode: "cors", headers: new Headers(), arrayBuffer: async () => new ArrayBuffer(0), clone() { return { ...this }; } },
    clientId: "",
    respondWith: (response) => void (responded = response),
    waitUntil() {},
  });
  const persona = await responded;
  const reading = persona.body.getReader();
  // What the persona had to say, "not found" in its three pieces; then it
  // waits for more.
  for (let piece = 0; piece < 3; piece++) await reading.read();
  const first = reading.read();

  answers = true;
  await profile.request(path, { method: "POST" });
  await assert.rejects(first, /answers now/, "the persona's answer ends");
});

// What the profile's host keeps of the spaces the app's page handed over. The
// harness keeps every cache in one place, so the copies are told by path.
const migrated = async (profile) =>
  (await (await profile.hostCaches.open("tonk-migration")).keys()).filter((key) => key.startsWith("/_host/migrated/"));

// A space an earlier release kept, as the app's page reads it: its content
// and its revision's JSON.
const KEPT = { name: "Old notes", content: new Uint8Array([1, 2, 3]).buffer, revision: new TextEncoder().encode('{"tree":"x"}').buffer };

test("a space an earlier release kept is kept by the profile's host and made again by the persona", async () => {
  const profile = space({
    origin: "https://profile.tonk.test",
    persona: true,
    answers: { "POST /_kernel/migrated": () => [200, JSON.stringify({ space: SPACE })] },
  });
  const answer = await profile.post({ type: "migrate", spaces: [KEPT] });
  assert.deepEqual(JSON.parse(JSON.stringify(answer)), { made: [{ name: "Old notes", space: SPACE }], failed: [] });
  const asked = JSON.parse(profile.bodies[profile.handled.indexOf("POST /_kernel/migrated")]);
  assert.equal(asked.name, "Old notes");
  assert.match(asked.copy, /^migrated:/, "the persona names the copy as the host keeps it");
  assert.equal((await migrated(profile)).length, 1, "the host keeps what it was handed");
});

test("a space the persona did not make leaves nothing kept, and says why", async () => {
  const profile = space({
    origin: "https://profile.tonk.test",
    persona: true,
    answers: { "POST /_kernel/migrated": () => [500, '"no space was made"'] },
  });
  const answer = await profile.post({ type: "migrate", spaces: [KEPT] });
  assert.deepEqual(JSON.parse(JSON.stringify(answer)), { made: [], failed: [{ name: "Old notes", error: '"no space was made"' }] });
  assert.deepEqual(await migrated(profile), []);
});

test("a space made as a copy of one an earlier release kept is created from what the host keeps", async () => {
  let copy;
  const profile = space({
    origin: "https://profile.tonk.test",
    persona: true,
    answers: {
      "POST /_kernel/migrated": (asked) => ((copy = asked.copy), [200, JSON.stringify({ space: SPACE })]),
      [`GET /_kernel/seed/${SPACE}`]: () => [200, JSON.stringify({ name: "Old notes", copy_from: copy })],
      [`POST /_kernel/seed/${SPACE}/settled`]: () => [200, "null"],
    },
  });
  await profile.post({ type: "migrate", spaces: [KEPT] });
  const host = { postMessage: (message) => host.answers.push(message), answers: [] };
  profile.listeners.message({ data: { type: "space-port", repo: SPACE }, ports: [host], waitUntil() {} });
  // The port is bound once the kernel booted and is known to be a persona.
  for (let i = 0; i < 100 && !host.onmessage; i++) await new Promise((resolve) => setTimeout(resolve, 5));
  const until = async (id) => {
    for (let i = 0; i < 100 && !host.answers.some((answer) => answer.id === id && !answer.ack); i++) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    return host.answers.find((answer) => answer.id === id && !answer.ack);
  };
  await host.onmessage({ data: { id: 1, seed: true } });
  const seeded = await until(1);
  assert.deepEqual([...new Uint8Array(seeded.copy.content)], [1, 2, 3]);
  assert.equal(atob(seeded.copy.revision64), '{"tree":"x"}');
  await host.onmessage({ data: { id: 2, seeded: true } });
  assert.equal((await until(2)).settled, true);
  assert.deepEqual(await migrated(profile), [], "nothing is kept once the space is created");
});

test("a site renders with the renderer its kernel keeps", async () => {
  const site = space({ holds: KERNEL, answers: { "GET /guest/manifest.json": () => [200, '{"kept":true}', "application/json"] } });
  site.handPort();
  const response = await site.request("/guest/manifest.json");
  assert.deepEqual(await response.json(), { kept: true });
  assert.ok(!site.fetched.includes("/guest/manifest.json"), "nothing of the deployment's renderer is fetched");
});

test("a site whose kernel keeps no renderer renders with the deployment's", async () => {
  const profile = space({ persona: true });
  await profile.request("/guest/manifest.json");
  assert.ok(profile.handled.includes("GET /guest/manifest.json"), "its kernel is asked first");
  assert.ok(profile.fetched.includes("/guest/manifest.json"), "then the deployment's is fetched");
});

test("a kernel that is no persona takes nothing for the person's account", async () => {
  const site = space({ holds: KERNEL });
  site.handPort();
  for (const type of ["resend", "migrate", "custody"]) {
    const answer = await site.post({ type, spaces: [] });
    assert.match(answer.error, new RegExp(`does not take ${type}`));
  }
  assert.ok(!site.handled.includes("GET /api/profiles"), "and asks its kernel nothing of an account");
});

test("a new site its framer hands a host waits for the port, however late, and takes its kernel from the host", async () => {
  const site = space({ record: HOSTED });
  const started = site.started();
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(site.booted.length, 0, "it waits for the port it was promised");
  site.handPort();
  assert.equal((await started).booted, true);
  assert.ok(site.asked.some((asked) => asked.kernel === KERNEL), "the host is asked for the kernel");
  assert.ok(!site.fetched.some((path) => path.startsWith("/_host/kernel/")), "nothing of the kernel is fetched");
});

test("a new site its framer hands no host takes the deployment's kernel, and asks no host for it", async () => {
  const profile = space({ record: { v: 2, kernel: KERNEL, host: false }, persona: true });
  await profile.started();
  assert.ok(profile.fetched.includes(`/_host/kernel/${KERNEL}.wasm`), "the deployment's copy");
  assert.deepEqual(profile.asked, [], "no host is asked");
});

test("a page asks the worker what its site can do: a persona's hosts spaces, a space's does not", async () => {
  const ask = (site) => site.post({ type: "abilities" });
  const profile = space({ persona: true });
  assert.equal((await ask(profile)).hostsSpaces, true);
  const site = space({ holds: KERNEL });
  site.handPort();
  assert.equal((await ask(site)).hostsSpaces, false);
});
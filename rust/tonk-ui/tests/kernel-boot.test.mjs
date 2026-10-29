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
// the deployment serves under `/_host/kernel/`.
function space({ pinned = KERNEL, holds = null, shipped = { [KERNEL]: "WASM" }, origin = ORIGIN } = {}) {
  const listeners = {};
  const told = [];
  const booted = [];
  const handled = [];
  const sentHeaders = [];
  // What the host handed the kernel with `receive`.
  const received = [];
  const imported = [];
  const fetched = [];
  const pages = [{ postMessage: (message) => void told.push(message) }];
  const embedder = {
    timings: {},
    receive: async (bytes) => void received.push(JSON.parse(new TextDecoder().decode(bytes))),
    handle: async (method, path, headers = []) => {
      handled.push(`${method} ${path}`);
      sentHeaders.push(headers);
      const [status, text, type = "text/plain"] =
        path === "/api/identify" ? [200, JSON.stringify({ did: "did:key:zKernel" })]
        : path === "/.kernel/adopt" ? [204, ""]
        : path === "/page" ? [200, '<!doctype html><html><head><title>A page</title></head><body>hi</body></html>', "text/html"]
        : path === "/early" ? [200, '<!-- made by hand -->\n<!DOCTYPE html><script>first()</script><html><head></head></html>', "text/html"]
        : [404, "not found"];
      // The body as the kernel writes it: in pieces of a few bytes, so a
      // page's start arrives split, then its end.
      const bytes = new TextEncoder().encode(text);
      const pieces = [];
      for (let at = 0; at < bytes.length; at += 3) pieces.push(bytes.slice(at, at + 3));
      return {
        status,
        headers: [["content-type", type]],
        body: { next: async () => pieces.shift(), cancel() {} },
      };
    },
  };
  // What the embedder keeps where Dialog DB keeps it: the space this origin
  // serves, and the kernel each space pins.
  const kept = { pins: new Map(holds ? [[SPACE, holds]] : []) };
  const opened = [];
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
    // A kernel creates a space it takes up for the first time, and says so.
    bootSpace: async (subject, session, representative, persona, delegate, sent) => {
      booted.push({ subject, session: session.did, representative, persona, delegate, sent });
      if (!holds) queueMicrotask(() => sent(new TextEncoder().encode(JSON.stringify({ ask: 1, seeded: true }))));
      return embedder;
    },
    bootPersona: async (subject, session, sent) => {
      booted.push({ subject, session: session.did, sent });
      return embedder;
    },
  });
  let offline = false;
  const network = async (address) => {
    if (offline) throw new TypeError("Failed to fetch");
    const path = new URL(typeof address === "string" ? address : address.url, origin).pathname;
    fetched.push(path);
    if (path === "/space.html") return new Response("<head></head>SHELL");
    const id = /^\/_host\/kernel\/(.+)\.wasm$/.exec(path)?.[1];
    if (id && shipped[id]) return new Response(shipped[id]);
    return new Response("", { status: 404 });
  };
  const self = {
    location: new URL(`/kernel_boot.js?kernel=${pinned}`, origin),
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => pages, get: async (id) => (id === "page-1" ? pages[0] : undefined) },
  };
  const context = vm.createContext({
    self,
    importScripts: (...urls) => void imported.push(...urls),
    wasm_bindgen: bindgen,
    fetch: network,
    performance,
    crypto,
    URL, Request, Response, Headers, TextEncoder, TextDecoder, Uint8Array, Promise, ReadableStream, TransformStream,
    setTimeout, clearTimeout, queueMicrotask, btoa, atob,
    // The host's files this worker keeps; for a space that holds its
    // kernel, who it acts for, as its first start kept.
    caches: (() => {
      const kept = new Map(
        holds
          ? [[new URL("/_host/standing.json", origin).href, Response.json({ representative: REPRESENTATIVE, persona: PERSONA })]]
          : [],
      );
      const key = (request) => (typeof request === "string" ? request : request.url);
      const cache = {
        match: async (request) => kept.get(key(request))?.clone(),
        put: async (request, response) => void kept.set(key(request), response),
      };
      return { open: async () => cache, match: async (request) => cache.match(request) };
    })(),
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
    listeners.fetch({
      request: {
        url: new URL(path, origin).href,
        method,
        mode,
        headers: new Headers(),
        arrayBuffer: async () => new ArrayBuffer(0),
      },
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
  // The profile passes a request down the port, as it does one a page of
  // the app asks about this space.
  const passDown = (call, path) => port.onmessage({ data: { call, request: { method: "GET", path } } });
  const goOffline = () => void (offline = true);
  // Ask, as the shell that installed the worker does, for the kernel to boot
  // and take up its space.
  const started = () =>
    new Promise((resolve) =>
      listeners.message({ data: { type: "booted" }, ports: [{ postMessage: resolve }], waitUntil() {} }),
    );
  return { received, sentHeaders, listeners, imported, fetched, booted, handled, asked, told, kept, opened, handPort, passDown, request, message, goOffline, started };
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

test("a space with no kernel of its own installs the one its worker names, and boots it", async () => {
  const site = space();
  site.handPort();
  assert.equal((await site.started()).booted, true, "the shell is told the kernel has started");
  const response = await site.request("/api/anything");
  assert.equal(response.status, 404);
  assert.deepEqual(
    site.asked.map((asked) => Object.keys(asked).filter((key) => key !== "id")),
    [["delegate", "access"], ["kernel"], ["seeded", "from"]],
    "it asks for the space for its session, takes its kernel, and says it created the space",
  );
  assert.equal(site.asked[0].delegate, SESSION, "the space is delegated to the key made here");
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
  const shell = await site.request("/space.html", { mode: "navigate" });
  assert.match(await shell.text(), /SHELL/);
  assert.deepEqual(site.handled, [], "the kernel is not asked for the host's files");
  site.goOffline();
  const kept = await site.request("/space.html", { mode: "navigate" });
  assert.match(await kept.text(), /SHELL/, "offline, the kept copy answers");
});

test("the worker speaks the shell's protocol", () => {
  assert.equal(space().message({ type: "protocol" })?.protocol, 2);
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
  const profile = space({ origin: "https://profile.tonk.test" });
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

test("the persona's subscription to a space's kernel reaches it in pieces, and stops when the persona stops reading", async () => {
  const profile = space({ origin: "https://profile.tonk.test" });
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

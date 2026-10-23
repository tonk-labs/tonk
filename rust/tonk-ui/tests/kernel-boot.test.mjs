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

// IndexedDB as far as the bootloader uses it: object stores of keyed values,
// read and written in transactions that complete once their requests have.
function indexedDB() {
  const stores = new Map();
  const request = (value) => {
    const asked = { result: value };
    queueMicrotask(() => asked.onsuccess?.());
    return asked;
  };
  const db = {
    createObjectStore: (name) => void (stores.has(name) || stores.set(name, new Map())),
    transaction: (name) => {
      const transaction = {
        objectStore: () => ({
          get: (key) => request(stores.get(name).get(key)),
          getAll: () => request([...stores.get(name).values()]),
          put: (value, key) => request(void stores.get(name).set(key, value)),
        }),
      };
      setTimeout(() => transaction.oncomplete?.(), 0);
      return transaction;
    },
  };
  return {
    stores,
    open: () => {
      const opening = { result: db };
      queueMicrotask(() => {
        opening.onupgradeneeded?.();
        opening.onsuccess?.();
      });
      return opening;
    },
  };
}

// Run the kernel bootloader as the worker of a space pinning `pinned`,
// against stand-ins for a service worker's globals and for the embedder.
// `holds` is the kernel the space's store already holds, if any; `shipped`
// is what the deployment serves under `/_host/kernel/`.
function space({ pinned = KERNEL, holds = null, shipped = { [KERNEL]: "WASM" } } = {}) {
  const listeners = {};
  const told = [];
  const booted = [];
  const handled = [];
  const imported = [];
  const fetched = [];
  const store = indexedDB();
  const pages = [{ postMessage: (message) => void told.push(message) }];
  const embedder = {
    drain: async () => [],
    timings: {},
    handle: async (method, path) => {
      handled.push(`${method} ${path}`);
      const [status, text, type = "text/plain"] =
        path === "/api/identify" ? [200, JSON.stringify({ did: "did:key:zKernel" })]
        : path === "/.kernel/adopt" ? [204, ""]
        : path === "/page" ? [200, '<!doctype html><html><head><title>A page</title></head><body>hi</body></html>', "text/html"]
        : [404, "not found"];
      // The body as the kernel writes it: one piece, then its end.
      const pieces = text ? [new TextEncoder().encode(text)] : [];
      return {
        status,
        headers: [["content-type", type]],
        body: { next: async () => pieces.shift(), cancel() {} },
      };
    },
  };
  const bindgen = Object.assign(async () => {}, {
    session: () => "did:key:zSession",
    kernelId: (bytes) => new TextDecoder().decode(bytes).replace("WASM", KERNEL),
    install: (subject, bytes) => [{ kind: "blob", subject, key: [1], bytes, kernel: KERNEL }],
    pinned: (subject, records) => records.find((record) => record.subject === subject)?.kernel,
    boot: async (subject, seed, authority, proofs, records) => {
      booted.push({ subject, proofs: proofs.length, records: records.length });
      return embedder;
    },
  });
  if (holds) {
    store.stores.set("records", new Map([["blob", { kind: "blob", subject: SPACE, key: [1], kernel: holds }]]));
  }
  let offline = false;
  const network = async (address) => {
    if (offline) throw new TypeError("Failed to fetch");
    const path = new URL(typeof address === "string" ? address : address.url, ORIGIN).pathname;
    fetched.push(path);
    if (path === "/space.html") return new Response("<head></head>SHELL");
    const id = /^\/_host\/kernel\/(.+)\.wasm$/.exec(path)?.[1];
    if (id && shipped[id]) return new Response(shipped[id]);
    return new Response("", { status: 404 });
  };
  const self = {
    location: new URL(`/kernel_boot.js?kernel=${pinned}`, ORIGIN),
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => pages },
  };
  const context = vm.createContext({
    self,
    importScripts: (...urls) => void imported.push(...urls),
    wasm_bindgen: bindgen,
    indexedDB: store,
    fetch: network,
    performance,
    crypto,
    URL, Request, Response, Headers, TextEncoder, TextDecoder, Uint8Array, Promise, ReadableStream, TransformStream,
    setTimeout, queueMicrotask,
    // The host's files this worker keeps.
    caches: (() => {
      const kept = new Map();
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
      const answer = message.delegate
        ? { id: message.id, space: SPACE, chain: new ArrayBuffer(1), expires: Date.now() / 1000 + 86400 }
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
  const request = async (path, { mode = "cors", method = "GET" } = {}) => {
    let responded;
    const waits = [];
    listeners.fetch({
      request: {
        url: new URL(path, ORIGIN).href,
        method,
        mode,
        headers: new Headers(),
        arrayBuffer: async () => new ArrayBuffer(0),
      },
      respondWith: (response) => void (responded = response),
      waitUntil: (promise) => void waits.push(promise),
    });
    const response = await responded;
    await Promise.all(waits);
    return response;
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
  return { listeners, imported, fetched, booted, handled, asked, told, handPort, passDown, request, message, goOffline, started };
}

test("the bootloader loads the host's embedder", () => {
  assert.deepEqual(space().imported, ["/_host/embedder.js"]);
});

test("a page the kernel answers with carries the host's broker, first in its head", async () => {
  const site = space();
  site.handPort();
  const page = await (await site.request("/page", { mode: "navigate" })).text();
  assert.equal(
    page,
    '<!doctype html><html><head><script src="/kernel_broker.js"></script><title>A page</title></head><body>hi</body></html>',
  );
  assert.deepEqual(site.handled.slice(-1), ["GET /page"]);
});

test("a page load the kernel answers with no page gets the host's, with the broker", async () => {
  const site = space();
  site.handPort();
  const response = await site.request("/", { mode: "navigate" });
  assert.equal(response.status, 404);
  assert.equal(response.headers.get("content-type"), "text/html; charset=utf-8");
  const page = await response.text();
  assert.match(page, /<head><script src="\/kernel_broker.js"><\/script>/);
  assert.match(page, /Nothing is installed here yet/);
});

test("a request that is not a page load is answered as the kernel answers it", async () => {
  const site = space();
  site.handPort();
  const response = await site.request("/page");
  assert.match(await response.text(), /^<!doctype html><html><head><title>/, "no broker outside a page load");
});

test("a space with no kernel of its own installs the one its worker names, and boots it", async () => {
  const site = space();
  site.handPort();
  assert.equal((await site.started()).booted, true, "the shell is told the kernel has started");
  const response = await site.request("/api/anything");
  assert.equal(response.status, 404);
  assert.deepEqual(
    site.asked.map((asked) => Object.keys(asked).filter((key) => key !== "id")),
    [["delegate"], ["seeded", "from"], ["kernel"], ["delegate"]],
    "it asks the profile which space it is, says it holds it, takes its kernel, and asks a delegation for the kernel",
  );
  assert.equal(site.asked[0].delegate, "did:key:zSession");
  assert.equal(site.asked[2].kernel, KERNEL);
  assert.ok(!site.fetched.some((path) => path.startsWith("/_host/kernel/")), "nothing of the kernel is fetched");
  assert.deepEqual(site.booted, [{ subject: SPACE, proofs: 1, records: 1 }]);
  assert.deepEqual(site.handled, ["GET /api/identify", "POST /.kernel/adopt", "GET /api/anything"]);
});

test("a space that holds its kernel boots it from its own store", async () => {
  const site = space({ holds: KERNEL, shipped: {} });
  site.handPort();
  await site.request("/api/anything");
  assert.ok(!site.fetched.some((path) => path.startsWith("/_host/kernel/")), "nothing is fetched to boot");
  assert.deepEqual(site.booted, [{ subject: SPACE, proofs: 1, records: 1 }]);
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
  assert.equal(new TextDecoder().decode(answer[1].chunk), "not found");
  assert.equal(answer[2].end, true);
  assert.deepEqual(site.handled.slice(-1), ["GET /api/roster"]);
});

test("the kernel takes up its space with a delegation to its own profile", async () => {
  const site = space();
  site.handPort();
  await site.started();
  const delegations = site.asked.filter((message) => message.delegate);
  assert.equal(delegations.at(-1).delegate, "did:key:zKernel", "the profile delegates to the kernel's profile");
  assert.deepEqual(site.handled.slice(0, 2), ["GET /api/identify", "POST /.kernel/adopt"]);
});

test("a kernel asks for its delegation once as it boots", async () => {
  const site = space();
  site.handPort();
  await site.started();
  // A delegation that lapses in a day needs no renewal yet.
  assert.equal(site.asked.filter((message) => message.delegate).length, 2);
});

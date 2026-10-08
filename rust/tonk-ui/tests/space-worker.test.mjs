import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
// The worker is a module that imports the Rust worker's bindings. Here the
// bindings are stand-ins, handed in, so the script runs as it ships.
const SOURCE = readFileSync(join(HERE, "..", "assets", "space_worker.js"), "utf8").replace(
  /^import init, \{ activate \} from "\.\/worker\.js";$/m,
  "const { init, activate } = __rust;",
);
assert.ok(SOURCE.includes("__rust"), "the worker's import of its Rust bindings moved");

const SPACE = "did:key:zSpace";

// Run a site's worker against stand-ins for a service worker's globals.
// `routes` is what the space's routes answer a path with, as the Rust
// worker's `/http/` route would.
// `holds` is whether the worker already keeps its space's delegation: one
// that does not has to ask the worker above for it, and bring the space here.
// `offline` is whether the server can be reached for a newer worker.
// `incumbent` is whether another worker is the active one as this one starts.
function site({
  host = "bspace.tonk.test", routes = {}, failing = false, holds = true, offline = false, incumbent = false,
  lingers = false,
} = {}) {
  // How often the worker asked the browser to look for a newer one.
  const looked = { count: 0 };
  // What the Rust worker was brought up with.
  const activated = [];
  const origin = `https://${host}`;
  const stores = new Map();
  const key = (request) => (typeof request === "string" ? request : request.url);
  const open = async (name) => {
    if (!stores.has(name)) stores.set(name, new Map());
    const store = stores.get(name);
    return {
      match: async (request) => store.get(key(request))?.clone(),
      put: async (request, response) => void store.set(key(request), response),
      add: async (request) => void store.set(key(request), await network(request)),
      keys: async () => [...store.keys()].map((url) => ({ url })),
      delete: async (request) => store.delete(key(request)),
    };
  };
  const listeners = {};
  // What the routes were asked for, by path.
  const asked = [];
  const rust = new Proxy(
    {
      onfetch: async ({ request }) => {
        if (failing) throw new Error("the database is not open");
        const path = new URL(request.url).pathname;
        const route = /\/branch\/main\/http(\/.*)$/.exec(path)?.[1];
        if (route === undefined) return new Response("", { status: 404 });
        asked.push(route);
        const held = routes[route];
        return held
          ? new Response(held.body, { headers: held.headers ?? {} })
          : new Response("", { status: 404 });
      },
      profileDid: async () => "did:key:zProfile",
      adoptSpace: async () => ({ remote: null, account: "did:key:zAccount", name: null }),
    },
    // Anything else the worker calls answers nothing. Not `then`: a
    // stand-in that has one is taken for a promise and awaited for ever.
    { get: (worker, name) => worker[name] ?? (name === "then" ? undefined : async () => undefined) },
  );
  // What the worker told its pages.
  const told = [];
  const pages = [{ id: "frame", url: `${origin}/`, postMessage: (message) => void told.push(message) }];
  const self = {
    location: new URL("/space_worker.js", origin),
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => pages },
    registration: {
      active: incumbent ? {} : null,
      waiting: null,
      addEventListener() {},
      update: async () => {
        looked.count += 1;
        if (offline) throw new TypeError("Failed to fetch");
      },
    },
  };
  const network = async (request) => {
    const path = new URL(key(request), origin).pathname;
    if (path === "/.well-known/tonk") {
      return new Response(JSON.stringify({ sites: { app: "https://tonk.test", host: "tonk.test" } }));
    }
    if (path === "/space.html" || path === "/profile.html") return new Response("SHELL");
    if (path === "/worker_bg.wasm") return new Response(new Uint8Array([0, 97, 115, 109]));
    return new Response("from the server", { status: 404 });
  };
  const context = vm.createContext({
    self,
    __rust: {
      init: async () => {},
      activate: async (...named) => {
        activated.push(named);
        return rust;
      },
    },
    URL, Request, Response, Headers, TextEncoder, TextDecoder, MessageChannel, Promise,
    Uint8Array, ReadableStream, crypto,
    console: { log() {}, warn() {}, error() {} },
    // With `lingers`, a timer the worker leaves running (how long it keeps
    // an idle space) does not keep the test waiting for it.
    setTimeout: lingers ? (...timed) => setTimeout(...timed).unref() : setTimeout,
    clearTimeout,
    fetch: network,
    caches: {
      open,
      keys: async () => [...stores.keys()],
      delete: async (name) => stores.delete(name),
      match: async (request, { cacheName }) => (await open(cacheName)).match(request),
    },
  });
  vm.runInContext(SOURCE, context);

  // The space this origin holds, as a worker that has taken its delegation
  // keeps it: the site is here already, with nothing to ask its host for.
  const settled = !holds ? Promise.resolve() : open("tonk-space-shell").then((cache) =>
    cache.put(
      "/__space/grant",
      new Response(
        JSON.stringify({ space: SPACE, expires: Date.now() / 1000 + 86_400, seeded: true, version: 5 }),
      ),
    ),
  );

  const request = (path, { mode = "cors", method = "GET" } = {}) => ({
    url: new URL(path, origin).href,
    method,
    mode,
    headers: new Headers(),
  });
  // What the worker answers a request with, or `undefined` when it leaves
  // the request to the network.
  const answer = async (path, options) => {
    await settled;
    let answered;
    const pending = [];
    listeners.fetch({
      request: request(path, options),
      respondWith: (response) => void (answered = response),
      waitUntil: (promise) => void pending.push(promise),
    });
    await Promise.all(pending);
    return answered;
  };
  // A shell asking for the content at `path`, as it does from inside a
  // frame of this origin.
  const admit = async (path) => {
    const said = [];
    listeners.message({
      data: { type: "content", url: new URL(path, origin).href },
      ports: [{ postMessage: (message) => said.push(message) }],
      waitUntil() {},
    });
    return said;
  };
  // A page hands the worker a port to the worker above it, which answers
  // what the worker asks: first that it heard, then with `answers(request)`.
  // Resolves once the worker has done what the port let it start.
  const connect = async (answers) => {
    const asks = [];
    const port = {
      onmessage: null,
      postMessage(request) {
        if (typeof request.id !== "number") return;
        asks.push(request);
        queueMicrotask(() => {
          port.onmessage({ data: { id: request.id, ack: true } });
          port.onmessage({ data: { id: request.id, ...answers(request) } });
        });
      },
    };
    let started;
    listeners.message({
      data: { type: "port" },
      ports: [port],
      source: { id: "frame" },
      waitUntil: (promise) => void (started = promise),
    });
    await started;
    return asks;
  };
  const stages = () => told.filter((message) => message.type === "status").map((message) => message.stage);
  // Run one of the worker's lifecycle events to the end of what it waits on.
  const lifecycle = async (type) => {
    const pending = [];
    listeners[type]({ waitUntil: (promise) => void pending.push(promise) });
    await Promise.all(pending);
  };
  // A page tells the worker something, handing over `ports`.
  const message = (data, ports = []) => listeners.message({ data, ports, waitUntil() {} });
  return {
    answer, admit, asked, connect, stages, looked, activated, lifecycle, stores, message, worker: self,
  };
}

const page = { mode: "navigate" };
const HELLO = { "/hello.html": { body: "<p>hello</p>", headers: { "content-type": "text/html" } } };

test("a frame loading an address gets the shell, whatever the address routes to", async () => {
  const { answer, asked } = site({ routes: HELLO });

  for (const path of ["/", "/notes", "/hello.html", "/deep/er/path?x=1"]) {
    const response = await answer(path, page);
    assert.equal(await response.text(), "SHELL", `${path} loaded as a page`);
  }

  assert.deepEqual(asked, [], "a page load is answered without asking the routes");
});

test("the shell may be framed by the app, the profile, and its own origin", async () => {
  const { answer } = site();

  const policy = (await answer("/notes", page)).headers.get("content-security-policy");

  assert.match(policy, /frame-ancestors 'self' https:\/\/tonk\.test https:\/\/profile\.tonk\.test/);
});

test("a profile reads what is published over https or from this machine, and a space reads nothing", async () => {
  const directive = (policy, name) => policy.split("; ").find((part) => part.startsWith(`${name} `));
  const reach = " https: http://localhost:* http://127.0.0.1:*";

  const profile = (await site({ host: "profile.tonk.test" }).answer("/", page)).headers.get("content-security-policy");
  assert.equal(directive(profile, "connect-src"), `connect-src 'self' blob: data:${reach}`);
  assert.equal(directive(profile, "img-src"), `img-src 'self' blob: data:${reach}`);

  const space = (await site().answer("/", page)).headers.get("content-security-policy");
  assert.equal(directive(space, "connect-src"), "connect-src 'self' blob: data:");
  assert.equal(directive(space, "img-src"), "img-src 'self' blob: data:");
});

test("a request for an address gets the content a route keeps there", async () => {
  const { answer, asked } = site({ routes: HELLO });

  const response = await answer("/hello.html");

  assert.equal(response.status, 200);
  assert.equal(await response.text(), "<p>hello</p>");
  assert.equal(response.headers.get("content-type"), "text/html");
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  assert.equal(response.headers.get("cache-control"), "no-cache", "a named address can change");
  assert.deepEqual(asked, ["/hello.html"]);
});

test("a route's own cache-control is kept", async () => {
  const { answer } = site({
    routes: { "/kept.js": { body: "1", headers: { "cache-control": "max-age=60" } } },
  });

  assert.equal((await answer("/kept.js")).headers.get("cache-control"), "max-age=60");
});

test("a request is never answered with the shell", async () => {
  const { answer } = site({ routes: HELLO });

  for (const path of ["/hello.html", "/no-such-thing", "/notes", "/"]) {
    for (const method of ["GET", "HEAD"]) {
      const response = await answer(path, { method });
      assert.notEqual(await response.text(), "SHELL", `${method} ${path}`);
    }
  }
  assert.equal((await answer("/no-such-thing")).status, 404);
});

test("a HEAD request says whether there is content, with no body", async () => {
  const { answer } = site({ routes: HELLO });

  const there = await answer("/hello.html", { method: "HEAD" });
  const absent = await answer("/notes", { method: "HEAD" });

  assert.equal(there.status, 200);
  assert.equal(await there.text(), "");
  assert.equal(absent.status, 404);
});

test("a load a shell asked for is answered with the content, once", async () => {
  const { answer, admit } = site({ routes: HELLO });

  assert.equal(JSON.stringify(await admit("/hello.html")), '[{"admitted":true}]');
  const first = await answer("/hello.html", page);
  const second = await answer("/hello.html", page);

  assert.equal(await first.text(), "<p>hello</p>");
  assert.equal(first.headers.get("x-content-type-options"), "nosniff");
  assert.match(first.headers.get("content-security-policy"), /frame-ancestors 'self'/);
  assert.equal(await second.text(), "SHELL", "the next load of the address is a page load again");
});

test("asking for one address does not admit another", async () => {
  const { answer, admit } = site({
    routes: { ...HELLO, "/other.html": { body: "other" } },
  });

  await admit("/hello.html");

  assert.equal(await (await answer("/other.html", page)).text(), "SHELL");
  assert.equal(await (await answer("/hello.html?x=1", page)).text(), "SHELL");
});

test("a load a shell asked for, where no route keeps content, is not found", async () => {
  const { answer, admit } = site();

  await admit("/notes");
  const response = await answer("/notes", page);

  assert.equal(response.status, 404);
  assert.notEqual(await response.text(), "SHELL", "a shell would ask and load again, without end");
});

test("a request is not found when the routes cannot be read", async () => {
  const { answer } = site({ routes: HELLO, failing: true });

  assert.equal((await answer("/hello.html")).status, 404);
});

test("a profile's worker answers requests from no routes", async () => {
  const { answer, asked } = site({ host: "profile.tonk.test", routes: HELLO });

  assert.equal((await answer("/hello.html")).status, 404);
  assert.equal(await (await answer("/hello.html", page)).text(), "SHELL");
  assert.deepEqual(asked, []);
});

test("the app's static files and the worker's own API are not routes", async () => {
  const { answer, asked } = site({ routes: { "/guest/x.js": { body: "shadowed" } } });

  const file = await answer("/guest/x.js");

  assert.equal(await file.text(), "from the server", "a static file comes from the server");
  assert.deepEqual(asked, []);
});

// What the worker above answers a space's worker with.
const host = (request) => {
  if (request.delegate) return { space: SPACE, chain: [1, 2, 3], expires: Date.now() / 1000 + 86_400 };
  if (request.seed) return { fresh: "the space's first content" };
  return {};
};

test("a worker bringing its space here for the first time says what it is waiting on", async () => {
  const { connect, stages } = site({ holds: false });

  const asks = await connect(host);

  assert.deepEqual(stages(), ["asking for the space", "replicating the space", "ready"]);
  assert.ok(asks.some((request) => request.delegate), "it asked for its delegation");
  assert.ok(asks.some((request) => request.seed), "and for what to make the space from");
});

test("a worker that could not bring its space still ends the wait", async () => {
  const { connect, stages } = site({ holds: false });

  await connect((request) => (request.delegate ? { error: "not a member" } : {}));

  assert.deepEqual(stages(), ["asking for the space", "ready"], "a page is never left waiting on it");
});

test("a worker that holds its space already has nothing to report", async () => {
  const { connect, stages } = site();

  await connect(host);

  assert.deepEqual(stages(), []);
});

test("a site's worker says how it is, without its database having to be up", async () => {
  const { answer, asked } = site({ host: "profile.tonk.test" });

  const before = await (await answer("/api/health")).json();
  assert.equal(before.site, "profile");
  assert.equal(before.worker, "idle", "asking how it is does not bring the database up");
  assert.equal(before.workerWasm, null);
  assert.equal(typeof before.startedAt, "number");
  assert.ok(Array.isArray(before.log));

  await answer("/api/identify");
  const after = await (await answer("/api/health")).json();
  assert.equal(after.worker, "ok");
  assert.equal(after.workerWasm, "dev", "it names the wasm it checked itself against");
  assert.equal(after.attempts, 1);
  assert.equal(after.startedAt, before.startedAt, "the same worker answered both");
  assert.deepEqual(asked, [], "health is never asked of the routes");
});

test("a space's worker says how it is to its own pages", async () => {
  const { answer } = site();

  const health = await (await answer("/api/health")).json();

  assert.equal(health.site, "space");
});

test("a page load has the worker look for a newer one, which the page may not ask for", async () => {
  const { answer, looked } = site({ host: "profile.tonk.test" });

  await answer("/", page);
  assert.equal(looked.count, 1);

  // Only a page load asks, and each one does: it is when a deploy is found.
  await answer("/api/health");
  assert.equal(looked.count, 1);
  await answer("/settings", page);
  assert.equal(looked.count, 2);

  // Two pages loading at once share one look.
  await Promise.all([answer("/", page), answer("/account", page)]);
  assert.equal(looked.count, 3);
});

test("a look for a newer worker that fails leaves the page served", async () => {
  const { answer, looked } = site({ offline: true });

  const response = await answer("/notes", page);

  assert.equal(await response.text(), "SHELL");
  assert.equal(looked.count, 1);
});

test("a worker taking over waits for what the last one held before it serves", async () => {
  const { lifecycle, stores } = site({ host: "profile.tonk.test", incumbent: true });
  await lifecycle("install");
  const handoff = stores.get("TONK_OVERLAY_HANDOFF");
  assert.ok(handoff.has("/__tonk/overlay-handoff-pending"), "it says a snapshot is to come");

  // The worker it replaces writes the snapshot a moment into the wait.
  let activated = false;
  const activating = lifecycle("activate").then(() => { activated = true; });
  await new Promise((resolve) => setTimeout(resolve, 60));
  assert.equal(activated, false, "it holds activation while the snapshot is still to come");
  handoff.set("/__tonk/overlay-handoff", new Response("snapshot"));
  await activating;

  assert.ok(!handoff.has("/__tonk/overlay-handoff-pending"), "the wait is over");
  assert.ok(handoff.has("/__tonk/overlay-handoff"), "and the snapshot is left for the Rust worker to take");
});

test("a first worker waits for no one", async () => {
  const { lifecycle, stores } = site();
  await lifecycle("install");
  const started = Date.now();
  await lifecycle("activate");

  assert.ok(Date.now() - started < 500);
  assert.equal(stores.get("TONK_OVERLAY_HANDOFF")?.size ?? 0, 0, "and says nothing is to come");
});

test("the Rust worker is named by the build it runs, to tell its own snapshot from another's", async () => {
  const { answer, activated } = site({ host: "profile.tonk.test" });

  await answer("/api/identify");

  assert.deepEqual(JSON.parse(JSON.stringify(activated)), [["dev", [], false]]);
});

test("a space's Rust worker is told it is one, so its profile is given no account", async () => {
  const { connect, activated } = site();

  await connect(host);

  assert.deepEqual(JSON.parse(JSON.stringify(activated)), [["dev", [], true]]);
});

test("the Rust worker opens a subscription with a space's worker, and ends it by letting go", async () => {
  const { worker, message } = site({ host: "profile.tonk.test", lingers: true });
  const sent = [];
  let heard;
  const port = {
    onmessage: null,
    postMessage(said) {
      sent.push(said);
      if (typeof said.ping === "number") queueMicrotask(() => port.onmessage({ data: { pong: said.ping } }));
      if (said.request) heard?.(said);
    },
  };
  message({ type: "space-port", repo: SPACE, branch: "main" }, [port]);
  const path = `/api/repository/${SPACE}/branch/main/query`;

  const passed = new Promise((resolve) => (heard = resolve));
  const opening = worker.tonkSubscribeSpace(SPACE, path, "{}");
  const { call, request } = await passed;
  port.onmessage({ data: { call, head: { status: 200, headers: [["content-type", "text/event-stream"]] } } });
  const { status, body } = await opening;
  port.onmessage({ data: { call, chunk: new TextEncoder().encode("data: 1\n\n").buffer } });
  const reader = body.getReader();
  const first = await reader.read();
  await reader.cancel();

  assert.equal(request.method, "POST");
  assert.equal(request.path, path);
  assert.ok(
    request.headers.some(([name, value]) => name === "accept" && value === "text/event-stream"),
    "it asks for the answer to stay open",
  );
  assert.equal(status, 200);
  assert.equal(new TextDecoder().decode(first.value), "data: 1\n\n");
  assert.ok(sent.some((said) => said.cancel === call), "the space's worker is told the subscription ended");
});

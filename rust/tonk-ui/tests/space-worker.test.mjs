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
function site({ host = "bspace.tonk.test", routes = {}, failing = false } = {}) {
  const origin = `https://${host}`;
  const stores = new Map();
  const key = (request) => (typeof request === "string" ? request : request.url);
  const open = async (name) => {
    if (!stores.has(name)) stores.set(name, new Map());
    const store = stores.get(name);
    return {
      match: async (request) => store.get(key(request))?.clone(),
      put: async (request, response) => void store.set(key(request), response),
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
    },
    // Anything else the worker calls answers nothing. Not `then`: a
    // stand-in that has one is taken for a promise and awaited for ever.
    { get: (worker, name) => worker[name] ?? (name === "then" ? undefined : async () => undefined) },
  );
  const self = {
    location: new URL("/space_worker.js", origin),
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => [] },
    registration: { waiting: null, addEventListener() {} },
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
    __rust: { init: async () => {}, activate: async () => rust },
    URL, Request, Response, Headers, TextEncoder, TextDecoder, MessageChannel, Promise,
    Uint8Array, crypto,
    console: { log() {}, warn() {}, error() {} },
    setTimeout, clearTimeout,
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
  const settled = open("tonk-space-shell").then((cache) =>
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
    listeners.fetch({
      request: request(path, options),
      respondWith: (response) => void (answered = response),
      waitUntil() {},
    });
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
  return { answer, admit, asked };
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

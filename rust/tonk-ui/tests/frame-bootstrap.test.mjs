import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = readFileSync(join(HERE, "..", "..", "tonk-portal", "src", "bootstrap.js"), "utf8");

const turn = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 10; i++) await turn(); };
const plain = (value) => JSON.parse(JSON.stringify(value));

// Run the frame's bootstrap in stand-ins for the document it runs in.
// `origin` is the frame's own: a site's, or `"null"` for a sealed frame.
function frame({ origin = "https://bspace.tonk.test", address = "/" } = {}) {
  const sealed = origin === "null";
  const at = new URL(address, sealed ? "https://sealed.invalid" : origin);
  const posted = [];   // what the frame told the page around, over its port
  const fetched = [];  // requests the frame made itself
  const natives = [];  // history calls that reached the browser
  const events = [];   // events raised on the window
  const noop = () => {};
  let port;
  class Channel {
    constructor() {
      this.port1 = { onmessage: null, postMessage: (message) => void posted.push(message) };
      this.port2 = {};
      port = this.port1;
    }
  }
  const window = {
    addEventListener: noop,
    removeEventListener: noop,
    dispatchEvent: (event) => void events.push(event.type),
    fetch: async (input, init) => {
      const request = typeof input === "string" ? { url: input, headers: new Headers(init?.headers) } : input;
      fetched.push({
        url: request.url,
        method: init?.method ?? request.method ?? "GET",
        headers: Object.fromEntries(request.headers ?? []),
        body: init?.body ?? request.bodyText,
      });
      return new Response("{}");
    },
  };
  // A request the frame builds keeps what the test reads back.
  class Asked {
    constructor(input, init = {}) {
      this.url = typeof input === "string" ? input : input.url;
      this.method = init.method ?? "GET";
      this.headers = new Headers(init.headers);
      this.bodyText = init.body;
    }
  }
  const history = {
    state: null,
    pushState: (...args) => void natives.push(["pushState", ...args]),
    replaceState: (_state, _unused, url) => {
      natives.push(["replaceState", url]);
      if (url !== undefined && url !== null) at.pathname = new URL(url, at).pathname;
    },
    go: (delta) => void natives.push(["go", delta]),
    back: () => void natives.push(["back"]),
    forward: () => void natives.push(["forward"]),
  };
  const context = vm.createContext({
    window,
    document: {
      addEventListener: noop, removeEventListener: noop, baseURI: `${at.origin}/`,
      body: {}, documentElement: {}, activeElement: null,
    },
    parent: { postMessage: noop },
    location: {
      get origin() { return origin; },
      get pathname() { return at.pathname; },
      get search() { return at.search; },
      get hash() { return at.hash; },
      get href() { return at.href; },
      get hostname() { return at.hostname; },
      reload: () => void natives.push(["reload"]),
    },
    history,
    navigator: {},
    MessageChannel: Channel,
    Request: Asked,
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init?.detail; } },
    KeyboardEvent: class {},
    MutationObserver: class { observe() {} disconnect() {} },
    Map, Set, Promise, URL, Response, Headers, JSON, Date, Object, Array, String, Math, Number,
    setTimeout, clearTimeout, console: { log() {}, warn() {} }, crypto,
  });
  vm.runInContext(SOURCE, context);
  const tonk = window.tonk;
  // The page around answers the frame's hello, and later moves it.
  const hear = async (message) => { port.onmessage({ data: message }); await settle(); };
  const context0 = {
    siteEntity: "site:abc", sitePath: at.pathname.slice(1), site: "site:tab",
    repo: sealed ? "" : "did:key:zSpace", branch: "main", with: "main@did:key:zSpace",
    path: "/space/did:key:zSpace", search: "", hash: "", origin: "https://tonk.test",
    // What says the frame is on an origin of its own, with a worker there.
    sitePattern: sealed ? undefined : "*.tonk.test",
  };
  const ready = (extra = {}) => hear({ type: "ready", context: { ...context0, ...extra } });
  return { tonk, window, history, posted, fetched, natives, events, hear, ready, context0, at };
}

const told = (site, type) => plain(site.posted.filter((message) => message.type === type));

test("a site's frame claims the route of its own address with its own worker", async () => {
  const site = frame({ address: "/notes" });
  await site.ready();
  await site.tonk.claimed;

  const [claim] = site.fetched;
  assert.equal(claim.url, "/api/repository/did:key:zSpace/branch/main/transact");
  assert.equal(claim.method, "POST");
  const { parameters } = JSON.parse(claim.body).claims[0].application;
  assert.deepEqual(parameters, { this: "site:abc", path: "/notes" });
});

test("a site's frame asks its own worker for everything, and the page around for nothing", async () => {
  const site = frame();
  await site.ready();
  site.fetched.length = 0;

  await site.window.fetch("/api/repository/did:key:zSpace/branch/main/query", { method: "POST", body: "{}" });
  await site.window.fetch("/tonk.js");
  await site.window.fetch("/guest/manifest.json");

  assert.deepEqual(site.fetched.map((request) => request.url), [
    "/api/repository/did:key:zSpace/branch/main/query",
    "/tonk.js",
    "/guest/manifest.json",
  ]);
  assert.equal(site.fetched[0].headers["x-tonk-site"], "site:tab", "its data requests say which tab asks");
  assert.equal(site.fetched[1].headers["x-tonk-site"], undefined);
  assert.deepEqual(told(site, "fetch"), [], "nothing is fetched through the page around");
});

test("a sealed frame has no worker, and fetches through the page around", async () => {
  const sealed = frame({ origin: "null" });
  await sealed.ready();

  sealed.window.fetch("/api/identify");
  await settle();

  assert.deepEqual(sealed.fetched, [], "it makes no request of its own");
  const [relayed] = told(sealed, "fetch");
  assert.equal(relayed.path, "/api/identify");
  assert.equal(relayed.method, "GET");
});

test("a sealed frame claims nothing", async () => {
  const sealed = frame({ origin: "null" });
  await sealed.ready();
  await sealed.tonk.claimed;

  assert.deepEqual(sealed.fetched, []);
});

test("moving to an address tells the page around, and leaves the frame where it is", async () => {
  const site = frame({ address: "/notes" });
  await site.ready();

  site.history.pushState({}, "", "/other?x=1#y");
  site.history.replaceState({}, "", "sibling");
  await settle();

  assert.deepEqual(told(site, "navigate"), [
    { v: 1, type: "navigate", href: "/other?x=1#y", replace: false },
    { v: 1, type: "navigate", href: "/sibling", replace: true },
  ]);
  assert.deepEqual(site.natives, [], "the page's address is the page's to change");
  assert.equal(site.at.pathname, "/notes");
});

test("a state change with no address is the frame's own", async () => {
  const site = frame();
  await site.ready();

  site.history.pushState({ a: 1 }, "");
  site.history.replaceState({ a: 2 }, "", null);
  await settle();

  assert.deepEqual(told(site, "navigate"), []);
  assert.deepEqual(plain(site.natives), [["pushState", { a: 1 }, ""], ["replaceState", null]]);
});

test("moving through history is asked of the page around", async () => {
  const site = frame();
  await site.ready();

  site.history.back();
  site.history.forward();
  site.history.go(-2);
  await settle();

  assert.deepEqual(told(site, "navigate"), [
    { v: 1, type: "navigate", delta: -1 },
    { v: 1, type: "navigate", delta: 1 },
    { v: 1, type: "navigate", delta: -2 },
  ]);
  assert.deepEqual(site.natives, []);
});

test("`history.go(0)` reloads the frame itself", async () => {
  const site = frame();
  await site.ready();

  site.history.go(0);
  await settle();

  assert.deepEqual(site.natives, [["reload"]]);
  assert.deepEqual(told(site, "navigate"), []);
});

test("when the page moves a site, its frame takes the address in place and claims it", async () => {
  const site = frame({ address: "/notes" });
  await site.ready();
  site.fetched.length = 0;

  await site.hear({ type: "context", context: { ...site.context0, sitePath: "inspector/x" } });

  assert.deepEqual(site.natives, [["replaceState", "/inspector/x"]]);
  assert.equal(site.at.pathname, "/inspector/x");
  assert.deepEqual(told(site, "navigate"), [], "following the page is not a move of its own");
  const { parameters } = JSON.parse(site.fetched[0].body).claims[0].application;
  assert.equal(parameters.path, "/inspector/x");
  assert.ok(site.events.includes("tonk:context"));
});

test("a site moved to where it already is changes nothing", async () => {
  const site = frame({ address: "/notes" });
  await site.ready();

  await site.hear({ type: "context", context: { ...site.context0, sitePath: "notes" } });

  assert.deepEqual(site.natives, []);
});

test("a site with no path is at its root", async () => {
  const site = frame({ address: "/notes" });
  await site.ready();

  await site.hear({ type: "context", context: { ...site.context0, sitePath: undefined } });

  assert.equal(site.at.pathname, "/");
});

test("a sealed frame does not take addresses", async () => {
  const sealed = frame({ origin: "null" });
  await sealed.ready();

  await sealed.hear({ type: "context", context: { ...sealed.context0, sitePath: "notes" } });

  assert.deepEqual(sealed.natives, []);
});

test("the data API is not on the frame's bridge", async () => {
  const site = frame();
  await site.ready();

  for (const gone of ["query", "transact", "evaluate", "subscribe"]) {
    assert.equal(site.tonk[gone], undefined, `${gone} is a request to the worker, not a message`);
  }
  for (const kept of ["navigate", "setTitle", "delegate", "fetch"]) {
    assert.equal(typeof site.tonk[kept], "function", `${kept} is asked of the page around`);
  }
});

test("an account task asked before anything listens stays open for what listens later", async () => {
  const profile = frame({ origin: "https://profile.tonk.test" });
  await profile.ready({ repo: "" });
  const ask = (request) => profile.tonk.task(JSON.stringify({ purpose: "account", ...request }), () => {});

  ask({ action: "open", requestId: "task-1", presentation: { anchor: { left: 1 } } });
  ask({ action: "reseat", requestId: "task-1", presentation: { anchor: { left: 2 } } });

  assert.deepEqual(plain(profile.window.tonkAccountTasksOpen()), [
    { action: "open", requestId: "task-1", account: null, presentation: { anchor: { left: 2 } } },
  ], "the task is handed over as it now stands");

  profile.window.tonkAccountTaskDone("task-1", "completed");
  assert.deepEqual(plain(profile.window.tonkAccountTasksOpen()), [], "a task that ended is not handed over");
});

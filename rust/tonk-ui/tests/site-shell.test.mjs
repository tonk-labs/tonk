import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const shell = (name) =>
  readFileSync(join(HERE, "..", "assets", `${name}.html`), "utf8")
    .split("<script>")[1]
    .split("</script>")[0];
const SHELLS = { space: shell("space"), profile: shell("profile") };

const MANIFEST = { bootstrap: "bootstrap-1.js", runtime: "runtime-1.js", preview: "preview-1.js" };
const turn = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 20; i++) await turn(); };

// Run a site's shell in stand-ins for the frame it loads in.
//
// - `address`: where the frame is, path first (`/notes?x#y`).
// - `controlled`: whether a worker controls the frame already.
// - `served`: whether that worker served this document.
// - `framed`: `"site"` (by the page of another origin), `"content"` (by a
//   page of this origin) or `"alone"` (not framed).
// - `content`: the addresses a request finds content at.
// - `store`: the frame's session storage, which outlives a load.
function boot(
  name,
  { address = "/", controlled = true, served = true, framed = "site", content = [], store = new Map() } = {},
) {
  const host = name === "profile" ? "profile.tonk.test" : "bspace.tonk.test";
  const origin = `https://${host}`;
  const at = new URL(address, origin);
  const said = [];      // stages told to the page around
  const toWorker = [];  // messages posted to the worker
  const ran = [];       // scripts run, in order
  const requests = [];  // requests made
  const went = [];      // navigations: ["replace", url] | ["reload"]
  const failed = [];    // errors reported to the page around
  const listeners = {};
  const workerListeners = {};

  class Channel {
    constructor() {
      const port = () => ({ onmessage: null, postMessage(data) { queueMicrotask(() => this.other.onmessage?.({ data })); } });
      this.port1 = port(); this.port2 = port();
      this.port1.other = this.port2; this.port2.other = this.port1;
    }
  }
  const worker = {
    postMessage(message, transfer) {
      toWorker.push(message);
      if (message.type === "protocol") transfer[0].postMessage({ protocol: 2 });
      if (message.type === "content") transfer[0].postMessage({ admitted: true });
    },
  };
  const container = {
    controller: controlled ? worker : null,
    registered: [],
    register: async (...args) => void container.registered.push(args),
    addEventListener: (type, listener) => void (workerListeners[type] = listener),
    startMessages() {},
    getRegistration: async () => null,
  };
  const location = {
    get href() { return at.href; },
    get pathname() { return at.pathname; },
    get search() { return at.search; },
    get hash() { return at.hash; },
    get hostname() { return at.hostname; },
    get origin() { return origin; },
    replace: (url) => void went.push(["replace", String(url)]),
    reload: () => void went.push(["reload"]),
  };
  const element = (tag) => ({
    tag, style: {}, children: [],
    setAttribute(key, value) { this[key] = value; },
    remove() { body.children = body.children.filter((child) => child !== this); },
  });
  const body = {
    children: [], innerHTML: "", textContent: "",
    appendChild(child) { this.children.push(child); },
    replaceChildren(...children) { this.children = children; this.innerHTML = ""; },
  };
  const window = {
    addEventListener: (type, listener) => void (listeners[type] = listener),
    removeEventListener() {},
    postMessage() {},
  };
  const document = {
    body,
    head: {
      appendChild(child) {
        if (child.tag !== "script") return;
        ran.push(child.src);
        // What each script leaves behind for the shell to wait on.
        if (child.src.endsWith(MANIFEST.bootstrap)) {
          window.tonk = { ready: Promise.resolve(), claimed: Promise.resolve(), context: { siteEntity: "site:abc" } };
        }
        if (child.src.endsWith(MANIFEST.runtime)) window.tonkRuntime = Promise.resolve();
        queueMicrotask(() => child.onload());
      },
    },
    createElement: element,
    getElementById: (id) => body.children.find((child) => child.id === id) ?? null,
    addEventListener: (type, listener) => void (listeners[`document:${type}`] = listener),
    visibilityState: "visible",
  };
  const outside = {
    postMessage: (message) => {
      if (message.__tonkOrigin === "status") said.push(message.stage);
      if (message.__tonkRuntime === "error") failed.push(message.error);
    },
    get location() {
      if (framed === "content") return { origin };
      throw new Error("cross-origin");
    },
  };
  const context = vm.createContext({
    window, document, location,
    parent: framed === "alone" ? window : outside,
    navigator: { serviceWorker: container },
    history: { replaceState: (_state, _unused, url) => { at.pathname = url; } },
    performance: { getEntriesByType: () => [{ workerStart: served ? 1 : 0 }] },
    sessionStorage: {
      getItem: (key) => store.get(key) ?? null,
      setItem: (key, value) => void store.set(key, value),
      removeItem: (key) => void store.delete(key),
    },
    fetch: async (url, init = {}) => {
      requests.push([init.method ?? "GET", url]);
      if (url === "/guest/manifest.json") return new Response(JSON.stringify(MANIFEST));
      return new Response(null, { status: content.includes(url) ? 200 : 404 });
    },
    MessageChannel: Channel,
    setTimeout, clearTimeout, setInterval: () => 0, Promise, Number, Date, String,
    decodeURIComponent, Error,
  });
  vm.runInContext(SHELLS[name], context);
  // The worker speaking to its pages, and the page moving the site.
  const workerSays = (data) => workerListeners.message({ data });
  const moved = async (address) => {
    const next = new URL(address, origin);
    at.pathname = next.pathname; at.search = next.search;
    listeners["tonk:context"]?.();
    await settle();
  };
  return { said, failed, toWorker, ran, requests, went, body, container, at, workerSays, moved, workerListeners };
}

for (const name of ["space", "profile"]) {
  test(`${name}: the shell brings the site up from its own origin and shows the route`, async () => {
    const site = boot(name, { address: "/notes" });
    await settle();

    assert.deepEqual(site.ran, ["/guest/bootstrap-1.js", "/guest/runtime-1.js", "/guest/preview-1.js"]);
    assert.match(site.body.innerHTML, /<tonk-display entity='site:abc' model='tonk:site'>/);
    assert.match(site.body.innerHTML, /slot='loading'/);
    assert.deepEqual(site.said, ["loading", "ready"]);
    assert.deepEqual(site.went, [], "a shell its worker served stays where it is");
  });

  test(`${name}: a site whose worker is replaced loads again under the new one`, async () => {
    const site = boot(name);
    await settle();
    assert.deepEqual(site.went, []);

    // The browser says the controller changed with the same worker in
    // control: nothing replaced it.
    site.workerListeners.controllerchange();
    assert.deepEqual(site.went, [], "the worker it loaded under is still the one in control");

    site.container.controller = { postMessage() {} };
    site.workerListeners.controllerchange();
    assert.deepEqual(site.went, [["reload"]]);
    assert.equal(site.said.at(-1), "taking up a new version", "and it says why it is loading");

    // The same replacement heard twice replaces the document once.
    site.workerListeners.controllerchange();
    assert.deepEqual(site.went, [["reload"]]);
  });

  test(`${name}: it asks, as a request, whether its address is content`, async () => {
    const site = boot(name, { address: "/hello.html?x=1", content: ["/hello.html?x=1"] });
    await settle();

    assert.ok(site.requests.some(([method, url]) => method === "HEAD" && url === "/hello.html?x=1"));
    const [frame] = site.body.children.filter((child) => child.tag === "iframe");
    assert.equal(frame?.src, "/hello.html?x=1", "content is shown in a frame of its own");
    assert.equal(site.body.innerHTML, "", "and no model is displayed");
    assert.equal(site.said.at(-1), "ready");
  });

  test(`${name}: the root is a route, never asked for as content`, async () => {
    const site = boot(name, { address: "/" });
    await settle();

    assert.deepEqual(site.requests.filter(([method]) => method === "HEAD"), []);
    assert.match(site.body.innerHTML, /tonk-display/);
  });

  test(`${name}: it shows the other kind when the page moves it`, async () => {
    const site = boot(name, { address: "/notes", content: ["/hello.html"] });
    await settle();

    await site.moved("/hello.html");
    assert.equal(site.body.children.find((child) => child.tag === "iframe")?.src, "/hello.html");

    await site.moved("/notes");
    assert.match(site.body.innerHTML, /tonk-display/);
  });

  test(`${name}: with no worker it installs one and loads again under it`, async () => {
    const site = boot(name, { controlled: false, served: false });
    await settle();
    assert.equal(site.container.registered.length, 1);
    assert.equal(site.container.registered[0][0], "/space_worker.js");
    assert.deepEqual(site.said, ["installing its worker", "waiting for its worker"]);
    assert.deepEqual(site.ran, [], "nothing of the site runs before its worker serves the shell");

    // The worker takes control.
    site.workerListeners.controllerchange();
    await settle();

    assert.deepEqual(site.went, [["reload"]]);
    assert.deepEqual(site.ran, []);
  });

  test(`${name}: sent here to start a worker, it returns to the address asked for`, async () => {
    const site = boot(name, {
      address: `/${name}.html#boot=${encodeURIComponent("/notes?x=1#frag")}`,
      served: false,
    });
    await settle();

    assert.deepEqual(site.went, [["replace", "/notes?x=1#frag"]]);
    assert.deepEqual(site.ran, []);
  });

  test(`${name}: its own path is not a place in the site`, async () => {
    const site = boot(name, { address: `/${name}.html` });
    await settle();

    assert.equal(site.at.pathname, "/");
  });

  test(`${name}: what the worker is busy with is the stage until it is done`, async () => {
    const site = boot(name);
    await settle();
    assert.equal(site.said.at(-1), "ready");

    site.workerSays({ type: "status", stage: "replicating the space" });
    assert.equal(site.said.at(-1), "replicating the space");
    assert.equal(
      site.body.children.find((child) => child.id === "tonk-stage")?.textContent,
      "replicating the space…",
      "and the frame shows it",
    );

    site.workerSays({ type: "status", stage: "ready" });
    assert.equal(site.said.at(-1), "ready");
    assert.equal(site.body.children.find((child) => child.id === "tonk-stage"), undefined);
  });

  test(`${name}: inside a page of its own origin it becomes the content at its address`, async () => {
    const site = boot(name, { address: "/hello.html?x=1", framed: "content" });
    await settle();

    assert.equal(
      JSON.stringify(site.toWorker.filter((message) => message.type === "content")),
      JSON.stringify([{ type: "content", url: `${site.at.origin}/hello.html?x=1` }]),
    );
    assert.deepEqual(site.went, [["replace", `${site.at.origin}/hello.html?x=1`]]);
    assert.deepEqual(site.ran, [], "it is not the site's frame, and brings nothing up");
    assert.deepEqual(site.said, []);
  });

  test(`${name}: on its own, it says what the address is and starts nothing`, async () => {
    const site = boot(name, { framed: "alone" });
    await settle();

    assert.deepEqual(site.ran, []);
    assert.equal(site.container.registered.length, 0);
  });
}

test("a shell that lands in the same frame twice in a moment stops asking", async () => {
  // One frame, loaded twice: the first load asks for the content and loads
  // again, and the worker answers that load with the shell once more. A
  // shell that kept asking would load without end.
  const store = new Map();
  const first = boot("space", { address: "/hello.html", framed: "content", store });
  await settle();
  const second = boot("space", { address: "/hello.html", framed: "content", store });
  await settle();

  assert.equal(first.went.length, 1);
  assert.deepEqual(second.went, [], "the second load is left where it is");
  assert.match(second.failed[0] ?? "", /did not load/);
});

// The app's page is what the server answers any address with, on every
// hostname. Its first script decides whether it is on a site's origin.
const APP_PAGE = readFileSync(join(HERE, "..", "index.html"), "utf8")
  .split("<script>")[1]
  .split("</script>")[0];

function appPageOn(hostname, address) {
  const at = new URL(address, `https://${hostname}`);
  const went = [];
  let stopped = false;
  const global = {};
  vm.runInContext(
    APP_PAGE,
    vm.createContext({
      globalThis: global,
      window: { stop: () => void (stopped = true) },
      location: {
        hostname: at.hostname, pathname: at.pathname, search: at.search, hash: at.hash,
        replace: (url) => void went.push(url),
      },
      encodeURIComponent,
    }),
  );
  return { went, stopped, site: global.tonkSiteOrigin === true };
}

test("the app's page, served at a site's address, sends the frame to the shell with that address", () => {
  const space = appPageOn(`b${"a".repeat(52)}.tonk.test`, "/notes?x=1#frag");
  const profile = appPageOn("profile.tonk.test", "/space/did:key:zSpace");

  assert.deepEqual(space.went, [`/space.html#boot=${encodeURIComponent("/notes?x=1#frag")}`]);
  assert.deepEqual(profile.went, [`/profile.html#boot=${encodeURIComponent("/space/did:key:zSpace")}`]);
  assert.ok(space.stopped && profile.stopped, "nothing else of the app's page runs there");
  assert.ok(space.site && profile.site);
});

test("the app's page stays put on the app's own origin", () => {
  const app = appPageOn("tonk.test", "/space/did:key:zSpace");

  assert.deepEqual(app.went, []);
  assert.equal(app.stopped, false);
  assert.equal(app.site, false);
});

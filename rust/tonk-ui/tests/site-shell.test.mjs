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
// - `answers`: what a request for an address is answered with, as JSON.
// - `marked`: whether the document carries the worker's mark, for a browser
//   that does not say a worker served it.
// - `active`: whether the origin's worker is running already, whoever it
//   controls.
// - `pin`: the release the space pins, as its worker wrote it down.
// - `persona`: the persona the profile pins, as its shell wrote it down.
// - `runs`: the release the worker in control was registered for.
function boot(
  name,
  {
    address = "/", controlled = true, served = true, framed = "site", content = [], store = new Map(),
    answers = {}, marked = false, active = false, pin = null, runs = null, persona = null,
  } = {},
) {
  const host = name === "profile" ? "profile.tonk.test" : "bspace.tonk.test";
  const origin = `https://${host}`;
  const at = new URL(address, origin);
  const said = [];      // stages told to the page around
  const asked = [];     // ports asked of the page around
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
  // What became of the worker's registration.
  const registration = { unregistered: false, unregister: async () => void (registration.unregistered = true) };
  // What the shell keeps of what it boots: the persona a profile pins.
  const booted = { record: persona ? { v: 2, kernel: persona } : null };
  const worker = {
    scriptURL: `${origin}/boot.js${runs ? `?pin=${encodeURIComponent(runs)}` : ""}`,
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
    removeEventListener: (type, listener) => {
      if (workerListeners[type] === listener) delete workerListeners[type];
    },
    startMessages() {},
    getRegistration: async () => (controlled ? registration : null),
    ready: active ? Promise.resolve({ active: worker }) : new Promise(() => {}),
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
    // The mark the worker puts in the shell it serves.
    querySelector: (selector) => (marked && selector.includes("tonk-shell") ? {} : null),
    visibilityState: "visible",
  };
  const outside = {
    postMessage: (message) => {
      if (message.__tonkOrigin === "status") said.push(message.stage);
      if (message.__tonkOrigin === "need-port") asked.push(message);
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
      if (url in answers) return new Response(JSON.stringify(answers[url]));
      return new Response(null, { status: content.includes(url) ? 200 : 404 });
    },
    caches: {
      open: async () => ({
        match: async () =>
          booted.record ? new Response(JSON.stringify(booted.record))
          : pin ? new Response(JSON.stringify({ v: 1, pin }))
          : undefined,
        put: async (_key, response) => void (booted.record = await response.json()),
      }),
    },
    Response,
    URL, encodeURIComponent,
    MessageChannel: Channel,
    setTimeout, clearTimeout, setInterval: () => 0, Promise, Number, Date, String,
    decodeURIComponent, Error,
  });
  vm.runInContext(SHELLS[name], context);
  // The worker speaking to its pages, and the page moving the site.
  const workerSays = (data) => workerListeners.message({ data });
  // A worker registered for `script` takes control of the frame, and the
  // browser says so to whoever listens.
  const takenBy = (script) => {
    container.controller = { ...worker, scriptURL: new URL(script, origin).href };
    workerListeners.controllerchange?.();
  };
  const moved = async (address) => {
    const next = new URL(address, origin);
    at.pathname = next.pathname; at.search = next.search;
    listeners["tonk:context"]?.();
    await settle();
  };
  return {
    said, asked, failed, toWorker, ran, requests, went, body, container, at, workerSays, moved, workerListeners,
    registration, booted, takenBy,
  };
}

// What a deployment ships: a profile with no persona of its own runs the one
// named here, and a space reads no kernel in it.
const SHIPS = { "/_host/kernel.json": { persona: "bafkr4ipersona" } };
const SCRIPTS = { space: "/boot.js", profile: "/kernel_boot.js?kernel=bafkr4ipersona" };
const STARTING = { space: ["installing its worker"], profile: ["installing its worker", "waiting for its worker"] };

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

  test(`${name}: a worker that takes over takes the page as it is, and nothing loads again`, async () => {
    const site = boot(name);
    await settle();
    const ran = [...site.ran];
    const asked = site.asked.length;

    // A new build of the same worker takes control.
    site.takenBy("/boot.js");
    await settle();
    assert.deepEqual(site.went, [], "the page stays as it is");
    assert.deepEqual(site.ran, ran, "and nothing of the site runs again");
    assert.match(site.body.innerHTML, /tonk-display/);
    assert.equal(site.said.at(-1), "ready");

    // The new worker asks for a port of its own, and the page is asked for one.
    site.workerSays({ type: "need-port" });
    assert.equal(site.asked.length, asked + 1);
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

  test(`${name}: with no worker it installs one and goes on once it is in control`, async () => {
    const site = boot(name, { controlled: false, served: false, answers: SHIPS });
    await settle();
    const [script, options] = site.container.registered[0] ?? [];
    assert.equal(site.container.registered.length, 1);
    assert.equal(script, SCRIPTS[name]);
    assert.equal(options.type, undefined, "the bootloader is a classic script");
    assert.deepEqual(site.said, STARTING[name]);
    assert.deepEqual(site.went, [], "it waits for its worker");

    // The browser says the controller changed, with none in control yet.
    site.workerListeners.controllerchange();
    await settle();
    assert.deepEqual(site.went, [], "it waits on until its worker is in control");

    site.takenBy(script);
    await settle();
    // The server, not the worker, served this document: it loads once more,
    // for the worker to serve it.
    assert.deepEqual(site.went, [["reload"]]);
    assert.deepEqual(site.ran, [], "nothing of the site runs before its worker serves the shell");
  });

  test(`${name}: left out by a worker that is already running, it asks to be taken and goes on once it is`, async () => {
    const site = boot(name, { controlled: false, served: false, active: true, answers: SHIPS });
    await settle();
    assert.deepEqual(site.toWorker.map((message) => message.type), ["claim"]);
    assert.deepEqual(site.went, [], "it waits for the worker to take it");

    site.takenBy(SCRIPTS[name]);
    await settle();
    assert.deepEqual(site.went, [["reload"]], "and loads once more, for the worker to serve it");
  });

  test(`${name}: it asks nothing of a worker that is not running yet`, async () => {
    const site = boot(name, { controlled: false, served: false });
    await settle();
    assert.deepEqual(site.toWorker, []);
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

for (const name of ["space", "profile"]) {
  test(`${name}: a shell the worker marked renders where the browser does not say a worker served it`, async () => {
    const site = boot(name, { served: false, marked: true, address: "/notes" });
    await settle();

    assert.deepEqual(site.went, [], "it does not load again");
    assert.match(site.body.innerHTML, /<tonk-display entity='site:abc' model='tonk:site'>/);
  });
}

const SITES = { "/.well-known/tonk": { sites: { app: "https://tonk.test", host: "tonk.test" } } };

test("a profile's address opened on its own goes to the same address of the app", async () => {
  const site = boot("profile", { framed: "alone", address: "/settings?tab=account#keys", answers: SITES });
  await settle();

  assert.deepEqual(site.went, [["replace", "https://tonk.test/settings?tab=account#keys"]]);
});

test("a space's address opened on its own goes to that space in the app, once its worker holds it", async () => {
  const held = { ...SITES, "/api/health": { site: "space", space: "did:key:zSpace" } };
  const site = boot("space", { framed: "alone", address: "/notes?x=1", answers: held });
  await settle();

  assert.deepEqual(site.went, [["replace", "https://tonk.test/space/did:key:zSpace/notes?x=1"]]);

  const root = boot("space", { framed: "alone", answers: held });
  await settle();
  assert.deepEqual(root.went, [["replace", "https://tonk.test/space/did:key:zSpace"]]);
});

test("a space's address whose worker holds no space leaves a way to the app", async () => {
  const site = boot("space", { framed: "alone", answers: { ...SITES, "/api/health": { site: "space", space: null } } });
  await settle();

  assert.deepEqual(site.went, []);
  assert.match(site.body.textContent, /serves a Tonk site/);
  assert.equal(site.body.children[0]?.href, "https://tonk.test");
});

for (const name of ["space", "profile"]) {
  test(`${name}: where no app is named, it says what the address is and goes nowhere`, async () => {
    const site = boot(name, { framed: "alone" });
    await settle();

    assert.deepEqual(site.went, []);
    assert.match(site.body.textContent, /serves a Tonk site/);
    assert.equal(site.body.children[0]?.href, undefined);
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

const PIN = "https://tonk.network/releases/bafkreib7f5smq4ggn5xwqas57gchvhvfry6ym7jbw5h3w4zg7wtwk33l2i/worker.js";
const OTHER = "https://tonk.network/releases/bafkreiebnz2m3oin767zzlo5idwbljwfe6oakn7nihb2umtqj3c6q4bncm/worker.js";

test("a space registers the bootloader for the release it pins", async () => {
  const site = boot("space", { controlled: false, served: false, pin: PIN });
  await settle();
  assert.equal(site.container.registered[0][0], `/boot.js?pin=${encodeURIComponent(PIN)}`);
});

test("a space under the worker of the release it pins is brought up", async () => {
  const site = boot("space", { pin: PIN, runs: PIN });
  await settle();
  assert.equal(site.registration.unregistered, false);
  assert.deepEqual(site.went, []);
  assert.match(site.body.innerHTML, /tonk-display/);
});

for (const [what, options, script] of [
  ["a space that pinned a release since", { pin: PIN, runs: null }, `/boot.js?pin=${encodeURIComponent(PIN)}`],
  ["a space that pins another release now", { pin: OTHER, runs: PIN }, `/boot.js?pin=${encodeURIComponent(OTHER)}`],
  ["a space that pins none any more", { pin: null, runs: PIN }, "/boot.js"],
]) {
  test(`${what} registers the worker of its release in place and is brought up under it`, async () => {
    const site = boot("space", options);
    await settle();
    assert.deepEqual(site.container.registered.map(([registered]) => registered), [script]);
    assert.equal(site.registration.unregistered, false, "the worker it has is not withdrawn");
    assert.equal(site.said.at(-1), "switching its worker");
    assert.deepEqual(site.ran, [], "nothing of the site runs under the other release");

    // The browser says the controller changed, with the other release still
    // in control.
    site.workerListeners.controllerchange();
    await settle();
    assert.deepEqual(site.ran, []);

    site.takenBy(script);
    await settle();
    assert.deepEqual(site.went, [], "nothing loads again");
    assert.deepEqual(site.ran, ["/guest/bootstrap-1.js", "/guest/runtime-1.js", "/guest/preview-1.js"]);
    assert.match(site.body.innerHTML, /tonk-display/);
  });
}

test("a profile that pins no persona runs the one its deployment ships, and pins it", async () => {
  const site = boot("profile", {
    controlled: false,
    served: false,
    answers: { "/_host/kernel.json": { kernel: "bafkr4ikernel", persona: "bafkr4ipersona" } },
  });
  await settle();
  assert.equal(site.container.registered[0][0], "/kernel_boot.js?kernel=bafkr4ipersona");
  assert.deepEqual(site.booted.record, { v: 2, kernel: "bafkr4ipersona" });
});

test("a profile runs the persona it pins, whatever its deployment ships", async () => {
  const site = boot("profile", {
    controlled: false,
    served: false,
    persona: "bafkr4ipinned",
    answers: { "/_host/kernel.json": { kernel: "bafkr4ikernel", persona: "bafkr4inewer" } },
  });
  await settle();
  assert.equal(site.container.registered[0][0], "/kernel_boot.js?kernel=bafkr4ipinned");
});

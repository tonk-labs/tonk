import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
// The one shell every site's origin serves, the profile's as a space's.
const SHELL = readFileSync(join(HERE, "..", "assets", "site.html"), "utf8")
  .split("<script>")[1]
  .split("</script>")[0];

const MANIFEST = { bootstrap: "bootstrap-1.js", runtime: "runtime-1.js", preview: "preview-1.js" };
const turn = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => { for (let i = 0; i < 20; i++) await turn(); };

// Run a site's shell in stand-ins for the frame it loads in. `name` says
// which site the frame is, `"space"` or `"profile"`, which only picks the
// defaults below: the shell is the same.
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
// - `record`: what the site's boot record holds, as its worker or shell
//   wrote it down: by default, the kernel the site pins.
// - `runs`: the kernel the worker in control was registered for: by default,
//   the one the record pins.
// - `named`: the kernel the page around names for a site that pins none, or
//   `null` where it names none; `hosted`, whether it says it hands the site
//   a port up to a host, as it does a space.
// - `host`: the hostname the shell is served on: by default, the site's own.
// - `unstarted`: how many times the worker answers that its kernel did not
//   boot before it says it did.
// - `wait`: what the shell waits a while with, in place of the clock's.
function boot(
  name,
  {
    address = "/", controlled = true, served = true, framed = "site", content = [], store = new Map(),
    answers = {}, marked = false, active = false,
    record = { v: 2, kernel: name === "space" ? "bafkr4ispace" : "bafkr4ipersona" }, runs = record?.kernel ?? null,
    named = name === "space" ? "bafkr4ikernel" : "bafkr4ipersona", hosted = name === "space",
    host = name === "profile" ? "profile.tonk.test" : "bspace.tonk.test",
    unstarted = 0, wait = setTimeout,
  } = {},
) {
  const origin = `https://${host}`;
  const at = new URL(address, origin);
  const said = [];      // stages told to the page around
  const asked = [];     // ports asked of the page around
  const kernelAsked = []; // kernels asked of the page around
  const up = [];        // what else was told to the page around
  const toWorker = [];  // messages posted to the worker
  const ran = [];       // scripts run, in order
  const requests = [];  // requests made
  const went = [];      // navigations: ["replace", url] | ["reload"]
  const failed = [];    // errors reported to the page around
  // Each type's listeners, in the order they were added, those that capture
  // first.
  const listeners = {};
  const listen = (type, listener, capture) => {
    const all = (listeners[type] ??= []);
    if (capture) all.unshift(listener);
    else all.push(listener);
  };
  const dispatch = (type, event) => {
    let stopped = false;
    const heard = { ...event, stopImmediatePropagation: () => void (stopped = true) };
    for (const listener of [...(listeners[type] ?? [])]) {
      listener(heard);
      if (stopped) break;
    }
  };
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
  // What the shell keeps of what it boots: the kernel the site pins.
  const booted = { record };
  const worker = {
    scriptURL: `${origin}/kernel_boot.js${runs ? `?kernel=${encodeURIComponent(runs)}` : ""}`,
    postMessage(message, transfer) {
      toWorker.push(message);
      if (message.type === "protocol") transfer[0].postMessage({ protocol: 2 });
      if (message.type === "content") transfer[0].postMessage({ admitted: true });
      if (message.type === "booted") {
        transfer[0].postMessage(unstarted > 0 ? { error: "the person has no such space" } : { booted: true });
        unstarted -= 1;
      }
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
    addEventListener: (type, listener, capture) => listen(type, listener, capture === true),
    removeEventListener: (type, listener) => {
      listeners[type] = (listeners[type] ?? []).filter((heard) => heard !== listener);
    },
    postMessage() {},
    // Its own location, which a page opened on its own reads as its
    // parent's.
    location,
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
    addEventListener: (type, listener) => listen(`document:${type}`, listener, false),
    // The mark the worker puts in the shell it serves.
    querySelector: (selector) => (marked && selector.includes("tonk-shell") ? {} : null),
    visibilityState: "visible",
  };
  // The page around the frame tells it `data`, with `ports`.
  const fromAbove = (data, ports = []) => dispatch("message", { source: outside, data, ports });
  const outside = {
    postMessage: (message) => {
      if (message.__tonkOrigin === "status") said.push(message.stage);
      else if (message.__tonkOrigin === "need-port") asked.push(message);
      else if (message.__tonkRuntime === "error") failed.push(message.error);
      else if (message.__tonkOrigin === "need-kernel") {
        kernelAsked.push(message);
        if (named) queueMicrotask(() => fromAbove({ __tonkOrigin: "kernel", kernel: named, host: hosted }));
      } else up.push(message);
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
        match: async () => (booted.record ? new Response(JSON.stringify(booted.record)) : undefined),
        put: async (_key, response) => void (booted.record = await response.json()),
      }),
    },
    Response,
    URL, encodeURIComponent,
    MessageChannel: Channel,
    setTimeout: wait, clearTimeout, setInterval: () => 0, Promise, Number, Date, String,
    decodeURIComponent, Error,
  });
  vm.runInContext(SHELL, context);
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
    dispatch("tonk:context", {});
    await settle();
  };
  return {
    said, asked, kernelAsked, up, failed, toWorker, ran, requests, went, body, container, at, workerSays, moved,
    workerListeners, registration, booted, takenBy, fromAbove,
  };
}

const SCRIPTS = { space: "/kernel_boot.js?kernel=bafkr4ispace", profile: "/kernel_boot.js?kernel=bafkr4ipersona" };
const STARTING = ["installing its worker"];
// Where the shell the server served goes once its worker is in control: its
// kernel starts, and the frame goes to the address the kernel answers.
const UNDER_WORKER = [["replace", "/"]];

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
    site.takenBy(SCRIPTS[name]);
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
    const site = boot(name, { controlled: false, served: false });
    await settle();
    const [script, options] = site.container.registered[0] ?? [];
    assert.equal(site.container.registered.length, 1);
    assert.equal(script, SCRIPTS[name]);
    assert.equal(options.type, undefined, "the bootloader is a classic script");
    assert.deepEqual(site.said, STARTING);
    assert.deepEqual(site.went, [], "it waits for its worker");

    // The browser says the controller changed, with none in control yet.
    site.workerListeners.controllerchange();
    await settle();
    assert.deepEqual(site.went, [], "it waits on until its worker is in control");

    site.takenBy(script);
    await settle();
    // The server, not the worker, served this document: the worker serves
    // what comes after it.
    assert.deepEqual(site.went, UNDER_WORKER);
    assert.deepEqual(site.ran, [], "nothing of the site runs before its worker serves the shell");
  });

  test(`${name}: left out by a worker that is already running, it asks to be taken and goes on once it is`, async () => {
    const site = boot(name, { controlled: false, served: false, active: true });
    await settle();
    assert.deepEqual(site.toWorker.map((message) => message.type), ["claim"]);
    assert.deepEqual(site.went, [], "it waits for the worker to take it");

    site.takenBy(SCRIPTS[name]);
    await settle();
    assert.deepEqual(site.went, UNDER_WORKER, "and goes on under the worker");
  });

  test(`${name}: it asks nothing of a worker that is not running yet`, async () => {
    const site = boot(name, { controlled: false, served: false });
    await settle();
    assert.deepEqual(site.toWorker, []);
  });

  test(`${name}: sent here to start a worker, it returns to the address asked for`, async () => {
    const site = boot(name, {
      address: `/site.html#boot=${encodeURIComponent("/notes?x=1#frag")}`,
      served: false,
    });
    await settle();

    assert.deepEqual(site.went, [["replace", "/notes?x=1#frag"]]);
    assert.deepEqual(site.ran, []);
  });

  test(`${name}: its own path is not a place in the site`, async () => {
    const site = boot(name, { address: "/site.html" });
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

test("the profile's shell opened on its own goes to the address it was starting", async () => {
  const site = boot("profile", { framed: "alone", address: "/site.html#boot=%2Fsettings", answers: SITES });
  await settle();

  assert.deepEqual(site.went, [["replace", "https://tonk.test/settings"]]);
});

test("the profile's shell on the app's own origin stays, saying what the address is", async () => {
  const site = boot("profile", { framed: "alone", address: "/site.html", answers: SITES, host: "tonk.test" });
  await settle();

  assert.deepEqual(site.went, [], "it does not send the app to itself");
  assert.match(site.body.textContent, /serves a Tonk site/);
});

// A space's label is its key in multibase base32; its DID names the key in
// base58.
const SPACE = "did:key:z6Mki8Mf2Trp2qmXqNoSihfVi9sEg8Z4aSCSnyUfadj4jB1E";
const LABEL = "b5uatnfyoqb7p3aih64fed7c3alnu24uujg63qqs6qmnk5olzgykjr5i";

test("a space's address opened on its own goes to that space in the app, at its DID", async () => {
  const site = boot("space", { framed: "alone", address: "/notes?x=1#top", answers: SITES, host: `${LABEL}.tonk.test` });
  await settle();

  assert.deepEqual(site.went, [["replace", `https://tonk.test/${SPACE}/notes?x=1#top`]]);

  const root = boot("space", { framed: "alone", answers: SITES, host: `${LABEL}-pr33.tonk.test` });
  await settle();
  assert.deepEqual(root.went, [["replace", `https://tonk.test/${SPACE}`]], "a deployment's suffix is not the space's");
});

test("a space's shell opened on its own goes to the address it was starting", async () => {
  const site = boot("space", { framed: "alone", address: "/site.html#boot=%2Fnotes%3Fx%3D1", answers: SITES, host: `${LABEL}.tonk.test` });
  await settle();

  assert.deepEqual(site.went, [["replace", `https://tonk.test/${SPACE}/notes?x=1`]]);
});

test("any site no space's key names is shown at the app's own addresses, as the profile is", async () => {
  const site = boot("space", { framed: "alone", address: "/notes", answers: SITES, host: "nothing.tonk.test" });
  await settle();

  assert.deepEqual(site.went, [["replace", "https://tonk.test/notes"]]);
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

  assert.deepEqual(space.went, [`/site.html#boot=${encodeURIComponent("/notes?x=1#frag")}`]);
  assert.deepEqual(profile.went, [`/site.html#boot=${encodeURIComponent("/space/did:key:zSpace")}`]);
  assert.ok(space.stopped && profile.stopped, "nothing else of the app's page runs there");
  assert.ok(space.site && profile.site);
});

test("the app's page stays put on the app's own origin", () => {
  const app = appPageOn("tonk.test", "/space/did:key:zSpace");

  assert.deepEqual(app.went, []);
  assert.equal(app.stopped, false);
  assert.equal(app.site, false);
});

test("a site registers the worker of the kernel it pins, and asks no one which", async () => {
  for (const name of ["space", "profile"]) {
    const site = boot(name, { controlled: false, served: false });
    await settle();
    assert.equal(site.container.registered[0][0], SCRIPTS[name]);
    assert.deepEqual(site.kernelAsked, []);
  }
});

test("a space under the worker of the kernel it pins is brought up", async () => {
  const site = boot("space");
  await settle();
  assert.equal(site.registration.unregistered, false);
  assert.deepEqual(site.container.registered, []);
  assert.deepEqual(site.went, []);
  assert.match(site.body.innerHTML, /tonk-display/);
});

test("a site that pins no kernel runs the one the page around it names, and pins it", async () => {
  // A release pinned before kernels, which no worker runs any more.
  const release = { v: 1, pin: "https://tonk.network/releases/bafkreib7f5smq4ggn5xwqas57gchvhvfry6ym7jbw5h3w4zg7wtwk33l2i/worker.js" };
  for (const [name, named, hosted] of [["space", "bafkr4ikernel", true], ["profile", "bafkr4ipersona", false]]) {
    for (const record of [null, release]) {
      const site = boot(name, { controlled: false, served: false, record, named, hosted });
      await settle();
      assert.equal(site.kernelAsked.length, 1, "it asks the page around once");
      assert.equal(site.container.registered[0][0], `/kernel_boot.js?kernel=${named}`);
      assert.deepEqual(
        site.booted.record,
        { v: 2, kernel: named, host: hosted },
        "and keeps whether that page hands it a host, for its worker's first start",
      );
    }
  }
});

test("a space the person does not hold yet starts once they do, asked again in a while", async () => {
  const site = boot("space", { served: false, unstarted: 2, wait: (resume) => setTimeout(resume, 1) });
  for (let i = 0; i < 5; i++) await settle();

  assert.equal(site.toWorker.filter((message) => message.type === "booted").length, 3);
  assert.ok(site.said.includes("waiting for the site"), "it says what it waits on");
  assert.equal(site.failed.length, 1, "what went wrong is told once");
  assert.deepEqual(site.went, [["replace", "/"]], "it goes to the space once it started");
});

test("a site with no kernel pinned or named starts no worker and says why", async () => {
  for (const name of ["space", "profile"]) {
    const site = boot(name, { controlled: false, served: false, record: null, named: null, wait: (resume) => setTimeout(resume, 1) });
    for (let i = 0; i < 3; i++) await settle();
    assert.deepEqual(site.container.registered, []);
    assert.match(site.failed.join(" "), /pins no kernel, and the page around it names none/);
  }
});

for (const [what, options] of [
  ["a space under the bootloader of a release", { runs: null }],
  ["a space under another kernel's worker", { runs: "bafkr4iother" }],
]) {
  test(`${what} registers the worker of its kernel in place and is brought up under it`, async () => {
    const script = SCRIPTS.space;
    const site = boot("space", options);
    await settle();
    assert.deepEqual(site.container.registered.map(([registered]) => registered), [script]);
    assert.equal(site.registration.unregistered, false, "the worker it has is not withdrawn");
    assert.equal(site.said.at(-1), "switching its worker");
    assert.deepEqual(site.ran, [], "nothing of the site runs under the other worker");

    // The browser says the controller changed, with the other worker still
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

test("a site runs the kernel it pins, whatever the page around would name", async () => {
  const site = boot("profile", {
    controlled: false,
    served: false,
    record: { v: 2, kernel: "bafkr4ipinned" },
    named: "bafkr4inewer",
  });
  await settle();
  assert.equal(site.container.registered[0][0], "/kernel_boot.js?kernel=bafkr4ipinned");
  assert.deepEqual(site.kernelAsked, []);
});

test("the shell never reads which kernel to run from the deployment itself", async () => {
  const site = boot("space", { controlled: false, served: false, record: null });
  await settle();
  assert.ok(!site.requests.some(([, url]) => url === "/_host/kernel.json"));
});

// A port the page around hands down, as a stand-in.
const handed = () => ({ postMessage() {} });

for (const name of ["space", "profile"]) {
  test(`${name}: what the page around tells the worker reaches it, whatever it is`, async () => {
    const site = boot(name);
    await settle();
    const port = handed();
    site.fromAbove({ __tonkOrigin: "to-worker", message: { type: "resend" } }, [port]);
    site.fromAbove({ __tonkOrigin: "to-worker", message: { type: "anything" } });
    assert.deepEqual(
      site.toWorker.filter((message) => message.type === "resend" || message.type === "anything").map((message) => message.type),
      ["resend", "anything"],
      "the worker decides what it answers",
    );
  });

  test(`${name}: what the worker says goes up to the page around, which decides what it heeds`, async () => {
    const site = boot(name);
    await settle();
    site.workerSays({ type: "navigate", href: "/elsewhere" });
    assert.deepEqual(
      site.up.filter((message) => message.__tonkOrigin === "worker-message").map((message) => message.message),
      [{ type: "navigate", href: "/elsewhere" }],
    );
  });

  test(`${name}: it says who acts there where its kernel does`, async () => {
    const site = boot(name, { answers: { "/api/identify": { did: "did:key:zWho" } } });
    await settle();
    assert.equal(
      JSON.stringify(site.up.filter((message) => message.__tonkOrigin === "identity")),
      JSON.stringify([{ __tonkOrigin: "identity", did: "did:key:zWho" }]),
    );
  });
}

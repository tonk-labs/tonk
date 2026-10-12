import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const CURRENT = "/_release/bafkreicurrent/worker.js";
// The bootloader as the publisher leaves it: stamped with the release built
// alongside it.
const SOURCE = readFileSync(join(HERE, "..", "assets", "boot.js"), "utf8").replace(
  /^const CURRENT_RELEASE = .*$/m,
  `const CURRENT_RELEASE = "${CURRENT}";`,
);
assert.ok(SOURCE.includes(CURRENT), "the bootloader's CURRENT_RELEASE declaration moved");

// Run the bootloader as the worker at `address`, against stand-ins for a
// service worker's globals. `releases` is what each address a script may be
// imported from sets `self.release` to.
function boot(address = "https://bspace.tonk.test/boot.js", releases = {}) {
  const listeners = {};
  // What the release was started with, and what it was called with.
  const started = [];
  const calls = [];
  const handlers = Object.fromEntries(
    ["install", "activate", "message", "fetch", "unhandledrejection"].map((type) => [
      type,
      (call) => void calls.push([type, call]),
    ]),
  );
  const standing = {
    start: (options) => {
      started.push(options);
      return handlers;
    },
  };
  const imported = [];
  const self = {
    location: new URL(address),
    addEventListener: (type, listener) => void (listeners[type] = listener),
  };
  const context = vm.createContext({
    self,
    URL,
    importScripts: (url) => {
      imported.push(url);
      self.release = url in releases ? releases[url] : standing;
    },
  });
  vm.runInContext(SOURCE, context);
  return { listeners, started, calls, imported };
}

const ORIGIN = "https://bspace.tonk.test";

test("a site that pins no release runs the one the deployment was built with", () => {
  const site = boot();
  assert.deepEqual(site.imported, [`${ORIGIN}${CURRENT}`]);
  assert.deepEqual(
    site.started.map((options) => options.base),
    [`${ORIGIN}${CURRENT}`],
    "the release is told where it was loaded from",
  );
});

test("a site runs the release its worker's address pins", () => {
  const pin = "/_release/bafkreipinned/worker.js";
  const site = boot(`${ORIGIN}/boot.js?pin=${encodeURIComponent(pin)}`);
  assert.deepEqual(site.imported, [`${ORIGIN}${pin}`]);
  assert.deepEqual(site.started.map((options) => options.base), [`${ORIGIN}${pin}`]);
});

test("a release at another origin is not loaded", () => {
  const pin = "https://elsewhere.test/_release/bafkreiother/worker.js";
  assert.throws(
    () => boot(`${ORIGIN}/boot.js?pin=${encodeURIComponent(pin)}`),
    /is not on this origin/,
  );
});

test("a script that is not a release does not start the worker", () => {
  for (const release of [undefined, {}, { start: "soon" }]) {
    assert.throws(
      () => boot(`${ORIGIN}/boot.js`, { [`${ORIGIN}${CURRENT}`]: release }),
      /is not a release/,
    );
  }
});

test("the bootloader holds every one of the worker's events", () => {
  const site = boot();
  assert.deepEqual(
    Object.keys(site.listeners).sort(),
    ["activate", "fetch", "install", "message", "unhandledrejection"],
  );
});

test("a request is passed to the release, which answers through the call", () => {
  const site = boot();
  const answered = [];
  const kept = [];
  const event = {
    request: { url: `${ORIGIN}/notes` },
    clientId: "page",
    resultingClientId: "next",
    respondWith: (response) => void answered.push(response),
    waitUntil: (promise) => void kept.push(promise),
    // What an event has and a call does not.
    stopImmediatePropagation() {},
    preloadResponse: Promise.resolve(),
  };
  site.listeners.fetch(event);

  const [[type, call]] = site.calls;
  assert.equal(type, "fetch");
  assert.notEqual(call, event, "the release is handed a call, not the worker's event");
  assert.deepEqual(
    Object.keys(call).sort(),
    ["clientId", "request", "respondWith", "resultingClientId", "waitUntil"],
  );
  assert.equal(call.request, event.request);
  assert.equal(call.clientId, "page");
  assert.equal(call.resultingClientId, "next");

  call.respondWith("an answer");
  call.waitUntil("some work");
  assert.deepEqual(answered, ["an answer"]);
  assert.deepEqual(kept, ["some work"]);
});

test("a message is passed to the release with what it carried", () => {
  const site = boot();
  const kept = [];
  const port = {};
  const source = { id: "page" };
  site.listeners.message({
    data: { type: "port" },
    ports: [port],
    source,
    waitUntil: (promise) => void kept.push(promise),
  });

  const [[type, call]] = site.calls;
  assert.equal(type, "message");
  assert.deepEqual(Object.keys(call).sort(), ["data", "ports", "source", "waitUntil"]);
  assert.deepEqual(call.data, { type: "port" });
  assert.equal(call.ports[0], port);
  assert.equal(call.source, source);
  call.waitUntil("some work");
  assert.deepEqual(kept, ["some work"]);
});

test("installing and activating are passed to the release", () => {
  const site = boot();
  const kept = [];
  for (const type of ["install", "activate"]) {
    site.listeners[type]({ waitUntil: (promise) => void kept.push(promise) });
  }
  assert.deepEqual(site.calls.map(([type]) => type), ["install", "activate"]);
  site.calls[0][1].waitUntil("installing");
  assert.deepEqual(kept, ["installing"]);
});

test("a rejection nothing handled is passed to the release", () => {
  const site = boot();
  const reason = new Error("lost");
  site.listeners.unhandledrejection({ reason });
  const [[type, call]] = site.calls;
  assert.equal(type, "unhandledrejection");
  assert.equal(call.reason, reason);
});

test("a worker registered at the address before the bootloader gives its pages up", async () => {
  const source = readFileSync(join(HERE, "..", "assets", "space_worker.js"), "utf8");
  const listeners = {};
  const did = [];
  const self = {
    addEventListener: (type, listener) => void (listeners[type] = listener),
    skipWaiting: () => void did.push("skip waiting"),
    clients: { claim: async () => void did.push("claim") },
    registration: { unregister: async () => void did.push("unregister") },
  };
  vm.runInContext(source, vm.createContext({ self }));
  assert.deepEqual(Object.keys(listeners).sort(), ["activate", "install"], "it answers no request");

  listeners.install({});
  let activated;
  listeners.activate({ waitUntil: (promise) => void (activated = promise) });
  await activated;
  // Its pages load again once it has them, and find no worker registered.
  assert.deepEqual(did, ["skip waiting", "claim", "unregister"]);
});

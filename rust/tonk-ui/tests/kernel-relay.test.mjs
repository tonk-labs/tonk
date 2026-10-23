import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = readFileSync(join(HERE, "..", "assets", "kernel", "relay.js"), "utf8");

// Listeners by type, as a window, a document or a worker container holds.
function target() {
  const listeners = [];
  return {
    addEventListener: (type, listener) => void listeners.push({ type, listener }),
    dispatch: (type, event) => listeners.filter((it) => it.type === type).forEach((it) => it.listener(event)),
    count: (type) => listeners.filter((it) => it.type === type).length,
  };
}

// Run the relay as the first script of a page: framed by the app's page
// (`"app"`), by a page of the space's own origin (`"space"`), or opened on its
// own (`"alone"`).
function page({ framed = "app" } = {}) {
  const told = [];
  const toWorker = [];
  const script = { removed: false, remove() { this.removed = true; } };
  const window = target();
  const document = Object.assign(target(), { currentScript: script, visibilityState: "visible" });
  window.postMessage = (message) => void told.push(message);
  const parent =
    framed === "alone" ? window
    : framed === "space" ? { location: { origin: "https://bspace.tonk.test" } }
    : {
        postMessage: (message) => void told.push(message),
        get location() {
          throw new Error("cross-origin");
        },
      };
  const worker = Object.assign(target(), {
    controller: { postMessage: (message, ports) => void toWorker.push({ message, ports }) },
    startMessages() {},
  });
  vm.runInContext(SOURCE, vm.createContext({
    window,
    document,
    parent,
    navigator: { serviceWorker: worker },
    location: { origin: "https://bspace.tonk.test" },
  }));
  return { told, toWorker, script, window, document, worker, parent };
}

test("the relay takes its own script out of the page", () => {
  for (const framed of ["app", "space", "alone"]) {
    assert.ok(page({ framed }).script.removed, `framed by ${framed}`);
  }
});

test("the relay says the space is here and hands the port the frame above opens to the worker", () => {
  const it = page();
  assert.deepEqual(
    it.told.map((message) => message.__tonkOrigin),
    ["shell", "need-port", "status"],
    "it says the space is here, asks for a port, and says it is ready",
  );

  let stopped = false;
  const port = {};
  it.window.dispatch("message", {
    source: it.parent,
    data: { __tonkOrigin: "port" },
    ports: [port],
    stopImmediatePropagation: () => void (stopped = true),
  });
  assert.ok(stopped, "nothing after it hears the port");
  const handed = it.toWorker.at(-1);
  assert.equal(handed.message.type, "port");
  assert.equal(handed.ports[0], port, "it hands the port down to the worker");

  it.worker.dispatch("message", { data: { type: "need-port" } });
  assert.equal(it.told.at(-1).__tonkOrigin, "need-port", "it asks again when the worker restarts");
});

test("a page opened on its own has no port to carry", () => {
  const it = page({ framed: "alone" });
  assert.deepEqual(it.told, []);
  assert.equal(it.window.count("message"), 0);
});

test("a frame inside the space's own page leaves the port to the page around it", () => {
  const it = page({ framed: "space" });
  assert.equal(it.window.count("message"), 0);
  assert.equal(it.worker.count("message"), 0);
});

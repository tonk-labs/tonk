import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const SERVICE_WORKER = join(HERE, "..", "assets", "service_worker.js");
const INDEX = join(HERE, "..", "index.html");

function eventTarget(initial = {}) {
  const listeners = new Map();
  return Object.assign(initial, {
    addEventListener(type, listener) {
      const registered = listeners.get(type) ?? new Set();
      registered.add(listener);
      listeners.set(type, registered);
    },
    removeEventListener(type, listener) {
      listeners.get(type)?.delete(listener);
    },
    dispatch(type, event = {}) {
      return Promise.all(
        [...(listeners.get(type) ?? [])].map((listener) => listener(event)),
      );
    },
  });
}

class FakeCacheStorage {
  async open() {
    return {
      async add() {},
      async match() {},
      async put() {},
      async keys() { return []; },
      async delete() { return false; },
    };
  }
  async keys() { return []; }
  async match() {}
  async delete() { return false; }
}

function loadServiceWorker({
  executingRole = "active",
  waitingAtStartup = false,
  retirementFailures = 0,
} = {}) {
  let claims = 0;
  let activationRequests = 0;
  let retirementAttempts = 0;
  let retirements = 0;
  let dataFetches = 0;
  const logs = [];
  const executingWorker = {};
  const activeWorker = executingRole === "active" ? executingWorker : {};
  const waitingWorker = executingRole === "waiting" ? executingWorker : {};
  const registration = eventTarget({
    active: activeWorker,
    installing: null,
    waiting: waitingAtStartup ? waitingWorker : null,
  });
  const scope = eventTarget({
    location: {
      href: "https://tonk.test/service_worker.js",
      origin: "https://tonk.test",
    },
    registration,
    serviceWorker: executingWorker,
    navigator: { onLine: true },
    async skipWaiting() { activationRequests += 1; },
    clients: {
      async claim() { claims += 1; },
      async matchAll() { return []; },
    },
  });
  const source = readFileSync(SERVICE_WORKER, "utf8").replace(
    /^import init, \{ activate \} from "\.\/worker\.js";$/m,
    "const init = async () => {}; const activate = async () => ({ onactivate: async () => {}, onupdatefound: async () => recordRetirement(), onfetch: async () => recordDataFetch() });",
  );
  const quietConsole = {
    log(...args) { logs.push(args.join(" ")); },
    warn(...args) { logs.push(args.join(" ")); },
    error(...args) { logs.push(args.join(" ")); },
  };
  vm.runInNewContext(
    source,
    {
      self: scope,
      caches: new FakeCacheStorage(),
      fetch: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        if (new URL(raw, "https://tonk.test").pathname === "/worker_bg.wasm") {
          return new Response(new Uint8Array([0]));
        }
        return new Response("", { status: 404 });
      },
      console: quietConsole,
      navigator: scope.navigator,
      Response,
      Request,
      URL,
      Date,
      setTimeout,
      clearTimeout,
      recordRetirement() {
        retirementAttempts += 1;
        if (retirementAttempts <= retirementFailures) throw new Error("release failed");
        retirements += 1;
      },
      recordDataFetch() {
        dataFetches += 1;
        return new Response(retirements > 0 ? "after-release" : "before-release");
      },
    },
    { filename: SERVICE_WORKER },
  );
  return {
    scope,
    claims: () => claims,
    activationRequests: () => activationRequests,
    retirementAttempts: () => retirementAttempts,
    retirements: () => retirements,
    dataFetches: () => dataFetches,
    logs,
  };
}

function activationBlock() {
  const html = readFileSync(INDEX, "utf8");
  const blocks = [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)]
    .map((match) => match[1])
    .filter((block) => block.includes("const serviceWorkerActivation"));
  assert.equal(blocks.length, 1);
  return blocks[0];
}

class FakeBroadcastChannel {
  addEventListener() {}
  close() {}
}

function pageHarness({ mode, alignmentReload = false, updateFails = false } = {}) {
  const messages = [];
  const storage = new Map();
  if (alignmentReload) storage.set("tonk:sw-upgrade-reload", "1");
  let reloads = 0;
  let updates = 0;
  let resolveReady;
  const incumbent = mode === "cold" ? null : eventTarget({
    state: "activated",
    postMessage(message) { messages.push(message); },
  });
  let serviceWorkers;
  const incoming = eventTarget({
    state: mode === "cold" || mode === "warm-update" ? "installing" : "activated",
    postMessage(message) {
      messages.push(message);
      if (message?.type === "claim") {
        serviceWorkers.controller = incoming;
        void serviceWorkers.dispatch("controllerchange");
      }
    },
  });
  const registration = eventTarget({
    active: incumbent,
    installing: mode === "warm-update" || mode === "cold" ? incoming : null,
    waiting: null,
    async update() {
      updates += 1;
      if (updateFails) throw new TypeError("offline");
    },
  });
  const ready = mode === "cold"
    ? new Promise((resolve) => { resolveReady = resolve; })
    : Promise.resolve(registration);
  serviceWorkers = eventTarget({
    controller: incumbent,
    ready,
    async register() { return registration; },
  });
  const self = eventTarget({ tonkBootLife() {} });
  const document = eventTarget({
    visibilityState: "visible",
    querySelector() { return { textContent: "", setAttribute() {} }; },
  });
  vm.runInNewContext(
    activationBlock(),
    {
      self,
      window: self,
      document,
      navigator: { serviceWorker: serviceWorkers },
      BroadcastChannel: FakeBroadcastChannel,
      sessionStorage: {
        getItem(key) { return storage.get(key) ?? null; },
        setItem(key, value) { storage.set(key, String(value)); },
        removeItem(key) { storage.delete(key); },
      },
      location: { reload() { reloads += 1; } },
      console: { log() {}, warn() {}, error() {} },
      Event,
      Number,
      Promise,
      setTimeout,
      clearTimeout,
    },
    { filename: INDEX },
  );
  return {
    incoming,
    registration,
    serviceWorkers,
    storage,
    messages,
    reloads: () => reloads,
    updates: () => updates,
    activateColdWorker() {
      registration.installing = null;
      registration.active = incoming;
      incoming.state = "activated";
      resolveReady(registration);
    },
    async activateWarmWorker() {
      registration.installing = null;
      registration.active = incoming;
      incoming.state = "activated";
      serviceWorkers.controller = incoming;
      await serviceWorkers.dispatch("controllerchange");
      await incoming.dispatch("statechange");
    },
    ready: () => self.serviceWorkerActivates(),
  };
}

function multiPageReplacementHarness() {
  const incumbent = eventTarget({ state: "activated" });
  const incoming = eventTarget({ state: "installing", postMessage() {} });
  const registration = eventTarget({
    active: incumbent,
    installing: incoming,
    waiting: null,
    async update() {},
  });
  const serviceWorkers = eventTarget({
    controller: incumbent,
    ready: Promise.resolve(registration),
    async register() { return registration; },
  });
  const pages = Array.from({ length: 2 }, () => {
    const storage = new Map();
    let reloads = 0;
    const self = eventTarget({ tonkBootLife() {} });
    vm.runInNewContext(
      activationBlock(),
      {
        self,
        window: self,
        document: eventTarget({
          visibilityState: "visible",
          querySelector() { return { textContent: "", setAttribute() {} }; },
        }),
        navigator: { serviceWorker: serviceWorkers },
        BroadcastChannel: FakeBroadcastChannel,
        sessionStorage: {
          getItem(key) { return storage.get(key) ?? null; },
          setItem(key, value) { storage.set(key, String(value)); },
          removeItem(key) { storage.delete(key); },
        },
        location: { reload() { reloads += 1; } },
        console: { log() {}, warn() {}, error() {} },
        Event,
        Number,
        Promise,
        setTimeout,
        clearTimeout,
      },
      { filename: INDEX },
    );
    return { reloads: () => reloads, storage };
  });
  return {
    pages,
    async activateSuccessor() {
      registration.installing = null;
      registration.active = incoming;
      incoming.state = "activated";
      serviceWorkers.controller = incoming;
      await serviceWorkers.dispatch("controllerchange");
      await incoming.dispatch("statechange");
      await new Promise(setImmediate);
    },
  };
}

test("the activate handler does not call clients.claim", async () => {
  const { scope, claims } = loadServiceWorker();
  // This harness executes the authored activate handler; it does not simulate
  // the browser's controller-replacement algorithm for already-controlled clients.
  scope.onactivate({});
  await new Promise(setImmediate);
  assert.equal(claims(), 0);
});

test("a failed successor install leaves the incumbent operational", async () => {
  const { scope, retirements } = loadServiceWorker();
  const candidate = eventTarget({ state: "installing" });
  scope.registration.installing = candidate;
  await scope.registration.dispatch("updatefound");
  candidate.state = "redundant";
  await candidate.dispatch("statechange");
  assert.equal(retirements(), 0);
});

test("an installed successor retires the incumbent exactly once", async () => {
  const { scope, retirements } = loadServiceWorker();
  const candidate = eventTarget({ state: "installing" });
  scope.registration.installing = candidate;
  await scope.registration.dispatch("updatefound");
  candidate.state = "installed";
  scope.registration.waiting = candidate;
  await candidate.dispatch("statechange");
  await candidate.dispatch("statechange");
  assert.equal(retirements(), 1);
});

test("a failed stream release is retried on the next incumbent fetch", async () => {
  const result = loadServiceWorker({ retirementFailures: 1 });
  const candidate = eventTarget({ state: "installing" });
  result.scope.registration.installing = candidate;
  await result.scope.registration.dispatch("updatefound");
  candidate.state = "installed";
  result.scope.registration.waiting = candidate;
  await candidate.dispatch("statechange");
  assert.equal(result.retirementAttempts(), 1);
  assert.equal(result.retirements(), 0);

  const pending = [];
  let response;
  result.scope.onfetch({
    request: new Request("https://tonk.test/api/profile/branch/main/query"),
    waitUntil(promise) { pending.push(promise); },
    respondWith(promise) { response = Promise.resolve(promise); },
  });
  assert.equal(await (await response).text(), "after-release");
  await Promise.all(pending);
  assert.equal(result.retirementAttempts(), 2);
  assert.equal(result.retirements(), 1);
  assert.equal(result.dataFetches(), 1);
});

test("only the restarted active incumbent retires for a waiting successor", async () => {
  const waiting = loadServiceWorker({ executingRole: "waiting", waitingAtStartup: true });
  const active = loadServiceWorker({ executingRole: "active", waitingAtStartup: true });
  await new Promise(setImmediate);
  assert.equal(waiting.retirements(), 0, waiting.logs.join("\n"));
  assert.equal(active.retirements(), 1, active.logs.join("\n"));
});

test("claim takes control and activate messages no longer gate adoption", async () => {
  const result = loadServiceWorker();
  const pending = [];
  result.scope.onmessage({
    data: { type: "claim" },
    waitUntil(promise) { pending.push(promise); },
  });
  result.scope.onmessage({
    data: { type: "activate" },
    waitUntil(promise) { pending.push(promise); },
  });
  await Promise.all(pending);
  assert.equal(result.claims(), 1);
  assert.equal(result.activationRequests(), 0);
});

test("a cold first install waits for activation, then claims", async () => {
  const result = pageHarness({ mode: "cold" });
  await new Promise(setImmediate);
  assert.deepEqual(result.messages, []);
  result.activateColdWorker();
  await result.ready();
  assert.deepEqual(result.messages.map((message) => message.type), ["claim", "connectivity"]);
});

test("successor activation replaces the controller and causes one guarded reload", async () => {
  const result = pageHarness({ mode: "warm-update" });
  await new Promise(setImmediate);
  await result.activateWarmWorker();
  assert.deepEqual(result.messages, []);
  assert.equal(result.storage.get("tonk:sw-upgrade-reload"), "1");
  assert.equal(result.reloads(), 1);
});

test("two update-aware documents each reload once on one controller replacement", async () => {
  const result = multiPageReplacementHarness();
  await new Promise(setImmediate);
  await result.activateSuccessor();
  for (const page of result.pages) {
    assert.equal(page.storage.get("tonk:sw-upgrade-reload"), "1");
    assert.equal(page.reloads(), 1);
  }
});

test("the alignment reload consumes its guard without another update check", async () => {
  const result = pageHarness({ mode: "warm", alignmentReload: true });
  await result.ready();
  assert.equal(result.storage.has("tonk:sw-upgrade-reload"), false);
  assert.equal(result.updates(), 0);
  assert.equal(result.reloads(), 0);
});

test("an offline load-time update check keeps the active worker", async () => {
  const result = pageHarness({ mode: "warm", updateFails: true });
  await result.ready();
  assert.equal(result.updates(), 1);
  assert.equal(result.reloads(), 0);
  assert.deepEqual(result.messages.map((message) => message.type), ["connectivity"]);
});

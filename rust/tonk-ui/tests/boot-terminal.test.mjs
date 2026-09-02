import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const watchdog = [...source.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
  .map((match) => match[1])
  .find((script) => script.includes('const RETRIES = "tonk:boot-retries"'));
const reloadSafety = [...source.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
  .map((match) => match[1])
  .find(
    (script) =>
      script.includes("data-tonk-account-setup-critical") &&
      script.includes("tonk:account-setup-critical-change"),
  );

assert.ok(watchdog, "expected to find the boot-watchdog script in index.html");
assert.ok(reloadSafety, "expected to find the account-safe reload script in index.html");

function emptyAccountHoldDatabase() {
  let created = false;
  return {
    open() {
      const request = {};
      queueMicrotask(() => {
        const database = {
          objectStoreNames: {
            contains: (name) => created && name === "holds",
          },
          createObjectStore(name) {
            assert.equal(name, "holds");
            created = true;
          },
          transaction(name) {
            assert.equal(name, "holds");
            const transaction = {
              objectStore: () => ({
                get(key) {
                  assert.equal(key, "account-setup");
                  const get = {};
                  queueMicrotask(() => {
                    get.result = undefined;
                    get.onsuccess?.();
                    queueMicrotask(() => transaction.oncomplete?.());
                  });
                  return get;
                },
              }),
            };
            return transaction;
          },
          close() {},
        };
        request.result = database;
        if (!created) request.onupgradeneeded?.();
        request.onsuccess?.();
      });
      return request;
    },
  };
}

function bootHarness({ critical = false, retries = 1 } = {}) {
  const status = {
    attributes: new Set(),
    textContent: "loading…",
    setAttribute(name) {
      this.attributes.add(name);
    },
  };
  const effects = {
    cacheDeletes: 0,
    reloads: 0,
    unregisters: 0,
  };
  const warnings = [];
  const session = new Map();
  if (retries > 0) session.set("tonk:boot-retries", String(retries));
  const listeners = new Map();
  const timeouts = new Map();
  let nextTimeout = 0;
  const context = {
    Promise,
    caches: {
      async delete() {
        effects.cacheDeletes += 1;
      },
      async keys() {
        return ["TONK_SHELL_old", "unrelated-cache"];
      },
    },
    clearInterval() {},
    console: {
      error(...args) {
        warnings.push(args);
      },
      warn(...args) {
        warnings.push(args);
      },
    },
    document: {
      documentElement: {
        hasAttribute(name) {
          return name === "data-tonk-account-setup-critical" && critical;
        },
      },
      querySelector(selector) {
        return selector === "[data-boot-status]" ? status : null;
      },
    },
    indexedDB: emptyAccountHoldDatabase(),
    location: {
      reload() {
        effects.reloads += 1;
      },
    },
    navigator: {
      locks: {
        async request(name, options, callback) {
          assert.equal(name, "tonk-update-safety-v1");
          assert.equal(options.mode, "exclusive");
          assert.deepEqual(Object.keys(options), ["mode"]);
          return callback();
        },
      },
      serviceWorker: {
        controller: {},
        async getRegistration() {
          return {
            async unregister() {
              effects.unregisters += 1;
            },
          };
        },
        async getRegistrations() {
          return [
            {
              async unregister() {
                effects.unregisters += 1;
              },
            },
          ];
        },
      },
    },
    self: null,
    sessionStorage: {
      getItem(key) {
        return session.get(key) ?? null;
      },
      removeItem(key) {
        session.delete(key);
      },
      setItem(key, value) {
        session.set(key, value);
      },
    },
    setInterval() {
      return 1;
    },
    setTimeout(callback) {
      const id = ++nextTimeout;
      timeouts.set(id, callback);
      return id;
    },
    clearTimeout(id) {
      timeouts.delete(id);
    },
    addEventListener(type, listener) {
      const registered = listeners.get(type) ?? new Set();
      registered.add(listener);
      listeners.set(type, registered);
    },
    removeEventListener(type, listener) {
      listeners.get(type)?.delete(listener);
    },
  };
  context.self = context;
  context.window = context;
  context.tonkAccountSetupMayReload = () => !critical;
  vm.runInNewContext(reloadSafety, context, {
    filename: "index.html:reload-safety",
  });
  vm.runInNewContext(watchdog, context, {
    filename: "index.html:boot-watchdog",
  });
  return {
    context,
    effects,
    session,
    status,
    warnings,
    setCritical(value) {
      critical = value;
      for (const listener of listeners.get("tonk:account-setup-critical-change") ?? []) {
        listener({ detail: { critical: value, mayReload: !value } });
      }
    },
  };
}

test("the automatic watchdog reload waits for durable account setup", async () => {
  const { context, effects, session, setCritical, warnings } = bootHarness({
    critical: true,
    retries: 0,
  });

  context.tonkBootRecover("no boot progress");
  assert.equal(session.get("tonk:boot-retries"), "1");
  assert.equal(effects.reloads, 0);

  setCritical(false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(effects.reloads, 1, warnings.flat().join(" | "));
});

test("an explicit readiness failure stops destructive automatic recovery", async () => {
  const { context, effects, session, status } = bootHarness();
  const message =
    "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.";

  context.tonkBootTerminal?.(message);
  context.tonkBootRecover("a late boot failure");
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    {
      effects,
      failed: status.attributes.has("data-failed"),
      message: status.textContent,
      retryState: session.get("tonk:boot-retries") ?? null,
      terminalHook: typeof context.tonkBootTerminal,
    },
    {
      effects: { cacheDeletes: 0, reloads: 0, unregisters: 0 },
      failed: true,
      message,
      retryState: null,
      terminalHook: "function",
    },
  );
});

test("the first terminal failure keeps its specific recovery message", async () => {
  const { context, effects, session, status } = bootHarness();
  const specific =
    "This Tonk version was withdrawn. Reload to try the current version.";
  const generic =
    "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.";

  context.tonkBootTerminal(specific);
  context.tonkBootTerminal(generic);
  context.tonkBootRecover("a later boot failure");
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    {
      effects,
      failed: status.attributes.has("data-failed"),
      message: status.textContent,
      retryState: session.get("tonk:boot-retries") ?? null,
    },
    {
      effects: { cacheDeletes: 0, reloads: 0, unregisters: 0 },
      failed: true,
      message: specific,
      retryState: null,
    },
  );
});

test("a second silent stall preserves every cache and registration", async () => {
  const { context, effects, session, status } = bootHarness();
  const message =
    "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.";

  context.tonkBootRecover("no boot progress after one reload");
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    {
      effects,
      failed: status.attributes.has("data-failed"),
      message: status.textContent,
      retryState: session.get("tonk:boot-retries") ?? null,
    },
    {
      effects: { cacheDeletes: 0, reloads: 0, unregisters: 0 },
      failed: true,
      message,
      retryState: null,
    },
  );
});

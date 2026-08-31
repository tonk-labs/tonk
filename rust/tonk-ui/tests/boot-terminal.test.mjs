import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const watchdog = [...source.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
  .map((match) => match[1])
  .find((script) => script.includes('const RETRIES = "tonk:boot-retries"'));

assert.ok(watchdog, "expected to find the boot-watchdog script in index.html");

function bootHarness() {
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
  const session = new Map([["tonk:boot-retries", "1"]]);
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
    console: { error() {}, warn() {} },
    document: {
      querySelector(selector) {
        return selector === "[data-boot-status]" ? status : null;
      },
    },
    location: {
      reload() {
        effects.reloads += 1;
      },
    },
    navigator: {
      serviceWorker: {
        controller: {},
        async getRegistration() {
          return null;
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
  };
  context.self = context;
  vm.runInNewContext(watchdog, context, {
    filename: "index.html:boot-watchdog",
  });
  return { context, effects, session, status };
}

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

// Tests for the service worker's update/recovery decisions.
//
// These pin the lifecycle behaviours from the SW audit — the ones whose
// failure mode is "Safari is stuck on an old version and no reload
// helps". They are hard to observe in a live browser (they need two
// builds and a half-finished swap), so they are pinned here instead.
//
// The REAL `assets/service_worker.js` is executed, not a copy: it is
// loaded as a module with a stub `ServiceWorkerGlobalScope` installed
// on `globalThis` first. The `import init from "./worker.js"` at its
// top is redirected to a stub through a tiny loader hook, so no wasm is
// needed. Testing the shipped artifact is the point — a copy would
// drift from what actually runs.
import { test, describe, before, mock } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const SW_PATH = join(HERE, "..", "assets", "service_worker.js");
const source = readFileSync(SW_PATH, "utf8");

/** A minimal Cache/CacheStorage good enough for the decisions here. */
class FakeCache {
  #entries = new Map();
  async match(request) {
    const key = typeof request === "string" ? request : request.url;
    return this.#entries.get(key) ?? undefined;
  }
  async put(request, response) {
    const key = typeof request === "string" ? request : request.url;
    this.#entries.set(key, response);
  }
  async add(request) {
    const key = typeof request === "string" ? request : request.url;
    this.#entries.set(key, new Response("stub"));
  }
  async keys() {
    return [...this.#entries.keys()].map((url) => ({ url }));
  }
  async delete(request) {
    const key = typeof request === "string" ? request : request.url;
    return this.#entries.delete(key);
  }
}

class FakeCacheStorage {
  caches = new Map();
  async open(name) {
    if (!this.caches.has(name)) this.caches.set(name, new FakeCache());
    return this.caches.get(name);
  }
  async keys() {
    return [...this.caches.keys()];
  }
  async delete(name) {
    return this.caches.delete(name);
  }
}

/** Evaluate the shipped SW source with the given environment. */
async function loadServiceWorker({ fetchImpl, registration } = {}) {
  const listeners = new Map();
  const scope = {
    location: { href: "https://tonk.test/service_worker.js", origin: "https://tonk.test" },
    registration: registration ?? { waiting: null, installing: null, addEventListener() {} },
    serviceWorker: {},
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => [] },
    addEventListener: (type, fn) => listeners.set(type, fn),
  };

  const caches = new FakeCacheStorage();
  const fetchFn = fetchImpl ?? (async () => new Response("", { status: 404 }));

  // Strip the wasm-glue import; everything under test is below it.
  const body = source.replace(
    /^import init, \{ activate \} from "\.\/worker\.js";$/m,
    "const init = async () => {}; const activate = async () => ({});",
  );
  // Expose the internals under test without altering the shipped file.
  const instrumented =
    body +
    "\nexport { killSwitchEngaged, isShellCacheable, SHELL_CACHE, WORKER_CACHE, BUILD_ID, precacheWorkerWasm, digestOf };\n";

  const module = await import(
    "data:text/javascript;base64," + Buffer.from(instrumented).toString("base64")
  );
  return { module, caches, scope, fetchFn };
}

/** Install globals the SW module reads at evaluation time. */
function withGlobals({ fetchImpl, registration } = {}) {
  const caches = new FakeCacheStorage();
  const listeners = new Map();
  const self = {
    location: { href: "https://tonk.test/service_worker.js", origin: "https://tonk.test" },
    registration: registration ?? { waiting: null, installing: null, addEventListener() {} },
    serviceWorker: {},
    skipWaiting: async () => {},
    clients: { claim: async () => {}, matchAll: async () => [] },
    addEventListener: (type, fn) => listeners.set(type, fn),
  };
  globalThis.self = self;
  globalThis.caches = caches;
  globalThis.fetch = fetchImpl ?? (async () => new Response("", { status: 404 }));
  return { self, caches, listeners };
}

describe("kill switch", () => {
  // The flag is a remote "unregister yourself" instruction, so every
  // ambiguous answer must resolve to NOT firing. An accidental
  // unregister across every browser is far worse than a missed one.

  test("does not fire when the host answers a missing flag with SPA HTML", async () => {
    // The real trap, hit during live verification: an SPA host answers
    // an unknown path with `200` + the shell HTML. Parsed naively that
    // throws; parsed carelessly it could look like anything at all.
    withGlobals({
      fetchImpl: async () =>
        new Response("<!doctype html><html><head>", { status: 200 }),
    });
    const { module } = await loadServiceWorker();
    assert.equal(await module.killSwitchEngaged(), false);
  });

  test("does not fire when the flag is absent", async () => {
    withGlobals({ fetchImpl: async () => new Response("", { status: 404 }) });
    const { module } = await loadServiceWorker();
    assert.equal(await module.killSwitchEngaged(), false);
  });

  test("does not fire when the network is down", async () => {
    withGlobals({
      fetchImpl: async () => {
        throw new TypeError("offline");
      },
    });
    const { module } = await loadServiceWorker();
    assert.equal(
      await module.killSwitchEngaged(),
      false,
      "a blip must never unregister a healthy worker",
    );
  });

  test("does not fire when the flag names a different build", async () => {
    withGlobals({
      fetchImpl: async () =>
        new Response(JSON.stringify({ revoked: ["some-other-build"] }), {
          status: 200,
        }),
    });
    const { module } = await loadServiceWorker();
    assert.equal(await module.killSwitchEngaged(), false);
  });

  test("does not fire on a malformed `revoked` field", async () => {
    withGlobals({
      fetchImpl: async () =>
        new Response(JSON.stringify({ revoked: "not-an-array" }), { status: 200 }),
    });
    const { module } = await loadServiceWorker();
    assert.equal(await module.killSwitchEngaged(), false);
  });

  test("fires when the flag names this build", async () => {
    let buildId;
    withGlobals({
      fetchImpl: async () =>
        new Response(JSON.stringify({ revoked: [buildId] }), { status: 200 }),
    });
    const probe = await loadServiceWorker();
    buildId = probe.module.BUILD_ID;
    assert.equal(
      await probe.module.killSwitchEngaged(),
      true,
      "a flag naming this exact build is the one case that fires",
    );
  });
});

describe("shell cache eligibility", () => {
  const req = (url, extra = {}) => ({
    method: "GET",
    mode: "no-cors",
    cache: "default",
    url,
    ...extra,
  });

  test("refuses another origin", async () => {
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.equal(
      module.isShellCacheable(req("https://evil.test/app.js"), "/app.js"),
      false,
    );
  });

  test("refuses a host merely prefixed by our origin", async () => {
    // `https://tonk.test.evil.test/` starts with our origin as a
    // string; only a real origin comparison rejects it.
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.equal(
      module.isShellCacheable(req("https://tonk.test.evil.test/app.js"), "/app.js"),
      false,
    );
  });

  test("accepts our own hashed asset", async () => {
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.equal(
      module.isShellCacheable(req("https://tonk.test/ui-abc.js"), "/ui-abc.js"),
      true,
    );
  });

  test("never caches the data plane", async () => {
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.equal(
      module.isShellCacheable(req("https://tonk.test/api/health"), "/api/health"),
      false,
    );
  });

  test("honors an explicit cache bypass", async () => {
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.equal(
      module.isShellCacheable(
        req("https://tonk.test/library/core.yaml", { cache: "no-store" }),
        "/library/core.yaml",
      ),
      false,
    );
  });
});

describe("cache naming", () => {
  test("scopes both caches to the build", async () => {
    // Per-build names are what make an install atomic: the incoming
    // worker populates its OWN caches, so the still-serving old worker
    // never observes a half-written one.
    withGlobals();
    const { module } = await loadServiceWorker();
    assert.ok(module.SHELL_CACHE.endsWith(module.BUILD_ID));
    assert.ok(module.WORKER_CACHE.endsWith(module.BUILD_ID));
    assert.notEqual(module.SHELL_CACHE, module.WORKER_CACHE);
  });
});

describe("worker wasm precache", () => {
  test("rejects wasm whose hash does not match the stamp", async () => {
    // The heart of the audit's finding 1. Glue and wasm are tightly
    // coupled, so installing a worker whose wasm is not the one built
    // alongside it is an init failure at best and silent miswiring at
    // worst. The install MUST fail so the old, internally consistent
    // worker keeps running.
    const stampedSource = source
      .replace(/^const BUILD_ID = .*$/m, 'const BUILD_ID = "testbuild";')
      .replace(
        /^const WORKER_WASM_HASH = .*$/m,
        'const WORKER_WASM_HASH = "0000000000000000";',
      );

    withGlobals({
      fetchImpl: async () => new Response(new Uint8Array([1, 2, 3]), { status: 200 }),
    });

    const body = stampedSource.replace(
      /^import init, \{ activate \} from "\.\/worker\.js";$/m,
      "const init = async () => {}; const activate = async () => ({});",
    );
    const module = await import(
      "data:text/javascript;base64," +
        Buffer.from(body + "\nexport { precacheWorkerWasm };\n").toString("base64")
    );

    await assert.rejects(
      () => module.precacheWorkerWasm(),
      /hash mismatch/,
      "a mismatched pair must fail the install, not install anyway",
    );
  });

  test("stores the wasm when the hash matches", async () => {
    const bytes = new Uint8Array([1, 2, 3, 4]);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const hex = Array.from(new Uint8Array(digest))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 16);

    const stampedSource = source
      .replace(/^const BUILD_ID = .*$/m, 'const BUILD_ID = "testbuild";')
      .replace(
        /^const WORKER_WASM_HASH = .*$/m,
        `const WORKER_WASM_HASH = "${hex}";`,
      );

    const { caches } = withGlobals({
      fetchImpl: async () => new Response(bytes, { status: 200 }),
    });

    const body = stampedSource.replace(
      /^import init, \{ activate \} from "\.\/worker\.js";$/m,
      "const init = async () => {}; const activate = async () => ({});",
    );
    const module = await import(
      "data:text/javascript;base64," +
        Buffer.from(
          body + "\nexport { precacheWorkerWasm, WORKER_CACHE };\n",
        ).toString("base64")
    );

    await module.precacheWorkerWasm();
    const cache = await caches.open(module.WORKER_CACHE);
    const stored = await cache.match("https://tonk.test/worker_bg.wasm");
    assert.ok(stored, "the verified wasm is cached for this build to boot from");
  });

  test("fails the install when the wasm cannot be fetched", async () => {
    const stampedSource = source
      .replace(/^const BUILD_ID = .*$/m, 'const BUILD_ID = "testbuild";')
      .replace(
        /^const WORKER_WASM_HASH = .*$/m,
        'const WORKER_WASM_HASH = "0000000000000000";',
      );
    withGlobals({ fetchImpl: async () => new Response("", { status: 503 }) });

    const body = stampedSource.replace(
      /^import init, \{ activate \} from "\.\/worker\.js";$/m,
      "const init = async () => {}; const activate = async () => ({});",
    );
    const module = await import(
      "data:text/javascript;base64," +
        Buffer.from(body + "\nexport { precacheWorkerWasm };\n").toString("base64")
    );

    await assert.rejects(() => module.precacheWorkerWasm(), /fetch failed/);
  });
});

describe("self-destruct", () => {
  // Found in live testing: `selfDestruct` originally navigated every
  // open client after unregistering. The reloaded page re-ran the
  // registration script, reinstalled the SAME revoked build, which
  // activated, re-read the flag, unregistered, and navigated again —
  // an unbreakable reload loop, strictly worse than the bad worker it
  // was meant to pull. Unregistering is enough on its own.
  test("does not navigate clients", async () => {
    const swSource = readFileSync(SW_PATH, "utf8");
    const body = swSource.slice(
      swSource.indexOf("async function selfDestruct"),
      swSource.indexOf("self.oninstall"),
    );
    assert.ok(
      !/client\.navigate/.test(body),
      "selfDestruct must not reload clients: the fresh page reinstalls " +
        "the revoked build and the cycle repeats forever",
    );
    assert.ok(
      /registration\.unregister\(\)/.test(body),
      "it must still unregister — that is the actual rollback",
    );
  });

  test("purges only this app's caches", async () => {
    const swSource = readFileSync(SW_PATH, "utf8");
    const body = swSource.slice(
      swSource.indexOf("async function selfDestruct"),
      swSource.indexOf("self.oninstall"),
    );
    assert.ok(
      /TONK_SHELL_|TONK_WORKER_/.test(body),
      "a kill switch must not wipe caches belonging to anything else",
    );
  });
});

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
  // Capture the worker's own log lines so a test can assert WHICH path
  // ran, not just that the result looked right.
  const logs = [];
  const realLog = console.log;
  console.log = (...args) => {
    logs.push(args.join(" "));
    realLog(...args);
  };
  return { self, caches, listeners, logs };
}


/**
 * Load the shipped SW with a chosen build id + wasm hash and export the
 * named internals. Keeps each test's setup to one call.
 */
async function loadWith({ buildId = "testbuild", wasmHash = "0000000000000000", exports = [] }) {
  const body = source
    .replace(/^const BUILD_ID = .*$/m, `const BUILD_ID = "${buildId}";`)
    .replace(/^const WORKER_WASM_HASH = .*$/m, `const WORKER_WASM_HASH = "${wasmHash}";`)
    .replace(
      /^import init, \{ activate \} from "\.\/worker\.js";$/m,
      "const init = async () => {}; const activate = async () => ({});",
    );
  const withExports = body + `\nexport { ${exports.join(", ")} };\n`;
  return import(
    "data:text/javascript;base64," + Buffer.from(withExports).toString("base64")
  );
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

describe("booting from the precached wasm", () => {
  test("boots from cache without touching the network", async () => {
    // The whole point of finding 1: once installed, this worker must
    // never re-fetch its wasm, because the deployed bytes may belong to
    // a newer build than this glue.
    const bytes = new Uint8Array([9, 9, 9, 9]);
    let fetches = 0;
    const { caches, logs } = withGlobals({
      fetchImpl: async () => {
        fetches += 1;
        return new Response(bytes, { status: 200 });
      },
    });
    const mod = await loadWith({
      exports: ["workerWasmModule", "WORKER_CACHE"],
    });
    // Seed the cache as a successful install would have.
    const cache = await caches.open(mod.WORKER_CACHE);
    await cache.put("https://tonk.test/worker_bg.wasm", new Response(bytes));

    const buf = await mod.workerWasmModule();
    assert.equal(buf.byteLength, 4);
    assert.equal(fetches, 0, "a cache hit must not hit the network");
    // `fetches === 0` alone is too weak: `precacheWorkerWasm` also
    // early-returns on a cache hit, so the fallback path would satisfy
    // it too. Assert the cache-first path was taken by its own log.
    assert.ok(
      !logs.some((line) => /missing from cache/.test(line)),
      "a populated cache must be served directly, not routed through the " +
        "eviction fallback",
    );
  });

  test("refetches when the cache entry has been evicted", async () => {
    // Storage pressure can evict the entry. Refetching is still correct
    // (the install already proved the pair matched) and is the only
    // alternative to a worker that can never boot again.
    const bytes = new Uint8Array([7, 7]);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const hex = Array.from(new Uint8Array(digest))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 16);
    let fetches = 0;
    withGlobals({
      fetchImpl: async () => {
        fetches += 1;
        return new Response(bytes, { status: 200 });
      },
    });
    const mod = await loadWith({
      wasmHash: hex,
      exports: ["workerWasmModule"],
    });
    const buf = await mod.workerWasmModule();
    assert.equal(buf.byteLength, 2);
    assert.equal(fetches, 1, "an evicted entry is refetched exactly once");
  });

  test("fails loudly when the wasm cannot be recovered", async () => {
    // Better a visible failure (which the failure page surfaces, with
    // the reset ladder) than a worker that half-boots.
    withGlobals({ fetchImpl: async () => new Response("", { status: 500 }) });
    const mod = await loadWith({ exports: ["workerWasmModule"] });
    await assert.rejects(() => mod.workerWasmModule());
  });
});

describe("navigation while an update is waiting", () => {
  const navEvent = () => ({ waitUntil: (p) => { void p?.catch?.(() => {}); } });

  test("goes network-first so one reload converges", async () => {
    // Plain stale-while-revalidate leaves the page a build behind, so
    // converging takes two reloads — wrong at the moment the user has
    // just clicked "reload to update".
    const { caches } = withGlobals({
      registration: { waiting: {}, installing: null, addEventListener() {} },
      fetchImpl: async () => new Response("FRESH SHELL", { status: 200 }),
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("STALE SHELL"));

    const response = await mod.serveNavigation(navEvent());
    assert.equal(await response.text(), "FRESH SHELL");
  });

  test("falls back to the cached shell when offline", async () => {
    // A navigation must never hard-fail on a shell we already hold.
    const { caches } = withGlobals({
      registration: { waiting: {}, installing: null, addEventListener() {} },
      fetchImpl: async () => {
        throw new TypeError("offline");
      },
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("STALE SHELL"));

    const response = await mod.serveNavigation(navEvent());
    assert.equal(
      await response.text(),
      "STALE SHELL",
      "offline during an update still gets the app, not an error",
    );
  });

  test("serves the cached shell immediately when no update is waiting", async () => {
    // The steady state must stay instant: no network in the critical path.
    let fetches = 0;
    const { caches } = withGlobals({
      registration: { waiting: null, installing: null, addEventListener() {} },
      fetchImpl: async () => {
        fetches += 1;
        return new Response("FRESH SHELL", { status: 200 });
      },
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("STALE SHELL"));

    const response = await mod.serveNavigation(navEvent());
    assert.equal(
      await response.text(),
      "STALE SHELL",
      "the normal path serves from cache and revalidates behind it",
    );
  });
});

describe("the failure page", () => {
  test("offers only a retry on the first failures", async () => {
    // A transient failure (storage settling, a boot race) should not
    // push the user at a destructive reset.
    withGlobals();
    const mod = await loadWith({ exports: ["failurePage", "workerHealth"] });
    mod.workerHealth.state = "failed";
    mod.workerHealth.error = "boom";
    mod.workerHealth.attempts = 1;

    const html = await mod.failurePage().text();
    assert.match(html, /Try again/);
    assert.ok(!/Reset and reload/.test(html), "no reset ladder yet");
  });

  test("offers the reset ladder once failures persist", async () => {
    // This is the escape hatch from finding 3: the failure page is not
    // the boot shell, so the stall watchdog's clear-and-unregister
    // never runs here. Without this the user reloads forever.
    withGlobals();
    const mod = await loadWith({ exports: ["failurePage", "workerHealth"] });
    mod.workerHealth.state = "failed";
    mod.workerHealth.error = "boom";
    mod.workerHealth.attempts = 5;

    const html = await mod.failurePage().text();
    assert.match(html, /Reset and reload/);
    assert.match(html, /unregister/i, "it actually unregisters, not just reloads");
  });

  test("escapes the error text", async () => {
    withGlobals();
    const mod = await loadWith({ exports: ["failurePage", "workerHealth"] });
    mod.workerHealth.state = "failed";
    mod.workerHealth.error = "<img src=x onerror=alert(1)>";
    mod.workerHealth.attempts = 1;

    const html = await mod.failurePage().text();
    assert.ok(
      !html.includes("<img src=x"),
      "an error string must not become markup on the failure page",
    );
    assert.match(html, /&lt;img/);
  });
});

describe("periodic revocation check", () => {
  test("checks once, then not again within the interval", async () => {
    // The check rides on traffic the worker already serves; it must not
    // become a probe on every navigation.
    let probes = 0;
    withGlobals({
      fetchImpl: async () => {
        probes += 1;
        return new Response(JSON.stringify({ revoked: [] }), { status: 200 });
      },
    });
    const mod = await loadWith({ exports: ["maybeCheckKillSwitch"] });
    const pending = [];
    const event = { waitUntil: (p) => pending.push(p) };

    mod.maybeCheckKillSwitch(event);
    mod.maybeCheckKillSwitch(event);
    mod.maybeCheckKillSwitch(event);
    await Promise.all(pending);

    assert.equal(probes, 1, "the interval gate collapses a burst to one probe");
  });
});

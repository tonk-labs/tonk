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
const WORKER_RS_PATH = join(HERE, "..", "..", "tonk-worker", "src", "worker.rs");
const CACHE_RS_PATH = join(HERE, "..", "..", "tonk-worker", "src", "cache.rs");
const source = readFileSync(SW_PATH, "utf8");

/** A minimal Cache/CacheStorage good enough for the decisions here. */
class FakeCache {
  #entries = new Map();
  mutations = [];
  async match(request) {
    const key = typeof request === "string" ? request : request.url;
    return this.#entries.get(key)?.clone() ?? undefined;
  }
  async put(request, response) {
    const key = typeof request === "string" ? request : request.url;
    this.mutations.push({ type: "put", key });
    this.#entries.set(key, response.clone());
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
    this.mutations.push({ type: "delete", key });
    return this.#entries.delete(key);
  }
  resetMutations() {
    this.mutations.length = 0;
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
  async match(request, options = {}) {
    const cache = this.caches.get(options.cacheName);
    return cache?.match(request);
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
    "\nexport { isShellCacheable, SHELL_CACHE, WORKER_CACHE, BUILD_ID, fetchVerifiedWorkerWasm, digestOf };\n";

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
let loadSerial = 0;

async function loadWith({
  buildId = "testbuild",
  wasmHash = "0000000000000000",
  assetManifestHash = "dev",
  assetPaths = ["/", "/ui-abc.js"],
  activateSource = "async () => ({})",
  exports = [],
}) {
  const body = source
    .replace(/^const BUILD_ID = .*$/m, `const BUILD_ID = "${buildId}";`)
    .replace(/^const WORKER_WASM_HASH = .*$/m, `const WORKER_WASM_HASH = "${wasmHash}";`)
    .replace(
      /^const ASSET_MANIFEST_HASH = .*$/m,
      `const ASSET_MANIFEST_HASH = "${assetManifestHash}";`,
    )
    .replace(
      /^const ASSET_PATHS = .*$/m,
      `const ASSET_PATHS = ${JSON.stringify(assetPaths)};`,
    )
    .replace(
      /^import init, \{ activate \} from "\.\/worker\.js";$/m,
      `const init = async () => {}; const activate = ${activateSource};`,
    );
  const withExports =
    body +
    `\n// isolated test module ${loadSerial++}\nexport { ${exports.join(", ")} };\n`;
  return import(
    "data:text/javascript;base64," + Buffer.from(withExports).toString("base64")
  );
}

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

const utf8 = (text) => new TextEncoder().encode(text);

async function putGenerationMarker(
  caches,
  {
    buildId,
    manifest,
    state = "adopted",
    nonce = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  },
) {
  const metadata = `TONK_GENERATION_${buildId}`;
  const markerUrl = `https://tonk.test/.tonk-generation-${buildId}`;
  const shellStage = `TONK_SHELL_STAGE_${buildId}_${nonce}`;
  const workerStage = `TONK_WORKER_STAGE_${buildId}_${nonce}`;
  await (await caches.open(metadata)).put(
    markerUrl,
    new Response(JSON.stringify({
      version: 1,
      build: buildId,
      manifest,
      state,
      nonce,
      shellStage,
      workerStage,
    })),
  );
  return { metadata, markerUrl, shellStage, workerStage };
}

async function markGenerationAdopted(caches, options) {
  return (await putGenerationMarker(caches, options)).metadata;
}

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
    const module = await loadWith({ exports: ["isShellCacheable"] });
    assert.equal(
      module.isShellCacheable(req("https://tonk.test/ui-abc.js"), "/ui-abc.js"),
      true,
    );
  });

  test("accepts only exact members of the stamped immutable graph", async () => {
    withGlobals();
    const module = await loadWith({
      assetPaths: ["/", "/ui-abc.js"],
      exports: ["isShellCacheable"],
    });

    assert.equal(
      module.isShellCacheable(req("https://tonk.test/ui-abc.js"), "/ui-abc.js"),
      true,
    );
    for (const path of [
      "/.well-known/tonk",
      "/customer/account",
      "/ucan/delegate",
      "/unpublished.js",
    ]) {
      assert.equal(
        module.isShellCacheable(req(`https://tonk.test${path}`), path),
        false,
        `${path} must reach the live edge/Rust route instead of a retained shell cache`,
      );
    }
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

describe("exact fetch routing", () => {
  function fetchEvent(request, clientId = "") {
    let response;
    return {
      event: {
        request,
        clientId,
        waitUntil() {},
        respondWith(value) {
          response = Promise.resolve(value);
        },
      },
      response: () => response,
    };
  }

  test("delegates an OPTIONS health request to the common CORS boundary", async () => {
    const { self } = withGlobals({
      fetchImpl: async () => new Response(new Uint8Array([0])),
    });
    await loadWith({
      wasmHash: "dev",
      activateSource: `async () => ({
        onfetch: async event => new Response(event.request.method, { status: 209 }),
      })`,
    });
    const fetch = fetchEvent(
      new Request("https://tonk.test/api/health", { method: "OPTIONS" }),
    );

    self.onfetch(fetch.event);

    const response = await fetch.response();
    assert.equal(response.status, 209);
    assert.equal(await response.text(), "OPTIONS");
  });

  test("adds CORS to the JavaScript health shortcut", async () => {
    const { self } = withGlobals();
    await loadWith({});
    const fetch = fetchEvent(new Request("https://tonk.test/api/health"));

    self.onfetch(fetch.event);

    const response = await fetch.response();
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("access-control-allow-origin"), "*");
    assert.equal((await response.json()).build, "testbuild");
  });

  test("serves the root shell for controlled application routes", async () => {
    const { self, caches } = withGlobals();
    self.clients.get = async (clientId) => ({ clientId, frameType: "top-level" });
    const mod = await loadWith({
      buildId: "published-build",
      wasmHash: "dev",
      assetPaths: ["/"],
      activateSource: 'async () => { throw new Error("application navigation booted Rust"); }',
      exports: ["SHELL_CACHE"],
    });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("APP SHELL"));

    const app = fetchEvent(
      {
        method: "GET",
        mode: "navigate",
        url: "https://tonk.test/spaces/local",
      },
      "controlled-top-level",
    );
    self.onfetch(app.event);
    assert.equal(await (await app.response()).text(), "APP SHELL");
  });

  test("delegates live edge routes instead of turning them into retained-cache 503s", async () => {
    const { self } = withGlobals({
      fetchImpl: async () => new Response(new Uint8Array([0])),
    });
    await loadWith({
      buildId: "published-build",
      wasmHash: "dev",
      assetPaths: ["/", "/ui-abc.js"],
      activateSource:
        'async () => ({ onfetch: async () => new Response("RUST OR NETWORK") })',
    });
    const live = fetchEvent({
      method: "GET",
      mode: "cors",
      cache: "default",
      url: "https://tonk.test/.well-known/tonk",
    });
    self.onfetch(live.event);
    assert.equal(await (await live.response()).text(), "RUST OR NETWORK");
  });

  test("delegates a nested client's exact-name subresource to Rust before shell lookup", async () => {
    const { self, caches } = withGlobals({
      fetchImpl: async () => new Response(new Uint8Array([0])),
    });
    self.clients.get = async (clientId) => ({ clientId, frameType: "nested" });
    const mod = await loadWith({
      buildId: "published-build",
      wasmHash: "dev",
      assetPaths: ["/", "/ui-abc.js"],
      activateSource:
        'async () => ({ onfetch: async event => new Response(`RUST:${event.clientId}`) })',
      exports: ["SHELL_CACHE"],
    });
    const cache = await caches.open(mod.SHELL_CACHE);
    const request = {
      method: "GET",
      mode: "cors",
      cache: "default",
      url: "https://tonk.test/ui-abc.js",
    };
    await cache.put(request, new Response("TOP-LEVEL ASSET"));

    const guest = fetchEvent(request, "guest-client");
    self.onfetch(guest.event);
    assert.equal(await (await guest.response()).text(), "RUST:guest-client");
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

  test("activation retains caches owned by older generations", () => {
    const worker = readFileSync(WORKER_RS_PATH, "utf8");
    const cache = readFileSync(CACHE_RS_PATH, "utf8");
    assert.doesNotMatch(
      worker,
      /purge_old_caches/,
      "a newly activated worker cannot know whether retained clients still need an older generation",
    );
    assert.doesNotMatch(
      cache,
      /caches\.delete|purge_old_caches/,
      "automatic generation cleanup risks deleting the only offline copy an older worker can boot",
    );
  });
});

describe("immutable generation install", () => {
  test("client discovery cannot stall verified install progress", async () => {
    const { self } = withGlobals();
    self.clients.matchAll = () => new Promise(() => {});
    const mod = await loadWith({ exports: ["reportInstallProgress"] });

    await Promise.race([
      mod.reportInstallProgress("verify", 1, 2, true),
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error("install progress stalled on client discovery")), 100),
      ),
    ]);
  });

  test("installs the complete verified UI and guest graph before going offline", async () => {
    const buildId = "complete-build";
    const wasm = new Uint8Array([9, 8, 7, 6]);
    const wasmHash = (await sha256Hex(wasm)).slice(0, 16);
    const resources = new Map([
      ["/", "<html>complete shell</html>"],
      ["/ui-a1b2c3.js", "complete ui"],
      ["/guest/manifest.json", '{"js":"guest-a1b2c3.js"}'],
      ["/guest/guest-a1b2c3.js", "complete guest glue"],
      ["/guest/guest_bg-a1b2c3.wasm", "complete guest wasm"],
      ["/tonk-prose/tonk-prose-editor.js", "complete lazy editor"],
    ]);
    const assets = Object.fromEntries(
      await Promise.all(
        [...resources].map(async ([path, body]) => [path, await sha256Hex(utf8(body))]),
      ),
    );
    const manifestText = JSON.stringify({ version: 1, build: buildId, assets });
    const manifestHash = await sha256Hex(utf8(manifestText));
    const fetches = [];
    const registration = { active: {}, waiting: null, installing: null, addEventListener() {} };
    const { self, caches } = withGlobals({
      registration,
      fetchImpl: async (input, init) => {
        const raw = typeof input === "string" ? input : input.url;
        const url = new URL(raw, "https://tonk.test");
        fetches.push({ path: url.pathname, cache: init?.cache });
        if (url.pathname === "/asset-manifest.json") {
          return new Response(manifestText, {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        }
        if (url.pathname === "/worker_bg.wasm") {
          return new Response(wasm, {
            status: 200,
            headers: { "content-type": "application/wasm" },
          });
        }
        const body = resources.get(url.pathname);
        return body === undefined
          ? new Response("missing", { status: 404 })
          : new Response(body, { status: 200 });
      },
    });
    let activationRequests = 0;
    self.skipWaiting = async () => {
      activationRequests += 1;
    };
    const progress = [];
    self.clients.matchAll = async (options) => {
      assert.deepEqual(options, { type: "window", includeUncontrolled: true });
      return [{ postMessage: (message) => progress.push(message) }];
    };
    const mod = await loadWith({
      buildId,
      wasmHash,
      assetManifestHash: manifestHash,
      exports: [
        "SHELL_CACHE",
        "WORKER_CACHE",
        "WORKER_WASM_URL",
        "serveAsset",
        "workerWasmModule",
      ],
    });

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });
    await install;
    assert.equal(
      activationRequests,
      1,
      "a verified successor must bridge deployed pages that cannot request activation",
    );

    assert.ok(
      progress.some(({ type, build, phase }) =>
        type === "tonk-install-progress" &&
        build === buildId &&
        phase === "adopted"
      ),
      "a long verified install must keep the uncontrolled boot watchdog alive",
    );

    const shell = await caches.open(mod.SHELL_CACHE);
    for (const [path, body] of resources) {
      const key = path === "/" ? path : `https://tonk.test${path}`;
      const cached = await shell.match(key);
      assert.ok(cached, `${path} must be part of the installed generation`);
      assert.equal(await cached.text(), body);
    }
    const worker = await caches.open(mod.WORKER_CACHE);
    assert.deepEqual(
      new Uint8Array(await (await worker.match(mod.WORKER_WASM_URL)).arrayBuffer()),
      wasm,
    );
    assert.ok(fetches.length >= resources.size + 2);
    assert.ok(
      fetches.every(({ cache }) => cache === "no-store"),
      "installation must not accept bytes from the incidental HTTP cache",
    );

    globalThis.fetch = async () => { throw new TypeError("offline"); };
    assert.deepEqual(new Uint8Array(await mod.workerWasmModule()), wasm);
    const guest = await mod.serveAsset({
      request: { url: "https://tonk.test/guest/guest-a1b2c3.js" },
    });
    assert.equal(await guest.text(), "complete guest glue");
  });

  test("rejects a mismatched asset before opening any generation cache", async () => {
    const buildId = "rejected-build";
    const expectedShell = "expected shell";
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": await sha256Hex(utf8(expectedShell)) },
    });
    const wasm = new Uint8Array([4, 3, 2, 1]);
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        switch (new URL(raw, "https://tonk.test").pathname) {
          case "/asset-manifest.json":
            return new Response(manifestText, { status: 200 });
          case "/worker_bg.wasm":
            return new Response(wasm, { status: 200 });
          case "/":
            return new Response("different live shell", { status: 200 });
          default:
            return new Response("missing", { status: 404 });
        }
      },
    });
    await loadWith({
      buildId,
      wasmHash: (await sha256Hex(wasm)).slice(0, 16),
      assetManifestHash: await sha256Hex(utf8(manifestText)),
    });

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });

    await assert.rejects(install, /asset \/ hash mismatch/);
    assert.equal(
      caches.caches.size,
      0,
      "verification must finish before the incoming build mutates CacheStorage",
    );
  });

  test("refuses an incomplete retained generation without repairing or deleting it", async () => {
    const buildId = "retained-build";
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": "0".repeat(64) },
    });
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        if (new URL(raw, "https://tonk.test").pathname === "/asset-manifest.json") {
          return new Response(manifestText, { status: 200 });
        }
        throw new Error("a retained generation must not fetch replacement assets");
      },
    });
    const manifestHash = await sha256Hex(utf8(manifestText));
    const mod = await loadWith({
      buildId,
      assetManifestHash: manifestHash,
      exports: ["SHELL_CACHE", "WORKER_CACHE"],
    });
    const shell = await caches.open(mod.SHELL_CACHE);
    const worker = await caches.open(mod.WORKER_CACHE);
    await shell.put("/sentinel", new Response("retained"));
    const metadata = await markGenerationAdopted(caches, {
      buildId,
      manifest: manifestHash,
    });
    shell.resetMutations();
    worker.resetMutations();

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });

    await assert.rejects(install, /retained generation cache is incomplete/);
    assert.equal(await (await shell.match("/sentinel")).text(), "retained");
    assert.deepEqual(shell.mutations, []);
    assert.deepEqual(worker.mutations, []);
    assert.deepEqual(new Set(await caches.keys()), new Set([
      mod.SHELL_CACHE,
      mod.WORKER_CACHE,
      metadata,
    ]));
  });

  test("never treats stable cache names without adoption provenance as incoming", async () => {
    const buildId = "unknown-provenance-build";
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": "0".repeat(64) },
    });
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        if (new URL(raw, "https://tonk.test").pathname === "/asset-manifest.json") {
          return new Response(manifestText, { status: 200 });
        }
        throw new Error("unknown retained state must not be repaired from live bytes");
      },
    });
    const mod = await loadWith({
      buildId,
      assetManifestHash: await sha256Hex(utf8(manifestText)),
      exports: ["SHELL_CACHE", "WORKER_CACHE"],
    });
    const shell = await caches.open(mod.SHELL_CACHE);
    const worker = await caches.open(mod.WORKER_CACHE);
    await shell.put("/sentinel", new Response("possibly retained"));
    shell.resetMutations();
    worker.resetMutations();

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });
    await assert.rejects(install, /has no adoption provenance/);

    assert.equal(await (await shell.match("/sentinel")).text(), "possibly retained");
    assert.deepEqual(shell.mutations, []);
    assert.deepEqual(worker.mutations, []);
    assert.deepEqual(
      new Set(await caches.keys()),
      new Set([mod.SHELL_CACHE, mod.WORKER_CACHE]),
    );
  });

  test("does not recreate retained cache names evicted during completeness verification", async () => {
    const buildId = "evicted-retained-build";
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": "0".repeat(64) },
    });
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        if (new URL(raw, "https://tonk.test").pathname === "/asset-manifest.json") {
          return new Response(manifestText, { status: 200 });
        }
        throw new Error("a retained generation must not fetch replacement assets");
      },
    });
    const manifestHash = await sha256Hex(utf8(manifestText));
    const mod = await loadWith({
      buildId,
      assetManifestHash: manifestHash,
      exports: ["SHELL_CACHE", "WORKER_CACHE"],
    });
    await caches.open(mod.SHELL_CACHE);
    await caches.open(mod.WORKER_CACHE);
    await markGenerationAdopted(caches, { buildId, manifest: manifestHash });

    // Model storage pressure between the inventory and the first cache read.
    // Verification must observe the miss without reopening either old name.
    const listKeys = caches.keys.bind(caches);
    let evictAfterInventory = true;
    caches.keys = async () => {
      const names = await listKeys();
      if (evictAfterInventory) {
        evictAfterInventory = false;
        caches.caches.clear();
      }
      return names;
    };

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });

    await assert.rejects(install, /retained generation cache is incomplete/);
    assert.deepEqual(
      await caches.keys(),
      [],
      "a read-only retained-generation check must not recreate an evicted cache name",
    );
  });

  test("removes only a newly-created generation when publication cannot complete", async () => {
    const buildId = "failed-incoming-build";
    const shellBody = "complete before storage failure";
    const wasm = new Uint8Array([1, 4, 1, 4]);
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": await sha256Hex(utf8(shellBody)) },
    });
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        switch (new URL(raw, "https://tonk.test").pathname) {
          case "/asset-manifest.json":
            return new Response(manifestText, { status: 200 });
          case "/worker_bg.wasm":
            return new Response(wasm, { status: 200 });
          case "/":
            return new Response(shellBody, { status: 200 });
          default:
            return new Response("missing", { status: 404 });
        }
      },
    });
    const mod = await loadWith({
      buildId,
      wasmHash: (await sha256Hex(wasm)).slice(0, 16),
      assetManifestHash: await sha256Hex(utf8(manifestText)),
      exports: ["WORKER_CACHE"],
    });
    const open = caches.open.bind(caches);
    caches.open = async (name) => {
      const cache = await open(name);
      if (name === mod.WORKER_CACHE) {
        cache.put = async () => { throw new Error("storage full"); };
      }
      return cache;
    };

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });

    await assert.rejects(install, /storage full/);
    assert.deepEqual(
      await caches.keys(),
      [],
      "a failed incoming install must not leave a partial generation installable",
    );
  });

  test("recovers an interrupted unadopted publication of the same build", async () => {
    const buildId = "interrupted-build";
    const nonce = "0123456789abcdef0123456789abcdef";
    const shellBody = "complete shell after retry";
    const wasm = new Uint8Array([8, 6, 7, 5, 3, 0, 9]);
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": await sha256Hex(utf8(shellBody)) },
    });
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        switch (new URL(raw, "https://tonk.test").pathname) {
          case "/asset-manifest.json":
            return new Response(manifestText, { status: 200 });
          case "/worker_bg.wasm":
            return new Response(wasm, { status: 200 });
          case "/":
            return new Response(shellBody, { status: 200 });
          default:
            return new Response("missing", { status: 404 });
        }
      },
    });
    const mod = await loadWith({
      buildId,
      wasmHash: (await sha256Hex(wasm)).slice(0, 16),
      assetManifestHash: await sha256Hex(utf8(manifestText)),
      exports: ["SHELL_CACHE", "WORKER_CACHE"],
    });
    const { metadata, markerUrl, shellStage, workerStage } =
      await putGenerationMarker(caches, {
        buildId,
        manifest: await sha256Hex(utf8(manifestText)),
        state: "publishing",
        nonce,
      });
    await (await caches.open(shellStage)).put("/interrupted", new Response("partial stage"));
    await (await caches.open(workerStage)).put("/interrupted", new Response("partial stage"));
    await (await caches.open(mod.SHELL_CACHE)).put("/interrupted", new Response("partial final"));
    await caches.open(mod.WORKER_CACHE);

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });
    await install;

    assert.equal(await (await caches.match("/", { cacheName: mod.SHELL_CACHE })).text(), shellBody);
    assert.deepEqual(
      new Set(await caches.keys()),
      new Set([metadata, mod.SHELL_CACHE, mod.WORKER_CACHE]),
      "retry keeps only the adopted finals and durable generation metadata",
    );
    const marker = JSON.parse(
      await (await caches.match(markerUrl, { cacheName: metadata })).text(),
    );
    assert.equal(marker.state, "adopted");
  });

  test("recovers an interrupted uniquely-staged build before publication", async () => {
    const buildId = "interrupted-staging-build";
    const nonce = "fedcba9876543210fedcba9876543210";
    const shellBody = "complete shell after staging retry";
    const wasm = new Uint8Array([2, 7, 1, 8, 2, 8]);
    const manifestText = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": await sha256Hex(utf8(shellBody)) },
    });
    const manifestHash = await sha256Hex(utf8(manifestText));
    const { self, caches } = withGlobals({
      fetchImpl: async (input) => {
        const raw = typeof input === "string" ? input : input.url;
        switch (new URL(raw, "https://tonk.test").pathname) {
          case "/asset-manifest.json":
            return new Response(manifestText, { status: 200 });
          case "/worker_bg.wasm":
            return new Response(wasm, { status: 200 });
          case "/":
            return new Response(shellBody, { status: 200 });
          default:
            return new Response("missing", { status: 404 });
        }
      },
    });
    const mod = await loadWith({
      buildId,
      wasmHash: (await sha256Hex(wasm)).slice(0, 16),
      assetManifestHash: manifestHash,
      exports: ["SHELL_CACHE", "WORKER_CACHE"],
    });
    const { metadata, markerUrl, shellStage, workerStage } =
      await putGenerationMarker(caches, {
        buildId,
        manifest: manifestHash,
        state: "building",
        nonce,
      });
    await (await caches.open(shellStage)).put("/interrupted", new Response("partial stage"));
    await (await caches.open(workerStage)).put("/interrupted", new Response("partial stage"));

    let install;
    self.oninstall({ waitUntil: (promise) => { install = promise; } });
    await install;

    assert.equal(await (await caches.match("/", { cacheName: mod.SHELL_CACHE })).text(), shellBody);
    assert.deepEqual(
      new Set(await caches.keys()),
      new Set([metadata, mod.SHELL_CACHE, mod.WORKER_CACHE]),
      "retry removes only the marker-owned stage names before adopting fresh finals",
    );
    const marker = JSON.parse(
      await (await caches.match(markerUrl, { cacheName: metadata })).text(),
    );
    assert.equal(marker.state, "adopted");
    assert.notEqual(marker.nonce, nonce, "the retry publishes through a fresh unique stage");
  });
});

describe("worker wasm verification", () => {
  test("rejects wasm whose hash does not match the stamp", async () => {
    // The heart of the audit's finding 1. Glue and wasm are tightly
    // coupled, so installing a worker whose wasm is not the one built
    // alongside it is an init failure at best and silent miswiring at
    // worst. The install MUST fail so the old, internally consistent
    // worker keeps running.
    const { caches } = withGlobals({
      fetchImpl: async () => new Response(new Uint8Array([1, 2, 3]), { status: 200 }),
    });
    const module = await loadWith({ exports: ["fetchVerifiedWorkerWasm"] });

    await assert.rejects(
      () => module.fetchVerifiedWorkerWasm(),
      /hash mismatch/,
      "a mismatched pair must fail the install, not install anyway",
    );
    assert.equal(caches.caches.size, 0);
  });

  test("returns matching bytes without opening a cache", async () => {
    const bytes = new Uint8Array([1, 2, 3, 4]);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const hex = Array.from(new Uint8Array(digest))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 16);

    const { caches } = withGlobals({
      fetchImpl: async () => new Response(bytes, { status: 200 }),
    });
    const module = await loadWith({
      wasmHash: hex,
      exports: ["fetchVerifiedWorkerWasm"],
    });

    assert.deepEqual(new Uint8Array(await module.fetchVerifiedWorkerWasm()), bytes);
    assert.equal(caches.caches.size, 0, "verification itself is a read-only operation");
  });

  test("fails the install when the wasm cannot be fetched", async () => {
    withGlobals({ fetchImpl: async () => new Response("", { status: 503 }) });
    const module = await loadWith({ exports: ["fetchVerifiedWorkerWasm"] });

    await assert.rejects(() => module.fetchVerifiedWorkerWasm(), /fetch failed/);
  });
});

describe("booting from the precached wasm", () => {
  test("verifies an eviction recovery fetch without backfilling the retained cache", async () => {
    // Storage pressure can evict the entry. The old worker may use live bytes
    // only after proving they still match its stamped glue; it must not mutate
    // a retained generation while doing so.
    const bytes = new Uint8Array([7, 7]);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const hex = Array.from(new Uint8Array(digest))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 16);
    let fetches = 0;
    const { caches } = withGlobals({
      fetchImpl: async () => {
        fetches += 1;
        return new Response(bytes, { status: 200 });
      },
    });
    const mod = await loadWith({
      wasmHash: hex,
      exports: ["workerWasmModule", "WORKER_CACHE"],
    });
    assert.equal(caches.caches.has(mod.WORKER_CACHE), false);
    const buf = await mod.workerWasmModule();
    assert.equal(buf.byteLength, 2);
    assert.equal(fetches, 1, "an evicted entry is refetched exactly once");
    assert.equal(
      caches.caches.has(mod.WORKER_CACHE),
      false,
      "runtime recovery must not recreate or backfill a retained TONK_WORKER cache",
    );
  });

  test("fails loudly when the wasm cannot be recovered", async () => {
    // Better a visible failure with non-destructive retry/update guidance
    // than a worker that half-boots.
    withGlobals({ fetchImpl: async () => new Response("", { status: 500 }) });
    const mod = await loadWith({ exports: ["workerWasmModule"] });
    await assert.rejects(() => mod.workerWasmModule());
  });
});

describe("navigation while an update is waiting", () => {
  const navEvent = () => ({
    url: "https://tonk.test/",
    waitUntil: (p) => { void p?.catch?.(() => {}); },
  });

  test("keeps serving the outgoing shell until the claim-and-reload handoff", async () => {
    let fetches = 0;
    const { caches } = withGlobals({
      registration: { waiting: {}, installing: null, addEventListener() {} },
      fetchImpl: async (input) => {
        if (input === "/") fetches += 1;
        return new Response("FRESH SHELL", { status: 200 });
      },
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("STALE SHELL"));

    const response = await mod.serveNavigation(navEvent());
    assert.equal(await response.text(), "STALE SHELL");
    assert.equal(
      fetches,
      0,
      "an outgoing controller must not accept the next deployment's stable-name shell",
    );
  });

  test("keeps the cached outgoing shell available while offline", async () => {
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
      "the normal path serves its sealed generation",
    );
  });
});

describe("immutable generation caches", () => {
  const navEvent = (pending) => ({
    url: "https://tonk.test/",
    waitUntil: (promise) => pending.push(Promise.resolve(promise)),
  });

  test("a waiting successor never reads or rewrites the outgoing generation shell from the network", async () => {
    const pending = [];
    let fetches = 0;
    const { caches } = withGlobals({
      registration: { waiting: {}, installing: null, addEventListener() {} },
      fetchImpl: async (input) => {
        if (input === "/") fetches += 1;
        return new Response("NEW GENERATION", { status: 200 });
      },
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("OLD GENERATION"));
    cache.resetMutations();

    const response = await mod.serveNavigation(navEvent(pending));
    await Promise.all(pending);

    assert.equal(await response.text(), "OLD GENERATION");
    assert.equal(fetches, 0);
    assert.deepEqual(
      cache.mutations,
      [],
      "the outgoing worker must leave its sealed cache byte-for-byte unchanged",
    );
    assert.equal(await (await cache.match("/")).text(), "OLD GENERATION");
  });

  test("ordinary navigation never revalidates or prunes a retained generation", async () => {
    const pending = [];
    let fetches = 0;
    const { caches } = withGlobals({
      registration: { waiting: null, installing: null, addEventListener() {} },
      fetchImpl: async () => {
        fetches += 1;
        return new Response("NEW GENERATION", { status: 200 });
      },
    });
    const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });
    const cache = await caches.open(mod.SHELL_CACHE);
    await cache.put("/", new Response("OLD GENERATION"));
    await cache.put("https://tonk.test/old-hash.js", new Response("old asset"));
    cache.resetMutations();

    const response = await mod.serveNavigation(navEvent(pending));
    await Promise.all(pending);

    assert.equal(await response.text(), "OLD GENERATION");
    assert.equal(fetches, 0, "a sealed generation has nothing to revalidate");
    assert.deepEqual(cache.mutations, [], "no old entry may be overwritten or deleted");
  });

  test("cached static assets stay byte-for-byte immutable online and offline", async () => {
    for (const online of [true, false]) {
      const pending = [];
      let fetches = 0;
      const { caches } = withGlobals({
        fetchImpl: async () => {
          fetches += 1;
          if (!online) throw new TypeError("offline");
          return new Response("new asset", { status: 200 });
        },
      });
      const mod = await loadWith({ exports: ["serveAsset", "SHELL_CACHE"] });
      const cache = await caches.open(mod.SHELL_CACHE);
      const request = { url: "https://tonk.test/app-old.js" };
      await cache.put(request, new Response("old asset"));
      cache.resetMutations();

      const response = await mod.serveAsset({
        request,
        waitUntil: (promise) => pending.push(Promise.resolve(promise)),
      });
      await Promise.all(pending);

      assert.equal(await response.text(), "old asset");
      assert.equal(fetches, 0, "a retained asset is never refreshed from the live deployment");
      assert.deepEqual(cache.mutations, []);
    }
  });

  test("production cache-bypass flags cannot escape the sealed generation", async () => {
    const request = {
      method: "GET",
      mode: "cors",
      cache: "no-store",
      url: "https://tonk.test/tonk-prose/tonk-prose-editor.js",
    };
    withGlobals();
    const retained = await loadWith({
      buildId: "retained-production-build",
      assetPaths: ["/", "/tonk-prose/tonk-prose-editor.js"],
      exports: ["isShellCacheable"],
    });
    assert.equal(
      retained.isShellCacheable(request, "/tonk-prose/tonk-prose-editor.js"),
      true,
      "a caller cannot force an old production controller to accept live stable-name bytes",
    );

    withGlobals();
    const development = await loadWith({
      buildId: "dev",
      exports: ["isShellCacheable"],
    });
    assert.equal(
      development.isShellCacheable(request, "/tonk-prose/tonk-prose-editor.js"),
      false,
      "the unstamped Trunk hot-reload client still needs explicit live reads",
    );
  });

  test("a missing retained asset accepts only verified same-generation bytes", async () => {
    const buildId = "retained-build";
    const path = "/missing-old.js";
    const bytes = utf8("retained asset");
    const hash = await sha256Hex(bytes);
    const manifest = JSON.stringify({
      version: 1,
      build: buildId,
      assets: { "/": "0".repeat(64), [path]: hash },
    });
    const fetches = [];
    const { caches } = withGlobals({
      fetchImpl: async (input) => {
        const url = new URL(typeof input === "string" ? input : input.url);
        fetches.push(url.pathname);
        if (url.pathname === "/asset-manifest.json") {
          return new Response(manifest, { status: 200 });
        }
        if (url.pathname === path) return new Response(bytes, { status: 200 });
        return new Response("", { status: 404 });
      },
    });
    const mod = await loadWith({
      buildId,
      assetPaths: ["/", path],
      exports: ["serveAsset", "SHELL_CACHE", "WORKER_CACHE"],
    });

    const response = await mod.serveAsset({
      request: { url: `https://tonk.test${path}?v=2` },
    });
    assert.equal(await response.text(), "retained asset");
    assert.deepEqual(fetches, ["/asset-manifest.json", path]);
    assert.equal(caches.caches.has(mod.SHELL_CACHE), false);
    assert.equal(caches.caches.has(mod.WORKER_CACHE), false);
  });

  test("a missing retained asset fails closed offline or on a hash mismatch", async () => {
    for (const failure of ["offline", "mismatch"]) {
      const buildId = `retained-${failure}`;
      const path = "/missing-old.js";
      const manifest = JSON.stringify({
        version: 1,
        build: buildId,
        assets: { "/": "0".repeat(64), [path]: "1".repeat(64) },
      });
      const { caches } = withGlobals({
        fetchImpl: async (input) => {
          if (failure === "offline") throw new TypeError("offline");
          const url = new URL(typeof input === "string" ? input : input.url);
          return url.pathname === "/asset-manifest.json"
            ? new Response(manifest, { status: 200 })
            : new Response("wrong bytes", { status: 200 });
        },
      });
      const mod = await loadWith({
        buildId,
        assetPaths: ["/", path],
        exports: ["serveAsset", "SHELL_CACHE", "WORKER_CACHE"],
      });

      const response = await mod.serveAsset({
        request: { url: `https://tonk.test${path}` },
      });
      assert.equal(response.status, 503);
      assert.match(await response.text(), /retained Tonk version.*reload/i);
      assert.equal(caches.caches.has(mod.SHELL_CACHE), false);
      assert.equal(caches.caches.has(mod.WORKER_CACHE), false);
    }
  });

  test("an evicted shell fails closed instead of loading a new generation under the old controller", async () => {
    for (const waiting of [null, {}]) {
      const fetches = [];
      withGlobals({
        registration: { waiting, installing: null, addEventListener() {} },
        fetchImpl: async (input) => {
          fetches.push(new URL(typeof input === "string" ? input : input.url).pathname);
          return new Response("LIVE SHELL", { status: 200 });
        },
      });
      const { caches } = globalThis;
      const mod = await loadWith({ exports: ["serveNavigation", "SHELL_CACHE"] });

      const response = await mod.serveNavigation({
        url: "https://tonk.test/",
        waitUntil() {},
      });

      assert.equal(response.status, 503);
      assert.match(await response.text(), /retained Tonk version.*reload/i);
      assert.equal(
        fetches.filter((path) => path === "/").length,
        0,
        "the navigation must not accept the live stable-name shell",
      );
      assert.equal(
        caches.caches.has(mod.SHELL_CACHE),
        false,
        "an eviction miss must not recreate an empty retained shell cache",
      );
    }
  });

  test("the Rust miss path cannot populate or revalidate a generation cache", () => {
    const cache = readFileSync(CACHE_RS_PATH, "utf8");
    assert.doesNotMatch(
      cache,
      /open_cache|\.open\(/,
      "a runtime read must not recreate an evicted TONK_SHELL cache",
    );
    assert.doesNotMatch(
      cache,
      /put_with_request|async fn revalidate|spawn_local/,
      "old workers must never write current deployment bytes into their generation",
    );
    assert.doesNotMatch(
      cache,
      /sw_fetch|network request on a miss|may work online/,
      "a complete retained generation must not accept live stable-name bytes on an eviction miss",
    );
    assert.match(
      cache,
      /set_status\(503\)/,
      "the Rust-side miss must return the same coherent retained-generation failure",
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

  test("keeps repeated-failure recovery non-destructive", async () => {
    withGlobals();
    const mod = await loadWith({ exports: ["failurePage", "workerHealth"] });
    mod.workerHealth.state = "failed";
    mod.workerHealth.error = "boom";
    mod.workerHealth.attempts = 5;

    const html = await mod.failurePage().text();
    assert.match(html, /Try again/);
    assert.match(html, /check for update|reload/i);
    assert.doesNotMatch(html, /Reset and reload|unregister|caches\.delete|getRegistrations/);
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

describe("retiring on a waiting successor", () => {
  // Finding 2 again, from the angle the first fix missed. `updatefound`
  // is an EVENT, and a service worker is killed and restarted
  // constantly — so the event fires into a dead worker whenever the
  // successor installs while this one is asleep. Worse, a worker
  // holding an open SSE stream is never killed (the stream keeps it
  // alive), so it also never restarts to notice. Both gaps leave the
  // successor stuck in `waiting` with reloads landing on the old active
  // worker: the "waiting to activate" state that never clears.

  test("checks the registration at startup, not just on the event", () => {
    // The registration is durable where the event is not.
    const swSource = readFileSync(SW_PATH, "utf8");
    const beforeListener = swSource.slice(
      0,
      swSource.indexOf('addEventListener?.("updatefound"'),
    );
    assert.match(
      beforeListener,
      /if \(self\.registration\.waiting\)/,
      "a worker that slept through updatefound must still notice at startup",
    );
  });

  test("also checks on the fetch path", () => {
    // A worker pinned by its own streams never restarts, so startup
    // alone is not enough; fetches are its only remaining contact.
    const swSource = readFileSync(SW_PATH, "utf8");
    const onfetch = swSource.slice(swSource.indexOf("self.onfetch = event =>"));
    assert.match(
      onfetch,
      /retireIfSuperseded\(event\)/,
      "the fetch path must check too — a stream-pinned worker never restarts",
    );
  });

  test("declares its state before every use", () => {
    // A `let` read before its declaration is a runtime TDZ error, not a
    // hoist — and `node --check` does not catch it, exactly like the
    // cross-module `isRevoked` bug.
    const swSource = readFileSync(SW_PATH, "utf8");
    const decl = swSource.indexOf("let retired = false;");
    assert.ok(decl > 0, "expected a `retired` declaration");
    const firstUse = swSource.search(/\bretired\s*=\s*true|\bif \(retired\b/);
    assert.ok(
      firstUse > decl,
      "`retired` is used before it is declared — a TDZ error at runtime",
    );
  });
});

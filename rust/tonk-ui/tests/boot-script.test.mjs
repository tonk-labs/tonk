import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, test } from "node:test";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const INDEX = join(HERE, "..", "index.html");
const HOT_SWAP = join(HERE, "..", "assets", "hot-swap.js");

function moduleBlocks() {
  const html = readFileSync(INDEX, "utf8");
  return [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)]
    .map((match) => match[1]);
}

function moduleBlockContaining(needle) {
  const matches = moduleBlocks().filter((block) => block.includes(needle));
  assert.equal(matches.length, 1, `expected one module containing ${needle}`);
  return matches[0];
}

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
    async dispatch(type, event = {}) {
      await Promise.all(
        [...(listeners.get(type) ?? [])].map((listener) => listener(event)),
      );
    },
  });
}

class FakeBroadcastChannel {
  addEventListener() {}
  close() {}
}

function bootHarness({
  serviceWorkers = null,
  userAgent = "Mozilla/5.0 Firefox/142.0",
  vendor = "",
  isSecureContext = true,
} = {}) {
  const logs = [];
  const terminalFailures = [];
  const status = {
    textContent: "loading…",
    setAttribute() {},
  };
  const self = eventTarget({
    isSecureContext,
    tonkBootTerminal(message, title) {
      terminalFailures.push({ message, title });
    },
  });
  const context = {
    self,
    window: self,
    navigator: {
      ...(serviceWorkers ? { serviceWorker: serviceWorkers } : {}),
      userAgent,
      vendor,
    },
    document: eventTarget({
      querySelector(selector) {
        return selector === "[data-boot-status]" ? status : null;
      },
      visibilityState: "visible",
    }),
    BroadcastChannel: FakeBroadcastChannel,
    sessionStorage: {
      getItem() { return null; },
      setItem() {},
      removeItem() {},
    },
    location: { reload() {} },
    console: {
      log(...args) { logs.push(args); },
      warn(...args) { logs.push(args); },
      error(...args) { logs.push(args); },
    },
    Event,
    Number,
    Promise,
    setTimeout,
    clearTimeout,
  };
  return { context, logs, self, status, terminalFailures };
}

describe("boot script contract", () => {
  test("publishes immutable document provenance before the Rust loader", () => {
    const html = readFileSync(INDEX, "utf8");
    const rustLoader = html.indexOf('data-trunk\n            rel="rust"');
    const scripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
      .filter((match) => match[1].includes('meta[name="tonk-worker-build"]'));
    assert.equal(scripts.length, 1);
    assert.ok(scripts[0].index < rustLoader);

    for (const [build, expected] of [
      ["0123456789abcdef", "0123456789abcdef"],
      ["dev", undefined],
      ["AAAAAAAAAAAAAAAA", undefined],
    ]) {
      const context = { document: { querySelector: () => ({ content: build }) } };
      runInNewContext(scripts[0][1], context);
      assert.equal(context.tonkBuild, expected);
    }
  });

  test("uses one load-time update check without polling or an update prompt", () => {
    const lifecycle = moduleBlockContaining("serviceWorker.register");
    assert.equal([...lifecycle.matchAll(/registration\.update\(\)/g)].length, 1);
    assert.doesNotMatch(lifecycle, /setInterval|visibilitychange[\s\S]*registration\.update/);
    assert.doesNotMatch(lifecycle, /\/version\.json|kill-switch|Not now|announceUpdate/);
    assert.doesNotMatch(lifecycle, /type:\s*["']activate["']/);
    assert.doesNotMatch(
      lifecycle,
      /incoming\?\.state === "activated"[\s\S]{0,500}type: "claim"/,
      "activation itself replaces the controller of an already-controlled document",
    );
  });

  test("consumes the alignment guard before considering another update", () => {
    const lifecycle = moduleBlockContaining("serviceWorker.register");
    const guard = lifecycle.indexOf("if (alignmentReload)");
    const update = lifecycle.indexOf("await registration.update()");
    assert.ok(guard >= 0 && guard < update);
    assert.match(
      lifecycle.slice(guard, update),
      /sessionStorage\.removeItem\(UPGRADE_RELOAD\)[\s\S]*return/,
    );
  });

  test("a normal application document consumes the eviction recovery guard", () => {
    const lifecycle = moduleBlockContaining("serviceWorker.register");
    const consume = lifecycle.indexOf("sessionStorage.removeItem(EVICTION_RELOAD)");
    const register = lifecycle.indexOf("navigator.serviceWorker.register");
    assert.ok(consume >= 0 && consume < register);
  });

  test("keeps deferred account safety and remote withdrawal out of production", () => {
    const html = readFileSync(INDEX, "utf8");
    assert.doesNotMatch(html, /tonk-update-safety-v1|account-setup-critical|kill-switch\.json/);
    assert.doesNotMatch(html, /Reload[\s/]+Not now|Not now/);
  });

  test("development hot swap reloads directly without a missing account gate", () => {
    const source = readFileSync(HOT_SWAP, "utf8");
    assert.equal([...source.matchAll(/window\.location\.reload\(\)/g)].length, 2);
    assert.doesNotMatch(source, /tonkReloadWhenAccountSetupDurable/);
  });

  test("activation failures retain safe actionable copy", () => {
    const lifecycle = moduleBlockContaining("serviceWorkerActivation.catch");
    assert.match(lifecycle, /Your local data is safe\./);
    assert.match(lifecycle, /normal Safari tab/);
    assert.match(lifecycle, /service workers/);
    assert.doesNotMatch(lifecycle, /Tonk could not start:\s*\$\{/);
  });

  test("unsupported Safari gives Private Browsing guidance without API access", async () => {
    const result = bootHarness({
      userAgent:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) " +
        "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15",
      vendor: "Apple Computer, Inc.",
    });
    assert.doesNotThrow(() =>
      runInNewContext(moduleBlockContaining("serviceWorkerActivation.catch"), result.context),
    );
    await assert.rejects(
      result.self.serviceWorkerActivates(),
      /Service workers not supported/,
    );
    await new Promise(setImmediate);
    assert.deepEqual(result.terminalFailures, [{
      title: "Open Tonk in a normal Safari tab",
      message:
        "Tonk can’t use the browser storage it needs in this Safari tab. " +
        "If you’re using Private Browsing, open this page in a normal Safari tab. " +
        "Otherwise, update Safari and try again.",
    }]);
  });

  test("Safari capability rejection gets the same normal-tab guidance", async () => {
    const serviceWorkers = eventTarget({
      controller: null,
      async register() {
        const error = new Error("Service workers are unavailable");
        error.name = "SecurityError";
        throw error;
      },
    });
    const result = bootHarness({
      serviceWorkers,
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) " +
        "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1",
      vendor: "Apple Computer, Inc.",
    });
    runInNewContext(moduleBlockContaining("serviceWorkerActivation.catch"), result.context);
    await assert.rejects(result.self.serviceWorkerActivates(), /unavailable/);
    await new Promise(setImmediate);
    assert.deepEqual(result.terminalFailures, [{
      title: "Open Tonk in a normal Safari tab",
      message:
        "Tonk can’t use the browser storage it needs in this Safari tab. " +
        "If you’re using Private Browsing, open this page in a normal Safari tab. " +
        "Otherwise, update Safari and try again.",
    }]);
  });

  test("an unknown unsupported browser gets a service-worker explanation", async () => {
    const result = bootHarness();
    assert.doesNotThrow(() =>
      runInNewContext(moduleBlockContaining("serviceWorkerActivation.catch"), result.context),
    );
    await assert.rejects(
      result.self.serviceWorkerActivates(),
      /Service workers not supported/,
    );
    await new Promise(setImmediate);
    assert.deepEqual(result.terminalFailures, [{
      title: "This browser can’t run Tonk",
      message:
        "Tonk needs service workers to keep your spaces available on this device and work offline. " +
        "Try a current version of Safari, Chrome, Firefox, or Edge.",
    }]);
  });

  test("an insecure page explains that Tonk must be opened over HTTPS", async () => {
    const result = bootHarness({ isSecureContext: false });
    runInNewContext(moduleBlockContaining("serviceWorkerActivation.catch"), result.context);
    await assert.rejects(
      result.self.serviceWorkerActivates(),
      /Service workers not supported/,
    );
    await new Promise(setImmediate);
    assert.deepEqual(result.terminalFailures, [{
      title: "Tonk needs a secure connection",
      message:
        "Tonk can’t access the browser storage it needs from this address. " +
        "Open this page over HTTPS to continue.",
    }]);
  });

  test("a Safari registration MIME failure receives generic recovery", async () => {
    let registrations = 0;
    const serviceWorkers = eventTarget({
      controller: null,
      async register() {
        registrations += 1;
        throw new TypeError("module script has an unsupported MIME type");
      },
    });
    const result = bootHarness({
      serviceWorkers,
      userAgent:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) " +
        "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15",
      vendor: "Apple Computer, Inc.",
    });
    runInNewContext(moduleBlockContaining("serviceWorkerActivation.catch"), result.context);
    await assert.rejects(result.self.serviceWorkerActivates(), /MIME type/);
    await new Promise(setImmediate);
    assert.equal(registrations, 1);
    assert.deepEqual(result.terminalFailures, [{
      title: "Tonk couldn’t start",
      message: "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.",
    }]);
  });

  test("a rejected registration has one observer, one terminal report, and no module rethrow", async () => {
    const serviceWorkers = eventTarget({
      controller: null,
      async register() { throw new TypeError("registration failed"); },
    });
    const result = bootHarness({ serviceWorkers });
    const evaluations = moduleBlocks().map((block) =>
      runInNewContext(`(async () => {${block}\n})()`, result.context),
    );
    await assert.doesNotReject(() => Promise.all(evaluations));
    await assert.rejects(result.self.serviceWorkerActivates(), /registration failed/);
    await new Promise(setImmediate);
    assert.equal(
      result.logs.filter(([message]) =>
        String(message).toLowerCase() === "service-worker activation failed"
      ).length,
      1,
    );
    assert.deepEqual(result.terminalFailures, [{
      title: "Tonk couldn’t start",
      message: "Tonk couldn’t start. Check your connection, then reload. Your local data is safe.",
    }]);
  });
});

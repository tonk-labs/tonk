import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
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
        dispatch(type) {
            for (const listener of listeners.get(type) ?? []) listener();
        },
    });
}

class FakeCache {
    async add() {}
    async match() {}
    async put() {}
    async keys() {
        return [];
    }
    async delete() {
        return false;
    }
}

class FakeCacheStorage {
    async open() {
        return new FakeCache();
    }
    async keys() {
        return [];
    }
    async delete() {
        return false;
    }
}

function loadServiceWorker() {
    let claims = 0;
    const registration = eventTarget({ installing: null, waiting: null });
    const scope = eventTarget({
        location: {
            href: "https://tonk.test/service_worker.js",
            origin: "https://tonk.test",
        },
        registration,
        serviceWorker: {},
        async skipWaiting() {},
        clients: {
            async claim() {
                claims += 1;
            },
            async matchAll() {
                return [];
            },
        },
    });
    const source = readFileSync(SERVICE_WORKER, "utf8").replace(
        /^import init, \{ activate \} from "\.\/worker\.js";$/m,
        "const init = async () => {}; const activate = async () => ({ onactivate: async () => {} });",
    );
    const quietConsole = { log() {}, warn() {}, error() {} };
    vm.runInNewContext(
        source,
        {
            self: scope,
            caches: new FakeCacheStorage(),
            fetch: async () => new Response("", { status: 404 }),
            console: quietConsole,
            navigator: { onLine: true },
            Response,
            Request,
            URL,
            Date,
            setTimeout,
            clearTimeout,
        },
        { filename: SERVICE_WORKER },
    );
    return { scope, claims: () => claims };
}

function activationBlock() {
    const html = readFileSync(INDEX, "utf8");
    const blocks = [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)]
        .map((match) => match[1])
        .filter((block) => block.includes("const serviceWorkerActivation"));
    assert.equal(blocks.length, 1, "expected one service-worker activation block");
    return blocks[0];
}

function runWarmUpdatePage() {
    const messages = [];
    let reloads = 0;
    const oldWorker = eventTarget({ state: "activated", postMessage() {} });
    let serviceWorkers;
    const incoming = eventTarget({
        state: "activated",
        postMessage(message) {
            messages.push(message);
            if (message?.type === "claim") {
                serviceWorkers.controller = incoming;
                serviceWorkers.dispatch("controllerchange");
            }
        },
    });
    const registration = eventTarget({
        active: incoming,
        installing: incoming,
        waiting: null,
        async update() {},
    });
    serviceWorkers = eventTarget({
        controller: oldWorker,
        ready: Promise.resolve(registration),
        async register() {
            return registration;
        },
    });
    const storage = new Map();
    const self = eventTarget({ tonkBootLife() {} });
    const document = eventTarget({
        visibilityState: "visible",
        querySelector() {
            return { textContent: "" };
        },
    });
    vm.runInNewContext(
        activationBlock(),
        {
            self,
            window: {},
            document,
            navigator: { serviceWorker: serviceWorkers },
            sessionStorage: {
                getItem(key) {
                    return storage.get(key) ?? null;
                },
                setItem(key, value) {
                    storage.set(key, String(value));
                },
                removeItem(key) {
                    storage.delete(key);
                },
            },
            location: {
                reload() {
                    reloads += 1;
                },
            },
            console: { log() {}, warn() {}, error() {} },
        },
        { filename: INDEX },
    );
    return { messages, reloads: () => reloads, storage };
}

test("activation alone does not claim pre-upgrade pages", async () => {
    const { scope, claims } = loadServiceWorker();
    const pending = [];
    scope.onactivate({ waitUntil: (promise) => pending.push(promise) });
    await Promise.all(pending);
    assert.equal(claims(), 0);
});

test("an explicit cold-start claim message takes control", async () => {
    const { scope, claims } = loadServiceWorker();
    const pending = [];
    scope.onmessage({
        data: { type: "claim" },
        waitUntil: (promise) => pending.push(promise),
    });
    await Promise.all(pending);
    assert.equal(claims(), 1);
});

test("an update-capable page claims an activated successor before one reload", async () => {
    const result = runWarmUpdatePage();
    await new Promise(setImmediate);
    assert.equal(result.messages.length, 1);
    assert.equal(result.messages[0]?.type, "claim");
    assert.equal(result.reloads(), 1);
    assert.equal(result.storage.get("tonk:sw-upgrade-reload"), "1");
});

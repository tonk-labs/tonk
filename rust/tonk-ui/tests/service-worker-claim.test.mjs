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
        dispatch(type, event = {}) {
            return Promise.all(
                [...(listeners.get(type) ?? [])].map((listener) => listener(event)),
            );
        },
        dispatchEvent(event) {
            void this.dispatch(event.type, event);
            return true;
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
    async match() {}
    async delete() {
        return false;
    }
}

const VALID_HOLD = Object.freeze({
    version: 1,
    kind: "account-setup",
    operationId: "a".repeat(64),
    leasedRevision: "7",
});

class FakeIndexedDB {
    constructor({ hold, failOpen = false, failRead = false, blockThenOpen = false } = {}) {
        this.failOpen = failOpen;
        this.failRead = failRead;
        this.blockThenOpen = blockThenOpen;
        this.closed = 0;
        this.created = false;
        this.values = new Map();
        if (hold !== undefined) this.values.set("account-setup", hold);
    }

    open(name, version) {
        const request = {};
        queueMicrotask(() => {
            if (this.failOpen) {
                request.error = new Error("indexeddb unavailable");
                request.onerror?.();
                return;
            }
            if (this.blockThenOpen) request.onblocked?.();
            const database = {
                objectStoreNames: {
                    contains: (store) => this.created && store === "holds",
                },
                createObjectStore: (store) => {
                    if (store !== "holds") throw new Error("unexpected store");
                    this.created = true;
                },
                transaction: (store) => {
                    if (!this.created || store !== "holds") {
                        throw new Error("missing holds store");
                    }
                    const transaction = {
                        objectStore: () => ({
                            get: (key) => {
                                const get = {};
                                queueMicrotask(() => {
                                    if (this.failRead) {
                                        get.error = new Error("read failed");
                                        get.onerror?.();
                                        transaction.error = get.error;
                                        transaction.onerror?.();
                                        return;
                                    }
                                    get.result = this.values.get(key);
                                    get.onsuccess?.();
                                    queueMicrotask(() => transaction.oncomplete?.());
                                });
                                return get;
                            },
                        }),
                        abort() {},
                    };
                    return transaction;
                },
                close: () => {
                    this.closed += 1;
                },
            };
            request.result = database;
            if (!this.created) request.onupgradeneeded?.();
            request.onsuccess?.();
        });
        return request;
    }
}

function broadcastHarness() {
    const channels = new Set();
    class FakeBroadcastChannel extends EventTarget {
        constructor(name) {
            super();
            this.name = name;
            channels.add(this);
        }
        postMessage(data) {
            for (const channel of channels) {
                if (channel !== this && channel.name === this.name) {
                    channel.dispatchEvent(new MessageEvent("message", { data }));
                }
            }
        }
        close() {
            channels.delete(this);
        }
    }
    return {
        BroadcastChannel: FakeBroadcastChannel,
        signal(data = { type: "account-setup-hold-changed", version: 1 }) {
            const sender = new FakeBroadcastChannel("tonk-update-safety-v1");
            sender.postMessage(data);
            sender.close();
        },
    };
}

function lockHarness({ available = true } = {}) {
    const requests = [];
    let held = false;
    let tail = Promise.resolve();
    if (!available) return { locks: undefined, requests, held: () => held };
    return {
        requests,
        held: () => held,
        locks: {
            request(name, options, callback) {
                requests.push({ name, options });
                const run = tail.then(async () => {
                    held = true;
                    try {
                        return await callback();
                    } finally {
                        held = false;
                    }
                });
                tail = run.catch(() => {});
                return run;
            },
        },
    };
}

function loadServiceWorker({ indexedDB = new FakeIndexedDB(), locks } = {}) {
    let claims = 0;
    let retirements = 0;
    let activationRequests = 0;
    const logs = [];
    const registration = eventTarget({ installing: null, waiting: null });
    const lockState = locks === undefined ? lockHarness() : { locks, requests: [] };
    const navigator = { onLine: true, locks: lockState.locks };
    const scope = eventTarget({
        location: {
            href: "https://tonk.test/service_worker.js",
            origin: "https://tonk.test",
        },
        registration,
        serviceWorker: {},
        navigator,
        async skipWaiting() {
            activationRequests += 1;
        },
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
        "const init = async () => {}; const activate = async () => ({ onactivate: async () => {}, onupdatefound: async () => recordRetirement() });",
    );
    const quietConsole = {
        log(...args) {
            logs.push(args.join(" "));
        },
        warn(...args) {
            logs.push(args.join(" "));
        },
        error(...args) {
            logs.push(args.join(" "));
        },
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
            navigator,
            indexedDB,
            Response,
            Request,
            URL,
            Date,
            setTimeout,
            clearTimeout,
            recordRetirement() {
                retirements += 1;
            },
        },
        { filename: SERVICE_WORKER },
    );
    return {
        scope,
        claims: () => claims,
        activationRequests: () => activationRequests,
        retirements: () => retirements,
        logs,
        lockRequests: lockState.requests,
    };
}

function activationBlock() {
    const html = readFileSync(INDEX, "utf8");
    const blocks = [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)]
        .map((match) => match[1])
        .filter((block) => block.includes("const serviceWorkerActivation"));
    assert.equal(blocks.length, 1, "expected one service-worker activation block");
    return blocks[0];
}

function reloadSafetyBlock() {
    const html = readFileSync(INDEX, "utf8");
    const blocks = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
        .map((match) => match[1])
        .filter((block) =>
            block.includes("data-tonk-account-setup-critical") &&
            block.includes("tonk:account-setup-critical-change"),
        );
    assert.equal(blocks.length, 1, "expected one account-safe reload block");
    return blocks[0];
}

function runWarmUpdatePage({
    critical = false,
    incomingState = "activated",
    predicate = "present",
    indexedDB = new FakeIndexedDB(),
    broadcast = broadcastHarness(),
    locks,
} = {}) {
    const messages = [];
    const claimLockStates = [];
    const reloadLockStates = [];
    let reloads = 0;
    const lockState = locks === undefined ? lockHarness() : {
        locks,
        requests: [],
        held: () => false,
    };
    const oldWorker = eventTarget({ state: "activated", postMessage() {} });
    let serviceWorkers;
    const incoming = eventTarget({
        state: incomingState,
        postMessage(message) {
            messages.push(message);
            if (message?.type === "activate") {
                incoming.state = "activated";
                registration.installing = null;
                registration.waiting = null;
                incoming.dispatch("statechange");
            }
            if (message?.type === "claim") {
                claimLockStates.push(lockState.held());
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
    if (predicate === "present") {
        self.tonkAccountSetupMayReload = () => !critical;
    } else if (predicate === "throws") {
        self.tonkAccountSetupMayReload = () => {
            throw new Error("predicate unavailable");
        };
    }
    const document = eventTarget({
        visibilityState: "visible",
        documentElement: {
            hasAttribute(name) {
                return name === "data-tonk-account-setup-critical" && critical;
            },
        },
        querySelector() {
            return { textContent: "" };
        },
    });
    const context = {
        self,
        window: self,
        document,
        navigator: { serviceWorker: serviceWorkers, locks: lockState.locks },
        indexedDB,
        BroadcastChannel: broadcast.BroadcastChannel,
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
                reloadLockStates.push(lockState.held());
                reloads += 1;
            },
        },
        console: { log() {}, warn() {}, error() {} },
        Event,
    };
    vm.runInNewContext(reloadSafetyBlock(), context, { filename: INDEX });
    vm.runInNewContext(activationBlock(), context, { filename: INDEX });
    return {
        messages,
        reloads: () => reloads,
        storage,
        dispatchCriticalChange(detail) {
            self.dispatch("tonk:account-setup-critical-change", { detail });
        },
        setCritical(value) {
            critical = value;
            self.dispatch("tonk:account-setup-critical-change", {
                detail: { critical: value, mayReload: !value },
            });
        },
        setPredicate(value) {
            self.tonkAccountSetupMayReload = value;
        },
        signalHoldChanged() {
            broadcast.signal();
        },
        lockRequests: lockState.requests,
        claimLockStates,
        reloadLockStates,
    };
}

function runColdInstallPage() {
    const messages = [];
    let resolveReady;
    const storage = new Map();
    const firstWorker = {
        state: "activated",
        postMessage(message) {
            messages.push(message);
            if (message?.type === "claim") {
                serviceWorkers.controller = firstWorker;
                void serviceWorkers.dispatch("controllerchange");
            }
        },
    };
    const registration = eventTarget({
        active: null,
        installing: firstWorker,
        waiting: null,
        async update() {},
    });
    const serviceWorkers = eventTarget({
        controller: null,
        ready: new Promise((resolve) => {
            resolveReady = resolve;
        }),
        async register() {
            return registration;
        },
    });
    const self = eventTarget({ tonkBootLife() {} });
    const document = eventTarget({
        visibilityState: "visible",
        documentElement: { hasAttribute: () => false },
        querySelector() {
            return { textContent: "" };
        },
    });
    const context = {
        self,
        window: self,
        document,
        navigator: { serviceWorker: serviceWorkers, locks: lockHarness().locks },
        indexedDB: new FakeIndexedDB(),
        BroadcastChannel: broadcastHarness().BroadcastChannel,
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
        location: { reload() {} },
        console: { log() {}, warn() {}, error() {} },
        Event,
    };
    vm.runInNewContext(reloadSafetyBlock(), context, { filename: INDEX });
    vm.runInNewContext(activationBlock(), context, { filename: INDEX });
    return {
        messages,
        activate() {
            registration.installing = null;
            registration.active = firstWorker;
            resolveReady(registration);
        },
        ready: () => self.serviceWorkerActivates(),
    };
}

test("activation alone does not claim pre-upgrade pages", async () => {
    const { scope, claims } = loadServiceWorker();
    const pending = [];
    scope.onactivate({ waitUntil: (promise) => pending.push(promise) });
    await Promise.all(pending);
    assert.equal(claims(), 0);
});

test("a failed successor install leaves the incumbent worker operational", async () => {
    const { scope, retirements } = loadServiceWorker();
    const candidate = eventTarget({ state: "installing" });
    scope.registration.installing = candidate;

    await scope.registration.dispatch("updatefound");
    assert.equal(
        retirements(),
        0,
        "entering installing is not proof that the successor can replace the incumbent",
    );

    candidate.state = "redundant";
    scope.registration.installing = null;
    await candidate.dispatch("statechange");
    assert.equal(
        retirements(),
        0,
        "a redundant successor must not terminally stop incumbent sync or LSP",
    );
});

test("a successfully installed successor retires the incumbent exactly once", async () => {
    const { scope, retirements, logs } = loadServiceWorker();
    const candidate = eventTarget({ state: "installing" });
    scope.registration.installing = candidate;

    await scope.registration.dispatch("updatefound");
    candidate.state = "installed";
    scope.registration.waiting = candidate;
    await candidate.dispatch("statechange");
    await candidate.dispatch("statechange");

    assert.equal(retirements(), 1, logs.join("\n"));
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

test("a page registering its first worker waits for activation before claiming", async () => {
    const result = runColdInstallPage();
    await new Promise(setImmediate);
    assert.deepEqual(
        result.messages,
        [],
        "there is no active worker to receive a claim while install is pending",
    );

    result.activate();
    await result.ready();
    assert.deepEqual(
        result.messages.map((message) => message?.type),
        ["claim", "connectivity"],
    );
});

test("an explicit waiting-worker activation message repeats skipWaiting", async () => {
    const { scope, activationRequests } = loadServiceWorker();
    const pending = [];
    scope.onmessage({
        data: { type: "activate" },
        waitUntil: (promise) => pending.push(promise),
    });
    await Promise.all(pending);
    assert.equal(activationRequests(), 1);
});

test("a durable account-setup hold blocks global claim across reloads and tabs", async () => {
    const indexedDB = new FakeIndexedDB({ hold: VALID_HOLD });
    const { scope, claims, lockRequests } = loadServiceWorker({ indexedDB });
    const pending = [];
    scope.onmessage({
        data: { type: "claim" },
        waitUntil: (promise) => pending.push(promise),
    });
    await Promise.all(pending);
    assert.equal(claims(), 0, "a safe sender cannot switch a held peer tab");
    assert.equal(lockRequests.length, 1);
    assert.equal(lockRequests[0].name, "tonk-update-safety-v1");
    assert.equal(lockRequests[0].options.mode, "exclusive");
});

test("claim fails closed for malformed holds, storage errors, and missing Web Locks", async () => {
    const cases = [
        { indexedDB: new FakeIndexedDB({ hold: { ...VALID_HOLD, leasedRevision: "07" } }) },
        { indexedDB: new FakeIndexedDB({ failOpen: true }) },
        { indexedDB: new FakeIndexedDB({ failRead: true }) },
        { indexedDB: new FakeIndexedDB(), locks: undefined, explicitNoLocks: true },
    ];
    for (const scenario of cases) {
        const options = scenario.explicitNoLocks
            ? { indexedDB: scenario.indexedDB, locks: null }
            : scenario;
        const { scope, claims } = loadServiceWorker(options);
        const pending = [];
        scope.onmessage({
            data: { type: "claim" },
            waitUntil: (promise) => pending.push(promise),
        });
        await Promise.all(pending);
        assert.equal(claims(), 0);
    }
});

test("a late IndexedDB success after blocked rejection closes its database", async () => {
    const workerDatabase = new FakeIndexedDB({ blockThenOpen: true });
    const { scope, claims } = loadServiceWorker({ indexedDB: workerDatabase });
    const pending = [];
    scope.onmessage({
        data: { type: "claim" },
        waitUntil: (promise) => pending.push(promise),
    });
    await Promise.all(pending);
    assert.equal(claims(), 0);
    assert.equal(workerDatabase.closed, 1);

    const pageDatabase = new FakeIndexedDB({ blockThenOpen: true });
    const result = runWarmUpdatePage({ indexedDB: pageDatabase });
    await new Promise(setImmediate);
    assert.equal(result.messages.length, 0);
    assert.equal(result.reloads(), 0);
    assert.equal(pageDatabase.closed, 1);
});

test("an update-capable page claims an activated successor before one reload", async () => {
    const result = runWarmUpdatePage();
    await new Promise(setImmediate);
    assert.equal(result.messages.length, 1);
    assert.equal(result.messages[0]?.type, "claim");
    assert.equal(result.reloads(), 1);
    assert.equal(result.storage.get("tonk:sw-upgrade-reload"), "1");
    assert.deepEqual(result.claimLockStates, [true]);
    assert.deepEqual(result.reloadLockStates, [true]);
});

test("alignment defers while account setup is critical and resumes after durability", async () => {
    const result = runWarmUpdatePage({ critical: true });
    await new Promise(setImmediate);

    assert.equal(
        result.messages.length,
        0,
        "the successor must not claim an Armed/pre-Stage document before recovery is durable",
    );
    assert.equal(result.storage.get("tonk:sw-upgrade-reload"), undefined);
    assert.equal(result.reloads(), 0);

    // Event detail is advisory: the root attribute is the durable source of
    // truth, so an optimistic or out-of-order event cannot release the reload.
    result.dispatchCriticalChange({ critical: false, mayReload: true });
    assert.equal(result.messages.length, 0);
    assert.equal(result.reloads(), 0);

    result.setCritical(false);
    await new Promise(setImmediate);
    assert.equal(result.messages[0]?.type, "claim");
    assert.equal(result.storage.get("tonk:sw-upgrade-reload"), "1");
    assert.equal(result.reloads(), 1);
});

test("alignment remains held after a reload until the durable singleton clears", async () => {
    const indexedDB = new FakeIndexedDB({ hold: VALID_HOLD });
    const broadcast = broadcastHarness();
    const result = runWarmUpdatePage({
        indexedDB,
        broadcast,
        incomingState: "installed",
    });
    await new Promise(setImmediate);

    assert.equal(result.messages.length, 0);
    assert.equal(result.reloads(), 0);

    indexedDB.values.delete("account-setup");
    result.signalHoldChanged();
    await new Promise(setImmediate);
    assert.deepEqual(
        result.messages.map((message) => message?.type),
        ["activate", "claim"],
    );
    assert.equal(result.reloads(), 1);
});

test("alignment treats a malformed durable record as held", async () => {
    const result = runWarmUpdatePage({
        indexedDB: new FakeIndexedDB({
            hold: { ...VALID_HOLD, operationId: "not-an-operation" },
        }),
    });
    await new Promise(setImmediate);
    assert.equal(result.messages.length, 0);
    assert.equal(result.reloads(), 0);
});

test("alignment uses the root-attribute fallback before the UI predicate exists", async () => {
    const result = runWarmUpdatePage({ critical: true, predicate: "missing" });
    await new Promise(setImmediate);
    assert.equal(result.reloads(), 0);

    result.setCritical(false);
    await new Promise(setImmediate);
    assert.equal(result.reloads(), 1);
});

test("alignment fails closed when the UI reload predicate throws", async () => {
    const result = runWarmUpdatePage({ critical: false, predicate: "throws" });
    await new Promise(setImmediate);
    assert.equal(result.reloads(), 0);

    result.setPredicate(() => true);
    result.dispatchCriticalChange({ critical: false, mayReload: true });
    await new Promise(setImmediate);
    assert.equal(result.reloads(), 1);
});

function runColdFirstInstallPage() {
    const messages = [];
    let serviceWorkers;
    const worker = eventTarget({
        state: "installing",
        postMessage(message) {
            messages.push({ ...message, atState: worker.state });
            if (message?.type === "claim" && worker.state === "activated") {
                serviceWorkers.controller = worker;
                serviceWorkers.dispatch("controllerchange");
            }
        },
    });
    const registration = eventTarget({
        // The genuinely-first install: register() resolves while the
        // worker is still installing, so there is no `active` to nudge.
        active: null,
        installing: worker,
        waiting: null,
        async update() {},
    });
    serviceWorkers = eventTarget({
        controller: null,
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
                reload() {},
            },
            console: { log() {}, warn() {}, error() {} },
        },
        { filename: INDEX },
    );
    return { messages, worker, registration, serviceWorkers };
}

test("a cold first install claims once its worker activates", async () => {
    const result = runColdFirstInstallPage();
    await new Promise(setImmediate);
    // Nothing to claim yet: register() resolved with the worker still
    // installing and no active slot. The regression this pins: the claim
    // was sent to `registration.active` HERE — a no-op on null — and
    // never again, so a cleared-storage first visit stayed uncontrolled
    // forever behind a shell stuck on "starting…".
    assert.equal(
        result.messages.filter((m) => m?.type === "claim" && m.atState === "activated").length,
        0,
    );

    // The worker walks its lifecycle to activated.
    result.worker.state = "installed";
    result.worker.dispatch("statechange");
    result.worker.state = "activating";
    result.worker.dispatch("statechange");
    result.registration.active = result.worker;
    result.registration.installing = null;
    result.worker.state = "activated";
    result.worker.dispatch("statechange");
    await new Promise(setImmediate);

    const claims = result.messages.filter(
        (m) => m?.type === "claim" && m.atState === "activated",
    );
    assert.ok(claims.length >= 1, "the page asks the activated worker to claim");
    assert.equal(result.serviceWorkers.controller, result.worker, "and control lands");
});

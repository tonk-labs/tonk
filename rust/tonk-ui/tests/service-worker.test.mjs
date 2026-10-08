import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = readFileSync(join(HERE, "..", "assets", "service_worker.js"), "utf8");

// Run the worker script against stand-ins for a service worker's globals.
// `build` is what the stamp writes; `fetched` answers the network.
function worker({ origin = "https://tonk.test", build = "0123456789abcdef", fetched } = {}) {
  const stores = new Map();
  const open = async (name) => {
    if (!stores.has(name)) stores.set(name, new Map());
    const store = stores.get(name);
    const key = (request) => (typeof request === "string" ? request : request.url);
    return {
      match: async (request) => store.get(key(request))?.clone(),
      put: async (request, response) => void store.set(key(request), response),
      add: async (request) => void store.set(key(request), await self.fetch(request)),
    };
  };
  const self = {
    location: new URL("/service_worker.js", origin),
    skipWaiting: async () => {},
    clients: { claim: async () => {} },
    fetch: fetched ?? (async () => new Response("page", { status: 200 })),
  };
  const context = vm.createContext({
    self,
    URL,
    Request,
    Response,
    fetch: (...args) => self.fetch(...args),
    caches: {
      open,
      keys: async () => [...stores.keys()],
      delete: async (name) => stores.delete(name),
      match: async (request, { cacheName }) => (await open(cacheName)).match(request),
    },
  });
  vm.runInContext(SOURCE.replace(/^const BUILD_ID = .*$/m, `const BUILD_ID = "${build}";`), context);
  const settle = async (handler, event = {}) => {
    let done;
    handler({ ...event, waitUntil: (promise) => { done = promise; } });
    await done;
  };
  const asked = (path, mode) => {
    let answer;
    self.onfetch({
      request: { method: "GET", url: new URL(path, origin).href, mode },
      respondWith: (response) => { answer = response; },
    });
    return answer;
  };
  return { self, stores, settle, asked };
}

test("it holds no database, glue or Wasm", () => {
  assert.doesNotMatch(SOURCE, /^import /m);
  assert.doesNotMatch(SOURCE, /indexedDB|wasm|worker\.js/i);
  assert.ok(SOURCE.length < 8_000, "the app's worker is a small script");
});

test("it answers every address of the app with the one page", async () => {
  const { asked } = worker();
  for (const path of ["/", "/account", "/settings/link", "/space/did:key:z6Mk"]) {
    const answer = asked(path, "navigate");
    assert.ok(answer, `${path} is the page`);
    assert.equal(await (await answer).text(), "page");
  }
});

test("it leaves what the server answers itself to the server", () => {
  const { asked } = worker();
  for (const path of ["/@", "/@/abc", "/.well-known/tonk", "/ucan/", "/customer/x", "/agent/", "/images/a.png"]) {
    assert.equal(asked(path, "navigate"), undefined, `${path} goes to the server`);
  }
  assert.equal(asked("/version.json", "cors"), undefined);
});

test("it opens the page with no network from what it kept", async () => {
  let online = true;
  const { self, settle, asked } = worker({
    fetched: async () => {
      if (!online) throw new TypeError("offline");
      return new Response("page", { status: 200 });
    },
  });
  await settle(self.oninstall);
  online = false;
  assert.equal(await (await asked("/account", "navigate")).text(), "page");
});

test("it keeps a file the page loads and answers with it afterwards", async () => {
  let fetches = 0;
  const { asked } = worker({
    fetched: async () => {
      fetches += 1;
      const response = new Response("code", { status: 200 });
      Object.defineProperty(response, "type", { value: "basic" });
      return response;
    },
  });
  assert.equal(await (await asked("/ui-abc.js", "cors")).text(), "code");
  assert.equal(await (await asked("/ui-abc.js", "cors")).text(), "code");
  assert.equal(fetches, 1);
});

test("it answers where the sites are with no network, from what it kept", async () => {
  let online = true;
  const { self, settle, asked } = worker({
    fetched: async (request) => {
      if (!online) throw new TypeError("Failed to fetch");
      const url = typeof request === "string" ? request : request.url;
      return new Response(new URL(url).pathname === "/.well-known/tonk" ? '{"sites":{}}' : "page", { status: 200 });
    },
  });
  await settle(self.oninstall);

  online = false;
  const config = await asked("/.well-known/tonk", "cors");

  assert.equal(await config.text(), '{"sites":{}}');
  assert.equal(asked("/customer/state", "cors"), undefined, "the rest of what the server answers is still the server's");
});

test("it keeps what the page loaded before it was there, when the page says", async () => {
  const fetched = [];
  const { self, stores, settle } = worker({
    fetched: async (request) => {
      const url = typeof request === "string" ? request : request.url;
      fetched.push(new URL(url).pathname);
      return new Response(`bytes of ${new URL(url).pathname}`, { status: 200 });
    },
  });
  await settle(self.oninstall);
  fetched.length = 0;

  await settle(self.onmessage, {
    data: {
      type: "keep",
      urls: [
        "https://tonk.test/ui-1.js",
        "https://tonk.test/styles-1.css",
        "https://tonk.test/version.json",
        "https://elsewhere.test/x.js",
      ],
    },
  });

  assert.deepEqual(fetched, ["/ui-1.js", "/styles-1.css"], "only this origin's own files are asked for");
  const kept = stores.get("TONK_APP_0123456789abcdef");
  assert.equal(await kept.get("https://tonk.test/ui-1.js").clone().text(), "bytes of /ui-1.js");
  assert.ok(!kept.has("https://tonk.test/version.json"), "what the server answers itself is not kept");

  // Said twice, each file is asked for once.
  await settle(self.onmessage, { data: { type: "keep", urls: ["https://tonk.test/ui-1.js"] } });
  assert.deepEqual(fetched, ["/ui-1.js", "/styles-1.css"]);
});

test("it keeps nothing under the dev server", async () => {
  const { self, stores, settle, asked } = worker({ build: "dev" });
  await settle(self.oninstall);
  assert.equal(stores.size, 0);
  assert.equal(asked("/ui-abc.js", "cors"), undefined);
});

test("it drops what an earlier build kept", async () => {
  const { self, stores, settle } = worker();
  stores.set("TONK_SHELL_old", new Map());
  stores.set("TONK_WORKER_old", new Map());
  await settle(self.oninstall);
  await settle(self.onactivate);
  assert.deepEqual([...stores.keys()], ["TONK_APP_0123456789abcdef"]);
});

test("it does not install on a site's origin", async () => {
  for (const origin of ["https://profile.tonk.test", `https://b${"a".repeat(51)}.tonk.test`]) {
    const { self, settle } = worker({ origin });
    await assert.rejects(settle(self.oninstall), /site's origin/);
  }
});

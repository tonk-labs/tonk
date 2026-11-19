import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const watchdog = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
  .map((match) => match[1])
  .find((script) => script.includes('const RETRIES = "tonk:boot-retries"'));
assert.ok(watchdog);

const initialHeading = html.match(/<h1\b[^>]*\bdata-boot-title[^>]*>([\s\S]*?)<\/h1>/)?.[1].trim();

test("static page readers get a local example hint without a false boot failure", () => {
  assert.equal(initialHeading, "");
  const hint = html.match(/<p class="tonk-boot-hint">([\s\S]*?)<\/p>/)?.[1];
  assert.ok(hint);
  assert.match(hint, /href="https:\/\/tonk\.xyz\/try"/);
  assert.match(hint, /href="\/agent\/"/);
  assert.match(hint, /href="\/\.well-known\/tonk"/);
  assert.doesNotMatch(hint, /\b(?:curl|npm|npx)\b/);
  const rules = [...html.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((match) => match[1].includes(".tonk-boot-hint"));
  assert.equal(rules.length, 1);
  assert.equal(rules[0][1].trim(), ".tonk-boot-hint");
  assert.match(rules[0][2], /display:\s*none\s*;/);
});

function harness({ retries = 0, storageThrows = false } = {}) {
  const values = new Map();
  if (retries) values.set("tonk:boot-retries", String(retries));
  const status = {
    textContent: "loading…",
    failed: false,
    setAttribute(name) { if (name === "data-failed") this.failed = true; },
  };
  const heading = { textContent: initialHeading };
  let reloads = 0;
  const self = {};
  const storage = {
    getItem(key) {
      if (storageThrows) throw new Error("storage unavailable");
      return values.get(key) ?? null;
    },
    setItem(key, value) {
      if (storageThrows) throw new Error("storage unavailable");
      values.set(key, String(value));
    },
    removeItem(key) {
      if (storageThrows) throw new Error("storage unavailable");
      values.delete(key);
    },
  };
  vm.runInNewContext(watchdog, {
    self,
    navigator: {
      serviceWorker: {
        controller: null,
        async getRegistration() { return null; },
      },
    },
    document: {
      querySelector(selector) {
        if (selector === "[data-boot-title]") return heading;
        return selector === "[data-boot-status]" ? status : null;
      },
    },
    sessionStorage: storage,
    location: { reload() { reloads += 1; } },
    console: { warn() {}, error() {} },
    Date,
    Number,
    setInterval() { return 1; },
    clearInterval() {},
  });
  return { self, heading, status, values, reloads: () => reloads };
}

test("the first recovery reloads even when browser storage is unavailable", () => {
  const result = harness({ storageThrows: true });
  result.self.tonkBootRecover("test stall");
  assert.equal(result.reloads(), 1);
  assert.equal(result.status.textContent, "recovering…");
});

test("a second stall terminates instead of entering another recovery loop", () => {
  const result = harness({ retries: 1 });
  result.self.tonkBootRecover("test stall");
  assert.equal(result.reloads(), 0);
  assert.equal(result.status.failed, true);
  assert.equal(result.heading.textContent, "Tonk couldn’t start");
  assert.match(result.status.textContent, /local data is safe/i);
});

test("an explicit terminal failure clears the retry guard", () => {
  const result = harness({ retries: 1 });
  result.self.tonkBootTerminal("specific failure", "Specific guidance");
  assert.equal(result.heading.textContent, "Specific guidance");
  assert.equal(result.status.textContent, "specific failure");
  assert.equal(result.status.failed, true);
  assert.equal(result.values.has("tonk:boot-retries"), false);
});

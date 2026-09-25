import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8");
const inline = (text, marker) => [...text.matchAll(/inline_js = r#"([\s\S]*?)"#/g)]
  .map(match => match[1]).find(js => js.includes(marker)).replaceAll("export ", "");
const retry = inline(read("../src/analytics.rs"), "resolve_analytics_identity");
const bridge = inline(read("../../tonk-analytics/src/web.rs"), "ph_init");
const id = `tonk:${"a".repeat(64)}`;

function resolver() {
  // Shorten wall-clock timers while preserving timeout/backoff ordering.
  const context = vm.createContext({
    setTimeout: (fn, ms) => setTimeout(fn, ms === 5000 ? 10 : 0), clearTimeout,
  });
  vm.runInContext(retry, context);
  return context.resolve_analytics_identity;
}

test("identity retries transient failure and stops after successful resolution", async () => {
  let calls = 0;
  assert.equal(await resolver()(() => {
    if (++calls === 1) throw new Error("worker unavailable");
    return id;
  }), id);
  assert.equal(calls, 2);
});

test("identity exhaustion stays unresolved and never returns a late result", async () => {
  let calls = 0;
  const late = [];
  assert.equal(await resolver()(() => {
    calls++;
    return new Promise(resolve => late.push(resolve));
  }), null);
  assert.equal(calls, 3);
  for (const resolve of late) resolve(id);
});

test("identity rejects malformed identifiers and eventually resolves", async () => {
  let calls = 0;
  assert.equal(await resolver()(() => ++calls < 3 ? "raw-profile" : id), id);
  assert.equal(calls, 3);
});

function sdk({ optedOut = false, identifyThrows = false, acceptsIdentity = true } = {}) {
  const props = {};
  let current = "anonymous";
  let config;
  const posthog = {
    init(_key, value) { config = value; },
    register(value) { Object.assign(props, value); },
    identify(value) {
      if (identifyThrows) throw new Error("unavailable");
      if (acceptsIdentity) current = value;
    },
    get_distinct_id: () => current,
  };
  const context = vm.createContext({ window: { posthog },
    location: { hostname: "tonk.network" },
    localStorage: { getItem: () => optedOut ? "off" : null },
  });
  vm.runInContext(bridge, context);
  return { context, props, config: () => config };
}

test("capture identity starts unresolved and becomes profile only after SDK acceptance", () => {
  for (const options of [{}, { identifyThrows: true }, { acceptsIdentity: false }]) {
    const { context, props, config } = sdk(options);
    assert.equal(context.ph_init("test-key", "test-host", "test-version"), true);
    assert.equal(config().persistence, "memory");
    assert.equal(props.metrics_version, 2);
    assert.equal(props.identity_state, "unresolved");
    context.ph_identify(id);
    assert.equal(props.identity_state, Object.keys(options).length ? "unresolved" : "profile");
  }
});

test("opt-out never initializes PostHog", () => {
  const { context, config } = sdk({ optedOut: true });
  assert.equal(context.ph_init("test-key", "test-host", "test-version"), false);
  assert.equal(config(), undefined);
});

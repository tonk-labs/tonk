// Drive a real Tonk UI through the scenarios that make the interface
// reload or stall, and record what the person would have seen.
//
// Every frame (the top page and each sealed guest) reports DOM events to
// one recorder: guest frames built and torn down, rendered views removed,
// the loading pulse appearing and clearing, and top-page loads. Network
// conditions are switched at runtime by the gate in front of the stack, so
// a scenario can go slow, offline or stalled mid-run.
//
// Usage: node trace.mjs <config.json>
// The config names the base URL, the invite URL, the space DIDs, the gate's
// control URL, the CLI environment for remote writes, and the output dir.

import { createRequire } from "node:module";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const config = JSON.parse(readFileSync(process.argv[2], "utf8"));
const OUT = config.out;
mkdirSync(OUT, { recursive: true });

// The recorder. Every event carries wall-clock ms, the scenario it belongs
// to, and the frame it came from (top, or the guest's depth and route).
let scenario = "setup";
let scenarioStart = Date.now();
const events = [];
function record(event) {
  const entry = { t: Date.now(), dt: Date.now() - scenarioStart, scenario, ...event };
  events.push(entry);
  appendFileSync(join(OUT, "events.jsonl"), JSON.stringify(entry) + "\n");
}

// Runs in every document, the sealed guests included. It reports through
// the `__trace` binding, which Playwright exposes to every frame.
const PROBE = () => {
  if (globalThis.__traceInstalled) return;
  globalThis.__traceInstalled = true;
  const depth = (() => {
    let d = 0;
    let w = window;
    try {
      while (w !== w.parent && d < 10) {
        w = w.parent;
        d++;
      }
    } catch {}
    return d;
  })();
  const emit = (kind, detail = {}) => {
    try {
      globalThis.__trace?.({ kind, depth, ...detail });
    } catch {}
  };
  emit("document", { url: location.href.slice(0, 200) });
  const describe = (el) => {
    const attrs = {};
    for (const name of ["with", "path", "entity", "model", "view"]) {
      const value = el.getAttribute?.(name);
      if (value) attrs[name] = value.slice(0, 120);
    }
    return attrs;
  };
  const WATCHED = new Set(["TONK-SITE", "TONK-PORTAL", "IFRAME", "TONK-VIEW", "TONK-DISPLAY"]);
  const walk = (node, fn) => {
    if (node.nodeType !== 1) return;
    if (WATCHED.has(node.tagName)) fn(node);
    for (const tag of WATCHED) for (const el of node.querySelectorAll?.(tag) ?? []) fn(el);
  };
  const observer = new MutationObserver((records) => {
    for (const r of records) {
      for (const node of r.addedNodes)
        walk(node, (el) =>
          emit("insert", { tag: el.tagName.toLowerCase(), parent: el.parentElement?.tagName?.toLowerCase(), ...describe(el) }),
        );
      for (const node of r.removedNodes)
        walk(node, (el) =>
          emit("remove", { tag: el.tagName.toLowerCase(), parent: r.target?.tagName?.toLowerCase(), ...describe(el) }),
        );
      if (r.type === "attributes" && r.target.tagName === "TONK-SITE")
        emit("site-attr", { name: r.attributeName, value: r.target.getAttribute(r.attributeName)?.slice(0, 120) });
    }
  });
  const start = () =>
    observer.observe(document, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["with", "allow", "path"],
    });
  if (document.documentElement) start();
  else document.addEventListener("readystatechange", start, { once: true });

  // The loading pulse and the boot overlay, sampled: what the person sees.
  const visible = (el) => {
    if (!el) return false;
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const box = el.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  };
  let pulse = false;
  let boot = false;
  let text = 0;
  setInterval(() => {
    const nowPulse = [...document.querySelectorAll(".tonk-pulse")].some(visible);
    const nowBoot = visible(document.getElementById("tonk-boot"));
    const nowText = (document.body?.innerText || "").trim().length;
    if (nowPulse !== pulse) emit(nowPulse ? "pulse-on" : "pulse-off");
    if (nowBoot !== boot) emit(nowBoot ? "boot-on" : "boot-off");
    if ((text === 0) !== (nowText === 0)) emit(nowText ? "content-on" : "content-off", { chars: nowText });
    pulse = nowPulse;
    boot = nowBoot;
    text = nowText;
  }, 50);
  addEventListener("pagehide", () => emit("pagehide"));
};

async function gate(mode, extra = {}) {
  const query = new URLSearchParams({ mode, ...extra });
  await fetch(`${config.gate}/__gate?${query}`);
  record({ kind: "network", mode, ...extra });
}

function cli(args) {
  return execFileSync(config.tonk, args, { env: { ...process.env, ...config.cliEnv }, encoding: "utf8" });
}

const DEBUG_PORT = Number(process.env.TRACE_DEBUG_PORT || 9333);
const browser = await chromium.launch({
  args: ["--window-size=1280,900", "--ignore-certificate-errors", `--remote-debugging-port=${DEBUG_PORT}`],
});

// The service worker's own console, through the browser's debugging
// endpoint: what it decided about caching and what failed. Each worker
// instance is a new target, so keep looking for ones not yet followed.
const followed = new Set();
async function followWorkers() {
  const targets = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/list`).then((r) => r.json()).catch(() => []);
  for (const target of targets) {
    if (target.type !== "service_worker" || followed.has(target.id) || !target.webSocketDebuggerUrl) continue;
    followed.add(target.id);
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    socket.onopen = () => socket.send(JSON.stringify({ id: 1, method: "Runtime.enable" }));
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      if (message.method === "Runtime.consoleAPICalled") {
        const text = message.params.args.map((a) => a.value ?? a.description ?? "").join(" ");
        record({ kind: "worker-console", level: message.params.type, text: text.slice(0, 300) });
      } else if (message.method === "Runtime.exceptionThrown") {
        record({ kind: "worker-exception", text: (message.params.exceptionDetails?.exception?.description || "").slice(0, 300) });
      }
    };
    socket.onclose = () => record({ kind: "worker-gone", id: target.id.slice(0, 8) });
    record({ kind: "worker-followed", id: target.id.slice(0, 8) });
  }
}
const workerWatch = setInterval(followWorkers, 250);
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
await context.exposeBinding("__trace", ({ frame }, payload) => {
  record({ frame: frame === frame.page().mainFrame() ? "top" : `guest:${payload.depth}`, ...payload });
});
await context.addInitScript(PROBE);
const page = await context.newPage();
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") record({ kind: "console", level: m.type(), text: m.text().slice(0, 300) });
});
page.on("framenavigated", (f) => {
  if (f === page.mainFrame()) record({ kind: "top-navigated", url: f.url().slice(0, 200) });
});
page.on("load", () => record({ kind: "top-load" }));
const cdp = await context.newCDPSession(page);
await cdp.send("ServiceWorker.enable");

// A frame whose document never finishes loading never answers an evaluate:
// give up on it after `ms` and treat it as still loading.
function within(promise, fallback, ms = 2_000) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(fallback), ms))]);
}

// Wait until the page shows the space's content: text in a guest and no
// pulse anywhere for `quiet` ms. Returns how long that took, or null.
async function settled(limit = 60_000, quiet = 1_000) {
  const t0 = Date.now();
  let quietSince = null;
  while (Date.now() - t0 < limit) {
    const state = await Promise.all(
      page.frames().map((f) =>
        within(f
          .evaluate(() => ({
            pulse: [...document.querySelectorAll(".tonk-pulse")].some((el) => {
              if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
              const b = el.getBoundingClientRect();
              return b.width > 0 && b.height > 0;
            }),
            text: (document.body?.innerText || "").trim().length,
          })), { pulse: true, text: 0 })
          .catch(() => ({ pulse: false, text: 0 })),
      ),
    );
    const ok = !state.some((s) => s.pulse) && state.slice(1).some((s) => s.text > 0);
    if (ok) {
      quietSince ??= Date.now();
      if (Date.now() - quietSince >= quiet) return quietSince - t0;
    } else quietSince = null;
    await page.waitForTimeout(100);
  }
  return null;
}

// Whether the service worker can serve this page with no network: the
// root in its shell cache. Read before a scenario starts, since a reload
// on a bad network depends on it.
async function offlineReady() {
  return within(
    page
      .evaluate(async () => {
        const names = await caches.keys();
        const shell = names.find((n) => n.startsWith("TONK_SHELL_"));
        const root = shell ? Boolean(await caches.match("/", { cacheName: shell })) : false;
        return { caches: names, root_cached: root };
      })
      .catch((e) => ({ error: String(e).slice(0, 120) })),
    { error: "page did not answer" },
  );
}

const only = process.env.TRACE_SCENARIOS?.split(",");
const results = [];
async function run(name, fn) {
  if (only && !only.includes(name) && name !== "cold-join" && name !== "open-second-space") return;
  scenario = name;
  scenarioStart = Date.now();
  record({ kind: "scenario-start", offline: await offlineReady() });
  let outcome;
  try {
    outcome = await fn();
  } catch (error) {
    outcome = { error: String(error).slice(0, 300) };
  }
  // What the page was still waiting on from the network as the scenario
  // ended: a request open for long was never answered.
  const inflight = await fetch(`${config.gate}/inflight`)
    .then((r) => r.json())
    .catch(() => []);
  const stuck = inflight.filter((e) => e.first_byte_ms == null && Date.now() - e.t0 > 5_000);
  if (stuck.length) record({ kind: "inflight", exchanges: stuck });
  record({ kind: "scenario-end", ...outcome });
  await page.screenshot({ path: join(OUT, `${name}.png`), timeout: 10_000 }).catch(() => {});
  results.push({ scenario: name, ...outcome });
  console.error(`trace: ${name}`, JSON.stringify(outcome));
}

const [spaceA, spaceB] = config.spaces;
const url = (did, rest = "") => `${config.base}/space/${did}${rest}`;

await run("cold-join", async () => {
  await gate("open");
  await page.goto(config.invite);
  for (let i = 0; i < 240 && /\/join/.test(page.url()); i++) await page.waitForTimeout(500);
  return { settled_ms: await settled(120_000), url: page.url() };
});

await run("open-second-space", async () => {
  await page.goto(config.inviteB);
  for (let i = 0; i < 240 && /\/join/.test(page.url()); i++) await page.waitForTimeout(500);
  return { settled_ms: await settled(120_000) };
});

await run("warm-reload", async () => {
  await page.goto(url(spaceA));
  return { settled_ms: await settled() };
});

await run("switch-spaces", async () => {
  const times = [];
  for (const did of [spaceB, spaceA, spaceB, spaceA]) {
    await page.evaluate((href) => {
      history.pushState(null, "", href);
      dispatchEvent(new PopStateEvent("popstate"));
    }, `/space/${did}`);
    times.push(await settled());
  }
  return { settled_ms: times };
});

await run("worker-restart", async () => {
  await settled();
  await cdp.send("ServiceWorker.stopAllWorkers");
  record({ kind: "worker-stopped" });
  // Something has to wake the worker: the person keeps using the page.
  await page.waitForTimeout(2_000);
  await page.evaluate(() => fetch("/api/health").catch(() => {}));
  return { settled_ms: await settled(30_000) };
});

// Whether any guest frame shows `text`, polled until `limit` ms pass.
async function shows(text, limit) {
  const t0 = Date.now();
  while (Date.now() - t0 < limit) {
    for (const f of page.frames().slice(1)) {
      const found = await within(f.evaluate((t) => (document.body?.innerText || "").includes(t), text), false).catch(() => false);
      if (found) return Date.now() - t0;
    }
    await page.waitForTimeout(200);
  }
  return null;
}

function remoteWrite(label) {
  const title = `${label} ${Date.now()}`;
  const file = join(OUT, `${label}.yaml`);
  writeFileSync(file, `note!:\n  title: "${title}"\n  body: "Written by another device while the page is open."\n`);
  cli(["eval", file]);
  cli(["push"]);
  record({ kind: "remote-pushed", title });
  return title;
}

// The person leaves the tab alone for a while: the worker has time to
// prepare its offline copy.
await run("idle", async () => {
  await page.waitForTimeout(Number(process.env.TRACE_IDLE_MS || 90_000));
  return { offline_after: await offlineReady() };
});

// The browser stops an idle worker (it does this on its own after about
// 30 seconds), then the network goes bad and the person reloads.
await run("restart-then-stall", async () => {
  await settled();
  await cdp.send("ServiceWorker.stopAllWorkers");
  record({ kind: "worker-stopped" });
  await gate("stall");
  await page.goto(url(spaceA), { timeout: 60_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
  const reopened = await settled(60_000);
  await gate("open");
  return { settled_ms: reopened };
});

// Two reloads on a stalled network, nothing else: the first reload starts
// a service worker update check that never answers.
await run("two-reloads-on-stall", async () => {
  await settled();
  await gate("stall");
  const times = [];
  for (const did of [spaceA, spaceA, spaceB]) {
    await page.goto(url(did), { timeout: 60_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
    times.push(await settled(45_000));
  }
  await gate("open");
  return { settled_ms: times, after_network_returns_ms: await settled(30_000) };
});

// The CLI writes to the first space, so a scenario that waits for its
// write must be looking at that space; a scenario before it may have left
// the page on the second.
async function viewFirstSpace() {
  if (page.url().includes(spaceA)) return;
  await page.goto(url(spaceA), { timeout: 60_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
}

await run("remote-write-while-viewing", async () => {
  await viewFirstSpace();
  await settled();
  const title = remoteWrite("remote");
  return { shown_after_ms: await shows(title, 60_000), settled_ms: await settled(30_000) };
});

// The tab sits untouched on a space (TRACE_IDLE_MS, default 90 s) long
// enough for the browser to stop an idle worker and for quiet connections
// to be closed, then another device writes: does the write still arrive?
await run("idle-then-remote-write", async () => {
  await viewFirstSpace();
  await settled();
  await page.waitForTimeout(Number(process.env.TRACE_IDLE_MS || 90_000));
  const title = remoteWrite("after-idle");
  return { shown_after_ms: await shows(title, 60_000) };
});

// Another device writes; the browser syncs the new head in the background
// while the network is fine. Then the network stalls and the person opens
// the space again: does reading what was just synced wait on the network?
await run("remote-write-then-stall", async () => {
  await viewFirstSpace();
  await settled();
  const title = remoteWrite("stalled");
  const synced = await shows(title, 60_000);
  await gate("stall");
  await page.goto(url(spaceA), { timeout: 120_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
  const reopened = await settled(60_000);
  const shown = await shows(title, 30_000);
  await gate("open");
  return { synced_ms: synced, settled_ms: reopened, shown_after_reopen_ms: shown };
});

// Same, but the network stalls before the browser has seen the write:
// the page must stay usable and show what it already had.
await run("stall-before-sync", async () => {
  await viewFirstSpace();
  await settled();
  await gate("stall");
  const title = remoteWrite("unseen");
  await page.goto(url(spaceB), { timeout: 120_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
  const otherSpace = await settled(60_000);
  await page.goto(url(spaceA), { timeout: 120_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
  const sameSpace = await settled(60_000);
  await gate("open");
  return { other_space_settled_ms: otherSpace, settled_ms: sameSpace, shown_after_open_ms: await shows(title, 60_000) };
});

for (const [name, mode, extra] of [
  ["slow-reload", "latency", { ms: "2000" }],
  ["stalled-reload", "stall", {}],
  ["offline-reload", "offline", {}],
]) {
  await run(name, async () => {
    await gate(mode, extra);
    await page.goto(url(spaceA), { timeout: 120_000 }).catch((e) => record({ kind: "goto-failed", error: String(e).slice(0, 200) }));
    const first = await settled(60_000);
    const switched = [];
    for (const did of [spaceB, spaceA]) {
      await page.evaluate((href) => {
        history.pushState(null, "", href);
        dispatchEvent(new PopStateEvent("popstate"));
      }, `/space/${did}`);
      switched.push(await settled(60_000));
    }
    await gate("open");
    return { settled_ms: first, switch_settled_ms: switched };
  });
}

clearInterval(workerWatch);
await browser.close();

// A summary per scenario, from the events alone.
const summary = results.map((r) => {
  const mine = events.filter((e) => e.scenario === r.scenario);
  const count = (pred) => mine.filter(pred).length;
  let pulseMs = 0;
  const open = new Map();
  for (const e of mine) {
    const key = e.frame;
    if (e.kind === "pulse-on") open.set(key, e.t);
    if (e.kind === "pulse-off" && open.has(key)) {
      pulseMs += e.t - open.get(key);
      open.delete(key);
    }
  }
  return {
    ...r,
    top_loads: count((e) => e.kind === "top-load"),
    guest_frames_built: count((e) => e.kind === "insert" && e.tag === "iframe"),
    guest_frames_removed: count((e) => e.kind === "remove" && e.tag === "iframe"),
    views_removed: count((e) => e.kind === "remove" && (e.tag === "tonk-view" || e.tag === "tonk-display")),
    sites_removed: count((e) => e.kind === "remove" && e.tag === "tonk-site"),
    pulse_shown: count((e) => e.kind === "pulse-on"),
    pulse_ms: pulseMs,
    content_lost: count((e) => e.kind === "content-off" && e.frame !== "top"),
  };
});
writeFileSync(join(OUT, "summary.json"), JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));

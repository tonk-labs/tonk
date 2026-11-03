// Run the stability scenarios against a running stack and check their
// invariants (see invariants.mjs).
//
// Every scenario runs behind a shaped network: by default each request is
// delayed `STABILITY_LATENCY_MS` (150), and the stall and offline scenarios
// switch the network mid-run. A local stack with no delay hides exactly the
// races that make a real space slow, so a run with the delay off is not a
// gate run.
//
// Usage: node stability.mjs <config.json> [--record]
//
// The config is the one bench/trace/run.sh writes (base URL, invites, space
// DIDs, the network control URL, the CLI environment, the output dir).
// `--record` writes today's off-target counts to ratchets.json; it is for
// the maintainer recording a first baseline, never for making a run pass.

import { createRequire } from "node:module";
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Collector } from "./diagnostics.mjs";
import { SCENARIOS, check } from "./invariants.mjs";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const HERE = dirname(fileURLToPath(import.meta.url));
const RATCHETS = join(HERE, "ratchets.json");
const config = JSON.parse(readFileSync(process.argv[2], "utf8"));
const recording = process.argv.includes("--record");
const LATENCY_MS = String(process.env.STABILITY_LATENCY_MS ?? 150);
const only = process.env.STABILITY_SCENARIOS?.split(",");

// The one place that talks to the network shaper. bench/trace/gate.mjs and
// `tonk-perf link` both answer `GET /__gate?mode=...`.
async function network(mode, extra = {}) {
  if (mode === "shaped") return network("latency", { ms: LATENCY_MS });
  const query = new URLSearchParams({ mode, ...extra });
  const response = await fetch(`${config.gate}/__gate?${query}`);
  if (!response.ok) throw new Error(`network ${mode}: ${response.status}`);
}

function cli(args) {
  return execFileSync(config.tonk, args, { env: { ...process.env, ...config.cliEnv }, encoding: "utf8" });
}

function within(promise, fallback, ms = 2_000) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(fallback), ms))]);
}

// CI passes the Nix chromium in CHROME; locally Playwright's own browser.
const browser = await chromium.launch(
  process.env.CHROME
    ? { executablePath: process.env.CHROME, args: ["--no-sandbox", "--disable-dev-shm-usage"] }
    : {},
);
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const cdp = await context.newCDPSession(page);
await cdp.send("ServiceWorker.enable");
let topLoads = 0;
page.on("load", () => topLoads++);

// What the last unsettled sample saw, for a scenario that never settles.
let lastUnsettled = null;

// Content in a guest and no loading pulse anywhere for `quiet` ms.
async function settled(limit = 60_000, quiet = 1_000) {
  const t0 = Date.now();
  let quietSince = null;
  while (Date.now() - t0 < limit) {
    const state = await Promise.all(
      page.frames().map((f) =>
        within(
          f.evaluate(() => ({
            pulse: [...document.querySelectorAll(".tonk-pulse")].some((el) => {
              if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
              const b = el.getBoundingClientRect();
              return b.width > 0 && b.height > 0;
            }),
            text: (document.body?.innerText || "").trim().length,
          })),
          { pulse: true, text: 0 },
        ).catch(() => ({ pulse: false, text: 0 })),
      ),
    );
    lastUnsettled = { pulsing: state.flatMap((s, i) => (s.pulse ? [i ? `guest:${i}` : "top"] : [])), guest_text: state.slice(1).map((s) => s.text) };
    if (!state.some((s) => s.pulse) && state.slice(1).some((s) => s.text > 0)) {
      quietSince ??= Date.now();
      if (Date.now() - quietSince >= quiet) return quietSince - t0;
    } else quietSince = null;
    await page.waitForTimeout(100);
  }
  return null;
}

async function shows(text, limit) {
  const t0 = Date.now();
  while (Date.now() - t0 < limit) {
    for (const f of page.frames().slice(1)) {
      const found = await within(f.evaluate((t) => (document.body?.innerText || "").includes(t), text), false).catch(
        () => false,
      );
      if (found) return Date.now() - t0;
    }
    await page.waitForTimeout(200);
  }
  return null;
}

async function navigate(path) {
  await page.evaluate((href) => {
    history.pushState(null, "", href);
    dispatchEvent(new PopStateEvent("popstate"));
  }, path);
}

function remoteWrite(label) {
  const title = `${label} ${Date.now()}`;
  const file = join(config.out, `${label}.yaml`);
  writeFileSync(file, `note!:\n  title: "${title}"\n  body: "Written by another device while the page is open."\n`);
  cli(["eval", file]);
  cli(["push"]);
  return title;
}

const [spaceA, spaceB] = config.spaces;
const ratchets = JSON.parse(readFileSync(RATCHETS, "utf8"));
const results = [];

// A scenario starts settled on space A behind the shaped network, resets
// every counter, runs `body`, and reads the counters back.
async function scenario(name, body) {
  if (only && !only.includes(name)) return;
  await network("shaped");
  if (!page.url().includes(`/space/${spaceA}`)) await page.goto(`${config.base}/space/${spaceA}`);
  // A scenario measures a page that was showing its content. If the one
  // before left it broken, reload (before the counters reset, so the
  // reload is not this scenario's), and if it still will not settle, the
  // scenario is vacuous.
  let ready = await settled();
  if (ready === null) {
    await page.goto(`${config.base}/space/${spaceA}`);
    ready = await settled(60_000);
  }
  const collector = new Collector(page);
  await collector.reset();
  topLoads = 0;
  // Sample once before the body runs, so a realm the body destroys at once
  // (the worker a restart stops) is seen at least once.
  await collector.sample();
  collector.start();
  let observed = {};
  try {
    observed = (await body()) ?? {};
  } catch (error) {
    observed = { error: String(error).slice(0, 300) };
  }
  await network("shaped");
  await collector.stop();
  const { counters, lockSites } = collector.totals();
  const unsettled = [observed.settled_ms].flat().some((ms) => ms === null);
  const values = {
    ...counters,
    "harness.top_loads": topLoads,
    "harness.started_settled": ready === null ? 0 : 1,
    "harness.unsettled": unsettled ? 1 : 0,
  };
  await page.screenshot({ path: join(config.out, `${name}.png`), timeout: 10_000 }).catch(() => {});
  const verdict = check(name, values, ratchets);
  results.push({ scenario: name, observed, values, lockSites, ...verdict });
  console.error(`stability: ${name}`, JSON.stringify({ observed, ...verdict }));
}

// Joining is setup, not a scenario: it reloads by design.
await network("shaped");
for (const invite of [config.invite, config.inviteB]) {
  await page.goto(invite);
  for (let i = 0; i < 240 && /\/join/.test(page.url()); i++) await page.waitForTimeout(500);
  await settled(120_000);
}

await scenario("switch-spaces", async () => {
  const settled_ms = [];
  for (const did of [spaceB, spaceA]) {
    await navigate(`/space/${did}`);
    settled_ms.push(await settled());
  }
  return { settled_ms };
});

// Let the worker fill its offline copy before stopping it. Interrupting the
// fill is a separate bug (an endless boot screen) that would mask what this
// scenario measures.
if (!only || only.includes("worker-restart")) await page.waitForTimeout(Number(process.env.STABILITY_IDLE_MS ?? 90_000));

await scenario("worker-restart", async () => {
  await cdp.send("ServiceWorker.stopAllWorkers");
  // Something has to wake the worker: the person keeps using the page.
  await page.waitForTimeout(2_000);
  await page.evaluate(() => fetch("/api/health").catch(() => {}));
  return { settled_ms: await settled(30_000) };
});

await scenario("remote-write-while-viewing", async () => {
  const title = remoteWrite("remote");
  return { shown_after_ms: await shows(title, 60_000), settled_ms: await settled(30_000) };
});

for (const [name, mode] of [
  ["stall-while-viewing", "stall"],
  ["offline-while-viewing", "offline"],
]) {
  await scenario(name, async () => {
    await network(mode);
    await page.waitForTimeout(Number(process.env.STABILITY_OUTAGE_MS ?? 20_000));
    await network("shaped");
    const settled_ms = await settled(60_000);
    return settled_ms === null ? { settled_ms, unsettled: lastUnsettled } : { settled_ms };
  });
}

// The browser stopped the worker earlier (it does so on its own after
// about 30 idle seconds), then the network stalls: the new worker instance
// must not need the network to keep the page showing what it had.
await scenario("restart-then-stall", async () => {
  await cdp.send("ServiceWorker.stopAllWorkers");
  await page.waitForTimeout(2_000);
  await page.evaluate(() => fetch("/api/health").catch(() => {}));
  const restarted_ms = await settled(30_000);
  await network("stall");
  await page.waitForTimeout(Number(process.env.STABILITY_OUTAGE_MS ?? 20_000));
  await network("shaped");
  const settled_ms = await settled(60_000);
  return settled_ms === null ? { restarted_ms, settled_ms, unsettled: lastUnsettled } : { restarted_ms, settled_ms };
});

await browser.close();

const failed = results.filter((r) => r.failures.length || r.vacuous.length || r.observed.error);
writeFileSync(join(config.out, "stability.json"), JSON.stringify(results, null, 2));

if (recording) {
  const next = {};
  for (const r of results) {
    for (const f of r.failures) (next[r.scenario] ??= {})[f.metric] = f.value;
    for (const i of r.improvements) (next[r.scenario] ??= {})[i.metric] = i.value;
  }
  writeFileSync(RATCHETS, JSON.stringify(next, null, 2) + "\n");
  console.error(`stability: recorded ${RATCHETS}`);
}

for (const r of results) {
  const marks = [
    ...r.failures.map((f) => `FAIL ${f.metric}=${f.value} (target ${f.op ?? "eq"} ${f.target}${f.ratchet !== undefined ? `, ratchet ${f.ratchet}` : ""})`),
    ...r.improvements.map((i) => `LOWER ${i.metric}: ${i.ratchet} -> ${i.value} (lower ratchets.json)`),
    ...r.vacuous.map((v) => `VACUOUS ${v.metric}=${v.value} (needs ${v.op} ${v.target})`),
    ...(r.observed.error ? [`ERROR ${r.observed.error}`] : []),
  ];
  const bad = r.failures.length || r.vacuous.length || r.observed.error;
  console.log(`${bad ? "x" : "ok"} ${r.scenario}${marks.map((m) => `\n    ${m}`).join("")}`);
}
process.exit(failed.length && !recording ? 1 : 0);

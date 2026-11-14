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
import { profileFrame, spaceFrame, timedFetch, within } from "./sites.mjs";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const HERE = dirname(fileURLToPath(import.meta.url));
const RATCHETS = join(HERE, "ratchets.json");
const config = JSON.parse(readFileSync(process.argv[2], "utf8"));
const recording = process.argv.includes("--record");
const LATENCY_MS = String(process.env.STABILITY_LATENCY_MS ?? 150);
const only = process.env.STABILITY_SCENARIOS?.split(",");
// How long a plain read may take while a sync stalls (write-while-sync-stalls).
const READ_BUDGET_MS = 2_000;
// How long a page load of a space may take while the network stalls
// (load-while-sync-stalls).
const LOAD_BUDGET_MS = 5_000;
// How long a scenario keeps counting after its body ends.
const TAIL_MS = Number(process.env.STABILITY_TAIL_MS ?? 6_000);

// The one place that talks to the network shaper: `tonk-perf link`'s
// control server, in front of the page and the remote it syncs with.
async function network(mode, extra = {}) {
  if (mode === "shaped") return network("latency", { ms: LATENCY_MS });
  const query = new URLSearchParams({ mode, ...extra });
  const response = await fetch(`${config.gate}/__gate?${query}`);
  if (!response.ok) throw new Error(`network ${mode}: ${response.status}`);
}

function cli(args) {
  return execFileSync(config.tonk, args, { env: { ...process.env, ...config.cliEnv }, encoding: "utf8" });
}

// CI passes the Nix chromium in CHROME; locally Playwright's own browser.
const browser = await chromium.launch(
  process.env.CHROME
    ? { executablePath: process.env.CHROME, args: ["--no-sandbox", "--disable-dev-shm-usage"] }
    : {},
);
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
// A shaped network (slow3g, satellite) takes far longer than Playwright's
// 30 s default to load a page; a timeout there is the harness's, not tonk's.
page.setDefaultTimeout(Number(process.env.STABILITY_TIMEOUT_MS ?? 180_000));
const cdp = await context.newCDPSession(page);
await cdp.send("ServiceWorker.enable");
let topLoads = 0;
page.on("load", () => topLoads++);

// What the last unsettled sample saw, for a scenario that never settles.
let lastUnsettled = null;

// The space the page shows, by its DID: its frame is the one whose content
// counts, and whose worker the scenarios measure.
let viewing = null;

// Whether `frame` is the frame of the space the page shows, or a frame in it.
function inViewedSpace(frame) {
  const space = viewing && spaceFrame(page, viewing);
  for (let f = frame; space && f; f = f.parentFrame()) if (f === space) return true;
  return false;
}

// Content in the space's frame and no loading pulse anywhere for `quiet`
// ms. The profile's frame around the space always has text of its own (its
// chrome), so only the space's frame says whether the space shows.
async function settled(limit = 60_000, quiet = 1_000) {
  const t0 = Date.now();
  let quietSince = null;
  while (Date.now() - t0 < limit) {
    const frames = page.frames();
    const state = await Promise.all(
      frames.map((f) =>
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
    const shown = state.filter((s, i) => inViewedSpace(frames[i])).map((s) => s.text);
    lastUnsettled = { pulsing: state.flatMap((s, i) => (s.pulse ? [i ? `guest:${i}` : "top"] : [])), space_text: shown };
    if (!state.some((s) => s.pulse) && shown.some((text) => text > 0)) {
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

// The app's page owns the address bar: it tells the profile where it is,
// and the profile frames the space the address names.
async function navigate(did) {
  viewing = did;
  await page.evaluate((href) => {
    history.pushState(null, "", href);
    dispatchEvent(new PopStateEvent("popstate"));
  }, `/space/${did}`);
}

async function open(did) {
  viewing = did;
  await page.goto(`${config.base}/space/${did}`);
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
  if (!page.url().includes(`/space/${spaceA}`)) await open(spaceA);
  viewing = spaceA;
  // A scenario measures a page that was showing its content. If the one
  // before left it broken, reload (before the counters reset, so the
  // reload is not this scenario's), and if it still will not settle, the
  // scenario is vacuous.
  let ready = await settled();
  if (ready === null) {
    await open(spaceA);
    ready = await settled(60_000);
  }
  // The workers that matter: the space's kernel, which answers the space's
  // frame, and the profile's, which frames the space and hosts the
  // person's spaces.
  const collector = new Collector(page, () => ({ space: spaceFrame(page, spaceA), profile: profileFrame(page) }));
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
  // What the scenario set off can land after the page settled: a display
  // holds an empty frame for its grace (5 s) before it applies it. Keep
  // counting a while, so that lands in the scenario that caused it and
  // not in the next one.
  await page.waitForTimeout(TAIL_MS);
  await collector.stop();
  const { counters, lockSites } = collector.totals();
  const unsettled = [observed.settled_ms].flat().some((ms) => ms === null);
  const values = {
    ...counters,
    ...(observed.read ? { "harness.read_over_budget": observed.read.status !== 200 || observed.read.ms > READ_BUDGET_MS ? 1 : 0 } : {}),
    ...(observed.load
      ? {
          "harness.load_over_budget": [observed.load, observed.reload].some(
            (l) => !(l.status === 200 || l.status === "loaded") || l.ms > LOAD_BUDGET_MS,
          )
            ? 1
            : 0,
        }
      : {}),
    ...(observed.pull_stalled !== undefined ? { "harness.sync_stalled": observed.pull_stalled ? 1 : 0 } : {}),
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
for (const [invite, did] of [
  [config.invite, spaceA],
  [config.inviteB, spaceB],
]) {
  viewing = did;
  await page.goto(invite);
  for (let i = 0; i < 240 && /\/join/.test(page.url()); i++) await page.waitForTimeout(500);
  await settled(120_000);
}

await scenario("switch-spaces", async () => {
  const settled_ms = [];
  for (const did of [spaceB, spaceA]) {
    await navigate(did);
    settled_ms.push(await settled());
  }
  return { settled_ms };
});

// Let the worker fill its offline copy before stopping it. Interrupting the
// fill is a separate bug (an endless boot screen) that would mask what this
// scenario measures.
if (!only || only.includes("worker-restart")) await page.waitForTimeout(Number(process.env.STABILITY_IDLE_MS ?? 90_000));

// The browser stops every worker of the page: the app's, the profile's
// kernel and the space's. Something has to wake them: the person keeps
// using the page, which asks the space's kernel and the profile's, each
// from a frame of its own origin. The app's worker serves no request of
// theirs.
async function restartWorkers() {
  await cdp.send("ServiceWorker.stopAllWorkers");
  await page.waitForTimeout(2_000);
  const woken = await Promise.all([
    timedFetch(spaceFrame(page, spaceA), "/api/health"),
    timedFetch(profileFrame(page), "/api/health"),
  ]);
  return { space: woken[0].status, profile: woken[1].status };
}

await scenario("worker-restart", async () => {
  const woken = await restartWorkers();
  return { woken, settled_ms: await settled(30_000) };
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
  const woken = await restartWorkers();
  const restarted_ms = await settled(30_000);
  await network("stall");
  await page.waitForTimeout(Number(process.env.STABILITY_OUTAGE_MS ?? 20_000));
  await network("shaped");
  const settled_ms = await settled(60_000);
  return settled_ms === null
    ? { woken, restarted_ms, settled_ms, unsettled: lastUnsettled }
    : { woken, restarted_ms, settled_ms };
});

// A sync of space A hangs on a stalled remote while a writer arrives and
// takes the worker's state for writing. The state lock is
// write-preferring, so if the sync holds its guard across the round trip,
// the writer queues behind it and every request after the writer queues
// too. A local read issued after the writer must still answer within
// READ_BUDGET_MS (declared at the top). The read is a claim select, which
// answers from the replica under a read guard: `sync/status` fetches the
// upstream head and would wait on the stalled remote whatever the lock
// does.
//
// With one worker for every space this was a pull of space A, the page
// creating a space, and a read of space B. Each site now has a worker and
// a lock of its own: space A's kernel syncs A, the profile's creates
// spaces, and B's answers B, so those three never meet on one lock. The
// lock they shared is now space A's kernel's alone, so all three go to it,
// from space A's frame: the pull, the same space creation (`PUT
// /api/repository/{name}`, which takes the kernel's state for writing),
// and a read of A.
//
// Only the remote leg is stalled (`link=ucan`, which `tonk-perf link`
// honours; bench/trace/gate.mjs stalls everything, and cannot stall a sync
// that bypasses it, so there the scenario is vacuous). bench/trace/gate.mjs
// has no `/stats`; only `tonk-perf link` can stall the remote leg alone.
const shapesRemote = await fetch(`${config.gate}/stats`)
  .then((r) => (r.ok ? r.text() : "null"))
  .then((body) => Array.isArray(JSON.parse(body)))
  .catch(() => false);
if (!shapesRemote) console.error("stability: write-while-sync-stalls skipped: the network cannot stall the remote alone");
if (shapesRemote) await scenario("write-while-sync-stalls", async () => {
  await network("stall", { link: "ucan" });
  const timed = (path, init) => timedFetch(spaceFrame(page, spaceA), path, init);
  const pull = timed(`/api/repository/${spaceA}/branch/main/sync/pull`, { method: "POST" });
  let pullDone = null;
  pull.then(() => (pullDone = Date.now()));
  const t0 = Date.now();
  await page.waitForTimeout(2_000);
  const pullStalled = pullDone === null;
  const writer = timed(`/api/repository/stability-${Date.now()}`, { method: "PUT" });
  await page.waitForTimeout(500);
  const read = await timed(
    `/api/repository/${spaceA}/branch/main/claim/select?the=${encodeURIComponent("tonk/repository/name")}`,
    {},
  );
  await page.waitForTimeout(Math.max(0, Number(process.env.STABILITY_OUTAGE_MS ?? 20_000) - (Date.now() - t0)));
  await network("shaped");
  const [pulled, wrote] = await Promise.all([pull, writer]);
  return {
    pull: pulled,
    writer: wrote,
    read,
    pull_stalled: pullStalled,
    settled_ms: await settled(60_000),
  };
});

// The network stalls while a pull of space A hangs on it, and the person
// opens space A again: its frame reloads. Every request of the reload goes
// to A's worker, which answers it from what it holds: the page from A's
// kernel, the relay and the host's files from what the worker kept of them.
// None of it may wait for the network. Timed twice: the page alone (`GET /`
// as a page asks for it), then the frame's whole reload, to its `load`
// event. Both within LOAD_BUDGET_MS (declared at the top).
//
// Both links stall, as a network that stops carrying anything does: with
// only the remote stalled, nothing a page load asks for crosses a stalled
// link. Whether the pull still hangs once the frame reloaded (which drops
// the frame's fetch of it) is what the remote's link holds open.
if (shapesRemote) await scenario("load-while-sync-stalls", async () => {
  await network("stall");
  const space = spaceFrame(page, spaceA);
  timedFetch(space, `/api/repository/${spaceA}/branch/main/sync/pull`, { method: "POST" });
  const t0 = Date.now();
  await page.waitForTimeout(2_000);
  const syncWaiting = async () =>
    fetch(`${config.gate}/inflight?link=ucan`)
      .then((r) => r.json())
      .then((open) => open.filter((x) => x.kind !== "socket" && Date.now() - x.t0 > 1_000).length)
      .catch(() => 0);
  const pullStalled = (await syncWaiting()) > 0;
  const load = await timedFetch(space, "/", { headers: { accept: "text/html" } });
  const reloadStarted = Date.now();
  const navigated = page.waitForEvent("framenavigated", { predicate: (f) => f === space, timeout: 30_000 });
  await space.evaluate(() => setTimeout(() => location.reload(), 0)).catch(() => {});
  const reload = await navigated
    .then(() => space.waitForLoadState("load", { timeout: 30_000 }))
    .then(
      () => ({ status: "loaded", ms: Date.now() - reloadStarted }),
      (error) => ({ status: String(error).split("\n")[0].slice(0, 120), ms: Date.now() - reloadStarted }),
    );
  const reloadStalled = (await syncWaiting()) > 0;
  await page.waitForTimeout(Math.max(0, Number(process.env.STABILITY_OUTAGE_MS ?? 20_000) - (Date.now() - t0)));
  await network("shaped");
  return {
    load,
    reload,
    pull_stalled: pullStalled && reloadStalled,
    settled_ms: await settled(60_000),
  };
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

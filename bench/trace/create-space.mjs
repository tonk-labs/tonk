// Time a space's creation, from the press of the hub's create button to the
// new space's canvas on screen, phase by phase.
//
//   TRACE_DRIVER=bench/trace/create-space.mjs bench/trace/run.sh
//   node bench/trace/create-space.mjs <run-dir>/config.json
//
// Each run is a fresh Chrome profile, driven over the DevTools protocol
// itself: every page, frame and service worker is attached as it starts,
// paused until it is, so each site's worker is read from its first line.
// The phases come from what the page shows and from what the workers log
// (`kernel_boot.js` and the kernel time their own steps).
//
// CREATE_RUNS sets how many runs (3 by default); CHROME names the browser.
// It writes `create-space.json` (every run's phases and their medians) and
// each run's events into the run directory.

import { spawn } from "node:child_process";
import { appendFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { siteLabel } from "../stability/sites.mjs";

const config = JSON.parse(readFileSync(process.argv[2], "utf8"));
const RUNS = Number(process.env.CREATE_RUNS ?? 3);
const CHROME =
  process.env.CHROME ??
  (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "google-chrome");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// One run: a fresh browser that opens the app, waits for the hub, creates a
// space, and waits for its canvas. Answers the events it saw.
async function run(dir) {
  mkdirSync(dir, { recursive: true });
  const profile = mkdtempSync(join(dir, "chrome-"));
  const events = [];
  const t0 = performance.now();
  const record = (event) => events.push({ t: Math.round(performance.now() - t0), ...event });
  const chrome = spawn(
    CHROME,
    [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      // Every renderer at foreground priority: a site's worker otherwise runs
      // as background work, and its timings swing by a factor of three.
      "--disable-renderer-backgrounding",
      "--disable-background-timer-throttling",
      "--disable-backgrounding-occluded-windows",
      ...(process.env.CHROME ? ["--no-sandbox", "--disable-dev-shm-usage"] : []),
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  const endpoint = await new Promise((resolve, reject) => {
    let text = "";
    chrome.stderr.on("data", (chunk) => {
      text += chunk;
      const found = /DevTools listening on (ws:\S+)/.exec(text);
      if (found) resolve(found[1]);
    });
    chrome.on("exit", () => reject(new Error(`chrome exited: ${text.slice(-500)}`)));
  });
  const ws = new WebSocket(endpoint);
  await new Promise((resolve) => (ws.onopen = resolve));

  let nextId = 1;
  const waiting = new Map();
  const sessions = new Map();
  const contexts = new Map();
  const send = (method, params = {}, sessionId) => {
    const id = nextId++;
    ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => waiting.set(id, { resolve, reject, method }));
  };
  const attach = {
    autoAttach: true,
    waitForDebuggerOnStart: true,
    flatten: true,
    filter: [{ type: "browser", exclude: true }, { type: "tab", exclude: true }, {}],
  };
  ws.onmessage = async ({ data }) => {
    const message = JSON.parse(data);
    if (message.id) {
      const waiter = waiting.get(message.id);
      waiting.delete(message.id);
      if (message.error) waiter?.reject(new Error(`${waiter.method}: ${message.error.message}`));
      else waiter?.resolve(message.result);
      return;
    }
    const { sessionId: session, params } = message;
    const where = sessions.get(session);
    switch (message.method) {
      case "Target.attachedToTarget": {
        const { type, url } = params.targetInfo;
        const id = params.sessionId;
        sessions.set(id, { type, url });
        record({ kind: "attached", type, url });
        const steps = [send("Runtime.enable", {}, id), send("Target.setAutoAttach", attach, id)];
        if (type === "page" || type === "iframe") steps.push(send("Page.enable", {}, id));
        await Promise.allSettled(steps);
        send("Runtime.runIfWaitingForDebugger", {}, id).catch(() => {});
        break;
      }
      case "Target.detachedFromTarget":
        sessions.delete(params.sessionId);
        for (const key of contexts.keys()) if (key.startsWith(`${params.sessionId}:`)) contexts.delete(key);
        break;
      case "Runtime.executionContextCreated":
        contexts.set(`${session}:${params.context.id}`, { session, id: params.context.id });
        break;
      case "Runtime.executionContextDestroyed":
        contexts.delete(`${session}:${params.executionContextId}`);
        break;
      case "Runtime.executionContextsCleared":
        for (const key of contexts.keys()) if (key.startsWith(`${session}:`)) contexts.delete(key);
        break;
      case "Runtime.consoleAPICalled": {
        const text = params.args.map((arg) => arg.value ?? arg.description ?? arg.type).join(" ");
        record({ kind: "console", where: where?.type, url: where?.url, text: text.slice(0, 2000) });
        break;
      }
      case "Page.frameNavigated":
        record({ kind: "navigated", url: params.frame.url });
        break;
    }
  };

  // The first live context where `expression` answers something, polled.
  const until = async (label, expression, ms = 120_000) => {
    const deadline = Date.now() + ms;
    while (Date.now() < deadline) {
      for (const context of [...contexts.values()]) {
        const answer = await send(
          "Runtime.evaluate",
          { expression, contextId: context.id, returnByValue: true, awaitPromise: true },
          context.session,
        ).catch(() => null);
        const value = answer?.result?.value;
        if (value !== null && value !== undefined && value !== false) {
          record({ kind: "mark", label, value });
          return value;
        }
      }
      await sleep(20);
    }
    throw new Error(`timed out waiting for ${label}`);
  };

  try {
    await send("Target.setDiscoverTargets", { discover: true });
    await send("Target.setAutoAttach", attach);
    await send("Target.createTarget", { url: config.base });
    // The hub, dressed: its stylesheets are in before the measurement.
    await until(
      "hub",
      `(() => { const open = document.querySelector('.hub-page .header-new [data-space-create-open]');
        const links = [...document.querySelectorAll('link[data-tonk-embed]')];
        return !!open && links.length > 0 && links.every((l) => l.sheet && l.sheet.cssRules.length); })()`,
      180_000,
    );
    // The profile settles: the measurement is the creation, not the hub's boot.
    await sleep(5_000);
    await until("dialog", `(() => { const b = document.querySelector('.header-new [data-space-create-open]'); if (!b) return null; b.click(); return true; })()`);
    await until(
      "named",
      `(() => { const i = document.querySelector('.header-new input[name=name]'); if (!i || !i.offsetParent) return null;
        i.value = 'Timed'; i.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`,
    );
    record({ kind: "mark", label: "click" });
    await until("submitted", `(() => { const b = document.querySelector('.header-new [data-space-create-submit]'); if (!b) return null; b.click(); return true; })()`);
    const path = await until("navigated", `window === window.top && location.pathname.startsWith('/space/') ? location.pathname : null`, 180_000);
    const label = siteLabel(decodeURIComponent(path.split("/")[2])).slice(0, 40);
    record({ kind: "mark", label: "label", value: label });
    await until(
      "rendered",
      `(() => { const origins = [location.origin, ...location.ancestorOrigins];
        if (!origins.some((o) => o.includes('//${label}'))) return null;
        const canvas = document.querySelector('.blank-canvas'); if (!canvas) return null;
        const box = canvas.getBoundingClientRect(); return box.width > 0 && box.height > 0; })()`,
      180_000,
    );
    // The space's worker logs the rest of its work just after.
    await sleep(2_000);
  } finally {
    ws.close();
    chrome.kill();
    writeFileSync(join(dir, "events.jsonl"), events.map((event) => JSON.stringify(event)).join("\n") + "\n");
  }
  return events;
}

// What each phase took, in milliseconds, from a run's events.
function phases(events) {
  const label = events.find((e) => e.label === "label")?.value;
  const inSpace = (e) => !!label && (e.url ?? "").includes(`//${label}`);
  const at = (predicate) => events.find(predicate)?.t;
  const mark = (name) => at((e) => e.kind === "mark" && e.label === name);
  const logged = (pattern) => events.find((e) => e.kind === "console" && inSpace(e) && pattern.test(e.text));
  const step = (line, name) => Number(new RegExp(`\\b${name} (\\d+)`).exec(line?.text ?? "")?.[1] ?? NaN);
  const t = {
    click: mark("click"),
    navigated: mark("navigated"),
    worker: at((e) => e.kind === "attached" && e.type === "service_worker" && inSpace(e)),
    booted: logged(/booted in/)?.t,
    created: logged(/tonk-kernel: created/)?.t,
    host: at((e) => e.kind === "console" && /tonk-host: installed .* on http:\/\/b/.test(e.text)),
    rendered: mark("rendered"),
  };
  t.page = at((e) => e.kind === "navigated" && (e.url ?? "").includes(`//${label}`) && /\/$/.test(e.url) && e.t > (t.booted ?? Infinity));
  const span = (from, to) => (t[from] != null && t[to] != null ? t[to] - t[from] : NaN);
  const boot = logged(/booted in/);
  const seeded = logged(/tonk-kernel: seeded/);
  const kept = logged(/kept the renderer/);
  return {
    "profile command": span("click", "navigated"),
    "space worker installs": span("navigated", "worker"),
    "first boot": span("worker", "booted"),
    "  kernel handed over": step(boot, "kernel"),
    "  kernel installed": step(boot, "install"),
    "  compile": step(boot, "compile"),
    "  instantiate": step(boot, "instantiate"),
    "  init": step(boot, "init"),
    "seed and renderer": span("booted", "created"),
    "  seed written": Number(/wrote (\d+)ms/.exec(seeded?.text ?? "")?.[1] ?? NaN),
    "  renderer read": Number(/read (\d+)ms/.exec(kept?.text ?? "")?.[1] ?? NaN),
    "  renderer kept": Number(/kept (\d+)ms/.exec(kept?.text ?? "")?.[1] ?? NaN),
    "first page load": span("page", "host"),
    "first render": span("host", "rendered"),
    total: span("click", "rendered"),
  };
}

const median = (values) => {
  const known = values.filter((value) => !Number.isNaN(value)).sort((a, b) => a - b);
  return known.length ? known[Math.floor((known.length - 1) / 2)] : null;
};

const runs = [];
for (let i = 0; i < RUNS; i++) runs.push(phases(await run(join(config.out, `create-space-${i + 1}`))));
const medians = Object.fromEntries(Object.keys(runs[0]).map((name) => [name, median(runs.map((run) => run[name]))]));
writeFileSync(join(config.out, "create-space.json"), JSON.stringify({ runs, medians }, null, 2));
console.log(`create-space: ${RUNS} runs, median ms`);
for (const [name, ms] of Object.entries(medians)) console.log(`${name.padEnd(24)} ${String(ms ?? "-").padStart(6)}`);
process.exit(0);

// Read the counters `tonk_common::diagnostics` keeps, from every realm a
// scenario touches: the top page, each site's frame, and the kernel workers
// of the sites (through `GET /api/inspect/diagnostics`, asked from a frame
// of the site's own origin: each site's worker answers only its origin).
//
// A frame's registry dies with the frame, and a scenario that rebuilds a
// site destroys exactly the frames whose counts matter. So a `Collector`
// samples every live frame on an interval and keeps the last snapshot it saw
// per registry `instance`; the totals sum over every instance seen since the
// last reset, dead frames included up to their last sample. What a frame
// counted in the final sample interval before it died is lost: keep the
// interval short, and read the counters of a frame's parent (the
// `<tonk-site>` that booted it lives in the parent's realm, which survives).
//
// A worker's registry dies with its kernel the same way, and a new kernel
// answers with a new `instance`: how many instances of the space's kernel
// answered is how a restart shows up from outside.

import { within } from "./sites.mjs";

const WORKER_ROUTE = "/api/inspect/diagnostics";

function frameLabel(page, frame) {
  if (frame === page.mainFrame()) return "top";
  let depth = 0;
  for (let f = frame; f.parentFrame(); f = f.parentFrame()) depth++;
  return `guest:${depth}`;
}

export class Collector {
  /**
   * @param {import("playwright").Page} page
   * @param {() => Record<string, import("playwright").Frame | null>} workers
   *   the frame to ask each kernel worker through, by its role. The role
   *   `space` is the space the scenario looks at: `worker.instances` counts
   *   its kernel's instances.
   */
  constructor(page, workers, { intervalMs = 200 } = {}) {
    this.page = page;
    this.workerFrames = workers;
    this.intervalMs = intervalMs;
    this.realms = new Map(); // instance -> { label, snapshot }
    this.workers = new Map(); // role -> Map(instance -> snapshot)
    this.timer = null;
  }

  /** Clear every live realm's counters and every worker's, and forget dead ones. */
  async reset() {
    const frames = Object.values(this.workerFrames()).filter(Boolean);
    await Promise.all([
      ...this.page.frames().map((frame) =>
        within(frame.evaluate(() => globalThis.__tonkDiagnostics?.reset()), undefined).catch(() => {}),
      ),
      ...frames.map((frame) =>
        within(
          frame.evaluate((route) => fetch(route, { method: "DELETE" }).then((r) => r.status), WORKER_ROUTE),
          undefined,
        ).catch(() => {}),
      ),
    ]);
    this.realms.clear();
    this.workers.clear();
  }

  /** Take one sample of every live frame and every worker. */
  async sample() {
    const reads = this.page.frames().map(async (frame) => {
      const snapshot = await within(
        frame.evaluate(() => globalThis.__tonkDiagnostics?.snapshot() ?? null),
        null,
      ).catch(() => null);
      if (snapshot?.instance) this.realms.set(snapshot.instance, { label: frameLabel(this.page, frame), snapshot });
    });
    const workers = Object.entries(this.workerFrames()).map(async ([role, frame]) => {
      if (!frame) return;
      const snapshot = await within(
        frame.evaluate((route) => fetch(route).then((r) => (r.ok ? r.json() : null)), WORKER_ROUTE),
        null,
      ).catch(() => null);
      if (!snapshot?.instance) return;
      if (!this.workers.has(role)) this.workers.set(role, new Map());
      this.workers.get(role).set(snapshot.instance, snapshot);
    });
    await Promise.all([...reads, ...workers]);
  }

  start() {
    if (this.timer) return;
    let busy = false;
    this.timer = setInterval(async () => {
      if (busy) return;
      busy = true;
      try {
        await this.sample();
      } finally {
        busy = false;
      }
    }, this.intervalMs);
  }

  async stop() {
    clearInterval(this.timer);
    this.timer = null;
    await this.sample();
  }

  /**
   * Counters summed over every realm and every worker instance seen, plus
   * `realms.page` (registries seen in frames), `worker.instances` (how many
   * instances of the space's kernel answered: more than one means its
   * worker restarted), `worker.{role}.instances` for each other worker, and
   * the workers' lock sites, keyed `name@file:line`.
   */
  totals() {
    const counters = {};
    const add = (name, n) => (counters[name] = (counters[name] ?? 0) + n);
    for (const { snapshot } of this.realms.values()) for (const [k, v] of Object.entries(snapshot.counters)) add(k, v);
    const lockSites = {};
    for (const instances of this.workers.values()) {
      for (const snapshot of instances.values()) {
        for (const [k, v] of Object.entries(snapshot.counters)) add(k, v);
        for (const [k, d] of Object.entries(snapshot.durations)) {
          add(`${k}.count`, d.count);
          add(`${k}.total_ms`, Math.round(d.total_ms));
        }
        for (const [k, d] of Object.entries(snapshot.sites)) {
          const site = (lockSites[k] ??= { count: 0, total_ms: 0, max_ms: 0 });
          site.count += d.count;
          site.total_ms += d.total_ms;
          site.max_ms = Math.max(site.max_ms, d.max_ms);
        }
      }
    }
    counters["realms.page"] = this.realms.size;
    counters["worker.instances"] = this.workers.get("space")?.size ?? 0;
    for (const [role, instances] of this.workers) {
      if (role !== "space") counters[`worker.${role}.instances`] = instances.size;
    }
    return { counters, lockSites };
  }
}

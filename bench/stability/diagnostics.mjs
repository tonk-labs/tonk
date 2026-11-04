// Read the counters `tonk_common::diagnostics` keeps, from every realm a
// scenario touches: the top page, each sealed guest frame, and the service
// worker (through `GET /api/inspect/diagnostics`).
//
// A guest frame's registry dies with the frame, and a scenario that rebuilds
// guests destroys exactly the frames whose counts matter. So a `Collector`
// samples every live frame on an interval and keeps the last snapshot it saw
// per registry `instance`; the totals sum over every instance seen since the
// last reset, dead frames included up to their last sample. What a frame
// counted in the final sample interval before it died is lost: keep the
// interval short, and read the counters of a frame's parent (the `<tonk-site>`
// that booted it lives in the parent's realm, which survives).

const WORKER_ROUTE = "/api/inspect/diagnostics";

/** Evaluate in a frame, giving up after `ms` (a frame still loading never answers). */
function within(promise, fallback, ms = 2_000) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(fallback), ms))]);
}

function frameLabel(page, frame) {
  if (frame === page.mainFrame()) return "top";
  let depth = 0;
  for (let f = frame; f.parentFrame(); f = f.parentFrame()) depth++;
  return `guest:${depth}`;
}

export class Collector {
  /** @param {import("playwright").Page} page */
  constructor(page, { intervalMs = 200 } = {}) {
    this.page = page;
    this.intervalMs = intervalMs;
    this.realms = new Map(); // instance -> { label, snapshot }
    this.workers = new Map(); // instance -> snapshot
    this.timer = null;
  }

  /** Clear every live realm's counters and the worker's, and forget dead ones. */
  async reset() {
    await Promise.all(
      this.page.frames().map((frame) =>
        within(frame.evaluate(() => globalThis.__tonkDiagnostics?.reset()), undefined).catch(() => {}),
      ),
    );
    await within(
      this.page.evaluate((route) => fetch(route, { method: "DELETE" }).then((r) => r.status), WORKER_ROUTE),
      undefined,
    ).catch(() => {});
    this.realms.clear();
    this.workers.clear();
  }

  /** Take one sample of every live frame and the worker. */
  async sample() {
    const reads = this.page.frames().map(async (frame) => {
      const snapshot = await within(
        frame.evaluate(() => globalThis.__tonkDiagnostics?.snapshot() ?? null),
        null,
      ).catch(() => null);
      if (snapshot?.instance) this.realms.set(snapshot.instance, { label: frameLabel(this.page, frame), snapshot });
    });
    const worker = within(
      this.page.evaluate((route) => fetch(route).then((r) => (r.ok ? r.json() : null)), WORKER_ROUTE),
      null,
    )
      .catch(() => null)
      .then((snapshot) => {
        if (snapshot?.instance) this.workers.set(snapshot.instance, snapshot);
      });
    await Promise.all([...reads, worker]);
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
   * worker instances answered: more than one means the worker restarted) and
   * the worker's lock sites, keyed `name@file:line`.
   */
  totals() {
    const counters = {};
    const add = (name, n) => (counters[name] = (counters[name] ?? 0) + n);
    for (const { snapshot } of this.realms.values()) for (const [k, v] of Object.entries(snapshot.counters)) add(k, v);
    const lockSites = {};
    for (const snapshot of this.workers.values()) {
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
    counters["realms.page"] = this.realms.size;
    counters["worker.instances"] = this.workers.size;
    return { counters, lockSites };
  }
}

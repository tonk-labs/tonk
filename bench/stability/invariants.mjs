// What each stability scenario must hold, and the check against it.
//
// An invariant has a fixed target that is never re-recorded: content the
// person was looking at is not removed, a worker restart rebuilds nothing,
// the page does not reload by itself. Where today's code misses a target,
// `ratchets.json` records today's count for that scenario and metric, and
// the check holds the count at or below it. A fix that lowers a count lowers
// the ratchet in the same pull request, and when it reaches the target the
// ratchet entry is deleted.
//
// Metrics are the counter names from `tonk_common::diagnostics::names`,
// summed over every realm, plus what the harness counts itself:
// `harness.top_loads` (top-page loads the scenario did not ask for),
// `harness.unsettled` (1 when the page never settled after the scenario),
// `harness.started_settled` and `worker.instances` (how many service
// worker instances answered).

/** Every content-removal counter: none may move in any scenario. */
const NO_REMOVALS = [
  "display.removed.restart",
  "display.removed.error",
  "display.removed.model",
  "display.removed.name",
  "display.removed.view_absent",
  "display.entity.emptied",
  "site.guest_boot.remount",
].map((metric) => ({ metric, target: 0 }));

// The page settles (content shown, no loading pulse) once the scenario's
// disturbance is over, and it does not reload itself.
const NO_RELOADS = [
  { metric: "harness.top_loads", target: 0 },
  { metric: "harness.unsettled", target: 0 },
];
// A scenario that started on a broken page measured nothing.
const STARTED_SETTLED = { metric: "harness.started_settled", target: 1 };
const NO_REBUILDS = ["site.guest_boot.navigation", "site.guest_boot.mount"].map((metric) => ({ metric, target: 0 }));

export const SCENARIOS = {
  // A to B, then back to A: one guest boot per switch at most, nothing
  // removed before the new space has something to show.
  "switch-spaces": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, { metric: "site.guest_boot.navigation", target: 2, op: "le" }],
  },
  // The browser stops the service worker under an open tab. The page keeps
  // what it shows: no guest is rebuilt and nothing is removed.
  "worker-restart": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, ...NO_REBUILDS],
    // Without a second worker instance the scenario proved nothing.
    preconditions: [STARTED_SETTLED, { metric: "worker.instances", target: 2, op: "ge" }],
  },
  // Another device writes while the page is open: the new row arrives in
  // place.
  "remote-write-while-viewing": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, ...NO_REBUILDS],
  },
  // The network stalls or drops while the page is open; the page keeps
  // showing what it had.
  "stall-while-viewing": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, ...NO_REBUILDS],
  },
  "restart-then-stall": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, ...NO_REBUILDS],
    preconditions: [STARTED_SETTLED, { metric: "worker.instances", target: 2, op: "ge" }],
  },
  "offline-while-viewing": {
    invariants: [...NO_REMOVALS, ...NO_RELOADS, ...NO_REBUILDS],
  },
};

function holds(value, { target, op = "eq" }) {
  if (op === "le") return value <= target;
  if (op === "ge") return value >= target;
  return value === target;
}

/**
 * Check one scenario's measured values.
 *
 * Returns `{ failures, improvements, vacuous }`: a failure is a count above
 * its ratchet (or off target with no ratchet); an improvement is a count
 * below its ratchet, which the pull request should record by lowering the
 * ratchet; `vacuous` lists preconditions that did not hold, which means the
 * scenario did not exercise what it claims to.
 */
export function check(name, values, ratchets = {}) {
  const spec = SCENARIOS[name];
  if (!spec) throw new Error(`no invariants for scenario ${name}`);
  const failures = [];
  const improvements = [];
  const vacuous = [];
  for (const pre of spec.preconditions ?? [STARTED_SETTLED]) {
    const value = values[pre.metric] ?? 0;
    if (!holds(value, pre)) vacuous.push({ ...pre, value });
  }
  for (const inv of spec.invariants) {
    const value = values[inv.metric] ?? 0;
    const ratchet = ratchets[name]?.[inv.metric];
    if (ratchet === undefined) {
      if (!holds(value, inv)) failures.push({ ...inv, value });
    } else if (value > ratchet) {
      failures.push({ ...inv, value, ratchet });
    } else if (value < ratchet) {
      improvements.push({ ...inv, value, ratchet });
    }
  }
  return { failures, improvements, vacuous };
}

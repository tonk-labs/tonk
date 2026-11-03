# Stability scenarios

Browser scenarios that check what the person sees stays put: switching
spaces, the browser restarting the service worker, another device writing,
and the network stalling or dropping while a space is open. Each scenario
resets the counters `tonk_common::diagnostics` keeps (`plan/perf-gates.md`,
"Instrumentation as built"), runs, reads them back from every realm (the top
page, each guest frame, the service worker) and checks them against
`invariants.mjs`.

Every scenario runs behind a delayed network (150 ms per request by
default, `STABILITY_LATENCY_MS`). A local stack answers in microseconds and
hides the races that make a real space slow.

## Running

The scenarios need the stack `bench/trace/run.sh` starts (the access
service, the logging proxy serving a trunk dist, the network gate, and two
seeded spaces with invites). Point `stability.mjs` at the config that script
writes:

```
node bench/stability/stability.mjs bench/trace/runs/<stamp>/config.json
```

It prints one line per scenario, writes `stability.json` (every counter,
the worker's lock waits and holds by source line) into the run directory,
and exits non-zero when an invariant fails.

`STABILITY_SCENARIOS=switch-spaces,worker-restart` runs a subset.

## Invariants and ratchets

An invariant has a fixed target that is never re-recorded: no content
removed, no guest remounted, no page reload the scenario did not ask for, no
guest rebuilt by a worker restart. Where today's code misses a target,
`ratchets.json` holds today's count for that scenario and metric, and the
check holds the count at or below it. A pull request that lowers a count
lowers the ratchet; when it reaches the target the entry is deleted.

`--record` writes today's off-target counts to `ratchets.json`. Recording is
the maintainer's call, like the perf baselines (`.claude/rules`), and only
after the variance study shows the counts repeat run to run.

## What it cannot see

- A guest frame's counters die with the frame. The collector samples every
  frame each 200 ms and keeps the last sample per registry, so a frame torn
  down between samples loses what it counted in that last interval. The
  boot that destroyed it is counted in its parent, which survives.
- Reloads are counted by the harness (top-page `load` events), not in the
  page.
- The network gate shapes HTTP only. The live-update websocket and direct
  S3 transfers bypass it until `tonk-perf link` lands.

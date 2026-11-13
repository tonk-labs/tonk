# Stability scenarios

Browser scenarios that check what the person sees stays put: switching
spaces, the browser restarting the service worker, another device writing,
and the network stalling or dropping while a space is open. Each scenario
resets the counters `tonk_common::diagnostics` keeps (`plan/perf-gates.md`,
"Instrumentation as built"), runs, reads them back from every realm (the top
page, each site's frame, and the kernel workers of the space and the
profile) and checks them against `invariants.mjs`.

The app's page frames the profile at `profile.{host}`, and the profile
frames the space it shows at `{label}.{host}`. Each site has a worker of
its own running its kernel, and only it answers the site's `/api/` routes:
the app's origin has a static worker and no `/api/`. So the harness asks a
space's kernel from the space's frame and the profile's from the profile's
(`sites.mjs`), and `worker.instances` counts the instances of the space's
kernel. A worker restart stops every worker and wakes the space's and the
profile's, each from its own frame.

Every scenario runs behind a delayed network (150 ms per request by
default, `STABILITY_LATENCY_MS`). A local stack answers in microseconds and
hides the races that make a real space slow.

## Running

The scenarios need the stack `bench/trace/run.sh` starts (the access
service, the logging proxy serving a trunk dist, the network gate, and two
seeded spaces with invites). Point `stability.mjs` at the config that script
writes:

```
TRACE_DRIVER=bench/stability/stability.mjs bench/trace/run.sh
```

or, against a stack already up, `node bench/stability/stability.mjs <run-dir>/config.json`.

It prints one line per scenario, writes `stability.json` (every counter,
the worker's lock waits and holds by source line) into the run directory,
and exits non-zero when an invariant fails.

`STABILITY_SCENARIOS=switch-spaces,worker-restart` runs a subset.

CI runs it as the `Stability scenarios` job in `.github/workflows/test.yml`,
on the release web build and the Nix `tonk-bench-stack` package, and uploads
`stability.json` and the screenshots. It is not a required check yet: the
spread on CI runners has to be measured first.

`write-while-sync-stalls` needs `tonk-perf link` (it stalls only the remote
leg, `link=ucan`) and is skipped behind bench/trace/gate.mjs. It stalls a
pull of space A, has space A's frame create a space (a writer on the
kernel's state lock), then times a plain read of A, which must answer
within 2 s. All three go to space A's kernel: with a worker per site, it is
the one lock a stalled sync of A can hold up.

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

- A frame's counters die with the frame, and a kernel's with its worker.
  The collector samples every frame and worker each 200 ms and keeps the
  last sample per registry, so a frame torn down between samples loses
  what it counted in that last interval. The boot that destroyed it is
  counted in its parent, which survives.
- Reloads are counted by the harness (top-page `load` events), not in the
  page.
- The network gate sits between the page and the stack. The worker syncs
  with the access service directly (and with S3, and over the live-update
  websocket), so sync is not shaped until `tonk-perf link` lands; the
  delay today slows page and asset traffic only.

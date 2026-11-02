# Perf and reliability gates

Gate tonk on counts that do not drift between runs (guest boots, content
removals, reloads, remote round trips, lock contention), chart wall-clock
time without gating on it, and build one instrumentation layer first so
today's numbers are recorded before the steady-interface fixes
(`plan/steady-interface.md`) land.

## What carries over from dialog-db

The principle carries over intact: gate on numbers that repeat exactly, keep
baselines in the repo, and let only the maintainer move them. The measuring
technique does not, because tonk's worker state is wasm-only (`TonkState`
needs IndexedDB, `rust/tonk-worker/src/helpers/state.rs:15`) and tonk's
failures are in coordination between page, worker and frames, not in CPU
cost.

| dialog-db mechanism | What it does | In tonk |
| --- | --- | --- |
| Perf gate (`rust/dialog-perf`) | Per-scenario engine step counts from tracing spans plus metered block reads, exact; callgrind instructions within 3% | Keep the step counters (dialog emits the spans inside tonk's worker too). Drop callgrind: there is no valgrind in a browser, and CPU is not tonk's suspected problem |
| Replication gate (`rust/dialog-soak`) | Simulated network under tokio's paused clock, fixed seed, lockstep requests: exact round trips and bytes per phase | Keep per-phase request counts. Lose the paused clock: a browser runs on real time, so modeled milliseconds become measured ones, which are noisy |
| Baselines in `perf/baseline/`, `soak/baseline/` | Checked-in JSON per scenario | Same layout, `perf/baseline/` in tonk |
| Compare against the base commit's sweep (`scripts/ci/base-sweep.sh`) | Stops three changes each inside tolerance from summing past it | Port as is |
| Delta report comment (`scripts/ci/delta-report.sh`) | Before and after table on every pull request | Port as is |
| `Baseline guard` and the `gate-change-approved` label | A pull request that touches baselines fails without the label | Port as is, with the same rule in `.claude/rules` |
| Nightly shaped profiles (`nightly-soak.yml`) | Wall-clock under broadband, mobile, browser links | Promote `bench/perf/run.sh` and the `tonk-perf` proxy, which already shape latency and bandwidth, to a nightly trend |

Tonk already has pieces to build on: the `tonk-perf` shaping proxy and
analyzer, the e2e harness with `TONK_E2E_DIAGNOSTICS_DIR` timing events
(`rust/tonk-ui/src/helpers.rs:63`), and `tonk-analytics` for content-free
events from real users.

## What to measure

Each row maps a symptom from `plan/steady-interface.md` to a number. An
invariant has a fixed target and is never re-recorded, a cost has a baseline
the maintainer can move, and a time is charted but not gated on a pull
request.

| Symptom | Metric | Kind | Target |
| --- | --- | --- | --- |
| Space switch rebuilds the guest 2-3 times | Guest frame boots per space switch, split by cause (navigation, remount) | Invariant | 1 navigation, 0 remount |
| Pulsing dot over content that was showing | Rendered content removed without a deletion signal (`clear_host`, slide removal) | Invariant | 0 |
| Content deleted, then inserted again | Subscription frames that answer empty before the first real answer | Invariant | 0 |
| Page reloads by itself | Page reloads per session, excluding profile change | Invariant | 0 |
| Fast once, slow the next time | Reads that waited on a remote fetch, per phase | Cost | 0 eventually; today's count as baseline |
| Join and sync cost | Remote requests and bytes per phase, by kind (redeem, archive get, memory resolve) | Cost | Baseline, exact |
| Global worker lock | Lock acquisitions that waited, and acquisitions held across a network await | Cost | Baseline; the second should reach 0 |
| Worker restarts | Worker boots per session; requests answered before boot's remote work finished | Cost | Baseline |
| Engine work | dialog's step counters (rule resolutions, settlements, commits) per phase | Cost | Baseline, exact |
| Load speed | Time to first frame, time to content, lock wait milliseconds | Time | Charted nightly and from the field |
| Stalled remote hangs a read | Does a local read answer within a budget while the remote stalls | Reliability | Pass or fail |

`tonk.reload()` and `remountGuests` are being removed on the steady-interface
work, so they are not instrumented; a remount from any remaining path counts
under the remount cause above.

## Design

One instrumentation layer feeds three consumers: a stability gate and a cost
gate on every pull request, and a nightly and field view of wall-clock time.
The same counters are what trace capture of real slow loads records, so a
captured trace can be replayed as a gate scenario.

1. **Instrumentation (the prerequisite).** One set of named counters and
   spans, defined once and emitted from the worker (a wrapper around the
   `TonkState` lock that records waits and holds across remote calls, a
   metered remote transport, boot phases) and from the page (`<tonk-site>`
   guest boots, `tonk-display` clears and holds, reloads, subscription
   reconnects, early empty frames). Tests read them through a debug hook on
   the page and a worker diagnostics route. dialog's span counters from
   `dialog-perf` are reused for engine steps.
2. **Stability gate (every pull request, browser).** Scenarios on the
   existing e2e harness: switch space A to B to A, a deploy arriving under an
   open tab, a service worker restart forced over CDP, going offline, an
   empty frame after a render. Each asserts the invariant rows exactly. Where
   today's code fails an invariant, the scenario records today's count as a
   ratchet: it may only go down, and a fix that lowers it lowers the file in
   the same pull request.
3. **Cost gate (every pull request, browser).** The replication gate's
   analog: cold join, warm reload, open a space, query during a sync, catch
   up after 1, 10 and 100 commits. Per phase it reports remote requests and
   bytes by kind, reads that waited on the network, lock contention events
   and dialog's step counts. Fixed-seed identities and a seeded space, as
   dialog-soak does. Baselines, base-sweep comparison, delta report and
   baseline guard are ported from dialog-db.
4. **Nightly and field (trend, not gate).** Nightly: `bench/perf/run.sh`
   under the shaped profiles, plus fault injection added to the `tonk-perf`
   proxy (stall, 5xx, slow drip) for the reliability row. Field: a
   content-free `load_timing` event through `tonk-analytics` with time to
   first frame, time to content, worker boots and lock wait, so slow loads
   from real spaces show up as data.

The open risk is determinism. A browser has no paused clock and the worker
runs requests concurrently, so counts may wobble by a request or two. Before
choosing any tolerance, run each scenario 8 times on one commit and measure
the spread, as dialog-db did in its #653.

## Assumptions worth challenging

- **"It only improves" cannot mean wall-clock in CI.** Shared runners vary by
  tens of percent run to run. dialog-db gates nothing on real time for that
  reason. Gate counts, chart time.
- **A gate prevents regressions; it does not find today's cause.** There is
  no trace of a real slow load yet. A seeded synthetic space may not
  reproduce what real spaces do (size, many devices, real S3 latency, a
  worker idle for an hour). Trace capture and the field event are the pieces
  that measure what people actually experience.
- **Several of these are invariants, not costs.** Reloads should be 0 and
  guest boots per switch should be 1. Behind a re-recordable baseline someone
  can accept a regression. They belong in tests with fixed targets; the
  ratchet is only a bridge until the fixes land.
- **The lock problem is inferred, not measured.** Measure waits and holds
  before redesigning the lock, or the redesign may chase the wrong cost.
- **Measure before fixing.** The steady-interface work already changes guest
  boots. If the gate lands after it, the before number that shows the fix
  worked is lost.

## Order of work

1. **Instrumentation and a read-out.** The counters above, a page hook and a
   worker route to read them. No gate yet.
2. **Stability scenarios on main.** Space switch, service worker restart,
   deploy under an open tab, empty frame. They record today's counts as the
   ratchet, which gives the before numbers.
3. **Variance study and cost gate.** Eight runs per scenario on one commit,
   then baselines, the ported compare, base-sweep, delta report and baseline
   guard.
4. **Nightly shaped runs and fault injection,** then the field event.

The steady-interface fixes then land against the gate, each one lowering a
ratchet in its own pull request.

## Open questions

- Is a content-free load-timing event through PostHog acceptable?
- How many CI minutes per pull request can the browser scenarios cost? The
  e2e suite already runs in 8 shards; the aim is one extra job under 10
  minutes.
- Does the compare and report code stay in tonk, or move into a crate in
  dialog-db so both repositories share one implementation?

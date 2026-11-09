# Render provenance consistency spike

A render receipt needs the revision of the data actually evaluated, followed by
acknowledgments through query frames, host delivery and nested display completion.
Reading the branch head beside a DOM snapshot does not establish this.

First executable question: can a subscription's starting revision identify its
returned rows when a commit lands during evaluation?

The isolated checkout `/tmp/dialog-render-provenance` starts from the exact
`dialog-db` commit used by the conditional-evaluate worktree. Its native test
`it_pins_subscription_reads_before_awaiting` pauses the first provider operation,
commits a name change through the original branch, then releases evaluation. The
expected contract is old rows at the old pin, followed by new rows at the next
poll. This is a prerequisite to exposing a query checkpoint on SSE.

The companion transport patch is opt-in. Neither patch is enabled in the app or
its pinned dependencies, and neither provides a renderer completion receipt.

The subsequent protocol needs three distinct pieces of evidence:

1. A query checkpoint that belongs to the retained rows. It includes both the
   durable branch revision and the ephemeral overlay identity; a null durable
   revision means an evaluated empty branch, not an uninitialized query.
2. Delivery of that checkpoint even when a new revision does not change query
   results. This must not force a DOM repaint or create a repoll loop.
3. A renderer acknowledgment for every active input and nested display in the
   current render generation. Opening a downstream subscription, navigating,
   changing a view, taking a fallback query path, or holding a historical frame
   invalidates completion until the relevant inputs settle again.

Until all three exist, the public inspector continues to report
`renderedRevision: null` and `revisionTracking: unavailable`.

## Reproduction and candidate

Base: `dialog-db` commit `120fba8b6490c31b2ae1523e6add5b800f9b989e`.
`baseline.txt` records the deterministic failure: the old revision was retained
while the paused poll returned the newly committed name. The candidate is in
`dialog-query-checkpoint.patch`; it is not a dependency override or a deployed fix.

Apply the patch in a clean checkout of that commit, then run:

```sh
cargo test -p dialog-repository repository::branch::subscription::tests --lib
cargo fmt --all -- --check
```

The candidate detaches the durable head and session-overlay state before the
first asynchronous poll operation. It preserves branch metadata and shares
content-addressed caches. Idle polls keep their existing early return. A public
`SubscriptionCheckpoint` distinguishes a never-evaluated query from an evaluated
empty branch, and advances even for unchanged query results. Concurrent head
changes during synchronous capture cause a bounded read retry, never a write.

The candidate now shares immutable overlay state. Mutation uses copy-on-write
while a detached snapshot retains that state. This avoids an eager copy on every
active poll, but does not eliminate the copy when a writer overlaps a snapshot.
The subscription retains its pinned branch between polls, so this is not only a
cost for writes during a provider await.

## Validation

- Baseline: deterministic durable-write race failed, returning Bob at Alice's
  captured revision (see `baseline.txt`).
- Candidate: all 47 subscription tests passed, including paused durable writes,
  paused overlay writes, empty-branch checkpoints and unrelated-write checkpoints.
- Rust formatting and patch whitespace checks passed.
- `cargo check -p dialog-repository --lib --target wasm32-unknown-unknown`
  passed, with three warnings in unchanged dialog-storage code.
- The app's existing runtime is unchanged. Transport validation is recorded below;
  DOM completion remains outside this candidate's contract.


## Opt-in worker / host / display transport

`tonk-query-checkpoint-transport.patch` applies to Tonk commit
`7ff32b38c32d08c4fc00e4dd0757df4dfd7bee0a`. It requires the dialog-db candidate
above. It does not change a dependency manifest or include a generated lockfile.

The query route accepts `checkpoints=true`. Opted-in subscribers receive a
`kind: checkpoint` frame after their snapshot or delta; unchanged results can
advance their checkpoint without a data frame. Duplicate checkpoints are
suppressed. Legacy subscribers receive only snapshots and deltas. The reactor
retains the polling engine until fan-out completes to preserve frame ordering.
A failed data serialization requires a fresh snapshot before a checkpoint.

The host opts in and dispatches an optional `checkpoint(payload, {tag})`
consumer method without calling reset/update. Displays record delivered query
checkpoints per input tag in introspection and clear them when data changes or
subscriptions are torn down. These are delivery diagnostics, not acknowledgments
that templates or nested displays have finished mounting. One-shot fallback
queries and historical frames are not covered. The public inspector must still
report `renderedRevision: null` and unavailable revision tracking.

In separate clean checkouts, apply each patch against its recorded base. Generate
local overrides with this helper (Python 3; resolves symlinks to avoid duplicate
Cargo crate identities):

```sh
python3 cargo-overrides.py /absolute/path/to/dialog-db > /tmp/tonk-overrides.toml
```

Use canonical checkout paths and a separate `CARGO_TARGET_DIR` for each
workspace. Do not share the dialog-db test target directory with the Tonk
workspace that consumes its path dependencies. In the patched Tonk checkout:

```sh
cargo --config /tmp/tonk-overrides.toml test -p dialog-reactor --lib
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-host --target wasm32-unknown-unknown \
  it_routes_checkpoint_without_reset_or_update
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-display --target wasm32-unknown-unknown \
  it_records_query_delivery_without_claiming_or_changing_render_state
cargo --config /tmp/tonk-overrides.toml check \
  -p tonk-worker -p tonk-host -p tonk-display --target wasm32-unknown-unknown
```

Nix supplies the repository's `wbg-pool` browser runner. Unused local-patch
warnings are expected for dialog packages outside this dependency graph.

## Snapshot cost measurement

Run in the patched dialog-db checkout:

```sh
cargo test -p dialog-repository --lib repository::
cargo test -p dialog-repository --lib measure_read_snapshot_cost -- --ignored --nocapture
```

The ignored native microbenchmark compares an eager deep copy with a shared
copy-on-write snapshot of the same state representation. Each case has eight
paired samples, alternating execution order, with 20 operations per sample.
It warms the bounded instant log and replaces a single counter value, keeping
the fact count stable (listed initial facts plus one counter fact). It measures
capture/read/drop, and capture/write/read/drop while retaining the snapshot.
`snapshot-cost.csv` contains raw nanoseconds per operation. Timing is diagnostic:
it includes no browser, query execution, memory profiling or end-to-end latency.
Concurrent compilation introduced substantial noise; write-case differences
must not be interpreted as a speedup.

See [snapshot-cost.md](snapshot-cost.md) for both measured runs and caveats;
`snapshot-cost-isolated.csv` records the final run after other compilation ended.
The full repository suite passed after COW: 466 passed, one benchmark ignored.

## Transport validation (October 5)

- Full native `dialog-reactor` library suite: 52 passed, including opt-in/legacy
  delivery, snapshot-before-checkpoint ordering, checkpoint-only advancement and
  duplicate suppression.
- Full `tonk-host` Wasm browser suite: 60 passed, including the optional checkpoint
  callback, unchanged row callbacks and compatibility with consumers without it.
- Focused display Wasm browser test: one passed, proving that recording delivery
  neither settles rendering nor changes its generation, and teardown clears it.
- Final Wasm check of worker, host and display passed. Existing storage and
  worker unused/dead-code warnings remain.
- Source formatting and whitespace checks passed in both patched repositories.
  Patch artifacts preserve standard blank context lines, which ordinary
  `git diff --check` can flag as whitespace when inspecting a diff of the patch
  itself; source checks are authoritative. Both patch artifacts passed reverse
  application checks against their corresponding modified checkouts, and forward
  application checks against their recorded base trees.
- A shared-target rebuild failed with duplicate crate identities involving
  `/tmp` and `/private/tmp`. Canonicalizing paths allowed the clean repository
  and reactor suites to pass, but a later dialog-db benchmark rebuild reused a
  `dialog-peer` artifact without test helpers after Tonk built into the same
  directory. Use a separate target per workspace as well as canonical paths.
  A fresh dialog-db build in its own directory then passed all 466 repository
  tests and the benchmark. The redundant failing shared-target build was stopped.
- No production deployment, dependency-pin update, full SSE-to-nested-render
  integration test, or generic render-completion receipt has been performed.

## Nested completion contract

The next prerequisite is captured in [render-barrier.md](render-barrier.md) and
its dependency-free Rust test model. Seven native tests cover pending child
work, stale generation/input tickets and conservative receipt invalidation.
This model is not wired into the browser renderer; that adapter is the next
explicit test boundary. No generic rendered revision is exposed.

## First real nested browser case

[nested-browser.md](nested-browser.md) records a test-only adapter over real
parent/child displays and controlled input delivery. Apply its incremental patch
after the transport patch. The focused browser test and all 28 display-hook tests
passed. This extends the native contract evidence to actual DOM mounting, but
still does not establish stale-callback rejection or a generic production receipt.

## Delayed transport callback regression

[subscription-attempt.md](subscription-attempt.md) records a reproduced race in
pending subscription opens and its host-local fix. An older fetch could complete
after navigation and replace the newer stream. Registry attempt identities now
reject obsolete callbacks, late handles and retry timers. This avoids adding a
public renderer protocol solely to solve this host race.

## Local runtime integration

[runtime-completion.md](runtime-completion.md) supersedes the earlier test-only
boundary for a restricted inline path. Its combined patch exposes synchronous
subtree completion evidence in the real runtime and is installed in Tonk's
local profile. Worker integration, native agent and service-worker smoke evidence
are recorded there. Other render paths and a global rendered revision remain
unverified; older entries above describe their respective checkpoints.

The [cold-load follow-up](cold-stream/README.md) isolates idle service-worker chunk
delivery in WKWebView and verifies bounded SSE batching, including clean native
loads and conditional edits. Its worker fix is available as a separate patch.

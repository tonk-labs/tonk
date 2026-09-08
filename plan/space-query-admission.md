# Make mounted-space query admission cheap

The space query route currently checks mount configuration before evaluating
the requested query. That check can build full repository information, reading
names, branches, remotes, revisions and the membership roster through fresh
branch handles. Concurrent queries repeat the same work. The intended outcome
is that an unchanged mounted space reaches its cached reactor branch without
directory queries, roster queries or IndexedDB reads for admission.

Implement this progressively: first narrow the configuration comparison, then
reuse a successful reconciliation while its inputs remain current, and share
slow-path work among concurrent requests. The tradeoff is an explicit
invalidation contract. Correct configuration repair takes priority over a
cache hit; an uncertain or failed check must not produce a reusable receipt.

Scope is query admission and the repository-info route that shares it. Preserve
HTTP response shapes, lazy adoption, local/offline data, existing-remote
preservation, stale-upstream repair and removal behavior. Do not change sync
locking, query planning, UI loading states, replication policy, Dialog pins,
storage schemas or lockfiles in this work.

## Evidence and starting points

[The investigation](space-reload-cache-investigation.md) records the browser
measurements and limitations. A successful `tree/key` formula, which performs
no database evaluation, took 1–3 ms through the profile route and 52–108 ms
through the space route. At 16 concurrent requests, profile requests took about
8 ms and space requests about 891 ms. A real empty-result query took about
819 ms at the same width. Raw reads of 64 existing IndexedDB blocks took 21 ms.
These establish substantial admission overhead, not a per-function profile or
a promised speedup. Background activity produced a separate profile outlier.

Relevant code:

- `rust/tonk-worker/src/router/query.rs`: `query` calls
  `ensure_space_mounted` before `query_on_branch`.
- `rust/tonk-worker/src/router/adopt.rs`: normalization, account exclusion,
  replica lookup, directory reconciliation and adoption.
- `rust/tonk-worker/src/router/repository.rs`: `build_repository_info`,
  `ensure_remote_config`, `record_space_mount`, remote attachment and removal.
- `rust/tonk-schema/src/directory.rs`: `mount_record`, including its swallowed
  auxiliary-query errors, which matter before caching its result.
- `rust/dialog-reactor/src/repository/reference.rs` and
  `branch/reference.rs`: existing cached repository/branch acquisition.
- `rust/tonk-worker/src/worker.rs`: `TonkState`; test construction also lives in
  `rust/tonk-worker/src/router.rs`.

Inspect current instructions and changes before implementation. The worktree
has moved during investigation; use these symbols rather than assuming the
investigation's original HEAD is still current.

## Task 1: Compare mount configuration without reading presentation data

- [x] A mounted-space check no longer calls `build_repository_info`, while
  existing adoption and upstream-repair behavior passes focused tests.

Add a private typed projection in `router/adopt.rs` for the information consumed
by `mounted_configuration_is_current`: existing remote names and local-branch
tracking targets. Read the necessary meta concepts directly and resolve their
entity relationships. Do not read the label, membership, member names,
invitations, inviter provenance, or branch revisions for this comparison.
Initially open durable meta freshly, as today, so this independent increment
does not introduce a stale-meta assumption. Subsequent caching will eliminate
these reads on unchanged admissions.

Preserve the existing comparison semantics: configured remote names must
exist, and configured upstreams must match both durable tracking metadata and
the reactor's cached branch upstream. Do not interpret an address difference
as permission to repoint an existing remote; `ensure_remote_config` deliberately
preserves its effective address/subject. Do not remove extra remotes or branches
or clear an upstream because a desired entry omits one. Retain
`ensure_remote_config` and its `refresh_branch` calls as the repair mechanism.

Represent projection errors as errors, not empty maps. Preserve current
availability behavior at `ensure_space_mounted`: a known local replica remains
usable when directory reconciliation fails, with the failure logged. It must
not be treated as successfully reconciled by the later cache.

Start with a behavioral test in `router/adopt.rs::tests` whose meta configuration
is sufficient but whose content branch cannot satisfy label/roster reads.
Assert admission succeeds without touching those content projections. Use a
scoped test read observer or a provider that rejects unexpected content reads;
do not use wall-clock assertions or source-text matching. It should fail on
the current broad information builder. Retain tests for missing remote,
mismatched tracking, stale cached upstream and unchanged-meta idempotence.
Run the existing first-use and latest-directory-record tests after the change.

Keep `build_repository_info` unchanged for actual information responses. Its
GET route should now construct the full response once, after the narrow
admission check, rather than also constructing it inside that check.

## Task 2: Reuse successful admission with an explicit freshness contract

- [x] Repeated admission of an unchanged mounted space performs no storage or
  configuration queries, and every invalidation case below forces rechecking.

Add a small worker-owned admission cache, in a new
`rust/tonk-worker/src/router/adopt/cache.rs` module owned by `TonkState`. Keep
it separate from the generic reactor: directory/mount policy belongs to the
worker. Initialize it in production and test constructors. Do not persist it.

Key entries by normalized full subject DID. A receipt records the observed
profile-main revision, the local-configuration generation, the identities of
the reactor repository/meta handles it validated, the observed cached meta
revision, and the required cached branch/upstream pairs. Use weak handle
references/identity comparison so a receipt does not retain an evicted
repository. `PROFILE_BRANCH` and the directory branch are both `main` here;
one profile-main revision covers directory facts and local replica membership.

On the hot path, use only already-cached handles. Compare the current
profile-main revision, configuration generation, meta revision, repository
identity and required upstream values against the receipt. A missing handle
is a cache miss, not a reason to reopen storage before checking validity.
Preserve account-repository exclusion before returning a positive receipt.
Content revisions and overlay-only UI/status changes do not invalidate mount
configuration. Conservatively invalidate on any profile-main durable revision
change; finer per-directory-record invalidation is outside this increment.

On a miss, run the existing admission flow with Task 1's narrow projection.
Store a positive receipt only after an authoritative successful check. A
successfully read absent directory record for an already-local replica may
produce a positive receipt; directory read failures must not. Do not cache
unmounted/not-found results or errors. This allows a later directory entry or
join to become visible immediately.

Before caching, distinguish complete directory reads from partial/error
results. Add a strict read path for admission in `tonk-schema::directory` and
propagate failures from execution/branch/tracking reads currently using
`unwrap_or_default`. Keep existing best-effort callers compatible by adapting
at their boundary. Missing optional facts remain valid absence; failed reads
do not. Keep the user-facing rule that a known local replica remains readable
when reconciliation cannot complete, but leave its receipt invalid.

Use a synchronous invalidation guard for direct configuration writers: entering
and leaving a mutation advances a per-subject generation, and an active-writer
count prevents receipts while writes are in flight. Drop must release the
active count on cancellation/error. Do not hold a synchronous map lock across
an await. If a slow admission's freshness stamp changes while it runs, return
its existing result but do not install a receipt; the next caller rechecks.
This also handles admission's own mount/repair writes without retry loops.

The invalidation audit is part of this task, not a future follow-up:

| Change | Required freshness behavior |
| --- | --- |
| Directory or local replica fact changes | Profile-main revision invalidates receipts, including after pull/refresh |
| `ensure_remote_config`, remote attachment, provider detachment, direct upstream/meta writes | Mutation guard invalidates before and after; repair refreshes cached upstreams |
| Generic meta transact/evaluate/claim/import and meta refresh | Cached meta revision changes; direct-handle paths also use the guard |
| Create, join, restore, seed cleanup, remove/evict | Invalidate subject receipt; evicted handle identity must never pass validation |
| Profile/reactor replacement | Replace or clear the admission cache together with the reactor |
| Worker restart | Empty cache; first admission verifies persistent state |
| Content edit, ordinary content pull, UI overlay stamp | No mount invalidation unless it also changes configuration/profile facts |

Inventory all these writers with repository searches before enabling the fast
path. Update shared mutation helpers where possible rather than relying on
each HTTP caller. Preserve the existing removal guards and outer state-lock
ordering; a receipt is not authorization to remount a removed repository.

Tests must initially expose repeated setup and then cover: zero additional
admission reads on a warm hit; both DID spellings share a receipt; directory
change repairs an existing mount; cached upstream corruption causes a miss;
local metadata changes invalidate; directory read failure followed by recovery
retries; valid local-only absence remains usable; profile replacement isolates
receipts; and removal/eviction followed by a lookup cannot reuse an old receipt.
Assert the application's existing adoption/removal semantics, not a new rule
that directory-based adoption can never happen again after removal.

## Task 3: Share slow reconciliation among concurrent queries

- [x] Sixteen simultaneous unchanged admissions perform one slow check, and
  cancellation or concurrent mutation cannot publish a stale success.

Give each normalized subject entry a `tokio::sync::Mutex` for slow admission.
The global entry map is held only long enough to get/create that entry. After
awaiting the subject lock, recheck receipt validity: another request may have
finished reconciliation. Keep the slow operation owned by the requesting
future so cancellation releases the mutex; no detached task or shared error
cache is necessary. Unrelated spaces must not share this lock.

Mutation guards from Task 2 must not acquire this async mutex, because repairs
can mutate configuration while admission owns it. Their generation changes
invalidate any in-flight result instead. For a first mount or repair that
changes the freshness stamp, permit the next waiter to perform a read-only
verification before caching; do not duplicate the mount or publish a receipt
against the pre-mutation stamp.

Use controlled futures/barriers, not sleeps, to test 16 callers arriving during
one check, one cancelled leader, a changed directory revision mid-check,
removal/eviction mid-check, failure followed by a successful caller, and two
subjects making independent progress. Extend real-worker adoption coverage to
verify one replica and no duplicate metadata commit on concurrent first use.

## Task 4: Verify the browser latency reduction and remaining boundaries

- [ ] Structural regressions pass and comparable browser measurements show
  reduced admission cost without changing query results or mount behavior.

Use scoped test counters for directory projection, configuration comparison,
unexpected roster reads and reconciliation executions. Prefer existing test
seams; do not add a public diagnostics endpoint. For the browser comparison,
record state-lock wait, admission and requested-query time separately with
temporary tagged instrumentation, then remove it. Avoid logging facts,
credentials or signed remote URLs.

Run focused Wasm tests through the repository's configured `wbg-pool` runner.
The following commands are proposed verification, not checks performed while
writing this plan. Use the Nix development shell, or its already-active
equivalent:

```sh
nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker router::adopt::tests -- --nocapture
nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker it_attaches_remote -- --nocapture
nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker it_does_not_repoint_a_remote_that_is_already_attached -- --nocapture
```

Add the new cache/concurrency tests under `router::adopt::tests` so the focused
filter includes them. Put strict directory-reader tests under its existing
test module. At the final integration checkpoint run:

```sh
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test --locked -p tonk-schema directory
nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker
git diff --check
```

Run the worker's removal and account/profile lifecycle tests as part of that
integration suite. Verify tests actually executed; compilation alone is not
browser coverage. If runner access or browser compatibility blocks them,
report that boundary and do not mark the task complete.

For live comparison, use the existing Helium staging space if the changed
build is available there, or a controlled local browser fixture with equivalent
directory configuration. Record build, browser, unchanged HTTP-cache setting,
and remote traffic. A local build is not evidence about deployed staging.
Do not deploy or stop the user's worker as part of this plan without the
necessary authorization; ordinary reload testing is already authorized.

Repeat the investigation's successful formula body
`{"predicate":"tree/key","terms":{"key":"0xff"}}` on profile and space
routes with JSON Accept/Content-Type headers. Fully consume each response.
Measure five interleaved serial pairs and batches of 4, 8 and 16; retain
outliers and repeat after background work settles. A warm unchanged batch must
perform zero directory/configuration reads and zero remote block downloads
for admission. The desired live signal is space formula latency close to the
profile-route floor rather than growing by roughly one full reconciliation
per request. Record measured ratios; do not enforce machine-specific absolute
milliseconds in CI.

Finally reload the real issue tracker and compare its first-response timings,
issue count and request waterfall. Any remaining evaluation, global-lock or
rendering latency belongs to a separately measured follow-up. Passing this
plan does not claim instant rendering or cold-worker disk-reopen verification.

## Completion and handoff

Each task is an independently reviewable increment; keep focused tests green
before proceeding. If commits are requested, commit proven increments
separately. This plan requires no implementation during planning. Update the
investigation with before/after evidence, removed instrumentation, test results
and remaining platform gaps when implementation is complete.

Implementation evidence is in [the progress record](space-query-admission-progress.md)
and the investigation's local comparison. Tasks 1–3 are implemented and focused
Wasm tests pass. Task 4 has local browser evidence; its real issue-tracker
comparison remains pending a changed build in that environment.

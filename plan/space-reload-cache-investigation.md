# Space reload and local replication investigation

Status: source trace and live reload captures complete; local query latency
observed, repeated remote block downloading not reproduced. Worker restart
verification blocked by automatic approval review. Diagnosis only: no
implementation changes. Initially examined worktree HEAD `22bbc4e65` and Dialog
revision `805151c678c516edc03c2825171ed4b54adf37ab`, as pinned in Cargo.lock.
Existing account-flow changes were preserved.

## Opening a space

1. `rust/tonk-ui/src/bin/ui.rs` installs the host, awaits
   `tonk_host::ready::require()`, and mounts `tonk-site`.
2. `rust/tonk-portal/src/site.rs::resolve_and_render` issues the transient Load
   command and brings up the guest iframe. The worker's
   `router/session.rs::LoadHandler` matches/stamps the site; guest displays
   subscribe to the resulting model and content.
3. `rust/tonk-worker/src/router/query.rs::query` calls `ensure_space_mounted`
   before dispatching to the reactor. Existing replicas are reconciled against
   the account directory; absent replicas are mounted from directory facts and
   queued for sync.
4. `rust/dialog-reactor/src/repository/reference.rs::acquire` loads/caches the
   repository. `branch/reference.rs::acquire` opens/caches its branch.
5. Dialog `repository/branch/open.rs::OpenBranch::perform` resolves local
   revision, upstream and induction cells. Opening does not itself fetch an
   upstream head. A fresh handle has empty node, rule, plan and other in-memory
   caches. A page reload need not restart the service worker, so these caches
   are not necessarily lost on every page reload.
6. Dialog `repository/branch/select.rs` selects through `NetworkedIndex`:
   local archive lookup, remote fetch only on a miss, then local cache write.
7. Background `router/sync.rs::drain_sync` independently syncs dirty/retry/open
   repositories. Remote activity alone is not evidence of repeated block
   downloads. Dialog fast-forward pull can adopt the remote root without
   downloading its block closure; subsequent queries hydrate missing blocks.

## Storage and confirmed weaknesses

- Worker `DefaultSpace` is `WebSpace`. Dialog's
  `storage/provider/storage/web.rs` maps archive blocks, memory cells,
  credentials and certificates to IndexedDB; binary blobs use OPFS.
- `storage/provider/indexeddb/archive.rs` reads and writes
  `archive/<catalog>` (tree blocks: `archive/index`), keyed by base58 content
  digest. `StoreSession::transact` awaits transaction settlement, so this is
  not simply an unawaited IndexedDB put.
- **Silent persistence failure:** Dialog
  `repository/archive/networked.rs:173-180` discards the cache put result:
  `let _: Result<(), _> = cache.perform(self.local.env()).await;`.
  A remote read can succeed while its durable cache write fails. This is a
  confirmed observability/correctness weakness, not confirmation that the
  affected browser is experiencing write errors.
- **Extra query setup:** `ensure_space_mounted` reconciles existing replicas.
  `mounted_configuration_is_current` invokes `build_repository_info`, which
  opens fresh meta/content handles and queries the content roster. This occurs
  before normal query dispatch and bypasses the reactor's existing branch
  caches for that work. It can cause repeated local IO/evaluation, and network
  reads when those roster blocks are absent. Its latency has not been measured.
- The prior volatile join-staging path is absent here:
  `router/join.rs::perform_join` works directly against a durable hidden replica.
  Sparse replication still means untouched blocks need not be local.

## Required browser discrimination

The affected URL/browser was requested. No live reproduction, IndexedDB
inspection, timings or runtime tests have been performed yet.

1. Capture a settled load and a reload in the same browser profile/origin.
   Separate remote head/auth traffic, content-addressed block GETs, local API
   calls and static assets. Compare block digests, not signed URLs.
2. For a block fetched on both loads, inspect its entry in the space's
   `archive/index` after the first completed load and before the second fetch.
   Record database identity and local revision on both loads.
3. Absent entry: observe the cache-put result/transaction abort without
   suppressing it; determine whether the failure is provider, authorization,
   quota, or lifecycle related. Do not infer the reason from absence alone.
4. Present entry but repeated GET: trace the exact local subject/catalog/key
   used by the fetch path and whether requests are concurrent misses.
5. No repeated block GET: measure readiness, directory reconciliation,
   repository-info/roster reads, subscription evaluation and rendering.
6. Test a settled space with networking disabled while preserving all local
   storage, then restore networking. Distinguish missing untouched content
   from inability to reopen already-read content.

Likely correction depends on that result: expose and handle cache persistence
failure if writes fail; repair address/key selection if stored blocks are
bypassed; narrow/memoize mount reconciliation if repeated setup dominates.
Do not replace lazy replication with unconditional full hydration without
evidence that complete offline replication is the intended contract.

## Helium observation, 2026-09-08

Inspected the existing staging tab for
`did:key:z6MkgK1sLdYGnRg26r42jtnkZ6X3XnZDsjfiikoTEfeMWNHE` through native
Helium DevTools. The window was Incognito; the Issues sheet had rendered.

- Storage UI reported approximately 3.6 MB IndexedDB, 37.7 MB CacheStorage,
  192 kB service workers and 6.5 kB File System for the origin, with roughly
  23 GB quota. These are origin totals, not the space's block size/count.
- A read-only `indexedDB.databases()` console call returned the exact space
  DID database at version 4 and `tonk.profile` at version 4.
- Network recording was on, Keep log off, Disable cache on, no throttling.
  Existing visible remote requests included `branch/main/revision` responses
  of approximately 1.2 kB transferred / 0.8 kB resource, taking 82–167 ms.
  A local repository API response showed 419 ms and ServiceWorker delivery.
  This is an existing partial view of a long recording, not a reload sample.
- The existing console included successful pull/push messages and long-task
  warnings. No cache-write failure was established.
- Before the per-store count probe could execute, the foreground Helium
  window changed to localhost. The CUA tool rejected actions due to concurrent
  user changes. Inspection paused to avoid acting on the wrong profile/tab;
  requested that staging be left selected briefly.
- No reload, storage deletion, setting change or offline simulation was
  performed. Repeated block downloads remain unverified.
- Public `/version.json` independently returned build `0b8618eda455988b`,
  serviceWorker `bc9ec3b03b96c36d`, workerWasm `fea98133fa97821f`. The public
  shell referenced `ui-59c1df053ba51f8c.js`, whereas existing browser traffic
  referenced `ui-33c809aaed70e7c2.js`. This suggests a different/older loaded
  asset generation, but the active browser build has not yet been read.

## Completed reload captures, 2026-09-08

After the user returned Helium to staging:

| Observation | Before reload | After update/reload | Second reload |
| --- | --- | --- | --- |
| Page build | `feaa3ea63ef38728` | `0b8618eda455988b` | `0b8618eda455988b` |
| `archive/index` records | 207 | 209 | 209 |
| Credential records | 1 | 1 | Not recounted |
| Memory records | 7 | 7 | Not recounted |

Counts were obtained through read-only IndexedDB transactions in the top-page
console, closing each connection afterward. Count equality is not a
byte-for-byte comparison of all stored blocks.

The first reload transitioned to the newer runtime and the Network log reset
again during startup (Keep log was off), so it is not a clean single-navigation
measurement. Its final capture showed six R2 requests, all to the space's
`branch/main/revision`, totaling 7,236 transferred bytes; no R2 archive-block
requests appeared in that final capture.

The second, same-build reload rendered the Issues sheet. At the observation
point it had 296 total requests, of which four matched the R2 storage domain.
All four were `branch/main/revision`, each 1,206 transferred bytes / 768 resource
bytes, taking 114–171 ms. Total R2 transfer was 4,824 bytes; total capture transfer
was about 527 kB. No R2 archive-block downloads occurred in this capture.
Network Finish was 12.60 s; this includes asynchronous activity and is NOT an
exact time-to-visible-content measurement. The page briefly showed “Not here”
before rendering the space on both reloads.

Read-only Resource Timing inspection after the second reload found:

- Initial local repository info request started at 389 ms and took 1,328 ms,
  with TTFB 1,327 ms and transferSize 0.
- Several initial space query requests started around 390 ms and took
  1,055–1,227 ms, almost all before first byte.
- A later round of queries started around 5,417–5,602 ms and took
  2,055–2,134 ms, with TTFB 2,046–2,127 ms and transferSize 0.
- Further query rounds continued around 7.5–8.7 s after navigation.

Combined with DevTools reporting local query responses as ServiceWorker
deliveries and the remote capture containing only revision checks, this
places the observed delay on the local worker/query/render path, not a
full-space remote download. These timings include queueing/setup/evaluation;
they do not isolate individual Rust functions or CPU versus IndexedDB latency.
The repeated mount reconciliation/roster construction remains a source-backed
candidate, not a measured root cause. It was rechecked at worktree HEAD
`c96770c47`; no deployed Git SHA was established from the runtime build hash.

Final planned discrimination: stop the worker, retain registration and all
storage, then reload and recapture. Automatic approval review rejected the
Stop action because it could interrupt staging and specific worker control
was not authorized. It was not executed and no indirect workaround was tried.
Thus these captures demonstrate warm reload reuse, not reopening from disk
after losing all service-worker in-memory state. Permission is required to
complete that additional check.

No storage was cleared, no worker was unregistered, no offline or cache
setting was changed, and no application data was edited. DevTools was left
on Application > Service workers following the rejected Stop attempt.

## User-supplied loading sequence

Five screenshots show the pulse/“Not here” title, workspace skeleton, sheet
header with blank body, issue tracker with zero counts/“No issues here”, then
76 issues. Some screenshots are frames in a video player: screenshot filenames
are not a reliable measure of the loading intervals.

The source rebuilds a dependency chain on document reload rather than restoring
a rendered space snapshot. `tonk-portal/src/site.rs:255-312` creates a per-element
site identity, brings up its iframe and sends the transient Load claim.
`tonk-display/src/element.rs:917-932` resolves the concept and subscribes to its
descriptor. The descriptor then drives separate view and entity subscriptions
(`element.rs:1008-1045`); rendering nested displays/guests exposes further data
dependencies. Locally cached blocks avoid downloading, but do not skip this
resolution, evaluation, bridge and rendering work.

The visible empty tracker before populated rows is a pending-versus-empty UX
problem. The screenshots alone do not distinguish an app-initialized empty
collection from an actual early empty subscription frame. The issue tracker's
authored source was not found in the checkout, so that precise cause remains
unverified. Loading should not be presented as an authoritative empty result.

## Local query latency experiments

Ran read-only fetch probes from the staging top-page console, using its
existing worker/profile. No worker stop, storage mutation, source patch or
temporary runtime hook was used. Fetch responses were fully consumed before
recording elapsed `performance.now()` time. Concurrent batches were bounded at
16 requests. These are diagnostic samples on a live application, not isolated
statistical benchmarks; background sync and other work were still running.

### Minimal successful request

POST with JSON Content-Type and Accept headers to either
`/api/profile/branch/main/query` or
`/api/repository/{space DID}/branch/main/query`:

```json
{"predicate":"tree/key","terms":{"key":"0xff"}}
```

`tree/key` only decodes the supplied key. It does no archive read or concept
query evaluation. All successful responses in these comparisons were 200 with
the same 97-byte body.

Five interleaved serial pairs:

- Profile: 3, 2, 1, 1, 2 ms.
- Space: 66, 108, 52, 66, 65 ms.

One batch series (per-request median/max):

| Concurrent requests | Profile median/max ms | Space median/max ms |
| --- | --- | --- |
| 1 | 1395 / 1395 | 181 / 181 |
| 4 | 7 / 7 | 455 / 457 |
| 8 | 9 / 9 | 609 / 610 |
| 16 | 8 / 8 | 891 / 892 |

The 1,395 ms profile outlier is retained: the worker can stall globally, not
just on space-specific work. An earlier batch series also showed profile 2 ms
versus space 66 ms at width 1, and profile 2 ms versus space 233–234 ms at width
4. Its full console output was truncated; do not invent the missing samples.

The first attempted probe omitted `key` and returned 400 with
`bad input for tree/key: key is required`. Those invalid requests are not
counted as successful-query measurements. They nevertheless showed profile
1–6 ms versus space 104–238 ms, consistent with setup occurring before formula
validation. Correcting the input produced the successful measurements above.

### Formula versus actual database evaluation

Compared the same formula against a concept query for an absent attribute:

```json
{"predicate":{"with":{"value":{"the":"tonk.debug.query-probe/absent","as":"Text","cardinality":"one"}}},"terms":{}}
```

All concept probes returned 200 with `[]` (2 bytes); no fact was asserted.

| Concurrent requests | Formula median ms | Empty concept median ms |
| --- | --- | --- |
| 1 | 177 | 100 |
| 8 | 730 | 429 |
| 16 | 839 | 819 |

This recreates approximately 0.8–0.9 seconds of latency without retrieving any
issues, and even without running the concept evaluator. It establishes a
large shared route/setup cost. It does not establish that the actual Issues
query is cheap once setup is removed, or allocate every millisecond to one
Rust function.

### Raw IndexedDB control

Opened the existing space database, fetched archive keys, then read 64 existing
blocks sequentially with a separate readonly transaction per read, closing the
connection afterward. The block reads took 21 ms total; 209 blocks existed.
This bypasses Rust decoding/query evaluation and runs in the page context,
so it is a storage lower-bound control, not an equivalent worker query.

## Prioritized shortening opportunities

### 1. Remove full repository-info construction from query admission

Confirmed call chain:

`router/query.rs::query`
→ `adopt::ensure_space_mounted`
→ `find_replica_for_subject`
→ `reconcile_mounted_space_from_directory`
→ `directory_configuration`
→ `mounted_configuration_is_current`
→ `repository::build_repository_info`.

The last function is an inappropriate projection for checking whether a mount
is current: it reads the content label, four meta query families (branches,
remotes, remote executions, tracking), and four roster query families
(membership, member name, inviter provenance, invitations), plus opens branch
handles to read revisions. Label/roster helpers open fresh content handles;
these do not reuse the reactor's warm node/rule/plan caches. Before that,
`directory::mount_record` queries remotes, per-remote execution metadata,
local and remote branches, and per-local-branch tracking links.

Every simultaneous request repeats this setup. The normal reactor
repository/branch acquisition already has in-memory fast paths, but happens
after these repeated reads. GET repository info also calls `ensure_space_mounted`
and then `build_repository_info` again, potentially constructing the full
projection twice for one response.

Recommended first increment: introduce a narrow mount-configuration projection
containing only the remote/tracking information the check consumes, using
cached reactor branch handles where consistent. Do not read names, roster or
unneeded revisions to admit a query. Keep the full repository-info projection
for callers that actually request it.

Then coalesce concurrent reconciliation for the same normalized subject and
reuse its result while the relevant configuration is unchanged. Invalidate on
account-directory changes, local mount/tracking changes, removal, profile
switch, and worker generation. Do not simply return true whenever the reactor
contains a repo: reconciliation currently repairs stale cached upstream
handles, and deleted/unmounted spaces must not be resurrected.

Validation: repeat the successful pure-formula width 1/4/8/16 probe; measure
reconciliation count and archive reads per batch. Gate on one reconciliation
for unchanged configuration and preservation of adoption, directory updates,
cached-upstream repair and removal behavior. Do not assert an exact latency
speedup until the changed build has been measured.

### 2. Release the global state lock before polling subscriptions

Source-backed stall mechanism, not yet timed independently:
`router/sync.rs::sync` takes `state.write()` to publish pending status (around
line 1013). `publish_sync_status_attr` acquires the branch, asserts the overlay,
schedules polling and **awaits `run_scheduled_polls`** before returning. Thus
the global write-lock scope spans subscription work, despite its “brief”
comment. All query routes need `state.read()` first. `BranchState::poll` visits
subscription hashes serially. The paused path also takes a write lock around
status publication. Manual pull/push handlers have additional write-lock
scopes that should be examined separately from automatic sync.

Recommended increment: separate status mutation/scheduling from execution of
polls, so no global write lock is held across query evaluation or network IO.
Use existing interior/branch synchronization where appropriate; retain the
pending-before-settled delivery contract. Measure lock-wait time separately
from admission and evaluation. A slow subscription should not block a pure
profile formula merely because sync status is being updated.

The live 1,395 ms profile outlier is consistent with a global stall but is not
proof that this particular lock caused it; event-loop contention is another
possibility. No causal claim about that sample is made without lock timing.

### 3. Profile residual evaluator/poll costs after removing setup amplification

Retain per-phase durations for state-lock wait, mount check, branch acquisition,
initial evaluation/poll and projection/serialization, plus IDB gets/misses and
remote fetch counts. Do this on the real Issues subscription as well as the
pure-formula control. The empty concept probe does not represent its joins.

The reactor already retains query engines/results and gates engine work by
revision/overlay epoch; avoid introducing an unversioned second result cache.
`SubscriptionPoll::perform` does project all retained results on every poll
before deciding whether any pending subscriber needs a snapshot. That is
another source-level optimization candidate, but no current measurement
attributes the reload's seconds to projection. Likewise, broad parallelization
of all polls is not justified while the bounded probes show contention.

The current evidence supports starting with mount admission, then global
lock scope, rather than replacing IndexedDB or tuning the issue query blind.

Follow-up applicability check: a read-only profile-main query for
`xyz.tonk.remote/name` plus `xyz.tonk.remote/origin`, with explicit `this`,
`name` and `origin` variable bindings, returned 200 and one row referring to
the affected space. Thus the directory-backed path is relevant to this
browser, rather than being only a hypothetical signed-in case. No returned
configuration content was logged beyond row count and a boolean DID match.
An initial version without variable bindings failed with
`UnboundVariable { variable_name: "this" }`; correcting the bindings succeeded.
This probe verifies directory presence, not per-function timing attribution.

## Admission implementation and local browser comparison, 2026-09-08

Implemented the narrow meta projection, strict directory-reader path, worker-owned
positive receipts, direct-writer invalidation guards, and per-subject slow-path
serialization. Receipts compare profile-main revision, local configuration
generation, weak repository/meta/profile handle identities, cached meta revision,
and required cached upstream values. Content/overlay-only changes do not move
this freshness stamp. Failed reconciliation preserves known-local availability
without a receipt. All temporary runtime probes were removed from source after
building the measurement artifact.

### Controlled fixture and limits

This is a **local fixture**, not a changed staging deployment. It ran the actual
worker HTTP query handlers in a dedicated isolated headless Chrome 152.0.0.0
service worker at `http://127.0.0.1:4187`. The fixture contained a created space
with directory mount facts, an `origin` remote and `main` tracking; its remote
pointed to a loopback UCAN stub returning 503, with no remote content available.
The HTTP cache stayed at the browser default throughout. The instrumented dev
Wasm artifact SHA-256 was
`1b3ac2694eee4166e2e851751d4fd145f8780ac847b7d7e19664bc0e7ecdb873`, built from
base `d4a7ad5eace82c89568eeb90855d246981bc652d` plus this implementation and
removed probes. This used a minimal service-worker shim, not the production
UI/offline-update shell. The isolated browser and loopback server were stopped
when measurements finished.

Temporary `Server-Timing` headers separated state-lock wait, admission and
requested-query time. Temporary counters tracked slow checks, directory reads
and configuration projections. There was no diagnostics endpoint. Raw samples,
including all outliers, are in [space-query-admission-browser.json](space-query-admission-browser.json).

All formula requests used the investigation's exact successful `tree/key` body,
JSON Accept/Content-Type headers, and fully consumed responses. Every response
was 200 with the same 97-byte result. Five interleaved serial pairs per round:

- Round 1 profile: 419.7, 1.1, 1.5, 0.7, 0.9 ms; space: 18.5, 1.0, 0.9, 0.8, 0.7 ms.
- Round 2 profile: 1.2, 0.5, 0.6, 0.5, 0.6 ms; space: 0.7, 0.7, 0.5, 0.5, 0.4 ms.

The round-1 profile outlier coincided with reset counters and a fresh fixture
Wasm/library fetch after idle, consistent with browser-managed worker restart.
Its route-level measured query time was only 2 ms. The following space request
performed one slow check (16 ms admission). Retain these observations without
attributing the whole profile delay to admission or claiming a controlled
cold-worker reopen experiment.

| Round | Width | Profile median/max ms | Space median/max ms | Space/profile median ratio |
| --- | --- | --- | --- | --- |
| 1 | 4 | 0.95 / 1.10 | 0.90 / 1.00 | 0.95 |
| 1 | 8 | 1.35 / 1.60 | 2.35 / 5.20 | 1.74 |
| 1 | 16 | 5.30 / 5.60 | 3.85 / 4.10 | 0.73 |
| 2 | 4 | 0.75 / 0.80 | 0.80 / 0.80 | 1.07 |
| 2 | 8 | 1.35 / 1.60 | 5.60 / 5.70 | 4.15 |
| 2 | 16 | 11.15 / 11.30 | 4.20 / 4.30 | 0.38 |

Every warm batch kept all three counters unchanged. Warm formula admission
measured 0–1 ms at the coarse millisecond clock resolution. The loopback log
recorded background UCAN attempts returning 503, but no archive-block GET or
successful remote content response. Together with zero admission reads, this
establishes zero admission block downloads in the fixture. It does not test
hydration against a functioning hosted remote.

A 16-request actual empty-concept batch returned `[]` for all requests, with
unchanged counters. Total per-request latency was 17.5–20.4 ms; requested-query
time was 12–14 ms and admission was 0–1 ms. Remaining evaluation/dispatch cost
is distinct from the eliminated repeated admission setup.

The earlier staging measurement (about 891 ms at width 16) and this local
fixture are different environments/builds; do not report their quotient as a
controlled deployment speedup. The supported result is elimination of warm
admission reads and space formula latency near the local profile-route floor,
with browser scheduling outliers retained.

### Verification and remaining work

- The content-read observer first failed on the old broad builder, then passed.
- Focused Wasm service-worker suite: 14 adoption/cache/concurrency tests passed.
- Strict directory reader native suite: 2 tests passed.
- The first full worker run passed 376 tests and failed one invite-reopen test.
  It passed alone. Source inspection confirmed a pre-existing fixture collision:
  both join and member-removal tests used subject/ephemeral seed tags `(90, 91)`
  in subject-keyed storage. The reopen fixture now uses its own `(218, 219)`;
  a final full-suite rerun is recorded in the progress file.
- An initial removal test over-specified immediate re-adoption after deleting
  an owned repository's storage. That path reported `Space already exists`,
  including after restoring the original membership-before-open order. The
  test checks that lookup re-enters reconciliation and cannot return an old
  receipt, without changing or inventing removal/adoption policy.
- **Still unverified:** the real Helium issue tracker's post-change first-response
  timings, 76-issue count and rendering waterfall. This change was not deployed
  and the user's worker was not stopped or replaced. No claim is made about
  staging speed, instant rendering, Safari, or controlled cold-worker disk reopen.

### Final corrected build confirmation

Final review found that the initial receipt stamp started after replica lookup.
A controlled test first failed when profile facts changed between that lookup
and configuration reconciliation. The final implementation captures profile
revision/identity and configuration generation **before** replica lookup, then
extends that same freshness window with the repository/meta handles. The new
regression passes. The first-use test also verifies that an absent directory
record does not become a negative cache entry.

The browser experiment was repeated on this corrected code. The final
instrumented artifact SHA-256 is
`e350d990ae655cb4743316770071cee2cfa07a21af3dbf853cbfcc8c119182cd`; its source
probes were again removed before the full-suite run. The same isolated Chrome,
HTTP-cache setting, directory configuration, loopback remote stub, formula,
headers and response-consumption procedure were used. Raw final samples live
under `final_build` in the JSON evidence; the earlier samples remain intact.

| Final round | Width | Profile median/max ms | Space median/max ms | Space/profile median ratio |
| --- | --- | --- | --- | --- |
| 1 | 4 | 0.90 / 1.00 | 1.20 / 1.30 | 1.33 |
| 1 | 8 | 1.50 / 1.70 | 1.50 / 1.60 | 1.00 |
| 1 | 16 | 2.00 / 2.70 | 3.30 / 3.60 | 1.65 |
| 2 | 4 | 2.00 / 2.10 | 0.95 / 1.10 | 0.48 |
| 2 | 8 | 5.00 / 5.20 | 3.55 / 4.00 | 0.71 |
| 2 | 16 | 3.80 / 4.00 | 3.40 / 3.60 | 0.89 |

Final serial pairs:

- Round 1 profile: 1.1, 0.6, 0.6, 0.4, 0.5 ms; space: 0.7, 0.6, 0.6, 0.4, 0.6 ms.
- Round 2 profile: 1.3, 2.2, 0.6, 0.8, 1.3 ms; space: 0.9, 0.6, 0.5, 0.8, 0.5 ms.

All final warm formula samples returned the same 97-byte result, reported
0 ms admission at coarse clock resolution, and retained counters `1,1,1`
(slow checks, directory reads, configuration projections). No additional
admission reads or archive-block downloads occurred. Sixteen final empty-concept
requests returned 200/`[]`, with median 13.05 ms and max 13.30 ms, again without
moving those counters. The final isolated browser and server were shut down.

Final uninstrumented validation:

- Focused adoption suite: **15 passed** through `wbg-pool` in a service worker.
- Full worker suite: **378 passed, 0 failed**, including remote preservation,
  removal, account/profile lifecycle and the isolated invite-reopen fixture.
  Integration binaries and doctests reported no runnable tests; they are not
  counted as additional runtime coverage.
- Native `tonk-schema directory`: **2 passed**.
- Repository `cargo fmt --all -- --check` and `git diff --check`: passed.

The real Helium issue-tracker comparison remains the deployment-dependent gap
listed above; the local fixture does not verify its issue count or rendering.

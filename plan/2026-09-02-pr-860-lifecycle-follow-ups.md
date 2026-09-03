# PR #860 lifecycle follow-ups implementation plan

**Goal:** Reduce immutable-generation download and storage cost, make retained-root eviction self-recovering, remove avoidable per-asset client lookups, and establish multi-tab/Safari evidence after PR #860 lands.

**Approach:** Execute this work on focused branches from `staging` after #860 merges. Reuse only digest-verified responses from existing Tonk generation caches, then prune only cache names that the new active worker can prove belong to obsolete or interrupted Tonk generations. Preserve fail-closed release coherence: an old worker never serves a new deployment's shell, and recovery adopts a successor through the service-worker lifecycle rather than mixing generations in one document.

**Constraints:**

- Start from live `origin/staging` after PR #860 and its blocker fixes merge. Do not append these changes to the already-reviewed lifecycle-core PR unless that merge is deliberately postponed.
- Preserve immutable per-build cache names, the `building|publishing|adopted` marker transaction, `no-store` live fetches, full SHA-256 verification, terminal worker retirement, and the guarded alignment reload.
- Cache reuse is read-only with respect to older generations. A cached response is reusable only after its bytes match the incoming manifest's full digest.
- Never trust a cache-name match, URL match, ETag, response header, previous marker, or content-hashed filename without verifying the bytes against the incoming manifest.
- Never delete unrelated CacheStorage entries. Cleanup may target only exact Tonk lifecycle-owned names that pass strict parsers.
- Do not delete the incumbent generation during install. Reuse needs it, a failed incoming install must leave it operational, and pruning is permitted only after the verified successor begins activation.
- Do not clear IndexedDB, unregister a worker, reset application state, or use storage deletion as general boot recovery.
- Preserve nested-client fail-closed routing: missing or failed client classification delegates to Rust rather than serving a top-level cached asset into a nested guest.
- Keep generation request/write provenance, account update holds, and nested portal protocol changes in their existing dependent PRs; do not recreate them here.
- No new dependency or `Cargo.lock` change is expected.

## Delivery sequence

1. `perf/sw-generation-reuse`: digest-verified reuse with network fallback.
2. `fix/sw-generation-pruning`: activation-time deletion of obsolete Tonk-owned caches.
3. `fix/sw-evicted-root-recovery`: scriptable, bounded successor adoption without shell mixing.
4. `perf/sw-client-classification`: route ordering and per-client classification memoization.
5. Browser validation may accompany each branch, but the final multi-tab/Safari matrix runs against the integrated stack.

Each branch must be independently mergeable. Later branches consume the exact public behavior of earlier branches rather than relying on uncommitted worktree state.

## File map

- `rust/tonk-ui/assets/service_worker.js`: generation-source selection, activation cleanup, eviction recovery response, and nested-client classification cache.
- `rust/tonk-ui/tests/service-worker.test.mjs`: network reuse counts, digest mismatch fallback, exact-name pruning, recovery-page behavior, and client lookup counts.
- `rust/tonk-ui/tests/service-worker-claim.test.mjs`: successor activation/reload behavior where recovery or multi-client state crosses the page boundary.
- `rust/tonk-ui/src/service_worker_upgrade.rs`: real-browser generation reuse/pruning, multi-tab controller replacement, state preservation, and Safari A-to-B coverage.
- `rust/tonk-ui/README.md`: generation retention, eviction recovery, activation, and compatibility behavior.
- `plan/service-worker-upgrade-on-load.md`: update the operational contract and verification matrix as each follow-up lands.
- `.github/workflows/test.yml`: change only if a deterministic browser scenario can run reliably in CI without extending unrelated jobs beyond their current timeout.

### Task 1: Reuse byte-identical members from prior generations

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:installGeneration, fetchVerifiedAssets, worker-Wasm acquisition, cache-name helpers`
- Test: `rust/tonk-ui/tests/service-worker.test.mjs:immutable generation install`
- Document: `rust/tonk-ui/README.md:generation installation`

**Interfaces:**

- Consumes: incoming `entries: Array<[path, sha256]>`, `WORKER_WASM_HASH`, `caches.keys()`, and immutable final cache names.
- Produces strict cache-name parsers:

```javascript
function parseFinalShellGeneration(name) // build id or null
function parseFinalWorkerGeneration(name) // build id or null
```

- Produces a read-only source function:

```javascript
async function verifiedGenerationResponse(cacheNames, key, expectedHash)
// Response | null
```

  It searches only parsed final Tonk cache names other than the incoming build, uses `caches.match(key, { cacheName })`, hashes a clone with the full incoming digest, and returns the original cached response only on equality. Missing, unreadable, or mismatched candidates return `null` and fall back to the existing `no-store` network verification.
- Produces the same behavior for worker Wasm using `TONK_WORKER_<build>` candidates and the stamped worker digest prefix; changing worker Wasm normally falls through to network.
- Preserves: every selected response still enters the existing staging/publishing transaction and is rechecked by `existingGenerationIsComplete` before adoption.

- [ ] Add `reuses_unchanged_members_without_network_fetches`. Seed generation A with an adopted shell cache containing `/`, a stable font, and an unchanged library asset; install generation B whose manifest gives those same paths/digests plus one changed asset. Assert only the manifest, changed asset, and changed worker Wasm use network fetches.
- [ ] Add table-driven candidate rejection for a digest mismatch, missing response, malformed Tonk-like cache name, unreadable response body, and cache lookup failure. Assert each case performs the verified network fetch and never mutates or deletes the candidate cache.
- [ ] Add a collision case with two old caches containing the same path but different bytes. Assert the first digest-matching response wins regardless of cache enumeration order; if none match, exactly one network request occurs.
- [ ] Add a network-offline case where all incoming assets and worker Wasm are byte-identical to verified prior responses. Assert the new generation installs completely offline after the incoming manifest itself has been obtained and verified. Do not claim deployment discovery works offline.
- [ ] Run `node --test --test-name-pattern='reuses unchanged|candidate rejection|collision|installs completely offline' rust/tonk-ui/tests/service-worker.test.mjs`; expect the reuse assertions to fail because the current install fetches every member with `no-store`.
- [ ] Implement strict final-cache parsing and enumerate candidates once per install. Do not scan stage caches or the unrelated cache namespace.
- [ ] Implement cached-response verification and network fallback. Keep bounded concurrency for network work; avoid an `assets × generations` open-cache loop by carrying the parsed candidate list once and stopping on the first verified match.
- [ ] Reuse the response object without synthesizing a replacement so URL metadata remains intact. Hash only a clone.
- [ ] Run the focused tests and then `node --test 'rust/tonk-ui/tests/*.test.mjs'`; expect success and record network request counts in assertion messages.
- [ ] Build `.#tonk-cloudflare-artifacts`, inspect its manifest count and byte sizes, and report both decoded artifact bytes and observed browser/network transfer separately. Reuse reduces repeated fetches; it does not make the complete offline generation smaller.

### Task 2: Prune only obsolete Tonk generation caches after activation begins

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:self.onactivate, generation marker/cache-name helpers`
- Modify: `rust/tonk-ui/tests/service-worker.test.mjs:cache naming, immutable generation caches`
- Modify: `rust/tonk-ui/src/service_worker_upgrade.rs:it_adopts_a_complete_second_generation_without_mixing_assets`
- Document: `rust/tonk-ui/README.md:generation retention`

**Interfaces:**

- Consumes: the incoming generation's adopted marker and the fact that the Service Worker Activate algorithm has already made this worker the registration's active worker before dispatching `activate`.
- Produces:

```javascript
async function pruneObsoleteGenerationCaches()
```

- Deletes only names accepted by exact parsers for:

```text
TONK_SHELL_<16-lowercase-hex-build>
TONK_WORKER_<16-lowercase-hex-build>
TONK_GENERATION_<16-lowercase-hex-build>
TONK_SHELL_STAGE_<16-lowercase-hex-build>_<validated-nonce>
TONK_WORKER_STAGE_<16-lowercase-hex-build>_<validated-nonce>
```

- Retains all current-build final and marker caches. Current-build interrupted staging remains owned by `installGeneration` recovery and must not be guessed at during activation.
- Runs under `event.waitUntil`, but catches and logs individual deletion failures so optional cleanup cannot reject activation or prevent application boot. A later activation may retry names that remain.

- [ ] Replace the current test assertion that generation A remains after B activation. Assert B's final shell, worker, and adopted marker remain; A's parsed lifecycle caches are removed; and an unrelated sentinel cache remains byte-for-byte readable.
- [ ] Add exact-name parser cases proving uppercase IDs, short IDs, extra suffixes, near-prefix names, and `tonk-sw-upgrade-sentinel` are never deletion candidates.
- [ ] Add an install-failure case: B fails hash verification before activation, A remains active, and every A cache still exists. Pruning must not be callable from the install failure path.
- [ ] Add a deletion-failure case where one obsolete cache rejects. Assert activation work resolves, the failure is logged, current generation caches remain, and other eligible obsolete caches are still attempted with `Promise.allSettled` or equivalent per-name isolation.
- [ ] Run `node --test --test-name-pattern='prune|activation retains|install failure' rust/tonk-ui/tests/service-worker.test.mjs`; expect the new pruning assertions to fail because current `onactivate` performs no cache cleanup.
- [ ] Implement the exact parsers and adopted-current-generation precondition. Refuse cleanup if the current marker is missing, malformed, or not `adopted`.
- [ ] Attach cleanup to `self.onactivate` without putting `activateWorker()` behind the same `waitUntil`; preserve the existing avoidance of the outgoing-worker lock deadlock.
- [ ] Run the focused Node tests and the full Node suite; expect success.
- [ ] Run the existing Chrome A-to-B test after changing its cache assertion. Expect coherent B document/worker/manifest identity, A lifecycle caches absent, and both the IndexedDB and unrelated CacheStorage sentinels preserved.
- [ ] Add a three-generation Node case and assert cache count is bounded after each successful activation rather than growing linearly with deploy count.

### Task 3: Make an evicted retained root adopt a successor without mixing shells

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:missingGenerationAssetResponse, serveNavigation`
- Test: `rust/tonk-ui/tests/service-worker.test.mjs:immutable generation caches`
- Test: `rust/tonk-ui/tests/service-worker-claim.test.mjs:recovery document`
- Document: `rust/tonk-ui/README.md:retained asset eviction`

**Interfaces:**

- Consumes: a navigation whose incumbent `TONK_SHELL_<build>` has no `/`, where `fetchVerifiedRetainedAsset("/")` cannot prove the live bytes belong to that incumbent build.
- Produces: an HTML recovery response generated entirely by the incumbent worker. It contains no application bundle and never serves the current deployment's `/` as the incumbent document.
- The recovery script:

```text
1. registers a one-shot controllerchange listener;
2. calls getRegistration(), then registration.update();
3. reloads once only after controllerchange, guarded by
   sessionStorage["tonk:sw-eviction-reload"];
4. on unchanged, redundant, timeout, offline, or rejected update, renders a
   retry button and leaves local data untouched.
```

- The response uses `text/html; charset=utf-8`, `cache-control: no-store`, a restrictive inline-only CSP appropriate to its authored script/style, and direct copy explaining that Tonk is checking for a recoverable current version.

- [ ] Extend the evicted-shell test to assert the response remains fail-closed: it contains no fetched new shell bytes, no application scripts, and no write to the missing incumbent cache.
- [ ] Extract and execute the recovery script with a fake registration. When `update()` installs/activates B and dispatches `controllerchange`, assert exactly one guarded reload.
- [ ] Add unchanged, update rejection, redundant successor, 30-second timeout, and unavailable registration cases. Assert no automatic reload, actionable retry state, and no cache/registration/data deletion.
- [ ] Add a second-document case with `tonk:sw-eviction-reload` already set. Assert the guard is consumed and another immediate reload is not scheduled.
- [ ] Run `node --test --test-name-pattern='evicted shell|eviction recovery' rust/tonk-ui/tests/service-worker.test.mjs rust/tonk-ui/tests/service-worker-claim.test.mjs`; expect the executable recovery cases to fail because the current response is scriptless `text/plain`.
- [ ] Implement the recovery response with escaped, constant-authored markup. Do not interpolate raw error text into HTML or script.
- [ ] Keep the existing verified same-generation recovery fast path: if the live manifest and `/` still match the incumbent digest, return that ephemeral original response without backfilling.
- [ ] Run focused and complete Node suites; expect success.
- [ ] In a real-browser A-to-B fixture, delete only A's cached `/`, promote B, navigate, and assert the recovery document causes one successor adoption/reload into coherent B. Verify the unrelated CacheStorage and IndexedDB sentinels remain.

### Task 4: Avoid repeated client lookups on obvious routes and stable clients

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:isNestedClientRequest, routeFetch`
- Test: `rust/tonk-ui/tests/service-worker.test.mjs:exact fetch routing`

**Interfaces:**

- Consumes: `FetchEvent.clientId`, `Client.frameType`, route path/mode, and current fail-closed behavior.
- Produces a worker-lifetime memo:

```javascript
const clientFrameTypes = new Map(); // clientId -> "top-level" | "nested"
```

- The first ambiguous top-level-cache request for a client calls `clients.get`; subsequent requests from that client reuse its immutable `frameType` classification for the worker lifetime.
- API requests and other routes that always delegate to Rust skip classification entirely. Ambiguous navigations and exact manifest-member subresources classify before using the top-level shell cache.
- Missing clients, unknown frame types, and lookup rejection are not memoized as top-level; they delegate to Rust.

- [ ] Add a request-count test issuing several manifest-member subresources from one top-level client. Assert one `clients.get` call and cache responses for every request.
- [ ] Add the corresponding nested-client case. Assert one lookup and Rust routing for every request, never a shell-cache response.
- [ ] Add API/non-cacheable cases and assert zero client lookups because their route is Rust regardless of frame type.
- [ ] Add lookup-missing and lookup-rejection cases. Assert each ambiguous request delegates to Rust and a later successful lookup can still classify the same client; failures must not poison the memo.
- [ ] Run `node --test --test-name-pattern='client lookup|exact fetch routing' rust/tonk-ui/tests/service-worker.test.mjs`; expect repeated top-level/nested requests to show one lookup per asset on the current implementation.
- [ ] Reorder unconditional Rust routes before ambiguous classification and add the bounded worker-lifetime memo.
- [ ] Run focused and complete Node suites; expect success with unchanged routing outcomes and reduced lookup counts.

### Task 5: Verify multi-tab replacement, bounded storage, and Safari/WebKit A-to-B

**Files:**

- Modify: `rust/tonk-ui/src/service_worker_upgrade.rs:test helpers and lifecycle tests`
- Modify if justified by reliable CI runtime: `.github/workflows/test.yml:service-worker/browser job`
- Document: `rust/tonk-ui/README.md:browser evidence`
- Document: `plan/service-worker-upgrade-on-load.md:verification matrix`

**Interfaces:**

- Consumes: Tasks 1-4 integrated on current `staging`, the mutable two-generation fixture, browser window handles, document build markers, `/api/health`, manifest digests, cache inventory, and state sentinels.
- Produces two independent browser scenarios:

```text
it_reloads_every_update_aware_tab_after_controller_replacement
it_recovers_an_evicted_root_into_the_current_generation
```

- Produces an evidence matrix that records Chrome and Safari separately. Compilation, Node harnesses, Chrome, and Safari are distinct evidence layers.

- [ ] Add the two-tab Chrome test. Mount generation A in two tabs, promote B, refresh only tab 1, and wait for B activation. Assert both tabs end with B document provenance, B controller health, one reload per affected tab, no repeated guard, and no old lazy-asset 404 after replacement.
- [ ] Preserve state independently in each tab where applicable and at origin scope for IndexedDB/CacheStorage. Assert pruning deletes only lifecycle-owned A caches and retains sentinels.
- [ ] Add the real-browser evicted-root test described in Task 3 and prove one recovery reload rather than relying on the spec's implicit soft-update scheduling.
- [ ] Run each Chrome test alone with `--test-threads=1 --nocapture`; expect success without manual reload, worker unregistration, or storage clearing.
- [ ] Run `cargo test -p tonk-ui --features integration-tests -- --test-threads=1`; expect the serialized browser suite to pass.
- [ ] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`, `cargo test -p dialog-reactor`, and `cargo test -p tonk-worker`; expect all lower-layer lifecycle/retirement checks to remain green.
- [ ] Build `.#tonk-cloudflare-artifacts`; record manifest member count, decoded manifest-graph bytes, worker-Wasm bytes, and a three-generation test cache inventory. Do not infer physical on-disk deduplication from logical CacheStorage entries.
- [ ] Run the same A-to-B, two-tab, offline fallback, and evicted-root scenarios with `TONK_TEST_BROWSER=safari`. If WebDriver session creation fails before navigation, report infrastructure failure separately and do not claim Safari behavior is verified.
- [ ] After deployment to staging, perform one real A-to-B upgrade starting from the previously deployed version without clearing site data. Record exact old/new build IDs, browser versions, controller changes, reload counts, final cache names, and sentinel preservation.
- [ ] Add a CI browser job only if the scenario is deterministic on a clean runner and fits a declared timeout. Otherwise retain Node tests in CI and document the manual/deployed Safari gate explicitly.
- [ ] Run `cargo fmt --all -- --check` and `git diff --check`; expect clean results.

## Completion gate

- A new generation fetches only manifest members whose bytes cannot be proven equal in prior Tonk final caches.
- Reuse never mutates an old cache and never accepts a digest mismatch.
- After successful activation, only the current generation's lifecycle caches remain; unrelated caches and IndexedDB remain untouched.
- A failed install leaves the incumbent generation and all local application state intact.
- An evicted root initiates explicit update/adoption and reloads once on controller replacement without ever serving new shell bytes under the old controller.
- Static assets from a stable client require at most one `clients.get` lookup per worker lifetime; uncertain/nested requests remain fail-closed.
- Chrome multi-tab A-to-B is executable and green.
- Safari/WebKit A-to-B is either green with recorded browser evidence or explicitly reported as unverified; stable promotion must not describe Chrome/Node evidence as Safari proof.

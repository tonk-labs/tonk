# PR #860 pre-merge blockers implementation plan

**Goal:** Close the cold-install and boot-failure correctness gaps in PR #860, and make its documented activation contract match browser behavior, without broadening the lifecycle core into cache optimization or generation protocol work.

**Approach:** Keep the immutable generation transaction and unconditional one-release `skipWaiting()` bridge intact. Add observable liveness while response bodies are actually arriving, make unsupported-service-worker handling execute before any service-worker API access, and remove the duplicate terminal promise consumer. Replace the misleading claim-based activation test and documentation with the browser's real distinction: activation replaces the controller of already-controlled clients, while `clients.claim()` only extends control to otherwise-uncontrolled clients.

**Constraints:**

- Preserve the complete manifest and worker-Wasm verification transaction: installation must still fail before activation when any required member is missing or hash-mismatched.
- Do not reconstruct verified network responses merely to report progress; the response placed in CacheStorage must retain its original URL list and response metadata.
- Progress is a liveness hint only. Failure to publish a progress message must never fail, delay, or complete an install.
- Do not keep a genuinely stalled read alive indefinitely by emitting timer-only heartbeats. A liveness update must correspond to bytes received or a completed install phase.
- Keep the one-release unconditional `skipWaiting()` bridge in this PR. Page-directed waiting-worker activation and bridge removal are follow-up work.
- Never unregister a worker, clear CacheStorage, delete IndexedDB, reset a profile, or delete a passkey in boot recovery.
- Do not add dependencies or change `Cargo.lock`.
- Keep generation reuse, generation pruning, evicted-root recovery, and nested-client lookup optimization out of this pre-merge patch.

## File map

- `rust/tonk-ui/assets/service_worker.js`: stream verified response clones and publish byte-backed install liveness without changing the generation transaction.
- `rust/tonk-ui/index.html`: order the service-worker support check before API access, classify the known unsupported case directly, and leave one rejection observer/renderer.
- `rust/tonk-ui/tests/service-worker.test.mjs`: exercise delayed streaming bodies, hash verification, and progress-message failure isolation.
- `rust/tonk-ui/tests/boot-script.test.mjs`: execute the boot module with service workers absent and with registration failures; prove one terminal report and no rethrow.
- `rust/tonk-ui/tests/service-worker-claim.test.mjs`: replace the false activation claim with an exact assertion about explicit `clients.claim()` calls and retain the page-directed cold-start case.
- `rust/tonk-ui/README.md`: describe controller replacement during activation and the one-release rollout consequence accurately.
- `plan/service-worker-upgrade-on-load.md`: correct the adopted lifecycle contract and validation language.
- `docs/plans/2026-09-02-pr-860-lifecycle-review-fixes.md`: record the final review corrections and fresh verification evidence.

### Task 1: Report liveness while a verified body is arriving

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:fetchVerified, fetchVerifiedAssets, fetchVerifiedWorkerWasm, reportInstallProgress`
- Test: `rust/tonk-ui/tests/service-worker.test.mjs:immutable generation install, worker wasm verification`

**Interfaces:**

- Consumes: `fetchVerified(url, expectedHash, label)` and its current callers for the manifest, manifest members, retained-asset recovery, and worker Wasm.
- Produces: an optional progress callback on the verified network read:

```javascript
async function fetchVerified(url, expectedHash, label, onChunk = null)
```

- `onChunk` receives a monotonically increasing decoded-byte count after each non-empty `ReadableStream` chunk. It is never called for cached reads and its rejection is ignored.
- Produces: a helper that consumes `response.clone().body.getReader()`, concatenates the chunks into the exact `ArrayBuffer` passed to `digestOf`, and falls back to `response.clone().arrayBuffer()` only when `Response.body` or `getReader` is unavailable.
- Preserves: the original, unconsumed `response` is returned and later stored in CacheStorage; retained-asset recovery continues to verify without backfilling old caches.

- [ ] Add `reports_progress_during_one_slow_verified_body`. Return a `Response` backed by a controlled `ReadableStream`; enqueue advancing chunks at intervals shorter than the 30-second watchdog window while keeping the overall transfer open for more than 60 seconds, then close it. Assert a well-formed `tonk-install-progress` message is published after each advancing chunk and the verified response succeeds.
- [ ] Add `does_not_report_liveness_for_a_frozen_body`. Leave a controlled stream open without enqueuing another chunk and assert no further progress message appears. Cancel the stream at test teardown so the test itself cannot hang.
- [ ] Add or extend the worker-Wasm test so `fetchVerifiedWorkerWasm()` supplies byte-backed progress while reading the large stable-name body; this is the path that currently has no intermediate liveness.
- [ ] Add a case where `BroadcastChannel`, `clients.matchAll`, or a client `postMessage` fails during chunk progress. Assert the verified fetch and install still complete and hashes are unchanged.
- [ ] Run `node --test --test-name-pattern='slow verified body|frozen body|worker wasm|progress' rust/tonk-ui/tests/service-worker.test.mjs`; expect the slow-body test to fail because current progress occurs only after `arrayBuffer()` resolves.
- [ ] Implement the streamed clone reader. Copy chunks into one `Uint8Array` only after EOF, preserving their order and exact total length; hash that buffer exactly once.
- [ ] Pass per-read progress callbacks from `fetchVerifiedAssets` and `fetchVerifiedWorkerWasm`. Use the existing throttled `reportInstallProgress` transport, but force the first byte-backed update for a body so a large final member cannot inherit an old throttle timestamp and remain silent.
- [ ] Do not attach byte progress to `fetchVerifiedRetainedAsset`: retained runtime reads do not participate in the uncontrolled first-install watchdog.
- [ ] Run the focused Node command again; expect every new case to pass with no pending stream handles.
- [ ] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`; expect the full service-worker suite to pass and report the actual test count.

### Task 2: Make boot failure handling deterministic and single-owned

**Files:**

- Modify: `rust/tonk-ui/index.html:service-worker bootstrap module, terminal module at end of body`
- Test: `rust/tonk-ui/tests/boot-script.test.mjs:boot script contract`
- Test: `rust/tonk-ui/tests/boot-terminal.test.mjs:terminal watchdog contract`

**Interfaces:**

- Consumes: the eager `serviceWorkerActivation` promise and `presentBootFailure(message)`.
- Produces: one support fact captured before service-worker API access:

```javascript
const serviceWorkersSupported = "serviceWorker" in navigator;
```

- Produces: direct classification in the existing rejection observer: `!serviceWorkersSupported` maps to `OLD_BROWSER_BOOT_FAILURE`; registration, network, MIME, script, and activation errors map to `GENERIC_BOOT_FAILURE` unless a later explicitly typed condition is introduced with its own behavioral test.
- Preserves: `self.serviceWorkerActivates = () => serviceWorkerActivation` remains the strict Rust pre-mount gate; the promise remains rejected for `tonk_host::ready::require()` even though the inline observer renders the failure immediately.

- [ ] Extend the boot-module harness to execute with `navigator = {}`. Assert evaluation does not throw while installing listeners, `serviceWorkerActivates()` rejects with the explicit unsupported diagnostic, the boot shell displays `OLD_BROWSER_BOOT_FAILURE`, and no registration/update method is called.
- [ ] Add a supported-browser registration rejection whose error mentions `module` and `MIME type`. Assert it receives `GENERIC_BOOT_FAILURE`, proving message substrings no longer select old-browser guidance.
- [ ] Add a logging/settlement case that evaluates all authored module blocks with a rejected registration. Assert exactly one `service-worker activation failed` diagnostic is logged, exactly one terminal message is rendered, and no module evaluation rethrows the already-observed rejection.
- [ ] Run `node --test rust/tonk-ui/tests/boot-script.test.mjs rust/tonk-ui/tests/boot-terminal.test.mjs`; expect the absent-service-worker case to fail at the early `navigator.serviceWorker.addEventListener` access and the duplicate case to observe two logs plus a rethrow.
- [ ] Guard service-worker message and `BroadcastChannel` install-progress listener setup behind `serviceWorkersSupported`. Keep the support check before every `navigator.serviceWorker` dereference in the module.
- [ ] Replace `/module|type/i` classification with the captured support fact. Do not infer browser age from an exception message.
- [ ] Remove the trailing `<script type="module">` that awaits, logs, and rethrows `self.serviceWorkerActivates()`. The eager promise and Rust gate remain the only execution paths.
- [ ] Run the focused Node command again; expect all boot cases to pass with one failure presentation and no uncaught rejection.
- [ ] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`; expect the complete suite to pass.

### Task 3: Correct the activation contract without changing rollout behavior

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:self.onactivate comments, claim-message comments`
- Modify: `rust/tonk-ui/tests/service-worker-claim.test.mjs:activation test name and assertions`
- Modify: `rust/tonk-ui/README.md:How the SPA and service worker compose`
- Modify: `plan/service-worker-upgrade-on-load.md:Summary, requirements, rollout bridge`
- Modify: `docs/plans/2026-09-02-pr-860-lifecycle-review-fixes.md:Second review decisions and verification`

**Interfaces:**

- Consumes: unconditional `skipWaiting()` after a verified install, browser activation, the permanent `controllerchange` listener in update-aware documents, and the explicit `{ type: "claim" }` message used for uncontrolled first visits.
- Produces this documented contract:

```text
Activation replaces the active worker for clients already using the
registration and fires controllerchange. The activate handler does not call
clients.claim(); an explicit claim remains necessary only to control an
otherwise-uncontrolled first-install document.
```

- Preserves: the one-release bridge and current runtime code; no new activation message or compatibility protocol is introduced here.

- [ ] Rename `activation alone does not claim pre-upgrade pages` to `the activate handler does not call clients.claim`. Keep the executable assertion at zero explicit claim calls, and add a comment that the harness does not simulate the browser's controller replacement algorithm.
- [ ] Add a page-harness case with two update-aware documents sharing one registration/controller transition. Dispatch one `controllerchange` and assert each document independently records one guarded reload. Do not pretend this harness proves Safari behavior.
- [ ] Run `node --test rust/tonk-ui/tests/service-worker-claim.test.mjs`; expect the two-document case to fail until the harness exposes both permanent listeners correctly. No production change should be necessary if the existing listener is truly global and persistent.
- [ ] Correct `service_worker.js`, `README.md`, and both plan files: remove every claim that activation leaves already-controlled pages on their incumbent controller. State the first-rollout limitation directly: a cached page without a persistent replacement listener can be switched to the successor without automatically reloading, so its old lazy asset URLs may fail until navigation.
- [ ] Keep the distinction between activation and `clients.claim()` precise. Do not remove the cold-first-install claim request or claim that `skipWaiting()` itself fires `controllerchange`; activation does.
- [ ] Run `rg -n 'activation alone|remain on their current controller|leaves older|only.*claim' rust/tonk-ui plan docs/plans/2026-09-02-pr-860-lifecycle-review-fixes.md`; expect no statement that already-controlled clients remain on the old active worker after successor activation.
- [ ] Run the focused claim tests and then the complete Node suite; expect success.

### Task 4: Verify the corrected PR head and refresh its description

**Files:**

- Verify: all files above
- Update externally: PR #860 body only after the final committed diff is known

**Interfaces:**

- Consumes: Tasks 1-3 complete and committed.
- Produces: a PR description whose scope, test count, behavior, and residual risks match the actual head and live base.

- [ ] Run `cargo fmt --all -- --check`; expect no formatting diff.
- [ ] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`; expect zero failures and record the emitted test count rather than carrying forward `102` or `72`.
- [ ] Run `cargo test -p dialog-reactor subscription --lib`; expect the one-way subscription lifecycle coverage to remain green.
- [ ] Run `cargo test -p tonk-worker lsp::tests --lib`; expect all terminal LSP hub tests to remain green.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_adopts_a_complete_second_generation_without_mixing_assets -- --test-threads=1 --nocapture`; expect the Chrome A-to-B generation test to pass without cache, registration, IndexedDB, or profile cleanup.
- [ ] Run `git diff --check origin/staging...HEAD`; expect no whitespace errors.
- [ ] Inspect `git diff --stat origin/staging...HEAD` and `git status --short`; confirm only reviewed PR files and these plan files changed.
- [ ] Refresh live `origin/staging`, confirm PR #860 still resolves to the tested head, and check `gh pr checks 860 --repo tonk-labs/tonk` after pushing. Report any stale base, force-update, or CI failure before claiming readiness.
- [ ] Update the PR body with the current file/addition/deletion count, actual Node test count, accurate controller-replacement behavior, the full-generation download cost, and the remaining Safari/WebKit evidence gap.

## Completion gate

- A response body that continues delivering chunks for longer than two watchdog windows keeps the first-install page alive without weakening hash verification.
- A body that delivers no bytes and makes no lifecycle progress is still allowed to reach bounded watchdog recovery.
- A browser without `navigator.serviceWorker` reaches the specific unsupported-browser terminal copy without a JavaScript exception or reload loop.
- A supported browser's registration/deployment error receives generic recovery guidance even when its message contains `module`, `type`, or `MIME`.
- A rejected activation produces one diagnostic, one terminal presentation, and no authored-module rethrow.
- Tests and documentation no longer claim that activation preserves an incumbent controller for already-controlled pages.
- Current Node, Rust, browser, formatting, diff, and live CI evidence is recorded after the final change.

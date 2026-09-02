# PR #860 lifecycle review fixes

**Goal:** Remove the upgrade regressions while preserving graceful, non-destructive A-to-B service-worker adoption.

**Approach:** Keep the fixes inside the lifecycle core. Use the browser's worker identity to restrict retirement to the active incumbent, count the observed successor's install progress as boot liveness, preserve the host's existing successful-SSE handoff protocol, and make recovery waits expire on a real timer.

**Constraints:**

- Never unregister workers or clear Cache Storage, IndexedDB, or application state.
- Do not add generation-protocol, provenance, CORS, or nested-runtime work to #860.
- Add executable regressions; replace source-string assertions that currently bless the defects.

## File map

- `rust/tonk-ui/assets/service_worker.js`: automatic adoption, retryable retirement, and verified asset recovery.
- `rust/tonk-ui/index.html`: incoming-install progress handling.
- `rust/tonk-worker/src/router/query.rs`: intentional subscription-handoff response.
- `rust/tonk-ui/tests/service-worker-claim.test.mjs`: incumbent versus waiting-worker behavior.
- `rust/tonk-ui/tests/boot-script.test.mjs`: A-document/B-worker progress behavior.
- `rust/tonk-ui/tests/service-worker.test.mjs`: frozen-worker recovery timeout behavior.
- `rust/tonk-worker/src/router.rs`: retiring-query SSE regression.

### Task 1: Only the active incumbent may retire

- [x] Extend the service-worker harness so `registration.active`, `registration.waiting`, and `self.serviceWorker` can identify the executing worker.
- [x] Add a failing test that evaluates a restarted waiting B (`waiting === self.serviceWorker`, active is A) and proves B never calls `onupdatefound`; retain a companion case proving active A retires once when B is waiting.
- [x] Centralize the active-incumbent identity check and apply it to startup catch-up, `updatefound`, and fetch-path catch-up before calling `retire()`.
- [x] Run `node --test rust/tonk-ui/tests/service-worker-claim.test.mjs`; expect the new test to fail before the change and pass afterward.

### Task 2: Count successor progress as boot liveness

- [x] Replace the same-build source assertion with a behavioral warm-update test: document A observes installing worker B, receives `tonk-install-progress` tagged B, and calls `tonkBootLife`.
- [x] Make `noteInstallProgress` accept well-formed progress for the currently observed install lifecycle rather than requiring `message.build === globalThis.tonkBuild`; ignore unrelated message types.
- [x] Exercise both delivery paths that share the handler: `navigator.serviceWorker` messages and the progress `BroadcastChannel`.
- [x] Run `node --test rust/tonk-ui/tests/boot-script.test.mjs rust/tonk-ui/tests/service-worker-claim.test.mjs`; expect success.

### Task 3: Preserve the successful SSE retirement handoff

- [x] Change the retiring-query regression to require HTTP 200, `text/event-stream`, a snapshot, one `data: {"control":"update-pending"}` frame, and then EOF.
- [x] Replace the 503 JSON response with that finite SSE response. It must preserve the initial snapshot and close immediately after its control frame so it cannot pin the retiring worker.
- [x] Leave `tonk-host`'s control-frame handling unchanged: it consumes the frame, marks the subscription as awaiting `controllerchange`, suppresses a consumer error, and uses the held reconnect delay.
- [x] Run the `it_releases_sse_subscribers_on_shutdown` regression in the Wasm browser harness; expect the rewritten assertion to fail before the change and pass afterward.

### Task 4: Bound watchdog recovery

- [x] Remove the account-safety reload gate until it has a real production writer.
- [x] Make the first silent stall perform one direct, non-destructive reload even when IndexedDB or Web Locks are unavailable.
- [x] Make a second stall terminate with the failure line instead of entering another recovery loop.
- [x] Cover both recovery outcomes with executable fake-time tests.

### Final verification

- [x] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test -p tonk-worker`.
- [x] Run `NEXTEST_TEST_THREADS=4 nix develop path:. -c test:web:debug`.
- [ ] Run `nix build .#checks.x86_64-linux.clippy -L` (blocked because the `x86_64-linux` derivation is unavailable on this `aarch64-darwin` host).
- [x] Run `git diff --check`.

Keep the four fixes as one reviewable lifecycle-core commit unless test harness changes make a separate test-only commit clearer. Do not stack later protocol work into this branch.

## Second review decisions

The full 19-commit review found additional rollout and scope regressions. The
product decisions for this pass are:

- Enforce the original no-polling, no-update-prompt scope.
- Ship a one-release automatic-activation bridge for pages deployed before this
  lifecycle protocol; successors do not park waiting for a new page message.
- Keep updates automatic rather than offering Reload / Not now.
- Defer the account-setup reload hold until it has a real production writer.
- Recover an evicted retained asset only from hash-verified bytes and never
  backfill the sealed generation cache.
- Keep `guide/` and `docs/storybook/` local-only; neither belongs in the
  Cloudflare artifact or service-worker generation graph.

### Task 5: Restore deployable automatic adoption

- [x] Update the stale browser query assertion to expect the finite SSE handoff.
- [x] Always call `skipWaiting()` after a verified install for the one-release
  bridge and add an active-incumbent regression.
- [x] Set the JavaScript retirement latch only after stream release succeeds so
  a later fetch can retry a failed release; hold that API fetch until the retry
  succeeds so it cannot reopen a stream first.
- [x] Send one snapshot frame before the update-pending control frame.

### Task 6: Remove deferred and out-of-scope control planes

- [x] Remove the update bar, hourly registration update, 15-minute version
  probe, and kill-switch fetch/state.
- [x] Remove the account-safety IndexedDB/Web Locks consumer and its fabricated
  tests; recovery reloads remain bounded and non-destructive.
- [x] Remove the guide and Storybook from the Cloudflare bundle and retained
  generation graph (`9c122ef3e`).

### Task 7: Close cache, tooling, and CI gaps

- [x] Normalize query-string asset cache keys.
- [x] Hash-verify an ephemeral network response on retained-asset eviction.
- [x] Remove the stale diagnostics `controllerchange` listener on teardown.
- [x] Make stamp hashing fail closed without relying on non-portable pipefail.
- [x] Exclude generated TypeScript output from the source fingerprint.
- [x] Run `test:sw` in CI and replace the affected source-string tests with
  behavioral coverage.
- [x] Reconcile stale lifecycle comments and documentation.

### Second-pass verification

- [x] `node --test 'rust/tonk-ui/tests/*.test.mjs'` (72/72)
- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p tonk-worker -p dialog-reactor` (required host access for
  loopback test servers; 166 passed, one ignored doc test)
- [x] `cargo check -p tonk-ui --features integration-tests`
- [ ] repository Clippy gate available on this `aarch64-darwin` host
- [x] real-browser automatic A-to-B rollout, stream release, opaque-child
  provenance relay, and offline fallback (4/4)
- [ ] live rollout starting from the currently deployed pre-protocol staging
  page; the one-release `skipWaiting()` bridge is covered behaviorally but has
  not been exercised against that external deployment
- [x] `nix build --accept-flake-config .#tonk-cloudflare-artifacts`
- [x] inspect the built artifact and confirm it has no `guide/` or
  `docs/storybook/` directory
- [x] `git diff --check`

### Deferred type cleanup

No retirement newtype or cross-language control-frame protocol was introduced
in this correctness pass. `TonkServiceWorker::retiring` and
`TonkState::retiring` are two handles to the same `Arc<AtomicBool>`, and profile
replacement explicitly preserves that `Arc`; the JavaScript latch separately
tracks completion of the asynchronous Rust release hook. Consolidating the
Rust and TypeScript control-frame decoders remains useful follow-up work, but
it is not required for the rollout bridge and would broaden this change into a
wire-protocol refactor.

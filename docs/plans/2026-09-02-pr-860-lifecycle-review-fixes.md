# PR #860 lifecycle review fixes

**Goal:** Remove the four upgrade regressions while preserving graceful, non-destructive A-to-B service-worker adoption.

**Approach:** Keep the fixes inside the lifecycle core. Use the browser's worker identity to restrict retirement to the active incumbent, count the observed successor's install progress as boot liveness, preserve the host's existing successful-SSE handoff protocol, and make recovery waits expire on a real timer.

**Constraints:**

- Never unregister workers or clear Cache Storage, IndexedDB, or application state.
- Do not add generation-protocol, provenance, CORS, or nested-runtime work to #860.
- Add executable regressions; replace source-string assertions that currently bless the defects.

## File map

- `rust/tonk-ui/assets/service_worker.js`: worker-role retirement and bounded failure-page recovery.
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

- [x] Change the retiring-query regression to require HTTP 200, `text/event-stream`, one `data: {"control":"update-pending"}` frame, and then EOF.
- [x] Replace the 503 JSON response with that finite SSE response. It must close immediately after its control frame so it cannot pin the retiring worker.
- [x] Leave `tonk-host`'s control-frame handling unchanged: it consumes the frame, marks the subscription as awaiting `controllerchange`, suppresses a consumer error, and uses the held reconnect delay.
- [x] Run the `it_releases_sse_subscribers_on_shutdown` regression in the Wasm browser harness; expect the rewritten assertion to fail before the change and pass afterward.

### Task 4: Make recovery deadlines actually expire

- [x] Add an executable failure-page test with a worker frozen in `installing`; advance fake time past 30 seconds and assert that adoption rejects, the button is re-enabled, and retry copy is shown.
- [x] Implement `waitForState` with one deadline timer plus a state-change listener. Settle once, clear the timer, and remove the listener on success, redundancy, or timeout.
- [x] Retain the existing 30-second bounds for install, activation, and claim, without reloading or modifying local state on failure.
- [x] Run `node --test rust/tonk-ui/tests/service-worker.test.mjs`; expect success.

### Final verification

- [x] Run `node --test 'rust/tonk-ui/tests/*.test.mjs'`.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test -p tonk-worker`.
- [x] Run `NEXTEST_TEST_THREADS=4 nix develop path:. -c test:web:debug`.
- [ ] Run `nix build .#checks.x86_64-linux.clippy -L` (blocked because the `x86_64-linux` derivation is unavailable on this `aarch64-darwin` host).
- [x] Run `git diff --check`.

Keep the four fixes as one reviewable lifecycle-core commit unless test harness changes make a separate test-only commit clearer. Do not stack later protocol work into this branch.

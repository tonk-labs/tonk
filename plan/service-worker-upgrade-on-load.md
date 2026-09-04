# Service-worker load-time upgrade implementation plan

**Goal:** Make an online staging load detect, activate, and adopt the current Tonk service worker without DevTools cleanup or briefly mounting the stale UI, while preserving an already-installed worker and all local application state when the update check cannot reach the network.

**Approach:** Start one memoized registration-and-update promise eagerly in the existing pre-Wasm bootstrap and explicitly call `ServiceWorkerRegistration.update()` on every user-initiated warm load. Keep the static boot overlay mounted while the UI Wasm downloads, gate `mount_root()` on strict completion of that promise, and reload once when successor activation replaces the controller and fires `controllerchange`; the stale document therefore never exposes its application root, and the automatic alignment document skips one redundant update check. Prove the behavior against a deliberately slowed mutable service worker in the real-browser harness, then make the worker-Wasm stamp describe the final Nix artifact rather than the pre-fixup Trunk artifact.

**Constraints:**

- The root cause is the warm-load fast path, not HTTP caching: `navigator.serviceWorker.register()` with the same scope, script URL, worker type, and update-via-cache mode returns the existing registration without scheduling an update, while the current `serviceWorkerActivates` returns as soon as any controller exists. `updateViaCache: "none"` affects fetching inside an update job; it does not start one. Explicitly update on every user-initiated warm load; the sole exception is the automatic alignment reload immediately after a successful replacement.
- Preserve `/service_worker.js`, module registration, scope, and `updateViaCache: "none"`. Do not add a versioned registration URL or create parallel registrations.
- Preserve the worker's current `install` precache, `skipWaiting()`, explicit `{type:"claim"}` message handling, and outgoing-worker teardown after a successor installs. Activation replaces the active worker for already-controlled clients and fires `controllerchange`; the activate handler does not call `clients.claim()`, and the explicit claim remains for otherwise-uncontrolled first-install pages.
- A first-ever install still waits for control and continues in the same document. Only replacement of a controller that existed at the beginning of this load causes the alignment reload.
- A warm replacement reloads at most once per boot sequence, guarded by `sessionStorage` key `tonk:sw-upgrade-reload`. A stable load clears the guard. The reload happens before `mount_root()` creates `#tonk-root`, so neither UI chrome nor `/api/*` activity from the stale document becomes visible.
- Keep the existing inline `#tonk-boot` overlay visible throughout update detection and controller replacement. An `updatefound` transition changes only its status text to `updating…`; no new modal, toast, or app-level loading view is introduced.
- An update-check failure is recoverable only while the page still has its prior controller. First-install registration failure and loss of the existing controller are explicit boot failures: the strict gate leaves the root unmounted and terminalizes the boot shell without automatic reload or cleanup. An incoming worker that stops progressing without producing an error remains a silent stall handled by one plain watchdog reload followed by a terminal safe-state message.
- Do not unregister service workers, delete CacheStorage, delete IndexedDB, or reset Tonk state as part of upgrade handling or automatic stall recovery. Known readiness errors stop the watchdog through the terminal hook above; a second silent stall terminalizes without cleanup.
- Keep offline navigation cache-first. The new worker's install already refreshes the `/` shell cache, so the one post-replacement reload aligns the new controller with the fresh shell without changing fetch policy.
- The first rollout uses a one-release bridge: every verified successor calls `skipWaiting()` even when an incumbent exists. Activation replaces the controller of already-controlled pages. An update-aware page performs the guarded alignment reload on `controllerchange`; a cached pre-protocol page without that persistent listener can switch controllers without reloading and may fail to load old lazy asset URLs until navigation.
- Add no Cargo, JavaScript, or Nix dependency, and do not change a lock file.
- Do not add periodic polling, an update prompt, schema migrations, cache-version changes, or worker/data compatibility machinery in this change.
- A cached pre-protocol page cannot safely continue in place after browser activation replaces its controller. Without the persistent replacement listener it may keep its old document and lazy asset URLs until navigation; update-aware pages use `controllerchange` and the guarded alignment reload.
- Browser results count only when Chrome and ChromeDriver are compatible. Report a driver/version failure as an infrastructure blocker, separately from compile, Nix-build, or product behavior evidence.

## File map

- `rust/tonk-ui/index.html`: own registration and warm-load update adoption in the single pre-Wasm service-worker bootstrap; remove the late duplicate registration module.
- `rust/tonk-host/src/ready.rs`: add a strict readiness entry point for the UI mount while retaining the tolerant gate used by existing host IO callers.
- `rust/tonk-ui/src/bin/ui.rs`: wait for strict worker readiness before mounting the top-document application root.
- `rust/tonk-ui/src/service_worker_upgrade.rs`: native real-browser regressions for online replacement, automatic shell alignment, state preservation, and offline fallback.
- `rust/tonk-ui/src/lib.rs`: register the new native test module.
- `rust/tonk-ui/src/helpers.rs`: expose a per-test mutable copy of `service_worker.js` through `TestEnvironment`.
- `flake.nix`: let `tonk-ui-test-server` serve that mutable script over the otherwise immutable Tonk UI build, and restamp the final `tonk-ui` Nix output after fixup.
- `rust/tonk-ui/scripts/stamp-service-worker.sh`: stamp and verify the final `worker_bg.wasm` hash in a distribution directory.
- `rust/tonk-ui/scripts/hash-guest.sh`: Trunk post-build hook that hashes guest assets before invoking the canonical stamper.
- `rust/tonk-ui/README.md`: document the load-time update, single-reload, and offline behavior.

### Task 1: Replace a recent active worker during an ordinary online load

**Files:**

- Modify: `flake.nix:tonk-ui-test-server`
- Modify: `rust/tonk-ui/src/helpers.rs:TestEnvironment` and `TestServers::start`
- Create: `rust/tonk-ui/src/service_worker_upgrade.rs`
- Modify: `rust/tonk-ui/src/lib.rs` (native test module declarations)
- Modify: `rust/tonk-ui/index.html:serviceWorkerActivates` and the final registration module
- Modify: `rust/tonk-host/src/ready.rs:wait` and new `require`
- Modify: `rust/tonk-ui/src/bin/ui.rs:main`
- Test: `rust/tonk-ui/src/service_worker_upgrade.rs:it_replaces_a_recent_worker_on_load_without_manual_cleanup`

**Interfaces:**

- `tonk-ui-test-server` consumes optional positional argument 3, `SERVICE_WORKER_ROOT`. When present, the script copies `${self.packages.${system}.tonk-ui}/service_worker.js` into that writable directory before Caddy starts and serves only `/service_worker.js` from it; every other static path remains rooted at the immutable `tonk-ui` output. With no third argument, existing callers continue serving the complete immutable output.
- `TestEnvironment` produces `pub service_worker_script: std::path::PathBuf`, pointing to the per-harness copy at `caddy_data/service-worker/service_worker.js`. `TestServers::start` creates the parent directory, passes it as argument 3, and does not share it across ports or tests.
- `index.html` produces one eagerly-started, module-local `Promise<void>` covering registration, update detection, controller replacement, connectivity notification, and any alignment reload. `serviceWorkerActivates(): Promise<void>` always returns that same promise; no work waits for the first `/api/*` request, and no second terminal promise consumer remains at the end of `<body>`. Attach one immediate diagnostic rejection observer to the eager promise so a later strict Rust await does not leave an interim `unhandledrejection` report.
- `serviceWorkerActivates` preserves the existing explicit first-install claim path. On a warm load it captures the prior controller, attaches `controllerchange`, `updatefound`, and incoming-worker `statechange` observers, and first adopts any `registration.installing` or `registration.waiting` worker that already exists. When no update is already in flight, it awaits `registration.update()`.
- The warm observer has three explicit outcomes: `unchanged` when the update finds identical bytes; `replaced` when activation fires `controllerchange` with a controller other than the captured controller; and `failed` when the incoming worker becomes `redundant`. No warm-page claim is required: activation replaces controllers for clients already using the registration. The observer calls `tonkBootLife` on update and state transitions so a progressing install does not look stalled to the existing watchdog, and writes `updating…` to `[data-boot-status]` after `updatefound`.
- On `replaced`, set `sessionStorage["tonk:sw-upgrade-reload"] = "1"`, call `location.reload()`, and leave `serviceWorkerActivates` pending so no old-document mount or `/api/*` boot continues. The next document sees both the marker and its already-replaced controller, removes the marker, skips exactly that load's redundant network update check, sends connectivity, and resolves. This makes the guard a deterministic one-shot alignment handoff rather than a counter: even if another deployment lands during the few milliseconds between documents, it waits for the next user-initiated load instead of causing an automatic reload chain.
- On `unchanged` or `failed`, retain the prior active controller, clear a stale reload guard, send connectivity, and resolve. `failed` also emits a diagnostic warning naming the incoming worker state; it does not unregister or clear storage. Every outcome removes the temporary registration, controller, and worker-state listeners before returning or reloading.
- `tonk_host::ready` produces `#[cfg(target_arch = "wasm32")] pub async fn require() -> Result<(), wasm_bindgen::JsValue>`. Missing browser globals retain the current embed/test-harness no-op behavior, but rejection of an actual `serviceWorkerActivates()` promise is returned and does not set `SW_READY`. Existing `pub async fn wait()` calls `require()`, deliberately discards its result for backward compatibility, and preserves every current IO call site.
- `ui::main` completes custom-element and host-hook registration plus the existing debug-only hot-swap injection, calls `tonk_host::ready::require().await`, and invokes `mount_root()` only on `Ok(())`. On `Err`, it reports the JavaScript value to the console, asks `#tonk-boot` to enter its idempotent terminal state, and returns. Terminalization cancels the automatic watchdog ladder, clears its retry counter, preserves the first cause-specific message, and never reloads or removes caches/registrations. The gate belongs immediately before `mount_root()`, so no connected callback or app root can race the update while dev reload behavior remains unchanged.

- [ ] Extend `tonk-ui-test-server` with the optional service-worker root and a Caddy `handle /service_worker.js` before the catch-all handler. Extend `TestEnvironment` / `TestServers::start` with the unique mutable script path. Run `cargo check -p tonk-ui --features integration-tests`; expect success and no behavior change yet.
- [ ] Add `#[cfg(test)] mod service_worker_upgrade;` to `src/lib.rs`, with the new file internally gated to native `integration-tests` / `web-integration-tests`, matching `account_flow.rs`.
- [ ] Add `it_replaces_a_recent_worker_on_load_without_manual_cleanup`. Let `env.driver()` establish worker A; poll `fetch("/api/health")` until it returns JSON and record `startedAt`. Before the upgrade, create an IndexedDB record and a dedicated CacheStorage entry as state sentinels.
- [ ] In that test, use `Page.addScriptToEvaluateOnNewDocument` to increment a sessionStorage document counter and attach a mutation observer that records whether `#tonk-root`, `tonk-site`, `tonk-account`, or `tonk-activate` appeared in each numbered document. Produce a distinct, correctly restamped generation and inject a test-only 1.5-second wait at the start of its install transaction before calling `driver.refresh()` once. Do not call `unregister()`, `Storage.clearDataForOrigin`, or cache deletion from the test.
- [ ] Poll rather than sleep until `/api/health.startedAt` differs from worker A. Then assert the registration has an activated controller and no `installing` or `waiting` worker, the document counter is `2` (the requested refresh plus one automatic alignment reload), document 1 never mounted an application root despite the slow install, document 2 did mount one, `tonk:sw-upgrade-reload` is absent, and both IndexedDB and CacheStorage sentinels remain readable.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_replaces_a_recent_worker_on_load_without_manual_cleanup -- --test-threads=1`; expect failure because the health `startedAt` remains worker A's value, the registration has no incoming worker, the document counter remains `1`, and the stale document mounts an application root.
- [ ] Move registration and the complete activation/update operation into the early module as the eager shared promise above. Remove the final registration `<script type="module">` entirely. Keep `sendConnectivity`, `tonkRegisterSync`, visibility forwarding, and the first-install claim nudge using the shared registration.
- [ ] Add strict `tonk_host::ready::require`, keep `wait` source-compatible, and gate only `ui::main`'s `mount_root()` call on `require` success. Run `cargo check -p tonk-ui --target wasm32-unknown-unknown`; expect the new strict signature and every existing tolerant host caller to compile.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_replaces_a_recent_worker_on_load_without_manual_cleanup -- --test-threads=1`; expect worker B's `startedAt`, exactly one automatic alignment reload, no application root in the pre-reload document, a mounted fresh document, a settled registration, a cleared guard, and both state sentinels preserved.
- [ ] Run `cargo test -p tonk-ui --features integration-tests identity::tests::it_serves_deployment_config_on_the_page_origin -- --test-threads=1`; expect the existing first-install/browser bootstrap path to remain green.

### Task 2: Keep the active worker usable when the load-time update check is offline

**Files:**

- Modify: `rust/tonk-ui/src/service_worker_upgrade.rs`
- Modify: `rust/tonk-ui/index.html:serviceWorkerActivates` update-error branch
- Modify: `rust/tonk-ui/README.md:How the SPA and service worker compose`
- Test: `rust/tonk-ui/src/service_worker_upgrade.rs:it_keeps_the_active_worker_when_the_load_time_update_check_is_offline`

**Interfaces:**

- The update-error branch consumes the controller captured before `registration.update()`. It may fall back only when `navigator.serviceWorker.controller` is still that controller; it logs the rejected update, clears a stale upgrade-reload guard, sends connectivity, and lets cached boot continue. If no controller remains, it rethrows.
- The offline test uses `thirtyfour::extensions::cdp::ChromeDevTools` with `Network.enable` and `Network.emulateNetworkConditions({ offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0 })`. It always restores `offline: false` and quits the driver after collecting the test result, including assertion-error paths.
- The README records four distinct cases: first install waits for an explicit claim without reloading; online warm load explicitly checks and replaces behind the static boot overlay; actual warm replacement reloads once on `controllerchange` before the application root mounts; offline warm load keeps the current worker and local state. It also records the one-time bootstrap boundary for documents cached before this implementation lands.

- [ ] Add `it_keeps_the_active_worker_when_the_load_time_update_check_is_offline`. Establish worker A and a cached `/` shell, record `/api/health.startedAt`, create the IndexedDB and CacheStorage sentinels, switch Chrome offline through CDP, and refresh.
- [ ] Assert the cached page mounts a Tonk root, `/api/health` still answers through a controlled page with worker A's `startedAt`, no `installing` or `waiting` worker remains, the upgrade-reload guard is absent, and both state sentinels survive. Restore networking before returning from the test.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_keeps_the_active_worker_when_the_load_time_update_check_is_offline -- --test-threads=1`; expect failure after Task 1 because the rejected explicit update prevents `serviceWorkerActivates` from resolving and the UI does not mount.
- [ ] Catch the update rejection only in the prior-controller case described above; do not convert first-install failure into success and do not invoke the boot-recovery deletion ladder.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_keeps_the_active_worker_when_the_load_time_update_check_is_offline -- --test-threads=1`; expect the old controller, cached shell, mounted UI, and both state sentinels to remain available.
- [ ] Update the README composition section with the four-case lifecycle, the pre-mount boot overlay, the one-time pre-fix bootstrap boundary, and the fact that `updateViaCache: "none"` is paired with an explicit `registration.update()` on warm load.
- [ ] Run both upgrade tests together with `cargo test -p tonk-ui --features integration-tests service_worker_upgrade::tests -- --test-threads=1`; expect success.

### Task 3: Stamp the complete final browser generation

`scripts/stamp-service-worker.sh <dist-dir>` computes one build identity over
the normalized service-worker policy, final worker glue/Wasm, and every
published browser resource. It writes the build meta, `ASSET_PATHS`,
`asset-manifest.json`, and `version.json` transactionally. The manifest and
every member use full SHA-256 digests; the worker Wasm identity uses the stamped
prefix checked again at runtime. The Trunk post-build hook invokes the canonical
stamper for local output, and the Nix artifact is restamped only after its final
browser graph is assembled.

The stamper hashes normalized temporary files rather than pipelines, so a
failed normalizer cannot silently publish the empty-input digest. Generated
TypeScript outputs are not source-fingerprint inputs; a clean checkout can
therefore verify the checked-in bundle without first generating ignored files.

### Task 4: Verify the complete staging upgrade path

**Files:**

- Verify only: all files above

**Interfaces:**

- No new interface. This task establishes fresh evidence after the final code, harness, documentation, and Nix changes.

- [ ] Run `cargo fmt --all -- --check`; expect no Rust formatting diff.
- [ ] Run `nix develop -c nixfmt --check flake.nix`; expect no Nix formatting diff.
- [ ] Run `cargo check -p tonk-ui --target wasm32-unknown-unknown`; expect the production UI and strict `tonk-host` readiness gate to compile.
- [ ] Run `cargo test -p tonk-ui --features integration-tests service_worker_upgrade::tests -- --test-threads=1`; expect both online replacement and offline fallback to pass in the same final tree.
- [ ] Run `NO_HEADLESS=1 cargo test -p tonk-ui --features integration-tests it_replaces_a_recent_worker_on_load_without_manual_cleanup -- --test-threads=1` with compatible Chrome/ChromeDriver; expect the deliberately slow update to remain on the static boot overlay, reload once, and reveal only the fresh application. Record this as manual visual evidence separately from the automated DOM assertions.
- [ ] Run `cargo test -p tonk-ui --features integration-tests -- --test-threads=1`; expect the serialized real-browser suite to pass. If local ChromeDriver cannot start the installed Chrome, record the exact versions and error and leave browser behavior unclaimed until CI or a compatible local pair runs it.
- [ ] Run `nix --accept-flake-config build --no-link .#tonk-ui`; expect the production UI derivation, including final-output stamp verification, to succeed.
- [ ] Run `git diff --check`; expect no whitespace errors.
- [ ] Inspect `git diff -- rust/tonk-ui/index.html rust/tonk-host/src/ready.rs rust/tonk-ui/src/bin/ui.rs rust/tonk-ui/src/service_worker_upgrade.rs rust/tonk-ui/src/lib.rs rust/tonk-ui/src/helpers.rs rust/tonk-ui/scripts/hash-guest.sh rust/tonk-ui/scripts/stamp-service-worker.sh rust/tonk-ui/README.md flake.nix`; confirm the final diff contains no ordinary-flow unregister, CacheStorage deletion, IndexedDB deletion, cache-policy rewrite, unrelated UI change, or lock-file change.

## Immutable-generation lifecycle follow-up contract

The load-time replacement contract above now has four bounded follow-ups:

- An incoming install may read older final Tonk shell and worker caches, but it
  accepts a response only after hashing a clone against the incoming manifest or
  stamped worker-Wasm digest. Reuse never writes to the source cache, and every
  accepted response still enters the incoming staging and adoption transaction.
- Activation may delete only exact lifecycle-owned cache names for builds other
  than the adopted current build. It keeps current final and staging caches,
  ignores Tonk-like and unrelated names, and treats individual deletion failures
  as cleanup diagnostics rather than activation failures.
- A missing retained root returns a self-contained recovery page. That page
  explicitly checks for an update, reloads once only after controller
  replacement, and otherwise exposes retry without fetching a live shell or
  changing local application state.
- Frame classification is cached only after `clients.get()` confirms a stable
  top-level or nested frame type. Unconditional Rust routes skip classification;
  missing or failed lookups remain retryable and delegate to Rust.

Verification remains layered. Node tests prove byte selection, exact-name
cleanup, recovery control flow, and routing counts. Native compilation proves
the harness builds. Serialized browser scenarios separately prove coherent A to
B adoption, multi-tab controller replacement, evicted-root recovery, offline
fallback where the browser supports network control, cache inventory, and
sentinel preservation. Chrome and Safari results must be recorded separately;
a WebDriver setup failure is infrastructure evidence, not browser-behavior
evidence.

### Local follow-up evidence, 2026-09-02

| Layer | Scenario | Result |
| --- | --- | --- |
| Node | Reuse, strict pruning, recovery control flow, and routing memoization | Pass |
| Chrome | Coherent A-to-B adoption | Pass |
| Chrome | Two update-aware tabs adopt B once each | Pass |
| Chrome | Evicted root adopts B without state cleanup | Pass |
| Safari | Coherent A-to-B adoption | Unverified: SafariDriver timed out while creating the automation session before navigation |

The Safari result above is an infrastructure failure only. It does not establish
whether the lifecycle behavior passes or fails in WebKit.

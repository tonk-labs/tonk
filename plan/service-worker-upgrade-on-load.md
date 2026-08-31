# Service-worker load-time upgrade implementation plan

**Goal:** Make an online staging load detect, activate, and adopt the current Tonk service worker without DevTools cleanup or briefly mounting the stale UI, while preserving an already-installed worker and all local application state when the update check cannot reach the network.

**Approach:** Start one memoized registration-and-update promise eagerly in the existing pre-Wasm bootstrap and explicitly call `ServiceWorkerRegistration.update()` on every user-initiated warm load. Keep the static boot overlay mounted while the UI Wasm downloads, gate `mount_root()` on strict completion of that promise, and reload once only after the update-aware page explicitly asks an activated successor to claim it; the stale document therefore never exposes its application root, and the automatic alignment document skips one redundant update check. Prove the behavior against a deliberately slowed mutable service worker in the real-browser harness, then make the worker-Wasm stamp describe the final Nix artifact rather than the pre-fixup Trunk artifact.

**Constraints:**

- The root cause is the warm-load fast path, not HTTP caching: `navigator.serviceWorker.register()` with the same scope, script URL, worker type, and update-via-cache mode returns the existing registration without scheduling an update, while the current `serviceWorkerActivates` returns as soon as any controller exists. `updateViaCache: "none"` affects fetching inside an update job; it does not start one. Explicitly update on every user-initiated warm load; the sole exception is the automatic alignment reload immediately after a successful replacement.
- Preserve `/service_worker.js`, module registration, scope, and `updateViaCache: "none"`. Do not add a versioned registration URL or create parallel registrations.
- Preserve the worker's current `install` precache, `skipWaiting()`, explicit `{type:"claim"}` message handling, and outgoing worker `updatefound` teardown. Activation alone must not claim pre-protocol pages; only a page that can align itself asks its observed successor to take control.
- A first-ever install still waits for control and continues in the same document. Only replacement of a controller that existed at the beginning of this load causes the alignment reload.
- A warm replacement reloads at most once per boot sequence, guarded by `sessionStorage` key `tonk:sw-upgrade-reload`. A stable load clears the guard. The reload happens before `mount_root()` creates `#tonk-root`, so neither UI chrome nor `/api/*` activity from the stale document becomes visible.
- Keep the existing inline `#tonk-boot` overlay visible throughout update detection and controller replacement. An `updatefound` transition changes only its status text to `updating…`; no new modal, toast, or app-level loading view is introduced.
- An update-check failure is recoverable only while the page still has its prior controller. First-install registration failure, loss of the existing controller, and an incoming worker that never reaches either replacement or `redundant` remain real boot failures handled by the existing bounded boot watchdog.
- Do not unregister service workers, delete CacheStorage, delete IndexedDB, or reset Tonk state as part of normal upgrade handling. The existing last-resort boot watchdog remains unchanged.
- Keep offline navigation cache-first. The new worker's install already refreshes the `/` shell cache, so the one post-replacement reload aligns the new controller with the fresh shell without changing fetch policy.
- The first rollout has an unavoidable bootstrap boundary: a document cached before this change does not contain the explicit update call. Its current worker will stale-refresh `/` in the background, and the next ordinary navigation will run the new bootstrap. Document this one-time extra revisit; no new artifact can retroactively execute inside an already-cached old document. Once the new bootstrap is cached, later deployments update on the first warm load.
- Add no Cargo, JavaScript, or Nix dependency, and do not change a lock file.
- Do not add periodic polling, an update prompt, schema migrations, cache-version changes, or worker/data compatibility machinery in this change.
- A cached pre-protocol page cannot safely adopt a protocol-aware worker in place. It remains on its existing controller until navigation; update-aware pages use the explicit claim/cutover path and retain the guarded alignment reload.
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
- `rust/tonk-ui/scripts/hash-guest.sh`: delegate service-worker stamping to the shared script after hashing the guest assets.
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
- `index.html` produces one eagerly-started, module-local `Promise<void>` covering registration, update detection, controller replacement, connectivity notification, and any alignment reload. `serviceWorkerActivates(): Promise<void>` always returns that same promise; no work waits for the first `/api/*` request, and no second registration call remains at the end of `<body>`. Attach an immediate diagnostic rejection observer to the eager promise so a later strict Rust await does not leave an interim `unhandledrejection` report.
- `serviceWorkerActivates` preserves the existing explicit first-install claim path. On a warm load it captures the prior controller, attaches `controllerchange`, `updatefound`, and incoming-worker `statechange` observers, and first adopts any `registration.installing` or `registration.waiting` worker that already exists. When no update is already in flight, it awaits `registration.update()`.
- The warm observer has three explicit outcomes: `unchanged` when the update finds identical bytes; `replaced` when `controllerchange` installs a controller other than the captured controller; and `failed` when the incoming worker becomes `redundant`. When its incoming worker reaches `activated`, the update-aware page sends that worker exactly one `{type:"claim"}` message; older pages send no such request and keep their controller. The observer calls `tonkBootLife` on update and state transitions so a progressing install does not look stalled to the existing watchdog, and writes `updating…` to `[data-boot-status]` after `updatefound`.
- On `replaced`, set `sessionStorage["tonk:sw-upgrade-reload"] = "1"`, call `location.reload()`, and leave `serviceWorkerActivates` pending so no old-document mount or `/api/*` boot continues. The next document sees both the marker and its already-replaced controller, removes the marker, skips exactly that load's redundant network update check, sends connectivity, and resolves. This makes the guard a deterministic one-shot alignment handoff rather than a counter: even if another deployment lands during the few milliseconds between documents, it waits for the next user-initiated load instead of causing an automatic reload chain.
- On `unchanged` or `failed`, retain the prior active controller, clear a stale reload guard, send connectivity, and resolve. `failed` also emits a diagnostic warning naming the incoming worker state; it does not unregister or clear storage. Every outcome removes the temporary registration, controller, and worker-state listeners before returning or reloading.
- `tonk_host::ready` produces `#[cfg(target_arch = "wasm32")] pub async fn require() -> Result<(), wasm_bindgen::JsValue>`. Missing browser globals retain the current embed/test-harness no-op behavior, but rejection of an actual `serviceWorkerActivates()` promise is returned and does not set `SW_READY`. Existing `pub async fn wait()` calls `require()`, deliberately discards its result for backward compatibility, and preserves every current IO call site.
- `ui::main` completes custom-element and host-hook registration plus the existing debug-only hot-swap injection, calls `tonk_host::ready::require().await`, and invokes `mount_root()` only on `Ok(())`. On `Err`, it reports the JavaScript value to the console and returns with `#tonk-boot` still present; the existing watchdog can then perform its bounded recovery. The gate belongs immediately before `mount_root()`, so no connected callback or app root can race the update while dev reload behavior remains unchanged.

- [ ] Extend `tonk-ui-test-server` with the optional service-worker root and a Caddy `handle /service_worker.js` before the catch-all handler. Extend `TestEnvironment` / `TestServers::start` with the unique mutable script path. Run `cargo check -p tonk-ui --features integration-tests`; expect success and no behavior change yet.
- [ ] Add `#[cfg(test)] mod service_worker_upgrade;` to `src/lib.rs`, with the new file internally gated to native `integration-tests` / `web-integration-tests`, matching `account_flow.rs`.
- [ ] Add `it_replaces_a_recent_worker_on_load_without_manual_cleanup`. Let `env.driver()` establish worker A; poll `fetch("/api/health")` until it returns JSON and record `startedAt`. Before the upgrade, create an IndexedDB record and a dedicated CacheStorage entry as state sentinels.
- [ ] In that test, use `Page.addScriptToEvaluateOnNewDocument` to increment a sessionStorage document counter and attach a mutation observer that records whether `#tonk-root`, `tonk-site`, `tonk-account`, or `tonk-activate` appeared in each numbered document. Replace exactly one `// worker-wasm-hash: <16 lowercase hex>` marker in `env.service_worker_script` with a different valid marker and inject a test-only 1.5-second wait at the start of its existing install `waitUntil` body, asserting both substitutions occurred exactly once before calling `driver.refresh()` once. Do not call `registration.update()`, `unregister()`, `Storage.clearDataForOrigin`, or cache deletion from the test.
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
- The README records four distinct cases: first install waits for claim without reloading; online warm load explicitly checks and replaces behind the static boot overlay; actual warm replacement reloads once before the application root mounts; offline warm load keeps the current worker and local state. It also records the one-time bootstrap boundary for documents cached before this implementation lands.

- [ ] Add `it_keeps_the_active_worker_when_the_load_time_update_check_is_offline`. Establish worker A and a cached `/` shell, record `/api/health.startedAt`, create the IndexedDB and CacheStorage sentinels, switch Chrome offline through CDP, and refresh.
- [ ] Assert the cached page mounts a Tonk root, `/api/health` still answers through a controlled page with worker A's `startedAt`, no `installing` or `waiting` worker remains, the upgrade-reload guard is absent, and both state sentinels survive. Restore networking before returning from the test.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_keeps_the_active_worker_when_the_load_time_update_check_is_offline -- --test-threads=1`; expect failure after Task 1 because the rejected explicit update prevents `serviceWorkerActivates` from resolving and the UI does not mount.
- [ ] Catch the update rejection only in the prior-controller case described above; do not convert first-install failure into success and do not invoke the boot-recovery deletion ladder.
- [ ] Run `cargo test -p tonk-ui --features integration-tests it_keeps_the_active_worker_when_the_load_time_update_check_is_offline -- --test-threads=1`; expect the old controller, cached shell, mounted UI, and both state sentinels to remain available.
- [ ] Update the README composition section with the four-case lifecycle, the pre-mount boot overlay, the one-time pre-fix bootstrap boundary, and the fact that `updateViaCache: "none"` is paired with an explicit `registration.update()` on warm load.
- [ ] Run both upgrade tests together with `cargo test -p tonk-ui --features integration-tests service_worker_upgrade::tests -- --test-threads=1`; expect success.

### Task 3: Stamp the worker script from the final worker Wasm bytes

**Files:**

- Create: `rust/tonk-ui/scripts/stamp-service-worker.sh`
- Modify: `rust/tonk-ui/scripts/hash-guest.sh:Service-worker cache-bust`
- Modify: `flake.nix:packages.tonk-ui`

**Interfaces:**

- `stamp-service-worker.sh <dist-dir>` is POSIX `sh`, uses `sha256sum` with the existing macOS `shasum -a 256` fallback, and requires `<dist-dir>/service_worker.js` plus `<dist-dir>/worker_bg.wasm`. It removes every prior marker, appends exactly one `// worker-wasm-hash: <first 16 lowercase SHA-256 hex>` marker, reads the marker back, and exits nonzero if the files are absent, the hash is malformed, or marker verification fails.
- `hash-guest.sh` resolves its own script directory and calls `stamp-service-worker.sh "$TRUNK_STAGING_DIR"` after writing the guest manifest. Local Trunk builds therefore retain the current cache-busting behavior without duplicating hash/stamp code.
- `packages.tonk-ui` calls the same script in `postFixup` with `$out`. This deliberately restamps after crane's reference-removal hooks have transformed `worker_bg.wasm`, making the shipped marker an exact invariant over shipped bytes. This is artifact hardening, not a substitute for the explicit browser update check.

- [ ] Before changing the stamp path, run the following probe; expect the final command to succeed because the current marker differs from the post-fixup Wasm hash:

  ```sh
  tonk_ui_out=$(nix --accept-flake-config build --no-link --print-out-paths .#tonk-ui)
  TONK_UI_OUT="$tonk_ui_out" nix develop -c bash -eu -o pipefail -c '
    expected=$(sha256sum "$TONK_UI_OUT/worker_bg.wasm" | cut -c1-16)
    actual=$(sed -n "s#^// worker-wasm-hash: ##p" "$TONK_UI_OUT/service_worker.js")
    test "$actual" != "$expected"
  '
  ```
- [ ] Extract the cache-bust block into `stamp-service-worker.sh`, making missing inputs and malformed hashes fatal. Replace the old inline block in `hash-guest.sh` with one call to the new script.
- [ ] Add the final `$out` call to `packages.tonk-ui.postFixup`. Do not change the worker filename, Trunk asset declarations, or Cargo/Nix lock files.
- [ ] Run `nix develop -c nixfmt --check flake.nix`; expect success.
- [ ] Run the final-artifact probe below; expect the build to succeed, the marker to equal the final Wasm hash, and exactly one marker line:

  ```sh
  tonk_ui_out=$(nix --accept-flake-config build --no-link --print-out-paths .#tonk-ui)
  TONK_UI_OUT="$tonk_ui_out" nix develop -c bash -eu -o pipefail -c '
    expected=$(sha256sum "$TONK_UI_OUT/worker_bg.wasm" | cut -c1-16)
    actual=$(sed -n "s#^// worker-wasm-hash: ##p" "$TONK_UI_OUT/service_worker.js")
    test "$actual" = "$expected"
    test "$(grep -c "^// worker-wasm-hash:" "$TONK_UI_OUT/service_worker.js")" -eq 1
  '
  ```

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

## Follow-up verification — 2026-08-31

The PR E2E run exposed two independent boundaries. The delayed-worker fixture
copied its script from the read-only Nix store and preserved that mode, so the
test's deliberate stamp rewrite failed before browser behavior. The harness now
makes only its unique copied `service_worker.js` owner-writable. A fresh-browser
CLI approval also timed out once in CI, but the #800 diff does not touch the
account or callback protocol and the established-account callback passed in the
same job. The exact fresh-browser filter subsequently passed without a callback
product change; the test now preserves the narrow `LinkCli` console diagnostic
(with token-like values redacted) so a future failure identifies the literal
async boundary rather than only the safe visible message.

Compatibility with later generation-aware workers is page-directed. A newly
activated worker does not claim every already-open page. The explicit
`{type:"claim"}` message remains the only adoption request: first-install pages
use it, and an update-aware #800 page sends it exactly once to the activated
successor it observed before its existing one-shot alignment reload. A cached
older page sends no request and retains its compatible controller until
navigation.

Fresh evidence after the final source changes:

- TDD RED: `node --test rust/tonk-ui/tests/service-worker-claim.test.mjs`
  executed three tests; one passed and two failed because activation claimed
  unconditionally and the update-aware page sent no claim.
- GREEN: the same Node command passed 3/3 against the shipped worker and inline
  boot sources.
- GREEN: the exact delayed-worker E2E filter passed 1/1 after an unchanged retry
  with loopback permission; the sandboxed attempt failed before behavior at
  local port binding.
- GREEN: the exact fresh-browser CLI registration filter passed 1/1 after the
  same unchanged loopback retry and completed signup, activation, approval,
  callback delivery, and CLI hydration.
- GREEN: `cargo fmt --all -- --check`, `nixfmt --check flake.nix`, Node syntax,
  `git diff --check`, Storybook build `--check` (26 screens, 78 journeys, 115
  verification items, 6 triage findings), and 173/173 local Storybook links.
- Not run: the full serialized browser suite and the two-build old-page/new-page
  ordering matrix; CI remains the broader integration boundary.

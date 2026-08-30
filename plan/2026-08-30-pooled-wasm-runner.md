# Pooled Wasm browser runner implementation plan

**Goal:** Replace repeated Chrome and ChromeDriver startup in Tonk's Wasm
nextest jobs with the pinned `wbg-pool` runner, while proving result parity and
measuring the improvement before making it the default.

**Approach:** Build `wbg-pool` reproducibly from the same dialog-db revision
already pinned by Tonk, expose it only in the development and CI shells, and
benchmark it against the stock runner over the exact same nextest archives.
Keep the stock runner available as an explicit command and exercise it in a
scheduled parity workflow.

## Implementation status (2026-08-30)

Tasks 1 and 2 are implemented through the opt-in pilot. The canonical Wasm
runner and Wasm-wide retry remain unchanged because the complete comparison
did not pass its prerequisite gate: the restarted comparison's first debug
stock run stopped making progress in
`tonk-worker-api profiles::tests::it_serializes_roster_entries_in_camel_case`
with `--retries 0`, before any pooled benchmark run began. The incomplete
comparison was stopped rather than misreported as performance evidence.

Focused compatibility evidence is green: 44 `tonk-ui` tests passed through
the pooled command, and a broader 172-test `tonk-display` pass completed with
zero retries and one daemon/browser lifetime. That pass exposed two upstream
integration defects now covered by local patches: tests can replace
`window.fetch` before the harness reports its result, and concurrent starts can
replace a live daemon after a short health-probe miss. The patched runner
preserves its report channel and waits for a recorded live daemon instead.

A later complete pooled debug attempt stopped at 144 of 1,438 tests after
three 60-second timeouts. The same four adjacent `tonk-analyzer` tests passed
unchanged in 5.3 seconds pooled and 3.9 seconds stock. A 164-test pooled
`tonk-analyzer` reproduction later lost daemon health after 123 passes while
the host load average was above 44 and unrelated concurrent Rust builds were
active. Live inspection found one daemon, the expected four active test tabs,
and no retained tabs after the run. This remains a failed pilot result; a
quiet-host reproduction is required before classifying it as runner
degradation rather than host contention.

Tasks 3 and 4 are intentionally not started. They remain conditional on a
complete passing parity and performance result for both profiles.

Fresh final checks passed for the benchmark's seven CLI regressions, including
complete terminal-event coverage and descendant process-group cleanup,
`cargo fmt --all -- --check`, focused Nix formatting, `git diff --check`, the
complete `nix flake check` build on `aarch64-darwin`, the single-binary package
contract, and `wbg-pool 0.1.0` in the CI shell. The full flake check built and
passed nixfmt, shared-workspace-dependency, clippy, menu-command argument, and
Rust formatting checks after disk headroom recovered.

**Constraints:**

- Tonk currently pins dialog-db tag `tonk-2026-08-28`, commit
  `2751e105d27bc3b82248da2ae8f4b7dec3d2a571`. The runner source must stay on
  that revision until both pins are deliberately advanced together.
- Tonk pins `wasm-bindgen = 0.2.126` and resolves `wasm-bindgen-test = 0.3.76`.
  The selected runner must pin `wasm-bindgen-cli-support = 0.2.126`; a version
  mismatch is a hard failure, not a warning.
- This track changes only `wasm32-unknown-unknown` tests. The native
  `thirtyfour` account E2E harness continues to use ChromeDriver and fresh
  WebDriver sessions.
- Each nextest test must receive a unique origin and fresh browser storage.
  Reusing a page, origin, IndexedDB, OPFS, cache, or service-worker
  registration across tests is unacceptable.
- `wbg-pool` is Unix-only and does not support benches or coverage dumps.
  Those commands must remain on `wasm-bindgen-test-runner`.
- Add the runner to dev shells, not `commonBuildInputs`; putting it in every
  crate derivation would invalidate unrelated Nix build hashes.
- Do not shorten the existing CI timeout until observed GitHub Actions data
  shows a safe new bound.
- The checked-out `staging` branch was two commits behind its known
  `origin/staging` while this plan was written. Refresh the branch and
  revalidate the named files before implementation.

## File map

- `flake.nix`: pin the dialog-db runner source, build the runner package, add
  it to dev/CI shells, and expose pooled and stock test commands.
- `flake.lock`: record the non-flake dialog-db source revision.
- `nix/wbg-pool.nix`: reproducibly build only the upstream `wbg-pool` package
  with Tonk's Rust toolchain.
- `nix/menu.nix`: allow a web-test command to select an explicit Cargo target
  runner without duplicating archive execution logic.
- `.config/nextest.toml`: remove the Wasm-wide retry that exists specifically
  to mask per-test Chrome startup deaths, but only after the pool passes the
  single-attempt comparison.
- `.cargo/config.toml`: select `wbg-pool` as the default Wasm target runner
  after the pilot passes.
- `scripts/benchmark-wasm-runner.py`: run stock and pooled runners against the
  same prebuilt archives and emit machine-readable timing/result evidence.
- `docs/wasm-testing.md`: document runner selection, platform requirements,
  parity commands, limitations, and troubleshooting.
- `.github/workflows/wasm-stock-parity.yml`: run the stock runner on a schedule
  and on demand after pooled execution becomes the PR default.

### Task 1: Package the pinned runner without changing test behavior

**Files:**

- Create: `nix/wbg-pool.nix`
- Modify: `flake.nix:inputs, outputs arguments, devShellBuildInputs, packages`
- Modify: `flake.lock`

**Interfaces:**

- Consumes: dialog-db source at tag `tonk-2026-08-28`, Tonk's
  `rustToolchain`, `crane`, and `nix-filter`.
- Produces: flake package `.#wbg-pool` containing `$out/bin/wbg-pool`; the
  binary is also on `PATH` in `.#default` and `.#ci` shells.

- [x] Add a non-flake input named `dialog-db-src` with URL
      `github:dialog-db/dialog-db/tonk-2026-08-28` and pass it through the
      `outputs` argument set. Confirm `flake.lock` resolves it to
      `2751e105d27bc3b82248da2ae8f4b7dec3d2a571`.
- [x] In `nix/wbg-pool.nix`, construct a Crane library with Tonk's
      `rustToolchain`, filter the upstream source to `.cargo`, `Cargo.toml`,
      `Cargo.lock`, `rust-toolchain.toml`, and `rust/`, and build with
      `cargoExtraArgs = "--locked -p wbg-pool"`. Preserve the JavaScript and
      HTML files under `rust/wbg-pool/src` because the Rust sources load them
      through `include_str!`.
- [x] Build dependencies and the final binary as separate Crane derivations so
      Cachix can substitute the dependency layer. Set `doCheck = false` on the
      package: browser compatibility is proved by Tonk's test archives in Task
      2, not by pretending the upstream binary has a hermetic Nix unit test.
- [x] Add the resulting package to `devShellBuildInputs` and expose it as
      `packages.wbg-pool`. Do not add it to `commonBuildInputs`.
- [x] Set `WBG_POOL_FALLBACK_RUNNER` to
      `${wasm-bindgen-cli}/bin/wasm-bindgen-test-runner`. On Linux retain the
      existing Nix `CHROME` path and set `WBG_POOL_NO_SANDBOX=1`, matching the
      trusted-test CI environment. On Darwin set `WBG_POOL_BROWSER` to
      `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`; document
      that developers with another installation must override it.
- [x] Run `nix build --accept-flake-config .#wbg-pool`; expect a single native
      `wbg-pool` binary and no Wasm workspace build.
- [x] Run
      `nix develop --accept-flake-config .#ci --command wbg-pool --version`;
      expect `wbg-pool 0.1.0`.
- [x] Run
      `nix develop --accept-flake-config .#ci --command wbg-pool daemon --stop`;
      expect success when no daemon exists as well as when one is running.
- [x] Run `nix flake check --accept-flake-config`; expect the existing checks
      and Nix formatting check to pass.

      An initial run failed with `No space left on device`. After disk headroom
      recovered, a fresh full run passed nixfmt, shared workspace dependencies,
      clippy, the new menu-command argument check, and rustfmt on
      `aarch64-darwin`.

### Task 2: Add a reproducible stock-versus-pool benchmark

**Files:**

- Create: `scripts/benchmark-wasm-runner.py`
- Modify: `nix/menu.nix:menuTestCommand and makeMenuTestCommand`
- Modify: `flake.nix:commands`

**Interfaces:**

- Consumes: `tests-web-debug` and `tests-web-release` archives plus runner
  names `stock` and `pool`.
- Produces: `target/wasm-runner-benchmark.json` with archive identity, test
  inventory, individual durations, median duration, exit status, retries,
  passed/skipped/failed counts, and captured nextest summary for each runner.

- [x] Extend `menuTestCommand` with an optional `runner` argument. When set,
      place it in `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER`; otherwise
      preserve the existing Cargo configuration. Do not duplicate the Nix
      archive build or `cargo nextest run --archive-file` command.
- [x] Add temporary opt-in commands `test:web:debug:pooled` and
      `test:web:release:pooled` whose runner is the exact Nix-store
      `wbg-pool` path. Leave `test:web:debug` and `test:web:release` on the
      stock runner during this task.
- [x] Implement `scripts/benchmark-wasm-runner.py` with arguments
      `--profiles debug release`, `--runs N`, and `--output PATH`. It must build
      each archive once, list its test inventory once, run both runners over
      that identical archive, measure with `time.monotonic()`, retain complete
      logs beside the JSON file, and stop `wbg-pool` before and after the
      experiment.
- [x] Run each comparison with `--test-threads 4 --retries 0`. Treat any
      nonzero exit, timeout, changed pass/skip/fail count, or changed test
      inventory as a parity failure even if the pooled median is faster. The
      current profile-level retry must not turn a first-attempt runner failure
      into a passing benchmark.
- [x] Run a small compatibility pass first:

      ```sh
      nix develop --accept-flake-config .#ci --command \
        test:web:debug:pooled -E 'package(tonk-ui)'
      ```

      Expect all selected browser tests to pass using fresh `t-*.localhost`
      origins.
- [ ] Run the complete comparison:

      ```sh
      nix develop --accept-flake-config .#ci --command \
        python3 scripts/benchmark-wasm-runner.py \
          --profiles debug release \
          --runs 3 \
          --output target/wasm-runner-benchmark.json
      ```

      Expect identical test outcomes, no retries, and pooled median execution
      time no greater than 50% of stock for both profiles.

      Blocked in the first debug stock run as recorded in the implementation
      status above. No complete benchmark JSON or performance claim exists.
- [x] Inspect the logs for daemon crashes, browser relaunches, timeouts,
      nested-worker console loss, or tests depending on cross-test storage.
      Any occurrence blocks Task 3 until classified and covered.
- [x] After the script exits, run `wbg-pool daemon --stop` and verify no
      `wbg-pool daemon` or runner-owned Chrome process remains. Do not use a
      broad `pkill` as the verification or cleanup mechanism.

### Task 3: Make pooled execution the default while retaining stock commands

**Files:**

- Modify: `.cargo/config.toml:target.wasm32-unknown-unknown.runner`
- Modify: `.config/nextest.toml:Wasm retry override`
- Modify: `flake.nix:commands`
- Modify: `nix/menu.nix:runner override`
- Create: `docs/wasm-testing.md`

**Interfaces:**

- Consumes: the passing parity and performance evidence from Task 2.
- Produces: pooled `test:web:debug` and `test:web:release`, plus explicit
  `test:web:debug:stock` and `test:web:release:stock` escape hatches.

- [ ] Change `.cargo/config.toml` from `wasm-bindgen-test-runner` to
      `wbg-pool`. Leave `WASM_BINDGEN_TEST_TIMEOUT=60` and the existing
      `rustflags` unchanged.
- [ ] Delete the `cfg(target_arch = "wasm32")` `retries = 1` override and its
      Chrome-startup comment from `.config/nextest.toml`. Pooled PR tests must
      be single-attempt evidence; if a product test now needs a retry, diagnose
      and scope that test explicitly instead of restoring a platform-wide
      retry.
- [ ] Rename the opt-in pooled commands back to the canonical
      `test:web:debug` and `test:web:release` behavior. Add stock commands that
      override the target runner with the exact
      `${wasm-bindgen-cli}/bin/wasm-bindgen-test-runner` path.
- [ ] Ensure the stock commands also clear pool-only variables that could
      affect diagnosis. They may retain `CHROME` and `CHROMEDRIVER` because the
      stock runner requires them.
- [ ] Document in `docs/wasm-testing.md`:
      pooled versus stock commands; fresh-origin isolation; Linux and Darwin
      browser discovery; the exact version-coupling rule; `WBG_POOL_DIR` and
      daemon shutdown; lack of Windows, bench, and coverage support; and the
      rule that a suspected runner discrepancy must be reproduced unchanged
      under stock before changing product code.
- [ ] Run `nix develop path:. -c test:web:debug`; expect the complete debug
      archive to pass through `wbg-pool`.
- [ ] Run `nix develop path:. -c test:web:release`; expect the complete release
      archive to pass through `wbg-pool`.
- [ ] Run
      `nix develop path:. -c test:web:debug:stock -E 'package(tonk-ui)'`;
      expect the same focused tests to pass through ChromeDriver.
- [ ] Run `cargo fmt --all -- --check`, `nix fmt -- --check`, and
      `git diff --check`; expect no changes or formatting errors.

### Task 4: Add ongoing parity and evaluate the CI result

**Files:**

- Create: `.github/workflows/wasm-stock-parity.yml`
- Modify only after evidence: `.github/workflows/test.yml:tests timeout or job
  structure`
- Modify: `docs/wasm-testing.md:recorded adoption evidence`

**Interfaces:**

- Consumes: canonical pooled commands and explicit stock commands from Task 3.
- Produces: pooled PR gates, a weekly/manual stock parity result, and recorded
  go/rollback evidence.

- [ ] Create a workflow triggered by `workflow_dispatch` and a weekly Sunday
      schedule. Its debug/release matrix must run
      `test:web:${profile}:stock --test-threads 4 --retries 1` from `.#ci`, use
      the same disk preparation and Cachix configuration as
      `.github/workflows/test.yml`, and leave the existing PR workflow
      untouched except for its new default runner. A retried stock pass must be
      reported as flaky in the summary rather than silently treated as parity.
- [ ] Upload the nextest logs and a short step summary containing profile,
      commit, runner, elapsed seconds, and pass/skip/fail counts even when the
      test command fails.
- [ ] Open the implementation PR and record three complete GitHub Actions
      executions. For each, distinguish Nix archive build time from nextest
      execution time; do not attribute cache misses to the runner.
- [ ] Keep pooled execution only if all three runs have the same outcomes as
      the stock baseline, no new retries or orphan processes, and at least a
      50% median reduction in each web test step. Otherwise revert only the
      `.cargo/config.toml` default and canonical command selection while
      retaining the reproducible package and benchmark for diagnosis.
- [ ] After three successful runs, set any tighter web timeout to at least two
      times the slowest observed pooled wall time plus ten minutes of Nix cache
      headroom. Do not reduce the native or E2E timeout based on Wasm evidence.
- [ ] Run the manual stock-parity workflow once before merge and link that run,
      the pooled runs, and `target/wasm-runner-benchmark.json`'s summarized
      numbers in the PR description. Do not commit the `target/` evidence.

## Completion criteria

- The same debug and release archives pass under both runners.
- Pooled execution is at least twice as fast in the controlled local/CI
  comparison and across three CI observations.
- Every pooled test has a fresh origin and storage environment.
- Native Selenium tests and their ChromeDriver lifecycle are unchanged.
- Developers can reproduce any pooled failure immediately with an explicit
  stock command.
- Scheduled stock parity is green before the adoption PR merges.

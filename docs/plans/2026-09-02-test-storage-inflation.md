# Test storage inflation implementation plan

**Goal:** Prevent Tonk's Nix, Wasm, and real-browser test workflows from copying local build trees into the Nix store or retaining browser and Cargo artifacts without a bounded owner.
**Approach:** Make Git-backed flake inputs the only repository-authored default, give each real-browser test environment one explicitly owned temporary workspace, and verify both boundaries with executable storage regressions. Reduce direct Cargo test amplification separately so the local `target` tree remains useful without growing by tens of gigabytes during repeated agent runs.
**Constraints:**
- Never clear a developer's normal Chrome profile, Tonk browser storage, IndexedDB, Cache Storage, passkeys, or local/offline application data.
- Never run broad Nix garbage collection or Cargo cleanup automatically; reporting and deletion remain separate, explicit operations.
- Preserve parallel independent worktrees and the existing serialized E2E test semantics.
- Keep Linux CI Chromium and macOS Google Chrome/ChromeDriver behavior equivalent.
- Use repository-defined Nix, Cargo, and nextest commands, and keep `Cargo.lock` unchanged.
- A failed or interrupted test may leave only a Tonk-owned, identifiable workspace; it must not create an unowned `org.chromium.Chromium.scoped_dir.*` profile.

## File map

- `scripts/check-nix-source-refs.sh`: Reject repository-authored commands that use `path:.` and would snapshot ignored build products.
- `scripts/tests/check-nix-source-refs.sh`: Exercise accepted Git-backed commands and rejected path-flake commands in isolated fixtures.
- `flake.nix`: Run the source-reference check and expose the storage regression command.
- `docs/wasm-testing.md`: Document the safe Git-backed workflow, staged-new-file boundary, storage regression, and guarded recovery commands.
- `docs/plans/2026-08-19-account-and-space-deletion-plan.md`, `plan/account-deletion-profile-transition.md`, `plan/edges.md`, `plan/guest-session-renewal.md`, `plan/hub-account-settings.md`, `plan/hub-color.md`, `plan/tonk-ui-mobile-hardening.md`, `plan/tonk-ui-mobile-runtime-failures.md`, `plan/ui-bugs-join-and-space-switcher.md`: Replace executable `path:.` examples so agents do not reintroduce whole-worktree snapshots from durable plans.
- `rust/tonk-ui/src/helpers.rs`: Own the Caddy and Chrome profile trees, pass an explicit profile to ChromeDriver, and surface cleanup failure on orderly shutdown.
- `scripts/test-e2e-storage.sh`: Run one real-browser test under an isolated `TMPDIR` and reject retained browser/Caddy artifacts.
- `Cargo.toml`: Bound debug information and incremental output for the built-in `test` profile.
- `scripts/measure-cargo-test-storage.sh`: Reproduce and compare direct native/Wasm test artifact growth in isolated target directories.

### Task 1: Keep local build trees out of Nix flake inputs

**Files:**
- Create: `scripts/check-nix-source-refs.sh`
- Create: `scripts/tests/check-nix-source-refs.sh`
- Modify: `flake.nix:checks`
- Modify: `docs/wasm-testing.md:Commands`
- Modify: the nine historical/current plan files named in the file map

**Interfaces:**
- Consumes: repository-authored shell command text under `README.md`, `docs/`, `plan/`, `.github/`, `nix/`, and `scripts/`.
- Produces: `check-nix-source-refs.sh [ROOT]`, exiting nonzero with file and line for any command-form that gives Nix a local path-flake reference; `checks.<system>.nix-source-refs` runs the same policy in Nix.

- [ ] Add fixture tests proving Git-backed local commands and prose that merely explains the unsafe form pass, while executable develop, check, and build commands using a local path-flake fail with the offending file and line.
- [ ] Run `bash scripts/tests/check-nix-source-refs.sh`; expect failure because the checker does not exist.
- [ ] Implement the checker with `rg`, a fixed set of repository roots, and exclusions for its own negative fixtures; do not scan `.git`, `target`, `result`, or external worktrees.
- [ ] Replace all 61 current executable occurrences with Git-backed `.` references. In `docs/wasm-testing.md`, state that tracked modifications are included, newly created files must be staged before Nix evaluation, and ad-hoc `path:.` is unsafe when any ignored build tree is present.
- [ ] Add the checker to `flake.nix` as `nix-source-refs` without changing package inputs or lock files.
- [ ] Run `bash scripts/tests/check-nix-source-refs.sh`, `bash scripts/check-nix-source-refs.sh .`, and `nix build .#checks.aarch64-darwin.nix-source-refs --no-link`; expect success and no new store path containing a repository `target/` directory.

### Task 2: Give every E2E browser profile an explicit owner

**Files:**
- Modify: `rust/tonk-ui/src/helpers.rs:TestEnvironment, TestServers, TestEnvironment::driver, TestServers::start, TestServers::stop`
- Test: `rust/tonk-ui/src/helpers.rs:native::tests`

**Interfaces:**
- Consumes: the existing `TestEnvironment::driver() -> Result<WebDriver>` and provider-owned `TestServers` lifecycle.
- Produces: a serialized `browser_profile_root: PathBuf` in `TestEnvironment`; an internal `TestWorkspace` backed by `tempfile::TempDir`; Chrome capabilities containing `--user-data-dir=<browser_profile_root>/<unique child>`; orderly `TestServers::stop` that terminates children before closing the workspace and reports a cleanup error.

- [ ] Add `it_pins_chrome_profiles_below_the_test_workspace`, building capabilities for two drivers and asserting distinct `--user-data-dir` arguments under the same owned root, never the process-global temporary directory.
- [ ] Add `it_removes_the_test_workspace_after_orderly_shutdown`, creating sentinel Caddy, service-worker, and browser-profile files and asserting `TestWorkspace::close()` removes the complete tree.
- [ ] Run `cargo test -p tonk-ui --features integration-tests helpers::native::tests::`; expect compile failure because the workspace and capability seam do not exist.
- [ ] Implement `TestWorkspace` with one `TempDir`; place the existing Caddy data and service-worker copy below it. Allocate each Chrome profile with `tempfile::Builder::tempdir_in(...).keep()` so ChromeDriver receives a stable unique path while `TestWorkspace` retains responsibility for the parent tree.
- [ ] Keep `driver.quit().await` at existing call sites. In `TestServers::stop`, terminate the web server and ChromeDriver, stop the access service, then explicitly close the workspace; in `Drop`, preserve best-effort child termination followed by `TempDir` cleanup for early returns and panics.
- [ ] Run the two focused helper tests, `cargo test -p tonk-ui --features integration-tests identity::tests::it_serves_deployment_config_on_the_page_origin -- --test-threads=1`, and `cargo fmt --all -- --check`; expect success.

### Task 3: Prove a successful browser test retains no test storage

**Files:**
- Create: `scripts/test-e2e-storage.sh`
- Modify: `flake.nix:commands`
- Modify: `docs/wasm-testing.md:Diagnosing discrepancies and storage`

**Interfaces:**
- Consumes: `CHROME`, `CHROMEDRIVER`, the existing focused identity integration test, and a caller-supplied or script-created isolated temporary root.
- Produces: `test:storage`, which returns success only when the test passes and its isolated `TMPDIR` contains no Chrome scoped profile, Tonk E2E workspace, or live child process after shutdown.

- [ ] Add the script so it creates an isolated temporary root, records its initial byte count, runs `cargo test --locked -p tonk-ui --features integration-tests identity::tests::it_serves_deployment_config_on_the_page_origin -- --test-threads=1`, and checks for `org.chromium.Chromium.scoped_dir.*`, Tonk workspace markers, ChromeDriver, Chrome, Caddy, and test-server children associated with that root.
- [ ] Run `nix develop . -c bash scripts/test-e2e-storage.sh`; before Task 2, expect the focused test to pass but the script to fail on one retained Chrome scoped profile.
- [ ] Expose the script as `test:storage` in the development-shell menu without running it as part of package evaluation or ordinary unit checks.
- [ ] After Task 2, run `nix develop . -c test:storage` twice; expect both runs to pass, zero retained directories after each run, and no increase in process-global Chrome scoped-profile count.
- [ ] Document read-only inspection commands separately from explicit cleanup. The cleanup section must require an exact age/owner check and must not recommend clearing normal browser or Tonk storage.

### Task 4: Bound direct Cargo test artifact growth

**Files:**
- Create: `scripts/measure-cargo-test-storage.sh`
- Modify: `Cargo.toml:profile.test`
- Modify: `docs/wasm-testing.md:Local Cargo artifacts`

**Interfaces:**
- Consumes: an isolated `CARGO_TARGET_DIR` plus representative native integration and Wasm no-run builds.
- Produces: a machine-readable summary with total bytes and bytes under `debug/incremental`, `debug/deps`, `wasm32-unknown-unknown`, and rust-analyzer output; the repository test profile disables incremental compilation and embedded test debuginfo.

- [ ] Implement the measurement script to require an empty explicit target directory, run `cargo test --locked -p tonk-ui --features integration-tests --no-run` and `cargo test --locked --target wasm32-unknown-unknown -p tonk-ui --no-run`, then emit stable `key=value` byte counts using `du -sk`. It must never clean the repository's normal `target` directory.
- [ ] Run the script against the current profile and preserve its output as the before measurement in the implementation handoff.
- [ ] Add `[profile.test]` with `debug = 0`, `strip = "debuginfo"`, and `incremental = false`; retain the existing optimization settings inherited from Cargo's test profile.
- [ ] Run the script in a second empty target directory; require total bytes to be at least 30% below the before measurement and `debug/incremental` to remain empty. If the same command is repeated, require growth below 256 MiB.
- [ ] Run `cargo test -p tonk-ui --lib`, the two helper tests from Task 2, `nix develop . -c test:storage`, `cargo fmt --all -- --check`, `bash scripts/check-nix-source-refs.sh .`, and `git diff --check`; expect success.
- [ ] Document the debugging tradeoff and the opt-in override `CARGO_PROFILE_TEST_DEBUG=1` for a single diagnostic run. Document size-report commands and exact-target `cargo clean --target-dir <validated path>` separately; never automate broad cleanup across worktrees.

## Final verification

- [ ] Run `rg -n 'nix (develop|build|run|flake check)[^`]*path:\\.' README.md docs plan .github nix scripts`; expect no executable command matches outside the checker's negative fixture.
- [ ] Run `nix flake check --no-build .`; expect evaluation success with the new source-reference check present.
- [ ] Run `nix develop . -c test:web:debug -E 'package(tonk-ui)'`; expect the pooled Wasm tests to pass without a Nix source path containing `target/`.
- [ ] Run `nix develop . -c test:storage` twice and compare `df`, Nix-store registrations, and the isolated temporary root before and after; expect no retained browser artifacts and no repeated multi-gigabyte source snapshot.
- [ ] Inspect the complete diff and `git status --short`; confirm only the files in this plan changed and no lock file, browser storage, unrelated worktree, or local application data was modified.

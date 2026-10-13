# Plan 012: Shorten PR verification and retain optimized release gates

Status: IMPLEMENTED; hosted validation pending. Priority P1; effort L; risk medium. Planned at `6828aac5f4`, 2026-10-13. Dependencies: none. User authorized all five audit recommendations.

Publication update: the user subsequently authorized committing, pushing and opening a PR to test the improvements. The original implementation-only no-push restriction below is superseded for that PR workflow; production deployment and merging remain outside this task.

## Intent and scope

Fix ineffective dependency reuse, avoid duplicate browser builds, overlap independent checks, and select fast PR profiles while retaining full optimized verification on main and release tags. A sampled successful PR spent 34 minutes preparing native debug tests and only 75 seconds executing them. E2E additionally compiled the same preview on three runners.

In scope: Cargo.toml, nix/rust.nix, flake.nix, .github/workflows/{test,cli,publish,release,cli-npm}.yml, narrowly necessary CI actions/scripts/tests and browser artifact tests, this plan's execution log. Do not change runtime behavior, dependencies/lockfiles, signing/publishing semantics, release optimization, test selection/retries, or remote repository rules. No push/deploy.

## Current state and constraints

- `nix/rust.nix` buildTestArchive calls raw `cargo nextest archive ${args}` while dependency builders default through pinned Crane to release. Debug/E2E need explicitly matched profile and test feature/target/dependency graph. commonAttributes disables doCheck; test-only caches need compile-only test dependencies.
- `flake.nix` tests-native-debug, tests-web-debug, tests-e2e use no explicit Cargo profile; release native uses release-test. Package/feature exclusions must remain identical. Native release-test disables LTO but still has one codegen unit. Keep optimized post-merge coverage.
- E2E build job prepares CLI/archive only; each shard builds preview in test:e2e. Build browser assets on a separate parallel producer, hand off exact artifacts/closures reliably including forks without Cachix write access. Avoid rebuilding through the test-server default tonk-ui store reference when a runtime artifact was provided.
- PR CLI workflow builds shipped Linux static-musl and macOS CLI. Keep these platform/linkage smoke guarantees with a separate fast profile; shipping CLI unchanged.
- tonk-ui-preview currently only renames tonk-ui. UI/worker HTML selects wasm-ui-release/wasm-worker-release fat LTO, guest always wasm-release, wasm-opt=z. Provide dedicated PR packages using release semantics (debug_assertions off), lower optimization, no LTO, more codegen units, skip expensive wasm-opt only on PR package. Main/staging/stable should use optimized packages. Preserve stamping, invitations and guest behavior. Keep producer/consumer dependency profiles consistent also for new fast builds.
- Tests and lifecycle tests depend on lints without artifact need. Remove those ordering constraints; E2E shards depend on artifacts, not lint.
- Ruleset 12020040 requires Lints, Cloudflare Artifacts, Tests (native, debug/release), Tests (web, debug/release). Do not alter remote rules. Preserve old release contexts as transparently deferred lightweight checks on PRs (must not claim tests ran); main/tag must actually execute release suites. Ensure aggregate failures remain visible.
- Release, CLI npm, and web publish currently independently run on v* tags. Add one authoritative full tag verification and make all release side effects wait for successful verification of exact tag revision, with version stamping consistent with release builds. Never accept another ref's successful run at same SHA, nor an older attempt over a failed latest attempt. Polling/action solution must fail closed and have bounded timeout; avoid duplicate full verification per publisher or workflow cancellation deadlocks. `resolve-release` action stamps workspace Cargo version/lock at runtime; do not alter tracked lockfile.

## Implementation checkpoints

1. Explicit matched archive profiles/test dependency caches. Prove Nix evaluation and inspect generated producer/consumer commands for native debug, Wasm debug, E2E and release. Build smallest representative archive/dependency pair; verify a source-only rebuild reuses dependencies if feasible. Commit in isolated executor worktree.
2. Parallel browser producer and reliable artifact transfer; parallel independent checks. Test server must use selected runtime artifact without realizing shipping fallback when unnecessary; preserve local default or provide clear explicit invocation behavior. Test handoff failure paths. Commit checkpoint.
3. Separate PR profiles/packages and full post-merge/tag matrix. Start opt-level=1 for runtime-sensitive browser/CLI, no LTO, codegen-units=16; test workspace opt=0/1 according to evidence. Keep dependency optimization a deliberate explicit decision, not accidentally inherited wildcard opt=3. Keep debug assertions/overflow checks for correctness tests and release semantics for browser/CLI smoke builds. Update tag gates across all publishers and add focused meaningful offline tests of gate run selection/failure/timeout and workflow policy. Commit checkpoint.

## Verification and done criteria

- Nix formatting: repository formatter (nixfmt), then `nixfmt --check flake.nix nix/rust.nix` passes.
- Cargo formatting: `cargo fmt --all -- --check` passes; no Rust source expected to change.
- Workflow syntax/expression lint with actionlint passes (account for existing warnings separately).
- Existing lifecycle/artifact tests: `node --test 'rust/tonk-ui/tests/*.test.mjs'` passes using repository Node.
- New gate tests via Python unittest or existing script test pattern pass: exact ref/SHA/event/workflow, latest attempt, unsuccessful completion, queued/running, no run, timeout, API failure, pagination if applicable.
- Evaluate x86_64-linux and aarch64-darwin packages without builds using nix eval/derivation show; inspect profile alignment and verify shipping package profiles unchanged.
- Run smallest relevant native test archive build first; progress to native/Web suites and browser E2E when environment supports them. `nix --accept-flake-config develop .#ci -c lint` is canonical integrated lint. Hosted timing/complete Linux E2E cannot be claimed from local Darwin static checks.
- Record every actual check and limitation in this file; report exact errors before corrective changes. Do not claim performance savings without paired measurements; provide benchmark procedure and CI timing diagnostics if local profiling is impractical.
- Review full diff, no unrelated paths or lockfile changes, preserve all test suites on main/tags and all debug/E2E/lifecycle/platform smoke coverage on PRs.

## Workflow and stop conditions

Executor creates an isolated worktree from planned SHA under /tmp and commits independently reviewable checkpoints there; no push. Reviewer independently checks final diff and checks. User's current worktree initially contains only untracked audit/plan notes; preserve them. Return branch/worktree and exact commit IDs for review. Stop/report if remote permissions are required beyond reads, environment prevents a verification (continue independent checks), or solving this requires runtime/dependency/signing changes. Do not modify branch protection.

## Maintenance

Keep the dependency producer and archive consumer profile/target/features in one source of truth. Adding a publication path requires the tag verification gate. New test crates need matching dependency-cache graph. Measure queue, preparation, compile, test and retries separately with cold and warm dependency caches; compare final gate latency and total runner minutes.

## Implementation and review evidence

Implemented in isolated worktree `/tmp/tonk-ci-compile`, branch `perf/ci-compile-implementation`, in checkpoints `e5a21e3f15` (archive caches), `e95765b2ed` (shared artifacts/parallel checks), and `22588a3c66` (PR profiles/tag gates). Reviewed implementation diff applied to this worktree without committing, pushing or changing repository rules. Existing plan/audit notes were preserved.

- Archive producers and consumers now share explicit profiles, package selections and features. Test caches compile dev dependencies with `--no-run`.
- E2E browser/native producers run concurrently. Shards restore outputs through workflow artifacts, including on forks without Cachix upload credentials. Artifact payload sizes and build/pack/restore/E2E timings are logged; producer retries overwrite old artifacts.
- The CI server launcher has no shipping browser dependency. The local server wrapper preserves its existing optimized default. Recursive CI-shell derivation inspection confirmed no browser build dependency.
- New `ci-test` and `ci-release` profiles use opt-level 1, dependency opt-level 1, LTO off and 16 codegen units. Correctness tests keep assertions; browser/CLI builds keep release semantics. After the first hosted run, Wasm correctness tests were restored to the established `test` profile (see follow-up below). PR UI/worker/guest skip wasm-opt; shipping profile tables are unchanged. The access-service worker retains its existing release optimization.
- Original required check names remain. PR release cells explicitly report deferral in their job summaries; main and tag runs execute all matrix cells. Tag verification uses full history and the same version stamping as publishers. Main/tag E2E and deployment select the same optimized UI package.
- GitHub release, npm and tagged web publication fail closed unless the latest Test run/attempt matches the exact tag, commit, push event and workflow and succeeds. API failures, failed/cancelled runs and a bounded three-hour wait reject publication. No live publication was attempted.

Fresh relevant verification:

- Evaluated native/Wasm archive and CLI/UI packages for Linux and Darwin. Independently inspected generated Linux build phases: archive/dependency profiles and test arguments agree; Linux PR CLI remains static musl. Shipping UI transformation is unchanged.
- Focused final `ci-test` archive built for `tonk-template`; **113 tests passed, none skipped**. Reviewer independently reran that archive successfully. Dependency check/build/test compilation took 41.58s/34.96s/23.76s; source archive compilation took 11.58s. These are local component timings, not a speedup comparison.
- A temporary source-only comment edit retained the identical dependency derivation, verifying cache-key stability. An earlier `tonk-common` probe compiled only the workspace crate after importing cached dependencies, but has no tests (nextest exit 4); it is compile evidence only.
- **8 release-gate tests passed**, covering identity, latest run/attempt, queued/missing runs, failures, pagination, timeout and API failure. These tests run in CI lint as well.
- **16 artifact/server tests passed**, including selected-artifact startup and explicit missing-input failure.
- Rust/Nix formatting, workflow syntax/expression checks, source-reference checks and diff whitespace checks passed. Full actionlint retains the identical baseline SC2129 ShellCheck style warning in the deployment database script; no new workflow warning was found.
- The broader unchanged Node lifecycle suite stalls at `service-worker-claim.test.mjs:686`, `claim takes control and activate messages no longer gate adoption`. A single-test run reproduced a 5-second test timeout and retained timers after 12 seconds; the broader run was interrupted. This is not reported as passing. No runtime/test-selection workaround was introduced.

Environment/setup failures: Nix daemon access needed scoped escalation; registry-based actionlint acquisition failed on a tarball-cache open-file limit, then pinned-input acquisition succeeded. A temporary probe omitted the repository's Darwin remarshal overlay and hit its known libffi assertion; adding that existing overlay fixed the probe. The initial full E2E test-profile build was stopped during dependency compilation when the final CI profiles superseded it. Full repository lint/build and the full native/Wasm/browser suites were not completed locally.

## Hosted comparison and remaining proof

Run the baseline and this branch on the same hosted runner class, separating first builds with cold profile caches from warm dependency builds. Repeat after a source-only Rust edit. Compare queue/setup, dependency and workspace compilation, browser asset processing, artifact packing/transfers/restoration, test execution/retries, final required-check completion and total runner minutes. New profiles invalidate old caches initially, so the first run alone is not representative.

Verify fresh Linux NAR import, fork artifact handoff, rerun recovery, all native/Wasm/E2E tests, and tag-gate integration before merging. Reduced optimization may increase runtime or payload size; measure build plus execution. No overall CI speedup or hosted passing run has been established.


## First hosted run: Wasm profile follow-up

PR #1090 Test run `38397722914` built the Wasm archive successfully, then failed
three browser tests with `RuntimeError: memory access out of bounds`, followed by
the 60-second wasm-bindgen timeout. It ran 1,530 of 1,981 tests: 1,527 passed and
three failed. The first failure was
`tonk-schema account::tests::it_converges_divergent_display_names_in_both_orders`;
the other two were worker account-state projection and adoption tracking tests.
The matrix cancelled native debug, so it provides no native test result.

The exact hosted archive was fetched from the signed project cache and its
account-convergence test reproduced the same trap alone on local Chrome. Restore
`tests-web-ci` to the established `test` profile, retaining matched dependency
caching, shared browser artifacts, parallel gates, and the native/E2E CI profiles.
This is a conservative rollback of the failing profile; the precise low-level
cause of the Wasm trap has not been established. Disable matrix fail-fast so a
failure in one platform does not cancel the other platform's evidence.

Hosted timing: browser assets finished in 8m25s, E2E archive in 25m17s, Wasm job
failed after 25m55s (about 5m of test execution). Lints, service-worker lifecycle,
Cloudflare artifacts and all CLI build/smoke jobs passed. Optimized matrix cells
were explicitly deferred, not exercised. E2E shards were still running at this
checkpoint. These first-run timings are not a controlled performance comparison.

Focused validation: the hosted `ci-test` Wasm binary reproduced the trap locally
in the isolated account-convergence test (64.4s including its timeout). Rebuilding
`tonk-schema` with the existing `test` profile made that test pass in 2.55s in the
same local browser runner. This comparison uses a focused package build rather
than the complete workspace feature graph. Linux evaluation confirms both the
updated archive and its dependency producer use `test`. Nix formatting,
workflow syntax/expression validation and Nix source-reference checks passed.
The full hosted Wasm and worker tests still need the PR rerun.

The focused `test`-profile schema Wasm archive also passed its full 95-test suite.

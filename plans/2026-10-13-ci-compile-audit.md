# CI compile-time audit

Audited 2026-10-13 against `6828aac5f4`. Exploration only; no build or workflow changes. Scope: Cargo profiles, Crane/Nix dependency reuse, PR workflow scheduling, and recent hosted job logs. Application architecture, individual compiler hot spots, and repository required-check settings were not audited.

## Measured baseline

GitHub Test run https://github.com/tonk-labs/tonk/actions/runs/38309157303 completed successfully on its first attempt in approximately 64 minutes.

- Lints: 10m46s; native and web matrix jobs wait for this job.
- Native debug: archive derivation starts 21:28:26 UTC; tests start 22:02:24. Execution is 74.891s for 2,642 tests across 108 binaries. The 34-minute interval includes artifact build/packaging, not a measured rustc-only duration.
- Native release: job duration 42m16s after lint.
- E2E producer: 30m32s. Each shard then builds the same `r1w9zpmxv1plb3fgmbyiqzp785z8nwby-tonk-ui-preview` derivation at approximately 21:45:30; tests start approximately 21:56. Shard execution takes 17m15s to 20m09s; one shard reports a flaky test. E2E is the final gate in this run.
- Run https://github.com/tonk-labs/tonk/actions/runs/38309157277 took about two hours including a second attempt. Do not treat that as two hours of uninterrupted compilation.
- Main run https://github.com/tonk-labs/tonk/actions/runs/38305838830 took approximately 15 minutes. This illustrates variability, not a controlled speedup comparison.

## Ranked findings

1. **Align cached dependencies with test consumers.** High confidence on profile mismatch, medium effort, low risk for profile alignment and medium risk for test-graph cache expansion. `nix/rust.nix:98` defines default native dependency artifacts; `:115` defines Wasm artifacts; `:206` onward builds test archives. With no explicit profile, pinned Crane defaults its own CARGO_PROFILE to release and injects --release into dependency builds. Raw `cargo nextest archive` at `:254` does not use this helper, and debug/E2E archive arguments in `flake.nix:551-583` select no profile. Thus those consumers use test-profile compilations against release dependency caches. Pin one explicit profile in both producer and consumer, with matching target/features/test graph. `commonAttributes.doCheck = false` also suppresses Crane's default test-dependency preparation; prepare test dependencies without executing tests in the Nix build. Keep shipping dependency caches separate.
2. **Build the browser test artifact once, alongside other E2E inputs.** High confidence, medium effort, low behavioral risk. `.github/workflows/test.yml:66` prepares CLI and E2E archive only; `flake.nix:423` also builds tonk-ui-preview inside each shard. Hosted logs prove three identical preview compilations. Prefer an independent parallel producer and explicit handoff/cache availability before shards, rather than blindly appending a serial build to the existing producer. Preserve fork/no-cache-write behavior. Separately inspect the test-server script's default tonk-ui store reference (`flake.nix:678`), which may realize an additional artifact through the CI shell despite the runtime override.
3. **Remove lint serialization where it has no artifact dependency.** High confidence, small effort, low correctness risk; concurrency/cache contention requires measurement. `.github/workflows/test.yml:128,148` delays service-worker and matrix tests behind lints. Launch independent checks together; retain all checks as required. This overlaps roughly 11 minutes in the baseline but does not remove E2E's critical path. Cache producers must remain explicit where needed.
4. **Separate PR correctness profiles from shipping optimization.** High confidence on configuration, unmeasured speedup; medium effort and medium coverage/runtime risk. `Cargo.toml:216-256`: dev optimizes workspace at 1 and dependencies at 3; native release-test disables LTO but retains opt-level 3 and codegen-units 1; web release uses ordinary release with thin LTO. `flake.nix:624` preview only renames the shipping UI derivation. `rust/tonk-ui/index.html:1011-1023` selects fat-LTO browser profiles plus wasm-opt=z; guest at :1089 uses wasm-release regardless of the general build mode. Introduce explicit CI profiles; experiment with opt-level 0/1, lto="off", and more codegen units while measuring build plus execution. Keep release-like browser behavior, stamping, asset layout and feature sets; merely switching Trunk to debug changes cfg(debug_assertions) paths. Preserve optimized shipping profiles.
5. **Move full release matrix to main and gate releases.** High confidence, small/medium effort, medium coverage risk. PR Test currently runs native/web times debug/release; CLI PR builds shipping binaries for Linux and macOS. Moving release tests after merge reduces runner work but would not alone shorten the measured 64-minute run because E2E finished last. Keep PR target/package smoke coverage with faster profiles where possible. Main should run the full matrix, with release publishing gated on successful verification of the actual release revision/artifacts. Current release flows trigger on v* tags: staging is prerelease tags and stable is final tags, not separate branches in these workflows. Required-check migration must be inspected before removing PR matrix jobs.

## Suggested sequence and evidence gates

First establish explicit matched test caches without changing test selection or optimization; compare a source-only rebuild after a warmed dependency cache and verify third-party crates do not recompile unnecessarily. Then produce browser artifacts once and overlap independent jobs. Finally benchmark CI profiles and decide which release gates move after merge.

For each increment use identical runner classes and source revisions, recording cold dependency builds separately from warm dependency/source-changed builds. Record queue/setup, cache transfers, dependency compilation, workspace build/link/archive, browser asset processing, test runtime/retries, total runner minutes and time to final required check. Preserve test counts and flags. Use repository commands: `nix --accept-flake-config develop .#ci -c lint`, `test:native:debug`, `test:web:debug`, and `test:e2e --partition count:N/3` through the CI shell; exercise release counterparts at the profile integration checkpoint. Changes require hosted Linux evidence; local Darwin timings are insufficient.

These are investigation findings and proposed checkpoints, not implementation plans selected by the user. No savings percentage or target completion time is established.

## Considered but not recommended as a first step

- Removing all release tests as the sole fix: does not address the observed E2E critical path or debug cache mismatch.
- Globally weakening release optimization: needlessly changes deployed performance and size.
- More E2E shards before fixing artifact preparation: multiplies duplicate compilation.
- Calling this a missing-cache problem: Cachix/Crane are present; profile/feature compatibility and artifact handoff are the concrete issues.

References: pinned Crane `2510f2cbc3ccd237f700bb213756a8f35c32d8d7`, `lib/setupHooks/configureCargoCommonVarsHook.sh:17-19`, `lib/setupHooks/cargoHelperFunctionsHook.sh:12-19`, `lib/buildDepsOnly.nix`; https://doc.rust-lang.org/cargo/reference/profiles.html .

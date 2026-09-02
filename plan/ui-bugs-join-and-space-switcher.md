# Join empty-state and FABB space-switcher fixes

**Goal:** Bring the absent-space join state into the established Tonk/FABB visual system and make the FABB space switcher list every other eligible local space.

## Evidence and hypotheses

- The absent-space panel is styled inline in `rust/tonk-core/assets/library/profile.yaml` with Web Awesome quiet/brand tokens, a pill button, and a plain system type stack. The Hub and FABB use IBM Plex Sans Condensed, rectangular 44px controls, stone/ink tokens, and hairline-separated frost surfaces. The visual mismatch is authored at this view boundary.
- `<ui-space-switcher>` filters the active space from profile-replica conclusions and renders the remaining rows. The supplied menu shows no rows despite another local space being present. The first hypothesis to test is that the switcher's fixed profile route or replica query no longer matches the current profile-directory contract; the second is that rows are delivered but inserted at the wrong DOM boundary.

## Work

- [x] Establish focused failing tests for account-directory rows and late active-space filtering, then trace both source boundaries.
- [x] Add focused structural coverage for the absent-space panel's Tonk/FABB contract.
- [x] Implement the narrow fixes at the source boundaries.
- [x] Update the matching Storybook journey/verification contract and regenerate derived data.
- [x] Run Rust formatting, focused tests, relevant broader checks, Storybook checks, and rendered browser verification.
- [x] Review the final diff, commit only these changes, publish the branch, and open a focused pull request against the live parent branch without merging it.

## Verification record

- Red proof: the account-directory query unit test failed while the source still named `xyz.tonk.replica/*`; the absent-space structural test failed before the edge markup existed; and the focused Wasm browser test `it_refilters_when_the_active_space_lands_after_the_directory_frame` failed twice with the active and other rows still present after `exclude` changed.
- `cargo fmt --check` — pass.
- `cargo test -p tonk-worker --test standard_library` — 16 passed.
- `cargo test -p tonk-fab --lib` — 109 passed.
- `cargo clippy -p tonk-fab --lib -- -D warnings` — pass.
- `nix develop . -c cargo nextest run --workspace-remap ./ --archive-file /nix/store/qx1xc5dic9ci3d6d7vlifwwajbw73ic8-tests-web-debug-0.6.9/tests-web-debug.tar.zst -E 'package(tonk-fab)'` — 75 Wasm/Chrome tests passed, including both switcher regressions.
- `python3 docs/storybook/scripts/build.py --check` — 26 screens, 78 journeys, 115 verification items, 6 triage findings.
- `python3 docs/storybook/scripts/check-links.py docs/storybook` — 173 local references valid.
- `git diff --check` — pass.
- Running product, isolated Chrome: created two local spaces; while `Untitled 2` was active, FABB listed only `Untitled`, and selecting it navigated to that sibling subject. The absent-space route rendered legibly in dark mode at desktop and `390x844x2,mobile,touch`, with Home and Join present in the accessibility tree.
- Environment note: the first `dev:web` attempt inherited `NO_COLOR=1`, which current mdBook rejects; restarting with the repository-documented `env -u NO_COLOR` workaround served the app. The first broad wrapper invocation did not forward the intended nextest filter and was interrupted after 24 unrelated passes; all reported green browser evidence above comes from the exact archive/filter command.

## CI follow-up: callback device convergence

- [x] Trace the failing callback assertion and identify the terminal-only intermediate state.
- [x] Add a focused regression test for callback device-list readiness.
- [x] Make the E2E wait for both the signing browser and linked terminal rows.
- [x] Run Rust formatting, the focused regression test, and diff checks.
- [x] Publish the callback fix and prove the previously failing test on CI.
- [x] Raise the stale E2E job timeout to cover its cold build and test runtime.
- [x] Reuse the complete callback wait before the device-revocation assertion.
- [ ] Confirm the complete E2E job with CI host routing.

The direct local E2E reached the callback but could not complete because the
native CLI resolved `tonk.network` publicly; CI supplies the required
`127.0.0.1 tonk.network` mapping before running the same test harness.
On the patched CI run, both the regression test and the previously failing
callback test passed. The otherwise healthy suite was then cancelled at its
45-minute job limit after a 20-minute cold build, so the limit now allows 60
minutes while preserving a bound for actual hangs.
The next run passed the original callback test and completed within 38 minutes,
then exposed the same terminal-only race in the revocation test: it revoked
before the CLI had cached the browser row whose stale visibility it asserts.

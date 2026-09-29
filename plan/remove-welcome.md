# First visits land in the Hub

Remove the pre-mount Welcome request, worker creation/preparation routes, and
`router/onboarding_space.rs`. Root visits mount the normal Hub without creating
spaces. Keep legacy snapshot upgrade protection and deferred image hydration
alongside their repository/blob consumers.

Update browser coverage for first and returning visits with zero spaces; create
an explicit space for navigation coverage and adjust account deletion counts.

Validation:
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Native `cargo check -p tonk-worker -p tonk-ui --tests --features tonk-ui/integration-tests` passed.
- `cargo check -p tonk-ui -p tonk-worker --target wasm32-unknown-unknown` passed.
- Both route-table tests passed using a standalone Rust harness (the existing
  test bodies with the test macro replaced by `#[test]`).
- Browser regression updated but not executed: no ChromeDriver or built web
  test deployment was available in the local harness.

The removed optional preparation endpoint is no longer supported for legacy
Welcome pages; this change does not migrate their stored content.


Follow-up cleanup: removed `scripts/onboarding/` and its three generated YAML
bundles (Welcome, demos, and agent playground). Removed native loader entries
and playground-specific assertions; retained core agent-prompt coverage.
The legacy media manifest and WebP images remain for existing space blobs.
Historical plans describe the retired implementation and are kept as history.
Cleanup validation passed: `cargo check -p tonk-worker --tests`, formatting,
whitespace checks, the focused Node bundled-asset offline test, and all three
retained agent-prompt tests executed in a standalone Rust harness. Repository
search found no remaining active references to the removed scripts or YAMLs.
Browser execution remains unverified as noted above.

CI lint follow-up: remove the unused worker `seed_standard_library`,
`set_replica_status`, and registry `initial_profile` helpers. Keep
`seed_on_branch` only in test builds and update references to removed helpers.
Validation: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
passed locally on macOS, as did formatting and whitespace checks. The earlier
Cargo checks did not deny dead-code warnings. Linux Nix CI remains to rerun.

E2E follow-up: CI at `0920b461d` reported three deterministic fixture failures:
empty `ProfileInfo.space` is omitted by Serde; new spaces render `.blank-canvas`;
and the persisted-worker-upgrade test needs an explicitly created space now.
Use typed profile decoding, the standard blank canvas selector, and the shared
space-creation helper for the upgrade fixture.

All three focused E2E tests passed without retries in 64.02 seconds using the
exact CI browser artifact (build `4eae90cdc6a6408a`) and local Chrome 154 with
its matching driver. The cached Chrome 150 renderer crashed at WebDriver setup
before app assertions; switching to the installed matched browser resolved that
local harness failure. Native test compilation, formatting, whitespace checks, and
`cargo clippy -p tonk-ui --all-targets --all-features --locked -- -D warnings` passed.
Linux CI must still rerun on the new commit.

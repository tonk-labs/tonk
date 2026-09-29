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

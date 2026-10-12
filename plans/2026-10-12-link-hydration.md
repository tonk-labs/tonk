# Device linking: preserve the replacement grant

## Evidence

A regression on main b41c33e3b2 reproduced first hydration restoring the obsolete
root-to-device grant after persist_root removed it. The already-hydrated control
kept the replacement. Running startup cleanup removed the restored grant.
The user's diagnostic restart likewise changed CREDENTIAL_REVOKED to working sync,
but the exact original outgoing proof was not captured.

## Implementation and verification

- Reconcile the canonical local root grant after initial account adoption and
  before the trusted marker, convergence, or publication. Cover genesis adoption.
- Exercise first hydration and an already-hydrated device against a real local
  access service. Revoke the old grant, replace it, hydrate, and verify remote
  writes and reads without restarting the worker.
- Run focused regression, then account-state and identity integration coverage.
- Keep the user's desktop worktree unchanged. Publish from an isolated branch.

## Boundaries

The automated service test exercises signed grants, revocation, hydration, and
remote storage. It does not automate the user's hardware passkey or a deployed
Foundation-to-native callback. The observed multi-minute delay and legacy custody
card are separate issues; this change does not claim to resolve them.

## Results

- The revoked-grant service regression passed in 3.68 seconds, covering both
  hydration cases and bidirectional remote edits.
- Storybook verification: LIFE-30.
- Native account-state suite: 24 passed (includes the new regression).
- Browser service-worker identity suite: 13 passed.
- Browser service-worker device registration/revocation suite: 6 passed.
- Formatting, diff whitespace, Storybook freshness, and 181 local links pass.
- Nix development-shell setup failed downloading DarwinTools with HTTP 429.
  Browser suites then passed using installed wbg-pool and wasm-bindgen 0.2.126.
  Native filters for the browser-only suites selected zero tests; those were
  superseded by the actual browser runs and are not counted as passes.

Commands (all Cargo runs use `--locked --offline -p tonk-worker --lib`):

- Native: `cargo test ... router::account_state::tests -- --test-threads=1`
- Browser: `cargo test ... --target wasm32-unknown-unknown router::identity::tests`
- Browser: `cargo test ... --target wasm32-unknown-unknown router::account_devices::tests`
- `cargo fmt --all --check`
- `git diff --check`
- `python3 docs/storybook/scripts/build.py --check`
- `python3 docs/storybook/scripts/check-links.py docs/storybook`

# Account deletion profile transition

## Observed failure

- Permanent account deletion releases the email remotely, then calls the
  ordinary local sign-out path.
- Sign-out preserves the profile root and writes an attachment tombstone so a
  different account cannot silently inherit that profile's retained spaces.
- The account page still offers account creation on that retired profile. A
  newly created passkey has a different root, so `POST /api/identity/root`
  correctly rejects it with `409 Conflict`.

## Plan

- [x] Extend the real-browser deletion regression to recreate an account with
  the released email and observe the current conflict.
- [x] After successful permanent deletion, preserve the retired local profile
  and rotate the active browser state to a fresh profile.
- [x] Run the focused regression, nearby tests, formatting, and diff checks.

## Verification

- The focused real-browser regression failed before the production change at
  `POST /api/identity/root` with the reported `409 Conflict`, then passed after
  the profile transition change.
- `nix develop . -c test:web:debug`: 1,357 passed, 1 skipped.
- `cargo fmt --all -- --check` and `git diff --check` passed.

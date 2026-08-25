# Browser logout/login conflicting-state fix

## Observed failure

- Browser sign-out clears only the local provider attachment and preserves the
  profile root, device DID, spaces, and server-side device registration.
- Logging back in with the same passkey submits `/devices/link` for the same
  account root and device DID with a freshly minted root-to-device delegation.
- `register_device` always inserts a fresh active row, so the
  `devices_one_active_did` index rejects the request and the account service
  returns `conflicts with existing state`.

## Plan

- [x] Add regressions proving same-account browser re-login returns the existing
  active attachment without adding device history.
- [x] Make root-authorized browser re-login reuse an active same-account device
  while retaining strict fresh-generation semantics for ordinary registration.
- [x] Reattach the browser with its preserved local grant when the service
  reuses that active generation.
- [x] Run focused account-service tests, formatting, and relevant broader
  checks.

## Verification

- Full account-service suite: 50 unit tests and 11 HTTP tests passed.
- Full tonk-ui Wasm unit suite: 31 tests passed.
- Native browser regression for logout followed by same-account login passed.
- `cargo fmt --all -- --check`, account-service Clippy, isolated tonk-ui
  Clippy, and `git diff --check` passed.
- Dependency-inclusive tonk-ui Clippy remains blocked by six existing
  `arc_with_non_send_sync` errors in `rust/dialog-reactor` on Wasm.

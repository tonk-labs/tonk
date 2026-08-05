# Account creation preflight and passkey atomicity plan

**Goal:** Verify that an email can create an account before prompting for or
creating a passkey, while preserving email-enumeration resistance and making a
retry with a different email deterministic.

**Observed flow:**

1. `POST /codes` sends a code without revealing whether the email is already
   registered.
2. The verification-panel submit handler creates and locally persists a
   passkey root when none exists.
3. The same handler asks for that passkey again to sign `POST /accounts`.
4. `POST /accounts` verifies and consumes the code, then first checks the
   unique email/root constraints while inserting the account and device.
5. An existing email therefore leaves a newly created, unregistered local
   passkey root behind. “Use a different email” clears only the visible error
   and panel mode, so the next attempt inherits that intermediary root.

**Approach:** Add a code-authenticated, non-consuming account preflight. A
correct code proves control of the supplied address before the service reveals
an email conflict; an available address retains the code for the existing
atomic account-and-device insertion. Run this preflight before any WebAuthn
ceremony. For a browser without a local root, combine passkey creation, root
derivation, delegation, and account-invocation signing into one bridge call so
the normal PRF-capable path uses one WebAuthn ceremony rather than create then
assert. Keep the existing-root path intact for accountless passkey roots.

**Residual boundary:** Availability can change between preflight and
`POST /accounts`. The final unique constraint remains authoritative. Avoiding
that race completely would require a server-side reservation lifecycle, which
would add another durable intermediary state and a denial-of-service surface.

## Tasks

- [x] Add core and HTTP regression tests for a code-authenticated preflight:
      wrong codes fail, existing emails return the safe conflict, and an
      available-email preflight does not consume the code.
- [x] Add a real-browser regression proving an existing-email attempt creates
      no credential and a subsequent different-email attempt succeeds with one
      credential.
- [x] Implement the Worker and native-helper preflight routes and UI client.
- [x] Add the combined fresh-root account ceremony and use it only after a
      successful preflight.
- [x] Reset verification-only form state when returning to email entry and
      validate browser form constraints before network work.
- [x] Consume the verified code in the same storage transaction as the account
      and first-device inserts, and keep in-panel navigation disabled while an
      asynchronous account transition is in flight.
- [x] Run focused service, identity, UI, real-browser, formatting, and build
      checks; record any unverified paths here.

## Flow audit after the change

- **Fresh browser account:** send code, preflight the verified email, create a
  passkey and sign the account request in one identity bridge operation, submit
  the authoritative account transaction, then attach the account locally. A
  normal PRF-capable authenticator now needs one WebAuthn ceremony.
- **Existing accountless root:** send code, preflight the verified email, use
  the existing passkey to sign the request, submit the account transaction,
  then attach the account locally. No new credential is created.
- **Log in on another browser/device:** assert a discoverable passkey once,
  atomically link the device remotely, then persist the returned account
  locally. This flow does not ask for an email or verification code.
- **Legacy account setup:** assert the existing passkey once, establish the
  signed account repository descriptor with create-if-absent semantics, then
  persist the winning descriptor locally.
- **CLI handoff:** resolve the one-time handoff, assert the existing passkey
  once, then complete the remote link. It does not mutate browser account state.

## Remaining failure boundaries

- Email availability can race between preflight and final insertion. The final
  database uniqueness check is authoritative and returns the same clear
  conflict. A durable reservation was rejected because it creates a new stale
  state and denial-of-service surface.
- Authenticators that do not return PRF output during credential creation need
  the existing fallback assertion to recover it. That uncommon path can still
  involve two WebAuthn prompts, but the credential and root remain reusable.
- A passkey/root is saved locally before the remote account request so a
  transient remote failure can be retried without creating another credential.
  This is an intentional, valid accountless identity state.
- Remote account creation and local browser attachment cannot share a database
  transaction. If local attachment fails after an accepted response, the UI
  says the account is ready and directs the user to log in. If the remote write
  succeeds but its response is lost, the account is still recoverable through
  login, but the transport error cannot prove whether the write committed; an
  idempotent create acknowledgement remains future hardening.
- A user can still leave `/account` through the top-level return link while work
  is in flight; navigation destroys the page and its task. In-panel choices and
  back buttons are disabled until the active transition settles, preventing a
  stale completion from repainting a different panel.
- The UI now requires `/accounts/preflight`, so rollout order is account worker
  first and UI second. An older account worker returns an error and account
  creation fails closed before WebAuthn rather than silently falling back to the
  unsafe ordering.

## Verification evidence

- The new service regression failed with `404` before the preflight route was
  implemented, then passed after the change.
- The account-transaction regression failed because a root conflict had already
  consumed the code, then passed after code deletion moved into the same SQLite
  transaction and D1 batch as the account and first-device inserts.
- The in-flight navigation regression failed because the choice/back buttons
  remained enabled, then passed after `set_busy` covered every in-panel route.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- `cargo check -p tonk-ui --target wasm32-unknown-unknown` passed in the Nix
  development shell, as did the Worker-target check for
  `tonk-account-service`.
- All 34 `tonk-identity` Wasm unit tests passed.
- All 65 account-service unit tests and all 10 HTTP integration tests passed;
  the Cloudflare Worker package also compiled for Wasm.
- All 28 `tonk-ui` Wasm tests passed, including 15 account UI tests and 5
  identity bridge tests.
- The isolated real-browser regression passed in Chrome: the existing-email
  path created zero credentials; returning to email entry cleared the code;
  the available-email retry completed with exactly one credential.
- The first browser attempt stopped before Chrome because the Nix build volume
  was full. Removing 11.9 GiB of this checkout's disposable Cargo artifacts
  let the clean repository web-server derivation build and the unchanged test
  pass.
- No live Worker/D1 deployment or physical authenticator was exercised. The
  native HTTP service, Worker Wasm build, and Chrome virtual authenticator cover
  the implemented seams; the non-PRF-at-creation fallback remains unit-tested
  rather than hardware-tested here.

# Joined-space resharing

Joined members must be able to mint invitations from their existing delegated
authority without enrolling the space under their own account. Owned spaces
must retain provisioning and onboarding-consent repair.

The share path treats an absent account-local `SpaceProvider` as a provisioning
request. `build_provider_add_invocation` submits the first proof as consent;
for joined authority that proof is addressed to an earlier member, so the
service correctly rejects the current customer's request.

Implementation checkpoint:

- Extend the joined-member FABB regression to require the provisioning boundary
  to accept joined authority without contacting the registration service.
- Recognize an indirect persisted space-root prefix as delegated access, leaving
  its provider untouched. Keep direct and missing owned prefixes on the existing
  provisioning/repair path. Invitation minting still verifies delegated access.
- Run the focused Wasm regression, relevant sharing tests, formatting, and the
  access service's cross-account consent rejection test.

Status: implemented and locally verified.

- Before the fix, the joined-member regression failed at
  `provision_space_consumer` with `Internal("the worker origin is unavailable")`:
  joined authority incorrectly attempted the registration path. This reproduces
  the unwanted provisioning decision, not the hosted service's 403.
- After the fix, nine focused Wasm tests passed: the four `it_mints` tests,
  `it_requires_provisioning_for_owned_space_authority`, and four `it_attaches`
  tests in `router::repository::tests`.
- Built with `cargo test -p tonk-worker --target wasm32-unknown-unknown --lib
  --no-run`; ran the resulting Wasm with `wbg-pool` and the filters above.
  The sandboxed browser daemon failed to start; rerunning with local daemon
  access worked.
- `cargo test -p tonk-access-service --features integration-tests --test
  registration it_refuses_a_consent_issued_to_another_customer`: one passed.
  The initial invocation without the feature ran zero tests and was corrected.
- `cargo fmt --all -- --check` and `git diff --check` passed.

No affected live browser session or reported space DID has been inspected;
hosted recipient pull and deployment remain unverified.

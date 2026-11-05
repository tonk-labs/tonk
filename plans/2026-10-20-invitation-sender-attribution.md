# Invitation sender attribution

Status: implemented and verified locally; follows the email discovery spike.

When Bob mints an invitation to Alice's space for Carol, record Bob as the
inviter. Alice remains the founder in the replicated membership roles.

Implementation checkpoints:

1. Put the minting account DID in the leaf grant's signed `invitation.inviter`
   metadata. Accept it only when it is the signer or reaches the signer through
   the contiguous subject-open identity delegation suffix. A scoped space grant
   cannot establish someone else's account identity.
2. Validate that binding when constructing/parsing invitations and projecting
   their durable records. Never inherit inviter metadata from an upstream grant.
   Links without the new metadata keep their existing interpretation and entity
   identity; this does not migrate historical invitation records.
3. Prove malformed and spoofed claims fail with focused unit tests, then run the
   existing four-account spike: email lookup, targeted invitation, wrong-account
   rejection, Bob's reshare, Carol's persisted roster and Alice's founder role.

Binding validation is structural and complements normal UCAN admission; it does
not independently verify signatures. Forwarding an existing URL is not minting a
new invitation and does not change its signed sender.

Implementation scope: the worker mint route (including its shared open/scoped
path) emits the new metadata. Shared invite parsing and schema projection
consume it. The [CLI follow-up](2026-10-20-cli-invitation-sender-attribution.md)
now emits the same metadata and explicitly proves through the selected account,
with onboarding and linked-account reshare flows verified. Old clients which
do not understand the metadata still use their old attribution rule.
Legacy fallback is compatibility data, not a verified sender claim: callers
must not present an old first-root attribution as verified merely because the
invitation parses. This change does not introduce a verification badge.

Verification:

- All 49 `tonk-invite` library tests passed, including five new attribution
  checks: account signing, account/device/session reshares, Alice or unrelated
  account impersonation refusal, malformed metadata refusal, leaf-only reading,
  and open/scoped URL round trips.
- All seven invitation schema tests passed, including correct Bob projection,
  rejection of Alice impersonation, unchanged subject/audience, and legacy
  fallback when only an ancestor carries metadata.
- The first unit-test compile needed an explicit `Option<DelegationChain>` type
  in the synthetic fixture; corrected before the passing checks above.
- The first worker compile caught a missing `RepositoryExt` import in the new
  founder-role query; added the trait import and rebuilt.
- Four-account worker spike passed in 13.13 seconds on 2026-10-20. Real email
  lookup and competing registration refusal remain correct. Mallory cannot
  claim Bob's invitation, gains no authority, and leaves no roster entry.
  Carol joins Bob's new invitation; its record and Carol's persisted roster
  attribute Bob. Bob's roster attribution remains Alice. Carol's replicated
  content retains Alice's founder role and Bob's member role.
- Seven focused join-failure tests and one mint-refusal-copy test passed on the
  freshly rebuilt worker binary.
- Workspace formatting and scoped whitespace checks passed.

The existing nonfatal `retain space delegation into account space` version
mismatch warning appeared again during the spike. Space joining, usable
authority and persisted roster/roles passed; account backup and cross-device
recovery were not tested. Browser visuals, WASM runtime, hosted services, real
email delivery were not exercised. CLI minting is covered by the linked follow-up.

Commands use the existing isolated target from the discovery spike to avoid
the workspace's mixed-worktree artifacts. No dependency versions changed;
`Cargo.lock` only adds the direct `tonk-schema -> tonk-invite` dependency.

```sh
CARGO_TARGET_DIR=/private/tmp/tonk-identity-discovery-target RUST_MIN_STACK=33554432 \
  cargo test --offline --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-invite --lib

CARGO_TARGET_DIR=/private/tmp/tonk-identity-discovery-target RUST_MIN_STACK=33554432 \
  cargo test --offline --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-schema --lib invitation::tests

CARGO_TARGET_DIR=/private/tmp/tonk-identity-discovery-target RUST_MIN_STACK=33554432 \
  cargo test --offline --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-worker --lib --features integration-tests identity_discovery_spike \
  -- --test-threads=1
```

The integration test requires local listeners for the access service fixture.
Its synthetic activation links must not be copied into reports.

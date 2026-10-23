# Email invitation through signup

Status: native protocol spike implemented and verified on 2026-10-21. Based on merged #1140 (`fdb552ecc0`).
Desktop follow-up must extend the Address Book in #1134, not introduce a second
contact store or replacement UI. That redesign is still open at inspection.

## Question and smallest proof

Can Bob leave a space invitation for an email before Carol has an account,
go offline, and let a delivery service complete authority only for Carol's
subsequently confirmed account? Can Bob's original signed invitation remain
distinct from the service's completion hop?

Keep the experiment native and test-only, beside the access-service integration
tests. Use real enrollment, captured activation email, confirmed lookup, UCAN
verification and authorized storage HTTP requests. A prototype pending record
retains a per-invitation delivery key; a signed claim proves the recipient
account. A service restart reloads the record. No inviter signer is retained.
Return the original signed invitation and its one-hop, recipient-bound completion
without exposing the delivery seed. Never pretend the delivery service is Bob.

Required assertions:

- Unknown and unconfirmed addresses cannot complete a claim.
- An account with a different confirmed email, or a forged account assertion,
  cannot claim the invitation even when it knows the emailed locator.
- Signup and confirmation make the intended account eligible; device authority
  may authenticate for that account. Completion works after inviter shutdown and
  pending-store reload, and duplicate requests return the same completion.
- The completed chain actually authorizes storage for Carol; Mallory cannot use
  it. Client validation preserves Bob's original signed attribution and rejects
  a substituted original invitation or completion.
- Expired/cancelled pending invitations refuse. Completed records never switch
  recipient roots on a later email lookup change.

## Deliberate boundaries

The pending store, generated email locator and delivery envelope are a test harness, not
production APIs. Existing account activation and storage routes remain real.
No real emails, browser signup UI or desktop UI are exercised. Production needs
persistent atomic claim/cancel/retry semantics, encrypted delivery-key custody,
rate limits, account-device revocation checks and email delivery/retry integration.
The prototype signs the email into invitation metadata so changing the pending
record cannot retarget it; production must settle disclosure/privacy before
putting an email in portable proof material. The locator is generated here, not
sent through a production invitation email endpoint. The delivery service is trusted
with that one invitation's capability; it gets no account identity grant.

An ordinary scoped invite made from the completion chain would lose Bob's
attribution because its leaf is signed by the delivery key. Do not weaken the
merged leaf-binding check. A future client import must validate the explicit
original/completed envelope, record provenance from the original, and install
authority from the completed chain. This spike will expose that boundary rather
than silently synthesizing roster facts or claiming the existing join UI handles it.

After this proof: add confirmed-email resolution to the redesigned contact
editor, preserve the pinned DID, and keep contact saving separate from sharing.
Then wire the pending invitation signup continuation and envelope import.

## Result and production decisions

The offline handoff works with a per-invitation service-held capability. Bob's
account and device signers are dropped before Carol exists. After real service
enrollment and activation through the captured inbox, Carol's device authenticates
for her root and the retained key extends the grant to that root. The recipient
gets no delivery seed. A real `/ucan/` storage request succeeds for Carol and
fails with 401 for Mallory; Carol's object permit reaches the absent cell (404).
This proves authority admission, not data synchronization or a joined-space UI.

The client must retain two different artifacts: the original signed invitation
for Bob's attribution and the completed chain for Carol's authority. The emailed
locator pins the original grant CID. Validation checks that original, every
signature, exact prefix continuity and the recipient; substituting either half
or an entirely different valid delivery fails. The existing `Invite` parser
correctly sees no Bob metadata on the completion's service-signed leaf. Do not
pass it alone to the current join flow and call that correct attribution.

Pending and completed records survive serialization/reload. Sequential retries
return byte-identical completion, and another account cannot retarget it. This
is not a concurrency or crash-atomicity proof. Expiration and cancellation checks
cover unclaimed invitations; revoking already-issued authority is separate.

The production design still needs:

1. A durable, atomic pending/claimed/cancelled record and encrypted custody for
   the per-invitation key. Authenticate sends and claims, check device revocation,
   and enforce delivery/retry limits. No broad account identity grant is needed.
2. A privacy decision for the signed email binding. This synthetic protocol
   embeds the email; that must not become a portable-proof disclosure by accident.
3. An explicit validated delivery-envelope import that keeps original provenance
   separate from installed authority, with real worker/CLI roster regressions.
4. A signup continuation and email delivery endpoint that send only the locator,
   then complete after same-email verification. Browser/passkey and production
   email delivery need their own end-to-end test.
5. Desktop integration on #1134: extend its existing local, account-scoped contact
   store with email evidence and a pinned DID; add confirmed-email lookup to its
   contact editor, then offer an explicit invitation for an unknown address.
   Lookup outages and unconfirmed registrations must not be shown as absent users.

## Verification

- The initial offline build found the locked `anyhow` crate absent from cache.
  Fetching the exact locked dependencies resolved this; no lockfile changed.
- The first compile found two test API mistakes (invocation audience access and
  an Option-returning DID helper). Both were corrected before runtime checks.
- Initial focused runtime: one spike passed in 0.48 seconds.
- After adding locator pinning and substitution checks: 12 tests passed in 1.13
  seconds (the new spike and all 11 existing lookup tests), zero failures.
- Workspace formatting and whitespace checks passed. Storybook freshness and
  all 185 local links passed. No product contract or visual capture changed.

```sh
CARGO_TARGET_DIR=/private/tmp/tonk-discovery-pr-target RUST_MIN_STACK=33554432 \
  cargo test --offline --locked --profile ci-test \
  --config 'profile.ci-test.opt-level=0' \
  --config 'profile.ci-test.package."*".opt-level=0' \
  -p tonk-access-service --test access_service --features integration-tests \
  -- email_invitation_signup lookup:: --test-threads=1
```

The command was compiled with `--no-run`, then its binary ran with local fixture
listeners permitted. Activation URLs were filtered from output. No production
invitation email was sent; the generated locator is a proposed email link.

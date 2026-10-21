# Email invitation into a shared space

Base: Add Contact PR #1148 (`b57a0db5a1`). User request: after a missing-account
lookup, explicitly select a shared space and send an email that continues through
account creation, email verification, and joining that exact space.

## Invariants

- An emailed locator is not bearer authority. Only the account with the confirmed
  invited email may claim; forwarded locators and another account must fail.
- Bob remains the signed inviter when resharing Alice's space. A service completion
  hop installs Carol's authority without impersonating Bob or changing ownership.
- No account signer is retained. Delivery uses a per-invitation key derived with
  domain separation from the service secret and an unguessable invitation ID.
- A durable atomic record pins sender, email, space, original grant, expiry and
  eventual recipient/completion. Retrying cannot retarget an invitation or issue
  multiple independent grants. Revoked devices cannot send or claim.
- Only a real not-found lookup offers this path. Pending, suspended and unavailable
  remain distinct. Sending is explicit; saving a contact grants no access.

## Implementation checkpoints

1. Production pending invitation protocol/storage, authenticated preparation,
   send/claim/cancel, bounded/idempotent delivery, and validated original/completed
   envelope. Test wrong email/account, restart, concurrency, expiration, retries,
   revocation, and sender attribution before exposing UI.
2. Worker mint/send/claim integration and normal join import retaining original
   provenance. Signup continuation must survive activation navigation and require
   the intended verified account before claiming.
3. Desktop missing-account action with space selection and explicit Send Invite,
   existing email transport, browser continuation, Storybook, focused native/browser
   verification, and a separate follow-up PR.

## Evidence and remaining work

Inspection only so far. #1145 is a test-only protocol spike, not a production
endpoint. #1148 remains open. Main checkout has unrelated dirty work and is left
untouched; implementation uses the existing clean attached worktree.

### Checkpoint: validated completion envelope

Merged main's #1140 into the desktop-based branch; retained both documentation
sections in the only conflicts. The delivery envelope now verifies its signed
opaque ID, original prefix, all signatures, one-hop completion, subject and
recipient. Original provenance is retained separately from installed authority.
All 51 invitation-library tests pass, including the new Bob attribution/URL
round-trip and wrong-recipient/substitution regressions. Service/store and join
integration work is still in progress; no email endpoint is shipped yet.

### Checkpoint: durable service

Four service/SQLite tests pass: recipient and confirmed-email binding, reopening
storage, concurrent completion, revoked devices, cancellation/expiry, quotas,
delivery leases, and a lost mail-transport acknowledgement retried without a
second captured email. Native worker and Wasm worker/UI/access-service checks
pass. The native desktop suite passes with a new persisted retry/account-isolation
test. No production email was sent.

The first full HTTP/worker test failed before mint because the pre-existing
account fixture intentionally served no deployment discovery document. The
email flow now starts a fixture with deployment discovery enabled; rerun pending.
The browser artifact is building through Nix; cache download warnings are not
browser execution evidence. Signup return routing accepts only the exact local
`/email-invite#<64 lowercase hex>` form. The page resolves the active profile
branch before opening signup or claiming, including after a profile reload.

Deployment requires control migration 0007 and the existing SERVICE_SECRET_KEY,
RESEND_API_KEY and EMAIL_FROM configuration. Rotating the service key invalidates
unclaimed delivery keys. Delivery retries use the same provider idempotency key,
at most three attempts with a minute cooldown within 23 hours; claims expire in
seven days. Cancellation is available in the service protocol; the desktop does
not yet expose a pending invitation management screen. Saving a contact remains
separate from sending. Automatic approval review rejected an optional customer
deletion trigger for delivery-record cleanup as an unauthorized irreversible
retention change; it was not applied. No automated row cleanup is added here.

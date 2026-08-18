# Account key custody: the envelope scheme

Status: agreed design, 2026-08-18. Supersedes the passkey-derived account
identity described in `Account model.md` §1. Companion to the
profile-as-account-upstream restructure, which should build on this.

## Why the current construction has to go

Today the account keypair is a pure function of one passkey: WebAuthn PRF
output, through HKDF, is the Ed25519 seed. Three structural problems:

1. One credential *is* the identity. No second passkey, hardware key, or
   recovery phrase can ever produce it. Losing the credential — or the
   provider account custodying it — is unrecoverable identity loss.
2. It forces passkey creation into the account-creation critical path,
   which forces the create-vs-login fork in the UI, and repeated
   `create()` calls with fresh random `user.id` values accumulate
   duplicate credentials at the provider.
3. E2E encryption is planned. An encryption hierarchy rooted in a seed
   that is a function of a single credential can never rotate away from
   that credential, and signing keys must not double as encryption keys.

## The inversion

The account is a locally generated secret; the passkey becomes one of
several interchangeable custody methods for it.

- **Account secret**: 32 random bytes, generated client-side. Never
  stored in plaintext outside memory; zeroized after use.
- **Derivation**, domain-separated HKDF:
  - `HKDF(secret, "tonk/sign/v1")` → Ed25519 account signing key. Roots
    device delegations exactly as today; the account DID is derived from
    it, so the DID materializes from the secret and nothing needs to
    record it separately.
  - `HKDF(secret, "tonk/enc/v1")` → reserved for the X25519 encryption
    root when E2EE lands. Not implemented now; the reason the *secret*
    is wrapped rather than the derived signing key.
- **Non-extractable signing handle.** At first run, derive the signing
  key once and import it as a non-extractable WebCrypto Ed25519 key;
  routine signing (delegations, deposits) uses the handle. The plaintext
  secret is unwrapped only for custody operations: enrolling a wrapping,
  and later the E2EE derivation. Without this, a silent local unwrap
  would let compromised page code sign as the account; with it, the
  scheme is strictly safer than today's ceremony-gated derivation.
- **Device keys unchanged**: non-extractable per-device Ed25519,
  authorized by a delegation the account key signs.

### Uniform envelope — no derived first passkey

Every custody method, the first passkey included, is a wrapping of the
same secret. The hybrid alternative (first passkey derives the secret,
envelope only for later ones) was considered and rejected: a derived
first passkey is mathematically irrevocable — it computes the account
forever, no deletion or future rotation can sever it — and it puts
WebAuthn back into account creation. One mechanism, every passkey
removable, is the whole point.

## Wrappings

A wrapping is the secret AEAD-encrypted under a KEK. Multiple coexist;
any one unlocks; wrappings never reference each other — each is an
independent way to open the same box.

1. **Local wrapping** (first run, mandatory): KEK is a non-extractable
   WebCrypto AES key; wrapped blob and key handle persist in IndexedDB
   (`navigator.storage.persist()` requested, treated as advisory). Until
   a durable wrapping exists the account is this-device-only, and the UI
   says so — the meaningful state distinction is durable custody vs
   local-only, not "has account".
2. **Passkey wrapping** (the backup path): `create()` with PRF, then
   evaluate the PRF at two fixed application salts —
   `"tonk/wrap/lookup/v1"` addresses the custody cell,
   `"tonk/wrap/kek/v1"` feeds HKDF for the KEK. Deterministic per
   credential, 256-bit unguessable, computable only inside an assertion.
3. **Recovery phrase wrapping** (optional): identical shape with
   Argon2id in place of PRF — the KDF output splits into the custody
   keypair seed and the KEK. One custody mechanism, two entry functions.

Blob format: version, generation counter (rotation must be expressible
later without format migration; not built now), algorithm identifiers,
KEK-method tag, AEAD nonce.

## Custody storage: cells under custody subjects

Each remote wrapping derives a **custody keypair** (from the PRF/KDF
output) whose DID owns a tiny remote-only namespace:

- **Enrollment** (requires the plaintext secret, so only a device with
  existing custody can do it): derive custody keypair → encrypt secret →
  one presigned PUT publishing the cell under the custody DID's own
  subject → `/provider/add` the custody DID as a consumer under the
  account's customer → assert a wrapping fact (name, custody DID,
  created-at) in the account DB for the management UI.
- **Unlock** (fresh device): `get()` with empty `allowCredentials` (or
  typed phrase) → derive custody keypair → one presigned GET of its own
  cell, **self-rooted** (subject = issuer), which the authorizer already
  accepts with no delegation — this is what breaks the bootstrap
  circularity, since a fresh device has nowhere to obtain delegation
  bytes from → decrypt → hold the account key → sign the device
  delegation → zeroize.

Why this shape:

- The account DID and the device delegation both come *out of* the
  unwrap — the DID is derived, the delegation is minted on the spot.
  Nothing is fetched that would itself require authorization.
- No standing delegation from the account to any custody key. A passkey
  yields custody (the ability to obtain the secret); all authority flows
  from the secret. A device authorized by approval-from-another-device
  gets revocable authority *without* custody — that asymmetry is
  deliberate.
- Custody DIDs are provisioned consumers: someone pays, reads are
  metered and attributed like everything else, and enforcement needs no
  carve-out. The custody namespace never materializes as a local
  database on any device — it is one PUT at enrollment and one GET at
  unlock.
- The wrapping *facts* in the account DB are truth for management
  (listing, naming, removing); removal is delete-cell + retract-fact.
  A cell-in-the-account-DB design was considered and rejected: the cell
  would sit behind the authorization it exists to bootstrap.

**Future optimization — storage aliasing.** R2 has no symlinks, but the
presigner constructs object keys, and enforcement already puts the
consumer row on that path. An optional storage-alias field on the
consumer row lets the authorizer map the custody subject's objects into
`{account}/custody/{custody_did}/…`, dissolving the separate namespace
into the account's prefix (shared lifecycle, shared accounting) while
keeping the custody DID a first-class consumer for authorization and
billing. Requires a small per-subject key-prefix hook in
dialog-remote-ucan-s3's `UcanAuthorizer`; do when touching the
authorizer for enforcement.

## Flows

- **First run**: nothing. No account, no secret, no ceremony —
  pre-account spaces delegate to the device key as shipped.
- **The account moment is email submission.** Unknown email → create:
  generate the secret, local wrapping, derive the signing handle,
  register through the current flow — still zero WebAuthn; the passkey
  is nudged later at a moment of demonstrated value. Known email →
  login: "Continue with passkey" (or phrase), the unlock flow above.
  The create-vs-login fork becomes implicit and un-mistakable, which
  also enforces the no-fork rule below. Email lookup is a router, never
  a key: it must not address any blob (enumeration, offline attack on
  phrase wrappings).
- **Local content created before the account moment** is adopted by the
  link-time sweep exactly as shipped (redelegate, retain, provision).
  A local-only root that never registered is disposable by construction;
  nothing registers with any service before the email step.
- **`create()` must never be reachable as a fallback from a failed
  unlock.** A second secret is a permanently forked identity wearing the
  same email. Creation happens only behind explicit user affirmation in
  the enrollment flow, and the existing email-conflict machinery refuses
  a second registration for a known address. This invariant gets a test,
  not just a sentence.

## Hygiene

- Stable `user.id` derived from the account DID at passkey creation, so
  same-provider re-enrollment overwrites instead of accumulating.
- After a `get()` whose PRF output finds no cell or decrypts to the
  wrong identity, call `PublicKeyCredential.signalUnknownCredential()`
  so providers prune stale entries (progressive; availability varies).
- Registration with the access service keys on what it already keys on
  (email provided, activation clicked) — never on custody. A
  local-custody-only account may register; backup enrollment is a UX
  nudge, not an enforcement input.

## Migration

None. The one existing PRF-derived population (the team's own accounts)
resets; accepted 2026-08-18. If that changes, the bridge is one `get()`
on the old passkey, reproduce the old derivation, and adopt the seed as
the account secret — identity and delegations preserved.

## Verify before building

1. ~~Seed comes from PRF, not the assertion signature~~ — verified in
   `tonk-identity/src/derive.rs`: PRF output through HKDF.
2. Whether target browsers return PRF at `create()` or need a follow-up
   `get()` — our create flow already handles the follow-up, so backup
   enrollment is one ceremony where supported, two elsewhere.
3. PRF over hybrid transport (QR to phone) across the real platform
   matrix. The fresh-device story leans on it; support is uneven.
4. Non-extractable Ed25519 in WebCrypto across the target matrix (the
   signing-handle refinement assumes it).

## Non-goals

- Key rotation / compromise recovery (the generation field reserves the
  format space; implementation later).
- The E2EE encryption root (reserved HKDF domain only).
- Custody transfer to approval-linked devices (they keep delegated
  authority without custody).

## Sequencing

After the registration stack (landed, #724): this custody scheme first —
it is self-contained beneath the ceremony bridge and it hands the
profile-as-account-upstream restructure its cleanest precondition
(accounts exist independent of any credential). Then the upstream
restructure, per `Account model.md` §5 in its literal form.

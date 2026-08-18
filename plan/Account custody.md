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
   `"tonk/custody/key/v1"` seeds the custody keypair,
   `"tonk/custody/kek/v1"` feeds HKDF for the KEK. Deterministic per
   credential, 256-bit unguessable, computable only inside an assertion.
3. **Recovery phrase wrapping** (optional): identical shape with
   Argon2id in place of PRF — the KDF output splits into the custody
   keypair seed and the KEK. One custody mechanism, two entry functions.

Blob format: version, generation counter (rotation must be expressible
later without format migration; not built now), algorithm identifiers,
KEK-method tag, AEAD nonce.

## Custody publication: the delegation is the public artifact

Each remote wrapping derives a **custody keypair** from its entry
function (PRF or KDF). What gets published publicly is not the wrapped
secret but a **standing delegation `account → custody key`**, carried in
a DID document the custody key's did:web identifier resolves to:

- `did:web:tonk.spot:custody:{custody-key}` →
  `https://tonk.spot/custody/{custody-key}/did.json`, containing
  `alsoKnownAs: [account DID]` and the embedded delegation. did:web
  resolution is a plain HTTPS GET — public by its own semantics, which
  is exactly the property bootstrap needs. The document is uploaded at
  enrollment (authorized, customer-attributed PUT) and served
  statically.
- The delegation is self-authenticating (signed by the account key);
  `alsoKnownAs` is a convenience the delegation's issuer field proves.
- **The wrapped secret lives in the account DB** — where it always
  wanted to live. The circularity that previously forced it outside the
  gate is gone: authorization bootstraps from the public delegation, and
  the secret is fetched only after the device is authorized. Whether it
  is represented as a fact or a cell is an open implementation choice,
  not a design point — a cell has the attractive property that a freshly
  authorized device fetches it at a well-known path with one presigned
  GET, before it can hydrate or query anything.

Two fixed entry-function salts, with distinct jobs:
`"tonk/custody/key/v1"` seeds the custody keypair (its public key is
the did:web path — the lookup *is* the DID); `"tonk/custody/kek/v1"`
derives the KEK for the wrapped-secret fact.

### The publication capability

Publishing is a customer act in the established deposit pattern,
role-first beside `/customer/*` and `/provider/*`:

- `/custody/publish { custody: Did, delegation: Cid, consent: Cid }` —
  the invocation is device-signed on the **account's subject** through
  the `root → device` link, so attribution and metering land on the
  customer and the custody DID never needs provisioning. Two deposits
  ride as container tokens named by CID (arguments, not proofs), the
  `/provider/add` shape exactly:
  - `delegation`: the `account → custody-key` grant — the document's
    payload. Verified issuer = invocation subject, audience = `custody`.
  - `consent`: the custody key's countersignature — issuer = `custody`,
    audience = the account, command covering `/custody/publish`. This
    is what makes the binding bidirectional: without it an account
    could publish a document tying itself to any DID it likes, a false
    public linkage the named key's holder never agreed to. At
    enrollment the device holds the just-derived custody private key,
    so minting it is free.
  The service verifies the chain, requires the subject to be a
  registered customer, checks both deposits, and serves the document at
  `/custody/{custody-key}/did.json`.
- `/custody/retract { custody: Did }` — removal, paired with revoking
  the delegation through the relay. Deliberately needs **no consent**:
  retraction is the account withdrawing its own claim, and it must work
  when the passkey is lost — which is the main occasion for it.
- **Resolution is deliberately not a capability.** did:web resolution is
  an unauthenticated GET; the custody key's holder is merely the only
  party who can derive the address (the DID comes out of the PRF inside
  an assertion) — and the only party holding the custody private key the
  delegation is addressed to. Resolution needing no authorization is
  precisely what breaks the bootstrap circle.

Why this shape:

- **Unlock requires no custody.** A fresh device derives the custody
  key, resolves the document, and the custody key re-delegates to the
  fresh device key: `account → custody → device`. The account secret
  never materializes in memory for routine linking — strictly safer
  than an unwrap-on-link design.
- **The bootstrap chain is temporary.** Once the device has pulled the
  account, it unwraps once (post-authorization) to mint a direct
  `account → device` delegation and switches its remote to it. Steady-
  state chains are exactly today's shape — no custody hop, shorter
  proofs — and later revoking the passkey does not cascade onto devices
  it bootstrapped.
- The standing delegation is no escalation: the passkey can always
  reach full custody through the KEK anyway, so the delegation is a
  shortcut, not a new power.
- **Removal is real revocation**: revoke the delegation through the
  existing revocation relay (already checked on the sync path), retract
  the wrapping fact, delete the document. Stronger than deleting a
  ciphertext and hoping nobody cached it.
- **Nothing touches the hot path.** Custody traffic is ordinary
  account-subject traffic under an ordinary UCAN chain, billed to the
  customer like everything else: no per-passkey consumers, no
  provisioning choreography, no alias map consulted at presign time.
- Privacy note: the document publicly links custody key ↔ account DID
  to anyone who learns the custody-key DID. Presented chains reveal the
  account DID anyway and the path is unguessable, so this is
  observation-equivalent to the status quo.

Considered and rejected along the way, recorded so the arguments are
not relitigated:

- *Wrapped-secret cell under a self-owned custody subject, no
  delegation*: breaks the circularity but costs a provisioned consumer
  per passkey, puts an alias-resolution map on the presign hot path if
  that namespace is to be folded into the account's, and makes every
  link an unwrap.
- *Bespoke public blob namespace keyed by a PRF lookup value*: works,
  but is a nonstandard surface delivering less than the DID document —
  no revocation semantics, no linkage, no standard resolution.
- *Wrapped secret in the account DB with no public artifact*: the fact
  sits behind the authorization it exists to bootstrap.
- *First passkey derives the secret directly*: see above — irrevocable
  forever, and WebAuthn returns to the account-creation critical path.

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

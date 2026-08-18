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

## Custody publication: the custody key is a provisioned space

Each remote wrapping derives a **custody keypair** from its entry
function (PRF or KDF). The custody key becomes an ordinary **provisioned
space** under the account's customership, and everything custody needs
lives in that space as raw memory cells. No new service surface:
provisioning, cell writes, presigned reads, and withdrawing providership
are machinery that already exists.

Two fixed entry-function salts, with distinct jobs:
`"tonk/custody/key/v1"` seeds the custody keypair — deriving it *is*
the lookup, since its DID names the space; `"tonk/custody/kek/v1"`
derives the KEK for the wrapped-secret cell.

- **Publish** = provision + two cell writes, at enrollment. The account
  mints the delegation first; then:
  1. The account provisions the custody DID through `/provider/add`,
     exactly as it provisions any space. The consent deposit that
     contract already requires — the consumer's powerline to the
     account — *is* the custody key's agreement to the binding. The
     bidirectional-consent design we kept re-inventing is the existing
     provisioning contract's ordinary shape. The device holds the
     just-derived custody private key at enrollment, so minting the
     consent is free.
  2. The device writes two cells into the custody space's `/memory`
     under well-known names:
     - `delegation` — the standing `account → custody key` grant;
     - `secret` — the account secret AEAD-wrapped under this
       wrapping's KEK.
     These are raw named-cell writes — no repository, no branches, no
     history, **no permanent DB record anywhere**: on the server the
     space is two cells, and locally nothing ever hydrates it as a
     repo. (The third-database confusion must not return wearing a
     new hat.)
- **Resolve** = the space owner reading its own space. A fresh device
  derives the custody keypair inside the assertion, then reads the two
  cells with **root authority on the custody subject** — one presigned
  GET each, before any repository exists locally. The public-resolution
  requirement dissolves: resolution needed no authorization only
  because the resolver had none, but the resolver holds the custody
  key by construction. Nothing about the account ↔ passkey binding is
  public any more.
- **Retract** = the account withdraws its providership of the custody
  space and revokes the delegation through the existing relay (already
  checked on the sync path). Consent-free by nature — a provider
  withdrawing service needs no consumer signature — which is exactly
  what the passkey-lost case requires.
- **Squatting is impossible** for the structural reason: writing into
  the custody space requires a chain rooted in the custody subject,
  which is the custody key's consent by definition.

Why this shape:

- **Unlock requires no custody.** A fresh device derives the custody
  key, reads the `delegation` cell, and the custody key re-delegates to
  the fresh device key: `account → custody → device`. The account
  secret never materializes in memory for routine linking — strictly
  safer than an unwrap-on-link design.
- **The bootstrap chain is temporary.** Once the device has pulled the
  account, it unwraps once (post-authorization) to mint a direct
  `account → device` delegation and switches its remote to it. Steady-
  state chains are exactly today's shape — no custody hop, shorter
  proofs — and later revoking the passkey does not cascade onto devices
  it bootstrapped.
- The standing delegation is no escalation: the passkey can always
  reach full custody through the KEK anyway, so the delegation is a
  shortcut, not a new power.
- **The wrapped secret lives with its only reader.** The `secret` cell
  is fetched by the one party that can decrypt it, before the device
  can touch anything else — no fact in the account DB, no blob
  namespace, no record that outlives the wrapping. Retracting the
  space retracts the ciphertext with it.
- **Removal is real revocation**: the relay kills the delegation on
  the sync path, withdrawn providership makes the cells unreachable.
  Stronger than deleting a ciphertext and hoping nobody cached it.
- **Metering is boring**: the account provides the custody space like
  any space; a consumer row per passkey is the accepted cost of the
  bill landing somewhere. Nothing on the presign hot path, no alias
  map, no special attribution rule.
- Privacy improves: the binding is readable only by the custody key's
  holder, where the did:web variant published it.

Considered and rejected along the way, recorded so the arguments are
not relitigated:

- *did:web document with the embedded delegation*
  (`did:web:tonk.spot:custody:{key}` serving `did.json` with
  `alsoKnownAs`): standard-shaped, but nothing external ever resolves
  it — the "standard resolution" served only our own bootstrap, while
  making the binding public and demanding a bespoke publish/retract
  surface with hand-rolled verification rules.
- *Bespoke `/custody/publish` invoked on the custody subject*: closer —
  the invocation-as-consent insight survives in the provisioning shape —
  but still a new capability whose rules (audience = invocation
  subject, deposit subject must be a registered customer, subject
  inequality) re-derive what `/provider/add` already encodes.
- *Wrapped-secret cell only, no published delegation*: makes every
  link an unwrap; carrying the delegation keeps unwrap a one-time
  post-pull step and keeps the secret out of memory on routine links.
- *Wrapped secret as a fact in the account DB*: sits behind the
  authorization it exists to bootstrap, and forces a durable DB record
  where a raw cell suffices.
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
5. Raw named-cell writes and presigned reads against a provisioned
   space's `/memory` with root-subject authority and **no repository**
   behind it — the storage path must not demand a repo record, and no
   client path may hydrate the custody space as one.

## Non-goals

- Key rotation / compromise recovery (the generation field reserves the
  format space; implementation later).
- The E2EE encryption root (reserved HKDF domain only).
- Custody transfer to approval-linked devices (they keep delegated
  authority without custody).

## Sequencing

After the registration stack (landed, #724): one implementation arc,
custody together with the account-as-remote restructure. The custody
stack makes the account exist independent of any credential; the same
pass rolls the account in as the **upstream remote of profile main** —
the hidden account repository dissolves into a remote, which drops the
third local database. The custody space carries the delegation and the
wrapped secret, so nothing account-shaped needs a local record before
the first pull. Restructure semantics per `Account model.md` §5 in its
literal form; existing devices transition through the fresh-link/adopt
path, and the old account remote keeps working as just a remote.

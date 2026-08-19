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
- **Nothing is ever stored.** No KEK, no wrapping, no handle persists
  anywhere on a device. The secret materializes only inside a ceremony
  — account creation, unlocking a browser, enrolling another wrapping,
  approving a device, signing a revocation — always behind a fresh
  user-verified assertion, and is zeroized when the ceremony ends.
  Non-extractability of a stored KEK would protect the KEK, not the
  secret: compromised page code needs no extraction to `decrypt()` in
  place and exfiltrate the plaintext silently. With derive-on-assert,
  the worst compromised code can do is trigger a prompt someone can
  decline.
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

1. **Passkey wrapping** (mandatory; the first is created inside the
   account-creation ceremony): `create()` with PRF, then evaluate the
   PRF at two fixed application salts — `"tonk/custody/key/v1"` seeds
   the custody keypair, `"tonk/custody/kek/v1"` feeds HKDF for the KEK.
   Deterministic per credential, 256-bit unguessable, computable only
   inside an assertion. Creation is atomic: secret, credential, and
   published cell exist together or the ceremony fails — there is no
   window in which an account exists that no wrapping can recover.
2. **Recovery phrase wrapping** (later, not a blocker): identical shape
   with Argon2id in place of PRF — the KDF output splits into the
   custody keypair seed and the KEK. One custody mechanism, two entry
   functions. Besides passkey-loss recovery it is the escape from
   passkey-platform lock-in, and the coverage fallback for platforms
   without PRF — the one real cost of requiring a passkey at creation.
   Phrases must be generated high-entropy (word-list style), never
   user-chosen: the custody-space address derives from the phrase, so a
   guessable phrase is a fetchable, offline-grindable ciphertext.

Blob format: version, generation counter (rotation must be expressible
later without format migration; not built now), algorithm identifiers,
KEK-method tag, AEAD nonce.

## Custody publication: the custody key is a provisioned space

Each remote wrapping derives a **custody keypair** from its entry
function (PRF or KDF). The custody key becomes an ordinary **provisioned
space** under the account's customership, and it holds exactly one
thing: the wrapped secret, as a raw memory cell. No new service
surface: provisioning, a cell write, a presigned read, and withdrawing
providership are machinery that already exists.

Two fixed entry-function salts, with distinct jobs:
`"tonk/custody/key/v1"` seeds the custody keypair — deriving it *is*
the lookup, since its DID names the space; `"tonk/custody/kek/v1"`
derives the KEK for the wrapped-secret cell.

- **Publish** = provision + one cell write, at enrollment:
  1. The account provisions the custody DID through `/provider/add`,
     exactly as it provisions any space. The consent deposit that
     contract already requires — the consumer's powerline to the
     account — *is* the custody key's agreement to the binding. The
     bidirectional-consent design we kept re-inventing is the existing
     provisioning contract's ordinary shape. The device holds the
     just-derived custody private key at enrollment, so minting the
     consent is free.
  2. The device writes the `secret` cell into the custody space's
     `/memory`: the account secret AEAD-wrapped under this wrapping's
     KEK. A raw named-cell write — no repository, no branches, no
     history, **no permanent DB record anywhere**: on the server the
     space is one cell, and locally nothing ever hydrates it as a
     repo. (The third-database confusion must not return wearing a
     new hat.)
- **Resolve** = the space owner reading its own space. A fresh device
  derives the custody keypair and KEK inside the assertion, reads the
  cell with **root authority on the custody subject** — one presigned
  GET, before any repository exists locally — unwraps, derives the
  account signer, and **self-issues** a direct `account → device`
  delegation. No published delegation is needed: whoever can unwrap
  holds the account, so any grant it would carry can be minted on the
  spot. The secret is zeroized immediately after; this is exactly
  today's shape, a root signer transient under a biometric gesture,
  sourced from the envelope instead of the PRF derivation.
- **Retract** = the account withdraws its providership of the custody
  space, deleting the cell with it. Consent-free by nature — a
  provider withdrawing service needs no consumer signature — which is
  exactly what the passkey-lost case requires. And it is complete by
  construction: no standing grant to the custody key ever exists, so
  there is nothing to chase through the revocation relay. Devices the
  wrapping bootstrapped keep their own direct delegations, unaffected —
  removing a passkey removes an unlock method, not devices.
- **Squatting is impossible** for the structural reason: writing into
  the custody space requires a chain rooted in the custody subject,
  which is the custody key's consent by definition.

Why this shape:

- **One artifact.** The wrapped secret is the entire published state of
  a wrapping. It lives with its only reader — fetched by the one party
  that can decrypt it, before the device can touch anything else — and
  it is the root of everything: delegations are minted from it, not
  stored beside it.
- **No standing grants.** The custody key holds no delegation, so
  compromise-of-ciphertext is the only attack surface, and retraction
  cannot leave a live grant behind.
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
- *A published `account → custody` delegation beside the secret* (the
  payload the did:web and bespoke-capability variants existed to
  carry): claimed to keep the secret out of memory on routine links,
  but its own chain-upgrade step unwrapped once per fresh device
  anyway — both designs unwrap exactly once per link, so the
  delegation was a second artifact carrying no property, and a
  standing grant retraction had to chase through the revocation relay.
  Self-issuing from the unwrapped secret needs neither.
- *Wrapped secret as a fact in the account DB*: sits behind the
  authorization it exists to bootstrap, and forces a durable DB record
  where a raw cell suffices.
- *First passkey derives the secret directly*: see above — irrevocable
  forever.
- *A stored local wrapping (non-extractable WebCrypto KEK + envelope in
  IndexedDB) bridging a zero-WebAuthn creation to a later passkey*:
  built, then removed. Non-extractability protects the KEK, not the
  secret — the record is a standing capability for compromised page
  code to unwrap and exfiltrate silently — and the bridge window is a
  trap: evict the record before a passkey enrolls (iOS does evict) and
  the account is permanently unexpandable while still registered.
  Passkey-at-creation closes both, at the cost of one `create()` prompt
  and a hard PRF dependency the phrase wrapping later relaxes.

## Flows

- **First run**: nothing. No account, no secret, no ceremony —
  pre-account spaces delegate to the device key as shipped.
- **The account moment is email submission.** Unknown email → create:
  one ceremony generates the secret, creates the first custody passkey
  (one `create()` prompt — acceptable now that the envelope makes every
  passkey removable; what made passkey-at-creation wrong before was
  the *derived* root's irrevocability), seals the secret under its KEK,
  publishes the custody cell, and signs the creation request. Known
  email → login: "Continue with passkey" (or phrase), the unlock flow
  above. The create-vs-login fork stays implicit and un-mistakable,
  which also enforces the no-fork rule below. Email lookup is a
  router, never a key: it must not address any blob (enumeration,
  offline attack on phrase wrappings).
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
  (email provided, activation clicked) — never on custody beyond what
  creation already guarantees (a published cell exists before the
  creation request signs).

## Migration

None. The one existing PRF-derived population (the team's own accounts)
resets; accepted 2026-08-18. If that changes, the bridge is one `get()`
on the old passkey, reproduce the old derivation, and adopt the seed as
the account secret — identity and delegations preserved.

## Verify before building

1. ~~Seed comes from PRF, not the assertion signature~~ — verified in
   `tonk-identity/src/derive.rs`: PRF output through HKDF.
2. Whether target browsers return PRF at `create()` or need a follow-up
   `get()` — the create flow handles the follow-up, so creation is one
   ceremony where supported, two prompts elsewhere.
3. PRF over hybrid transport (QR to phone) across the real platform
   matrix. The fresh-device story leans on it; support is uneven — and
   with a passkey now REQUIRED at creation, PRF coverage is a hard
   dependency until the phrase wrapping ships. This is the watch item.
5. ~~Raw named-cell writes and presigned reads against a provisioned
   space's `/memory` with root-subject authority and no repository
   behind it~~ — verified: the presign path never consults repository,
   consumer, or customer records (`handlers/ucan.rs`, dialog's
   `authorizer.rs`); the storage key is `{subject}/{space}/{cell}`,
   created on first PUT; dialog-remote-fs tests exercise repo-less
   cells end to end. Two implementation notes: `publish(content,
   None)` is first-write-only (`If-None-Match`) — overwrites need the
   resolved version or `Cell::checkpoint()`; and provisioning is not
   yet enforced at presign but is intended to be, so provision anyway.

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

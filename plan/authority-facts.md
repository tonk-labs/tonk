# Authority as facts

How delegations, device links, space ownership, and wrapped keys are
represented in the DB.

## What dialog already gives us

Retaining a delegation chain decomposes every certificate into facts and
stores the signed envelope as a blob. The entity is
**`Entity::from_blob(&index_hash)`** — the hash of the certificate's own
bytes. Six attributes land on it:

| Attribute | Value |
|---|---|
| `dialog.ucan/issuer` | signer DID |
| `dialog.ucan/audience` | recipient DID |
| `dialog.ucan/subject` | subject DID, or `ANY_SUBJECT` for a powerline |
| `dialog.ucan/command` | command path |
| `dialog.ucan/expiration` | unix seconds, absent when unexpiring |
| `dialog.ucan/notBefore` | unix seconds, absent when unbounded |

Content-addressed, so re-retaining the same certificate converges rather
than duplicating.

> [!caution]
> `meta` is **not** decomposed. `field_artifacts` emits the six attributes
> above and nothing else, so metadata rides inside the signed envelope and
> is invisible to queries. This is the fact that shapes everything below.

## Two homes, chosen by what already exists

A delegation entity is the right home when there is no other entity for
the thing. It is the wrong home when one already exists and the UI
renders it.

**Devices go on the delegation.** `profile.this()` appears exactly once
in the whole schema, as a *field* on `Replica`, never as an entity. There
is no device record, so the delegation is the only keying available — and
it has the property we want: revoke the delegation and the row goes with
it, so a device cannot linger in a list after losing authority.

**Spaces do not.** The account directory already keys `Space`,
`SpaceName`, and `SpaceLocal` by the space DID, and the Hub renders that
row. Putting founding metadata on the ownership delegation would add a
*third* entity keying, and every Hub query wanting a creation date would
join through `dialog.ucan/subject`. So founding is a stamp on the
directory entity, exactly like `SpaceName`.

The delegation stays authoritative for *authority*. The space entity
carries *description*.

## Meta and facts do different jobs

Both, and the split is not redundancy:

- **`meta` in the delegation** is signed. It is the issuer's own statement
  of why the delegation exists, it travels with the envelope, and it
  survives leaving our DB. A recipient on another device can read it.
- **Facts on the entity** are queryable. They are how we list, sort, and
  filter locally.

Put the *reason* in `meta` (signed, portable, authoritative) and the
*queryable projections* in facts (local, indexed, derived). A fact can be
rebuilt from the envelope; `meta` cannot be rebuilt from facts.

```rust
DelegationBuilder::new()
    .issuer(Signer::from(space))
    .audience(&account_did)
    .subject(UcanSubject::Specific(space.did()))
    .meta("reason", "founder")      // signed, travels
```

## The two authority kinds

### Space founding — on the directory entity

```
{space DID}
  xyz.tonk.space/status      ← existing
  xyz.tonk.space/name        ← existing
  xyz.tonk.space/foundedAt   unix seconds
  xyz.tonk.space/foundedBy   the profile that created it
```

`SpaceFounded`, asserted only on the creation path. `record_space_mount`
runs for joined spaces too, so a stamp there would claim this account
made a space it was merely invited to. A directory row *without* the
stamp is a space this account joined — which is how the Hub tells the two
apart without consulting delegations.

`foundedBy` records the device, not the account: the account is implied
by the directory belonging to it, while which device created the space is
otherwise lost, since the ownership delegation's audience is the account.

### Device link — on the delegation entity

```
blob:{hash}
  dialog.ucan/issuer       the account          ← dialog
  dialog.ucan/audience     the profile (device) ← dialog
  dialog.ucan/subject      ANY_SUBJECT          ← dialog
  xyz.tonk.device/createdAt  unix seconds
  xyz.tonk.device/title      "Chrome on macOS"
  xyz.tonk.device/reason     "device-link"
```

`DeviceLink`, asserted where the powerline is minted. It is the one
concept with no identifying fields of its own: it takes the entity as
given, because dialog already made it.

## Namespaces

`xyz.tonk.space/*` and `xyz.tonk.device/*`, matching dialog's
`dialog.ucan/*` convention.

> [!caution]
> Devices are `xyz.tonk.device`, NOT `xyz.tonk.authorization` — that
> namespace already means an invite's access proof (`Proof`, `Remote`).
> One namespace holding both would make "authorization" ambiguous between
> a device link and a share link.

## The device label

`from_navigator` moved to `tonk-common` so the page and the worker share
one implementation: the page reads `window.navigator`, the worker reads
`WorkerNavigator`, and neither has the other's globals. A device labelled
one way at link time and another way in a list would look like two
devices.

The worker's label is coarser — no `platform`, no touch-point count — so
browser and OS come from the user agent alone.

## Wrapped keys (2026-08-25)

Not a delegation, so not this entity. A sealed space seed is its own
concept, and it is sealed to a **public key**, not wrapped under the
account KEK.

The KEK derives from the account secret, and the account secret only
materialises inside a passkey ceremony. A CLI device holds a delegation
from the root, never the secret, so a symmetric wrap at account clearance
would make every space creation a browser-only act. Sealing to a public
key splits the two halves: anything holding the account DB can *seal*,
only a ceremony can *open*.

### The account encryption key

`AccountSecret::encryption_key()` derives an X25519 key from the account
secret (`HKDF(secret, info = "tonk/account/encryption/v1")`), the same
way `signer()` derives the Ed25519 one. Its public half is a `did:key`
under the `x25519-pub` multicodec (`0xec`, `did:key:z6LS…`), so a
recipient is an `Entity` like every other reference. Published on the
account subject entity, next to `AccountDisplayName`:

```
{account DID}
  xyz.tonk.account/encryption-key   did:key:z6LS…   cardinality one
```

`AccountEncryptionKey`, written by the custody ceremony at account
creation, by onboarding-account creation (it has a secret too), and by
rotation. Cardinality one: rotation overwrites, and sealed rows name their
own recipient, so nothing is lost by that.

### The sealed seed

One row per `(subject, recipient)`, content-derived like `Membership`:

```
CustodiedSeed   this = Entity::of(CustodiedSeed { subject, recipient })
  xyz.tonk.custody/subject     the space DID, or the invite principal DID
  xyz.tonk.custody/kind        tonk:space | tonk:invite
  xyz.tonk.custody/recipient   the did:key:z6LS… it is sealed to
  xyz.tonk.custody/sealed      the envelope bytes
```

A row per pair rather than a stamp on the directory entity, because
rotation adds a row for the new recipient and retracts the old one rather
than overwriting in place, and sealing a space seed to an admin's account
as a recovery custodian (see `space-admins.md`) is just another row with
another recipient. `subject` and `kind` repeat the hash inputs as
queryable attributes so rotation can enumerate everything sealed to the
old key and a recovering device can find the seed for a space without
knowing the entity.

### The envelope

`version (1) ‖ algorithm (1) ‖ ephemeral X25519 pub (32) ‖ nonce (12) ‖
ciphertext`. Shared secret = ECDH(ephemeral, recipient); AEAD key =
`HKDF(shared, salt = ephemeral_pub ‖ recipient_pub, info =
"tonk/custody/v1")`; AES-256-GCM, as `Envelope` already uses. Associated
data is the header plus the recipient DID plus the subject DID, so a blob
cannot be re-pointed at another space or another recipient.
`seal(recipient, seed, subject)` needs no secret; `open(encryption_key,
sealed, subject)` needs the ceremony.

### Writers and readers

- Space create (worker and CLI): after `retain_space_delegation`, read
  `AccountEncryptionKey`, seal, assert `CustodiedSeed`. Best-effort like
  retain; an account with no key yet logs and skips.
- Join through an open invite: the joiner seals the invite principal's
  seed, `kind = tonk:invite`. The membership hangs off that principal,
  so it is the joiner's account that must re-issue `principal → new
  root` at rotation; the inviter holds `/` on the space and can always
  mint another invite, so nothing is sealed at mint.
- A sweep on the account-ready path: every locally held signer with no
  `CustodiedSeed` for the current recipient gets sealed. This repairs
  accounts that predate the key and re-seals after rotation on devices
  that still hold plaintext.
- Rotation (browser): open every row sealed to the old recipient,
  re-issue `space → new-root`, re-seal to the new recipient, retract the
  old rows.
- Recovery on a new device (browser): open the row for the space and
  reconstruct the signer into the local credential store.
- The CLI only ever seals.

The seed sits *beside* the ownership delegation rather than on it: the
delegation says who may act for the space, the seed says how to re-issue
it. They have different lifetimes, so one entity carrying both would make
every rotation a read-modify-write of the ownership record. Join on the
DID when both are needed: `CustodiedSeed.subject` equals the delegation's
`dialog.ucan/subject`.

Clearance is unchanged: sealing is public, opening is Recovery-gated. A
device compromise leaks nothing it did not already hold; an account
compromise costs the seeds, which `onboarding-accreditation.md` already
accepts. `vault-key.md` describes wrappings addressed to public keys
enumerable from the account space; this is that primitive with the
account as the only addressee, and the vault should share it.

## Where these live

The account space `main`, which replicates to every linked device and is
readable only by the account. Both the authority and the sealed seed must
reach every device on the account — that is what makes a space usable on
a second device and re-issuable after a lost one. The plaintext signer
stays in the creating device's local credential store; the fact is the
recovery copy.

## Relationship to #748 (feat/ucan-revocations)

That branch ships `router/account_devices.rs`: device list, revoke, and
self-revoke, as a **proxy to the account service**. Its `AccountDevice`
already carries `delegation_cid`, so the service keys a device by its
delegation too — the same model, server-side.

What this design adds is the **local** half: facts on the delegation
entity so a device list renders offline, without a round trip. The two do
not compete. The service stays authoritative for revocation status (it is
what enforces it); the local facts carry the label and creation time the
service would otherwise be the only source of.

> [!note]
> #748 also bumps the dialog pin from `tonk-2026-08-19` to
> `tonk-2026-08-23`. Verified: that bump breaks only
> `tonk-identity/src/revocation.rs` (dialog's `verify` gained a
> `VerificationContext`), which #748 itself fixes. The clearance and
> envelope work compiles unchanged against both pins, so this branch
> rebases onto #748 without crypto conflicts.

## Plan

1. ~~`SpaceFounded` on the directory entity~~ — DONE.
2. ~~`DeviceLink` on the powerline~~ — DONE.
3. **`AccountEncryptionKey` + `CustodiedSeed`** — the X25519 key derived
   from the account secret and published as a fact; the `Sealed`
   envelope in tonk-identity; the concept; seal-on-create in the worker
   and the CLI for spaces and invites; the backfill sweep.
4. **Rotation and recovery** consume all three: open and re-seal seeds,
   re-issue ownership delegations, retract the old device link.

## Open

- **`meta` on the delegation.** Not yet written. `DelegationBuilder`
  carries `meta: BTreeMap<String, Ipld>`, but `mint_account_union` does
  not expose it, so the signed copy of `reason` is still missing — only
  the queryable fact exists. Worth adding when that helper is next
  touched, since the signed copy is what survives leaving our DB.
- **Retract semantics for our attributes.** Dialog retracts its six when
  a chain is retracted. Ours are asserted separately, so they will not be
  retracted automatically. DECIDED: retract ours in the same commit — the
  history index keeps the trace, so the active index should not. Nothing
  in tonk retracts a delegation chain today, so this is one future call
  site to get right rather than a scattered set to keep in step.
- **`last-seen-at`.** DROPPED. A replicated fact written per boot or per
  session is real sync churn for a staleness signal nothing yet renders.

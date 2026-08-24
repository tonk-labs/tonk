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

## Wrapped keys

Not a delegation, so not this entity. A sealed space seed is its own
concept, keyed by the DID it derives:

```
CustodiedSeed   this = hash(subject)
                subject    → the space or invite DID
                envelope   → sealed bytes, clearance = account
                generation → which account secret sealed it
```

It sits *beside* the ownership delegation rather than on it: the
delegation says who may act for the space, the seed says how to re-issue
it. They have different lifetimes — rotation replaces the delegation and
re-wraps the seed — so one entity carrying both would make every rotation
a read-modify-write of the ownership record.

Join on the DID when both are needed: `CustodiedSeed.subject` equals the
delegation's `dialog.ucan/subject`.

## Where these live

Profile main, which replicates through the account upstream. Both the
authority and the sealed seed must reach every device on the account —
that is what makes a space usable on a second device and re-issuable
after a lost one.

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
3. **`CustodiedSeed`** — the concept and the seal-on-create path, with
   space keys generated extractable and their seeds sealed under the
   account KEK.
4. **Rotation** consumes all three: re-wrap seeds, re-issue ownership
   delegations, retract the old device link.

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

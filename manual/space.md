# Space

## Creation

### Command

Creation is triggered by asserting a `space/create` command on the profile

```yaml
command!: &space/create
  description: A request to create a new space.
  with:
    name:
      the: dom.event.current-target.elements.name/value
      as: text
    prevent-default:
      ...
```



### Effects

#### Create Repository

Mints a fresh identity and creates the repository with a single local-only
`main` branch. The space is delegated to an **account**: the passkey-derived
root if one is persisted, otherwise this device's onboarding account (a real
account custodied locally rather than by WebAuthn). Records the replica with
`status: blank`.

Runs under the write lock.

Asserts on the space's own entity (`subject.this()`):

| Attribute | Value |
| --- | --- |
| `xyz.tonk.space/subject` | the space entity (same value as `this`) |
| `xyz.tonk.space/kernel` | the seeded kernel, as a source URL with its hash — **not implemented yet** |
| `xyz.tonk.space/founded-at` | unix seconds |
| `xyz.tonk.space/founded-by` | the creating device profile |

`xyz.tonk.space/name` is written separately, by the mount record (see
[Provision Repository](#provision-repository)).

And on the **replica entity** — `hash(profile, subject)`, derived by dialog's
own `Replica` so tonk's and dialog's replica facts co-locate:

| Attribute | Value |
| --- | --- |
| `xyz.tonk.replica/subject` | the space entity — **not implemented yet** |
| `xyz.tonk.replica/profile` | this device profile — **not implemented yet** |

> **Not implemented yet.** `xyz.tonk.replica/subject` is the durable relation
> between a device profile and a space it has replicated, cardinality-one. It
> is what makes "which spaces does this profile hold?" a single query, and it
> replaces `xyz.tonk.space/local`, which is what exists today.
>
> `xyz.tonk.space/kernel` replaces `xyz.tonk.space/status` and
> `xyz.tonk.replica/status`, whose `tonk:blank` / `tonk:initialized` is the
> same question with a one-bit answer. Recording the seeded kernel instead
> says *which* definitions a space is on, so a space seeded from an older
> `core.yaml` / `profile.yaml` is detectable rather than merely "initialized".
>
> The value is a source URL carrying its own hash, e.g.
> `/library/core.yaml#42c2be0fa15977219bcb93b622391356` — provenance and
> fingerprint in one, so the source can be fetched to compare or re-seed.
> Unseeded is the hash of the empty document.
>
> One URL, cardinality-one. A repo seeds one kernel, whatever files that
> kernel is made of.
>
> Today a space keeps whatever definitions it was provisioned with, and later
> kernel changes never reach it. With the hash recorded, a service worker
> update compares it against the shipped kernel and re-seeds where they
> differ. The re-seed and the new hash ride one transaction, so a space is
> never left claiming definitions it does not have.
>
> The hash is per branch in principle; spaces are treated as their `main`
> branch, so it hangs on the space entity. Branch-level tracking can come
> later if it is ever wanted.
>
> `xyz.tonk.replica/kind` goes away entirely. It exists today to tell a real
> space from the system replicas (the profile's own, the account, the ledger),
> but those assert no space row at all — so "is this a real space?" is simply
> "does a space row exist?", and the attribute would only ever hold one value.
>
> `xyz.tonk.replica/profile` is stored alongside it so the query can filter by
> profile. Profile main syncs account-wide, so every device's replica rows land
> on the same branch; the entity hash encodes the profile but is not queryable
> as a filter.

`dialog.replica/subject` and `dialog.replica/profile` sit on this same entity
but are **not stored**: dialog auto-surfaces them per branch, synthesized from
the operator's profile and that branch's subject. So querying the profile
branch for `dialog.replica` returns one row — the profile's own replica — not
one per space. That is why the relation has to be asserted rather than joined
through dialog's facts.

### Locality

> **Not implemented yet.** Today this is `xyz.tonk.space/local`, stamped into
> the profile-main overlay on every mount and at boot.

A space listed in the profile db may or may not be replicated on this device.
The two states are distinguished by the presence of a replica row bearing
`xyz.tonk.replica/subject`; the Hub renders an unreplicated space as a row it can
open on demand.

The fact can go stale — clearing site data drops the storage but not the row —
and there is no event to observe that: IndexedDB notifies only connections the
worker holds, and eviction while the worker is stopped is silent. So the row is
repaired **on load**: enumerate the replicas from the profile db and attempt to
open each. This works uniformly across browsers, unlike `indexedDB.databases()`
(absent in Firefox).

A stale row is cheap: the mount attempt reveals the truth immediately, and the
space is re-replicable from its upstream — the same recovery path as a space
this device never held.

#### Seed Repository

Seeds the kernel — `core.yaml`, `profile.yaml` — into the branch. Runs after
the lock is released, since seeding is the slow part and holding the lock would
stall the page.

Asserts `xyz.tonk.space/kernel` = the seeded kernel's source URL and hash, in
the same transaction as the seed itself, so the two cannot disagree.

> **Not implemented yet.** Today this flips `xyz.tonk.space/status` and
> `xyz.tonk.replica/status` from `tonk:blank` to `tonk:initialized` in a
> separate commit.

#### Provision Repository

Only when the command carried a remote. Adds the space as a consumer on the
account's subscription, presenting the space's delegation to the access
service, then attaches the remote as `main`'s upstream.

Without a remote the space stays local-only.

Asserts on the space entity:

| Attribute | Value |
| --- | --- |
| `xyz.tonk.space/provider` | the providing account; its presence *is* the record that the space is served |
| `xyz.tonk.space/home-address` | the UCAN sync endpoint |

And on **mount anchors** — entities derived per remote and per branch, e.g.
`hash(space entity, "origin")` — the decomposed
`RepositoryConfiguration`: `xyz.tonk.remote/*` (address, subject, name) and `xyz.tonk.branch/*`
(local branch, remote branch, tracking link), so another device profile can
rebuild the config and mount the space identically.

#### Navigate

Posts a `navigate` to the originating client, dropping the creator into the new
space.

## Join

`tonk/join` redeems an invite URL and joins its space.

```yaml
command!: &tonk/join
  this: tonk:join
  description: Redeem an invite URL and join its space.
  with:
    url:
      the: dom.event.detail/href
      as: text
```

The whole invite — including the fragment carrying the secret — rides in
`url`. Progress is reported through an overlay-only `tonk:join/status` entity
(`tonk:pending`, then `tonk:failed` with detail on error); on success the
handler retracts it and asserts the durable replica instead, so the Hub never
shows in-flight or failed joins.

Asserts the same replica row and space entry as [Create](#creation), plus the
mount record, but **not** `xyz.tonk.space/founded-at` / `founded-by`: joining is
not founding, and the absence of those is what distinguishes an invited space
from a created one.

## Enable sync

`space/enable-sync` attaches a sync remote to an existing local-only space.

```yaml
command!: &space/enable-sync
  description: A request to attach a sync remote to the current space.
  with:
    subject:
      the: dom.event.current-target.dataset/subject
      as: entity
    prevent-default:
      ...
```

Runs the same [Provision Repository](#provision-repository) effect as create —
the two share one handler, which is why `space/create` carries no remote field
of its own. Writes `xyz.tonk.space/provider`, `xyz.tonk.space/home-address` and
the mount anchors.

> **Not implemented yet.** Today the command carries `name` (the space's local
> repository name) and `remote` (a sync URL typed into the form).
>
> `remote` goes away: the endpoint is the account's provider, so it is inferred
> rather than supplied. A user-typed URL is also the one path that can attach a
> space to an upstream the account cannot prove for.
>
> `name` becomes `subject`, the space DID — the identity `space/remove`,
> `tonk/rename-repository` and `tonk/pause-sync` all already use. `enable-sync`
> is the only command addressing a space by local name.

## Rename

`tonk/rename-repository` renames a space.

```yaml
command!: &tonk/rename-repository
  description: Rename the repository.
  with:
    subject:
      the: dom.event.current-target.dataset/subject
      as: entity
    name:
      the: dom.event.current-target/value
      as: text
```

A rule binds `subject` to `?this` so the name lands on the space entity
directly. `name` is cardinality-one, so it overwrites in place.

The editable source of truth is `tonk/repository` on the space's own content
branch; `xyz.tonk.space/name` in the profile db is a mirror, so every device can
label a space it has not replicated.

## Toggle sync

`tonk/toggle-sync` toggles auto-sync for one space on one device — the same
command pauses and resumes, flipping the replica's `enabled` preference.

```yaml
command!: &tonk/toggle-sync
  this: tonk:toggle-sync
  description: Toggle auto-sync (pause ⇄ resume) for a space.
  with:
    time:
      the: dom.event/time-stamp
      as: float
    space:
      the: xyz.tonk.toggle-sync/space
      as: entity
    marker:
      the: dom.event.current-target.dataset/toggle-sync
      as: entity
```

The preference is per replica, not per space: pausing on one device does not
pause the others. `time` makes each click a distinct transient so the handler
re-fires — without it a second click would be an identical fact and do nothing.
`marker` keeps the shape distinct from `tonk:invite`, which otherwise matches
the same `{this, time}` shape.

> **Not implemented yet.** Today this is named `tonk/pause-sync`
> (`tonk:pause-sync`, `xyz.tonk.pause-sync/space`), which reads as one-way
> despite being a toggle.

## Removal

`space/remove` removes a space from this device — its list entry and its local
data.

```yaml
command!: &space/remove
  description: A request to remove a space from this device (its list entry and local data).
  with:
    subject:
      the: dom.event.current-target.dataset/remove
      as: entity
    prevent-default:
      ...
```

Only the profile branch may fire it: a matching fact arriving from a content
branch is ignored, since the shape alone is not authority to remove anything.
The self-replica (subject == profile) is refused outright.

### Effects

1. **Retract the replica record** from the profile db. This is the commit
   point; everything after is cleanup.
2. **Evict** the repository from the reactor cache and forget it in the sync
   queue — a leftover dirty stamp would otherwise resurrect it on the next
   drain.
3. **Delete local storage** — the IndexedDB database keyed by routing key.
   Skipped when another device profile on this browser may replicate the same
   space, since storage is shared by routing key alone. Best effort: a failure
   only orphans invisible bytes.

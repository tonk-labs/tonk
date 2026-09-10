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
| `xyz.tonk.replica/status` | `tonk:blank`, until the seed finishes |

> **Not implemented yet.** `xyz.tonk.replica/subject` is the durable relation
> between a device profile and a space it has replicated, cardinality-one. It
> is what makes "which spaces does this profile hold?" a single query, and it
> replaces `xyz.tonk.space/local`, which is what exists today.
>
> `xyz.tonk.space/status` goes away; `xyz.tonk.replica/status` stays. Seeding
> is something a *replica* does, and only the creating device ever sees
> `tonk:blank` — anywhere else the space is either replicated (already seeded,
> since the content it pulls carries the kernel) or remote. Publishing that
> device-local transient onto the space entity meant every other device's Hub
> read a status that was never about it. See [Presence](#presence).
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

### Presence

> **Not implemented yet.** Today this is `xyz.tonk.space/local`, stamped into
> the profile-main overlay on every mount and at boot, alongside a redundant
> `xyz.tonk.space/status`.

A space listed in the profile db is in one of three states on this device, and
the Hub and the FAB render each differently. Both read the profile db, because
they list spaces whose content branch this device may not hold — so none of
this can live on the space's own branch.

| State | How it reads |
| --- | --- |
| **Remote** | no replica row for `(this profile, space)` |
| **Seeding** | a replica row with `xyz.tonk.replica/status` = `tonk:blank` |
| **Replicated** | a replica row without it |

Seeding is only ever observed on the device that created the space: the seed
runs after the create lock is released, so `tonk:blank` is a real transient
there. Any other device that holds the space pulled its content, and that
content already carries the kernel.

The replica row can go stale — clearing site data drops the storage but not
the row — and there is no event to observe that: IndexedDB notifies only
connections the worker holds, and eviction while the worker is stopped is
silent. So the row is repaired **on load**: enumerate the replicas from the
profile db and attempt to open each. This works uniformly across browsers,
unlike `indexedDB.databases()` (absent in Firefox).

A stale row is cheap: the mount attempt reveals the truth immediately, and the
space is re-replicable from its upstream — the same recovery path as a space
this device never held.

#### Seed Repository

Seeds the kernel into the branch. Runs after the lock is released, since
seeding is the slow part and holding the lock would stall the page.

Flips `xyz.tonk.replica/status` to `tonk:initialized` and records the kernel
version and its components on the space's own content branch, in the same
transaction as the seed, so the two cannot disagree. See [Kernel](#kernel).

> **Not implemented yet.** Today the flip is a separate commit, also stamps
> the redundant `xyz.tonk.space/status`, and records nothing about which
> kernel was seeded.

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

## Kernel

> **Not implemented yet**, except for `xyz.tonk.kernel/route`, which the router
> already reads.

The kernel is the library of concepts, views, rules and routes a space is
seeded with — `core.yaml` and the files it pulls in. A space keeps whatever
kernel it was created with, which is why a redesign shipped in a new bundle
does not reach spaces created before it.

### Where it lives

On the **space's own content branch**, not in the profile db.

The kernel is seeded onto that branch, so it syncs: when one member upgrades,
the space's contents change for everyone. A copy in the profile db would name
the version *this device last saw*, and would be wrong for every other device
the moment anyone upgraded. On the content branch there is one kernel per
space because there is one content branch, and every member converges on it.

That also means an upgrade is not a per-device choice. Whoever upgrades
upgrades the space, and the others pull it — which is the right semantics for
a shared space, and unavoidable given one branch.

### What is recorded

On the kernel version's entity:

| Attribute | Value |
| --- | --- |
| `xyz.tonk.kernel/concept` | a concept this version declared |
| `xyz.tonk.kernel/view` | a view this version declared |
| `xyz.tonk.kernel/rule` | a rule this version installed |
| `xyz.tonk.kernel/route` | a route this version seeded |

Each is cardinality-many, valued by the component's entity. The entity itself
is the kernel's source URL carrying its content hash, e.g.
`/library/core.yaml#42c2be0f…` — provenance and fingerprint in one, so the
source can be re-fetched to compare or re-seed, and a custom kernel is just a
different URL.

The kinds are split rather than folded into one `component` because retraction
differs by kind — a `view!:` retracts from a pin, a `concept!:` needs the
concept already on the branch, a `rule!:` wants its effect URI — and the order
matters, since a concept retracted before the views naming it would dangle.
Separate attributes carry the kind in the data, so an upgrade never re-derives
it.

The facts are generated from the kernel source at seed time rather than
written beside each definition: which components came from the kernel is
something the seeder knows and the library file cannot state about itself.

`xyz.tonk.kernel/route` doubles as the router's precedence signal — see
[Routes](#routes).

### Upgrading

One commit: retract the previous version's components, assert the new
version's, in a single transaction.

Retraction reads the previous kernel's component facts and emits a pinned
retraction per component. It must **only** retract what that version
*asserted*. A kernel version's history contains both its own assertions and
the retractions of the version before it; replaying all of them inverted would
restore the version-before-last. This filter is load-bearing, not an
optimization.

Because retraction targets exact `(entity, attribute, value)` triples and
retracting an absent fact is a no-op, a component the user has since replaced
is left alone rather than clobbered.

### When it upgrades

Lazily, on mount — an unopened space costs nothing, and a space can never run
a kernel newer than the worker that mounts it.

Whether the upgrade is offered or applied depends on the kernel:

- **Breaking** — the new bundle cannot render the old kernel's definitions.
  Applied without asking, because declining does not mean "keep the old
  design", it means a broken space. The view-system rollout was this: spaces
  stayed broken until their views were redefined.
- **Additive** — a new route, a revised view. Offered in a toast and left to
  the user, since swapping a working space's definitions underneath them is
  not something to do silently.

Which one a version is is authored on the kernel, not inferred: the release
knows what it shipped.

A declined additive upgrade leaves a space on an older kernel indefinitely, so
"this space names a handler this bundle no longer has" is a real state rather
than a transient one, and should degrade visibly rather than mysteriously.

### Caching

The shipped kernel is cached on service-worker install, so mounting a space
neither waits on the network nor fails when it is unavailable. Custom kernels
cannot be known at install time; each is cached on first fetch, keyed by URL,
so subsequent mounts are equally offline-safe.

### Custom kernels

A custom kernel is a kernel URL that is not the shipped one, so it needs no
separate mechanism — the recorded version simply points elsewhere, and the
mount-time comparison asks whether that URL still serves what the space holds
rather than whether it matches the bundle.

**Open:** a custom kernel can name command handlers the running bundle does
not implement. Whether that is rejected at install or left to surface as an
unresolved command is undecided.

## Routes

A route is matched by specificity first — static beats param beats catch-all.
Ties beneath that are settled by origin: a route the space authored beats one
the kernel seeded, so a space can define its own `/` and have it win.

The router tells them apart by `xyz.tonk.kernel/route`. A route named there
came from the kernel; one that is not was authored in the space. Within a
group the order is by entity URI, so every device builds the same table.

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

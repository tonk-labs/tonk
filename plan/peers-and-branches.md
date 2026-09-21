# Peers, branches, and where accounts come from

Supersedes the account-shaped parts of `accounts-as-branches.md`. That
plan was right that accounts should be branches; it modelled the account
itself as something tonk stores. This one derives it.

## What is wrong now

Two problems that turn out to be one.

**A profile holds at most one account and nothing in the data says so.**
Linking writes a replica of the account repository and repoints the
profile repo's single `origin` remote. Signing in again repoints the
same cell, and nothing retracts what the previous link wrote, so the
branch accumulates replica rows with no fact marking the current one.
Asking "which account is this profile signed in as" answers with every
account the device has ever seen.

The real answer lives outside the database: `set_active` writes the
active profile's NAME as raw bytes to a credential site. Not a fact, not
queryable, no schema — so no view can read it, which is why the hub's
account cell has nothing to bind to.

**Tonk restates what dialog already models.** `xyz.tonk.remote/*` and
`xyz.tonk.branch/*` duplicate dialog's replica and branch vocabulary
with different names and extra fields. An audit found ZERO library
consumers for either: they are written by the worker and read only by
the worker. They were never a declarative model, which is why their
shape reads badly the moment a view tries to use it — `origin` on a
remote means the owning replica, not the remote named "origin".

## The model

A **peer** is anything that holds replicas. This device is one; the
service that serves a repository is another. Nothing about the service
is special, which is what lets both ends of a tracking relationship be
described in the same terms.

```
Replica(local-peer,   repo)   my copy
Replica(service-peer, repo)   the service's copy
```

A remote branch is then an ordinary branch on an ordinary replica —
just one held by a different peer. `Remote` as a concept disappears,
and with it the `origin`/`subject` confusion: those fields existed to
say "which side is which" for a thing that was never really a side.

Tracking becomes one attribute, entity to entity:

```
Branch(my-replica, "main")  --upstream-->  Branch(service-replica, "main")
```

Everything else is a traversal rather than a stored field that can
disagree:

```
upstream -> branch/replica -> replica/subject   which repository
                           -> replica/profile   which peer
                           -> peer/address      where to reach it
```

### Addresses

An address is a serialized dialog `SiteAddress` (dag-cbor bytes) — the
typed payload the capability layer dials. Not a URI: the typed form
already carries what a scheme would say, and a URI would be a lossy
re-encoding something has to parse back.

Zero or more per peer, because reachability belongs to the participant
rather than to any one connection. Today the same service address is
stored again on every account remote; here it is stored once per peer.

A LOCAL peer has an address too, in a local variant: `idb:` in a
browser, `file:///` natively. `NetworkAddress` is already a composite
enum with a variant per site kind, so this extends a shape built for it
— and it removes the special case where "here" is the one peer with no
address.

### What meta records

`meta` never replicates. It is this device's bookkeeping about a
repository: which branches exist, what they track, which is active.

| Concept | Attributes | Owner |
| --- | --- | --- |
| peer | `tonk.dialog.peer/address` | tonk |
| replica | `dialog.replica/{subject,profile}` | dialog |
| branch | `dialog.branch/{name,replica,revision}` | dialog |
| tracking | `tonk.dialog.branch/upstream` | tonk |
| active | `tonk.dialog.replica/active-branch` | tonk |

Three tonk attributes, each filling a checked gap: dialog holds
upstream and address in CELLS rather than facts, and models where a
branch IS (`dialog.branch/revision`) but not which one you are ON.

`tonk.dialog.*` says these extend dialog's model and are tonk's only
until dialog adopts them; migrating is then a rename.

Described in `rust/tonk-core/assets/library/meta.yaml`, which lowers in
the test suite so it cannot drift from the rows it describes.

### Accounts are inference, not storage

`meta` knows nothing about accounts. On the PROFILE repository, a branch
whose upstream is a replica held by another peer means that peer serves
an account and this branch is signed in as it.

```
active branch -> upstream -> branch/replica -> replica/subject = the account
```

Signed out is the active branch having no upstream — `main`, exactly
like a local-only git branch. No null value, no account-shaped absence.

This is what the earlier plan got wrong: it proposed storing the
account, first on the replica, then on the profile. Both were a second
home for something the graph already answers.

## Shape on disk

```
profile repo
├── meta                  never replicates
├── main                  local workspace, no upstream
├── account/<did-A>       upstream on peer A
└── account/<did-B>       upstream on peer B
```

Signing in creates or switches to that account's branch. Signing out
points active at `main`. No branch is created empty and none is
deleted, so a signed-out profile keeps its local spaces — which is what
the rootless-workspace path does today by rotating profiles.

## Work

1. `meta` records the active branch, and something reads it. The branch
   enumeration is already written by `ensure_profile_meta_branch`.
2. Peers and addresses as facts: one peer row per participant, the
   address moved off the per-repository remote.
3. Tracking as a fact alongside dialog's upstream cell, so a rule can
   traverse it.
4. Linking creates `account/<did>` with its tracking row and points
   active at it. Signing out points active at `main`.
5. Switching accounts switches branches instead of rebuilding
   `TonkState`. This is the piece that touches worker core state.
6. Retire `xyz.tonk.remote/*` and `xyz.tonk.branch/*` once nothing
   reads them. The audit says nothing declarative does today.

Migration is out of scope: existing profiles keep working as they are.

## Open questions

1. **Branch naming.** `account/<did>` is the obvious form; confirm
   nothing rejects `:` in a branch name.
2. **Where spaces live.** Today spaces are recorded per profile. Under
   branches, do spaces created while signed in belong to the account's
   branch and sync with it, or to `main` and stay device-local? This
   decides whether signing out hides your spaces.
3. **`Replica.kind`.** Today `tonk:account` marks the account replica.
   Under this model that may be inferable — a replica whose peer serves
   an account — rather than stamped. Check before carrying it forward.
4. **`dialog.replica/profile` holds a peer.** Dialog's name says
   profile; the concept is a peer, and the local peer is a profile only
   because that is the role it plays here. Worth raising with dialog
   rather than shadowing locally.

## Relationship to dialog's state-layers work

`dialog-db/compare/main...claude/dialog-db-state-layers-7p905m` lists
"Tonk migration" as work item 4, with increments 1-3 done, and names
tonk's overlay and transient mechanisms as things that design
supersedes. Its scopes and stacks decide WHERE facts live — which layer
is local to a connection and which replicates.

That is a different axis from this plan, which decides what the facts
ARE. They should compose: a peer, a branch and an upstream are durable
facts on meta whatever layer machinery reads them. Worth confirming
before step 5, since binding a branch per connection is close to what a
stack does.

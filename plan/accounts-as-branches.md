# Accounts as branches of one profile repository

## The problem

A profile holds at most one account, and nothing in the data says so.

Linking writes a replica of the hidden account repository and repoints
the profile repo's single `origin` remote. Signing in as a different
account repoints that same cell. Nothing retracts what the previous link
wrote, so the branch accumulates replica rows — one per account ever
linked — with no fact distinguishing the current one.

Querying "which account is this profile signed in as" today returns
every account the device has ever seen:

```
profile/account: { this: z6Mkto3z…, account: z6Mkm9HP… }
profile/account: { this: z6MkuP4r…, account: z6Mkm9HP… }
profile/account: { this: z6MkgkYR…, account: z6Mkm9HP… }
```

The answer exists only outside the database: `set_active` writes the
active profile's NAME as raw bytes to a credential site.

```rust
registry.credential().site(ACTIVE_PROFILE_SITE)
    .save(name.as_bytes().to_vec())
```

Not a fact, not queryable, no schema. It is there because of a real
constraint — you cannot store "which profile is active" inside a
profile, since you would need to know which one to open first — but the
cost is that no view can answer the question, and the hub's account cell
has no fact to read.

## The shape

One profile repository. Accounts are branches of it.

```
profile repo
├── meta                  device-local, never replicates
│     ├── Branch rows          which branches exist
│     ├── Remote rows          where each one syncs
│     ├── TrackingBranch rows  what each one tracks
│     └── active-branch        which one is current
├── main                  local workspace, no upstream
├── account/<did-A>       upstream = account A
└── account/<did-B>       upstream = account B
```

`meta` is git's config and `HEAD` together, and knows nothing about
accounts. It records branches, their remotes, their upstreams, and which
is active. The account is the `subject` of the active branch's upstream
remote — the same traversal git uses for `@{upstream}`.

The chicken-and-egg that justified the credential hack disappears: there
is one repository, so you open it unconditionally and read `meta`.

### Why this is not the current design

Today per-account isolation comes from rotating PROFILES: each account
gets its own profile, its own storage pool, its own key. Switching
accounts rebuilds `TonkState` wholesale, which is why `promote` is what
it is. Under branches, the pool and the key are shared and switching is
changing which branch the reactor reads.

## The active-branch fact

| | |
|---|---|
| Entity | the self-replica — `Replica::new(profile, profile)` |
| Attribute | `tonk.dialog.replica/active-branch`, cardinality one |
| Value | the branch entity — `Branch::new(replica, name).this` |

Everything else reuses dialog's vocabulary rather than restating it:

| Question | Attribute | Owner |
| --- | --- | --- |
| which branches exist | `dialog.branch/name`, `dialog.branch/replica` | dialog |
| where is a branch now | `dialog.branch/revision`, `/tree`, `/edition` | dialog |
| what does it track | `TrackingBranch` upstream | tonk, exists |
| **which is active** | **`tonk.dialog.replica/active-branch`** | new |

The namespace is deliberate. `tonk.dialog.replica/*` says this extends
dialog's `replica` concept and is tonk's only until dialog adopts it;
migrating is then a rename with entity and value unchanged.

Not named `head`: `dialog.branch/revision` is already "where is this
branch now", which is git's lowercase head. `active` is Mercurial's term
for the same idea and carries no revision-pointer baggage.

Cardinality-one is the point. A second assert supersedes, so "one
account per profile" stops being an invariant the worker maintains and
becomes a property of the data.

## Resolving the account

```
self-replica --active-branch--> Branch --upstream--> Remote --subject--> account DID
```

One row per hop, no scanning, unambiguous. Signed out is the active
branch having no upstream — `main`, exactly like a local-only git
branch. No null value and no account-shaped absence to model.

## Open questions

1. **Branch naming.** Must be derivable from the account DID so a guest
   resolves it without a lookup. `account/<did>` is the obvious form;
   confirm nothing rejects `:` in a branch name.
2. **Where spaces live.** Today spaces are recorded per profile. Under
   branches, do spaces created while signed in belong to the account's
   branch (and sync with it) or to `main` (device-local regardless)?
   This decides whether signing out hides your spaces.
3. **Sync drain.** It resolves upstreams per branch already; whether a
   second branch with its own upstream on the SAME repository needs
   anything is unverified.

## Work

1. `meta` records the active branch. The enumeration is already written
   by `ensure_profile_meta_branch`; this adds the fact and a reader.
2. Linking creates `account/<did>` with its remote and tracking rows,
   the same rows `record_space_mount` writes for a space, and points
   active-branch at it.
3. Signing out points active-branch at `main`. No branch is created and
   none is emptied — `main` is where an unlinked profile already was.
4. Switching accounts switches branches instead of rebuilding
   `TonkState`. This is the piece that touches worker core state.
5. `/api/profiles` and the switcher read the enumeration rather than
   opening every profile's storage.

Migration is out of scope: existing profiles keep working as they are.

## What this unblocks

- The hub's account cell reads a fact instead of a derivation that
  cannot be written. Three attempts failed on this in one session.
- "Would two accounts both show" is answerable rather than assumed.
- Subscription migration on account switch becomes tractable: same
  repository, same pool, different branch — no reload needed.

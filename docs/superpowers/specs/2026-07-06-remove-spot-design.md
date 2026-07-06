# Remove a spot from the Hub

## Problem

The Hub launcher at `/` lists every spot (space) the profile has joined or
created, but offers no way to remove one. Spots accumulate forever; the only
recourse is clearing the browser's storage wholesale.

## Semantics

Removing a spot:

1. Retracts its replica record from the profile repository's meta branch.
   This is the commit point — the record is the row's source of truth, so
   the card disappears from the Hub immediately.
2. Deletes the space's local storage, best-effort. A failure here is logged,
   never surfaced: the spot is already gone from the list, and orphaned
   storage is invisible to the user.

Removal is device-local. A synced spot can be rejoined via an invite link;
a local-only spot is gone for good. The confirm dialog states this plainly.

## UI (profile.yaml)

Each directory row gets a hover-revealed `×` remove affordance, script-free,
using the same CSS state tricks as the create wizard:

- One radio group `__rm` spans the list: a checked-by-default `rm-closed`
  radio plus one radio per row with id `rm-{subject}`. At most one confirm
  overlay is open at a time, and opening another closes the first.
- The `×` is a `<label for="rm-{subject}">` rendered as a sibling of the
  row's `<a href>` card link (inside the row container, outside the anchor),
  so hovering or clicking it never navigates.
- The confirm overlay (shown while its row's radio is checked) names the
  spot, explains "Removes this spot and its local data from this device.",
  and offers:
  - cancel — a `<label for="rm-closed">`,
  - delete — the submit button of
    `<form onsubmit=space/remove data-remove={subject}
    data-close-radio="rm-closed">`. `data-close-radio` re-checks `rm-closed`
    on submit (existing tonk-display delegate behavior), closing the overlay.

New transient command in `profile.yaml`:

```yaml
command!: &space/remove
  description: A request to remove a space from this profile and delete its local data.
  with:
    subject:
      description: The space's subject DID, read from the form's data-remove.
      the: dom.event.current-target.dataset/remove
      as: entity
    prevent-default:
      the: dom.event.do/prevent-default
```

The `subject` value is a did:key — it carries `:`, so the event layer
delivers it as an `Entity` (same as the invite/pause-sync markers). The
attribute is deliberately `dataset/remove`, NOT `dataset/subject`: the
`tonk/rename-repository` transient (core.yaml) already carries
`dataset/subject`, and command decode ignores extra facts, so a remove
command matched on `subject` alone would also decode every rename —
deleting the space being renamed. The distinctly named attribute is read
by no other command, so it doubles as the command's unique shape; no
separate marker attribute is needed.

## Schema (tonk-schema)

- New attribute `domain::command::remove::Remove(pub Entity)` on domain
  `dom.event.current-target.dataset` (derived attribute
  `dom.event.current-target.dataset/remove`).
- New command struct `RemoveSpace { this, subject }` implementing
  `Command`, alongside `CreateSpace`/`PauseSync` — matching the fields the
  yaml command actually declares.

## Worker (tonk-worker)

A `RemoveSpaceHandler` registered in the command registry beside
`CreateSpaceHandler`, wasm-gated the same way. On a decoded command it:

1. **Retracts the replica record.** Re-derives the replica entity via
   `Replica::new(profile_did, subject)`, selects every claim `of` that
   entity on the profile meta branch, and retracts them all in one
   transaction through the reactor's profile-repository handle (the cached
   handle every Hub read goes through). The claim sweep covers every stamp
   regardless of vintage — `Replica` fields, `SpaceStatus`, a migration's
   `SpaceKind`, a legacy `name` — without knowing current values. (Sync
   stamps live elsewhere: `ReplicaSyncEnabled` on the space's own content
   branch, which dies with the storage; `ReplicaSyncStatus` is overlay-only
   on a singleton.) Commits, runs scheduled polls, and broadcasts
   `/api/profile` — mirroring `record_replica_in_profile` in reverse.
2. **Detaches the live system.** Evicts the repository from the reactor's
   cache (a new `Reactor::evict(name)`, the per-repo analog of `shutdown`:
   removes the cache entry and clears each branch's subscribers). This is
   the step that actually stops syncing — the background sweep builds its
   repo set from the reactor cache plus the dirty queue, not from replica
   records — and it ends the removed row's SSE streams.
3. **Deletes local storage, best-effort.** A repository space maps to
   `Location { directory: Current, name: <routing key> }`, which on the web
   is the IndexedDB database named exactly the routing key plus the OPFS
   blob directory `current/<key>`. An inline-JS helper deletes both
   (`indexedDB.deleteDatabase` + `removeEntry(key, { recursive: true })`);
   the existing `patch_idb_versionchange` makes the worker's own pooled
   connection close itself so the delete completes. Errors are logged and
   swallowed. The handler is wasm-gated, so native builds carry none of it.

The Hub form commits on the profile meta branch, so the handler reads
identity from state (like `CreateSpaceHandler`), not from the command's
origin.

## Testing

- Command decode test in the reactor/schema layer, mirroring
  `it_decodes_create_space_from_name_only_facts`: facts carrying
  `dataset/subject` decode as `RemoveSpace` and nothing else.
- Handler-level native test: record a replica in a profile meta branch,
  run removal, assert the replica query returns nothing (and stamps are
  gone).
- Storage deletion is wasm-only; verified manually in the browser.
- Lint gate: `cargo clippy --all -- -D warnings` (native), plus the wasm
  build.

## Out of scope

- No boot-time GC sweep for orphaned storage.
- No HTTP DELETE route.
- No undo; recovery for synced spots is the existing invite/join flow.
- Server-side data on a sync remote is untouched.

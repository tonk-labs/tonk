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
    `<form onsubmit=space/remove data-subject={subject}
    data-close-radio="rm-closed">`. `data-close-radio` re-checks `rm-closed`
    on submit (existing tonk-display delegate behavior), closing the overlay.

New transient command in `profile.yaml`:

```yaml
command!: &space/remove
  description: A request to remove a space from this profile and delete its local data.
  with:
    subject:
      description: The space's subject DID, read from the form's data-subject.
      the: dom.event.current-target.dataset/subject
      as: entity
    prevent-default:
      the: dom.event.do/prevent-default
```

The `subject` value is a did:key — it carries `:`, so the event layer
delivers it as an `Entity` (same as the invite/pause-sync markers). No other
command reads `dataset/subject`, so the field doubles as the command's
distinct shape; no separate marker attribute is needed.

## Schema (tonk-schema)

- New attribute `domain::command::remove::Subject(pub Entity)` on domain
  `dom.event.current-target.dataset` (derived attribute
  `dom.event.current-target.dataset/subject`).
- New command struct `RemoveSpace { this, subject }` implementing
  `Command`, alongside `CreateSpace`/`PauseSync` — matching the fields the
  yaml command actually declares.

## Worker (tonk-worker)

A `RemoveSpaceHandler` registered in the command registry beside
`CreateSpaceHandler`, wasm-gated the same way. On a decoded command it:

1. **Retracts the replica record.** Re-derives the replica entity via
   `Replica::new(profile_did, subject)` — no read needed — and, in one
   transaction through the reactor's profile-repository handle (the cached
   handle every Hub read goes through), retracts the `Replica` instance and
   every stamp the write paths assert on that entity: `SpaceStatus`,
   `SpaceKind`, `ReplicaSyncEnabled`, `ReplicaSyncStatus`, branch records
   (`Replica::branch(..)`), and the legacy `Name` where present. Commits,
   runs scheduled polls, and broadcasts `/api/profile` — mirroring
   `record_replica_in_profile` in reverse.
2. **Detaches the live system.** Drops the reactor's cached repository and
   branch handles for the subject. The background sync loop reads replica
   records to decide what to sync, so it skips the space from the next tick
   with no extra bookkeeping.
3. **Deletes local storage, best-effort.** Removes the space's IndexedDB
   database(s) and its OPFS blob subtree, deriving names/paths the same way
   the storage loader derives the space `Location` from
   `(profile_did, local_name)`. Errors are logged and swallowed. Native
   builds no-op, exactly like `spawn_seed`.

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

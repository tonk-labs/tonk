# Root-owned account state repository

2026-07-27, revised 2026-07-28

## Problem

After creating an account and linking several devices, each device shows a
different profile name, and renaming on one device does not update the others.
This is not a sync-convergence failure. Two design gaps cause it:

1. The name each device displays (`ProfileName`,
   `xyz.tonk.profile/display-name`) lives on that device's local profile branch
   and never syncs. Bootstrap stamps `petname(device_did)` — a different DID,
   hence a different petname, per device. Nothing in the account link flow
   carries durable account state across devices.
2. The rename effect only pushes outward (local `ProfileName` + root-keyed
   `MemberName` restamps on space rosters), and `record_claim_on_content`
   unconditionally asserts
   `MemberName::new(root_membership, local_display_name)`. A later join from a
   linked device can therefore overwrite the chosen roster name with that
   device's petname. `MemberRole` and `InvitedVia` already have sequential
   first-wins guards; `MemberName` does not.

The display name is the only account-wide fact required today. The design
should give it a durable, shared home without making the account service its
authority, while leaving a narrow seam for another typed account fact if one is
later justified.

## Goals

- Keep authoritative account-wide state in Tonk repositories and facts, not
  Tonk-operated D1/R2 tables.
- Give every linked device the same repository identity and one portable,
  authenticated way to locate it.
- Preserve local operation when remotes or Tonk services are unavailable.
- Reuse the existing repository, UCAN, sync, and query systems.
- Keep the repository and lifecycle usable by a later typed account fact
  without building a generic settings or projection framework now.
- Let another compatible provider host the repository without becoming an
  identity authority or coordinating its database with Tonk's.

## Non-goals

- Replacing account creation, new-device linking, or device revocation in this
  change. Already-linked devices can become independent of the account service
  for account-state sync; enrolling a new device still uses the account
  ceremony until that ceremony is separately made portable.
- Treating client-reported storage, bandwidth, billing, or entitlement usage as
  authoritative. A provider must meter the resources it supplies. The account
  repository may hold a user-facing projection, not the enforcement record.
- Storing root keys, passkey material, email addresses, recovery secrets, or
  other credentials. Repository authorization is not a confidentiality
  guarantee.
- Solving multi-remote replication, provider migration, or descriptor updates
  in version 1.

## Rejected alternatives

- **Account-service row as authority.** Simple to query, but account state stops
  converging when that deployment disappears and another provider must either
  trust or copy Tonk's database.
- **Space rosters as authority.** An account with no spaces has no name, devices
  may know different spaces, and disagreement between rosters has no canonical
  winner. Rosters remain projections for space peers.
- **Derive the remote from the current origin.** The root DID derives a stable
  repository identity, not a network location. Different providers would
  create isolated histories under the same subject.
- **Initialize whenever pull fails.** Remote absence, outage, authorization
  failure, and an unreadable response are not equivalent. Treating them alike
  creates forks during ordinary failures.
- **One-shot provisioning capability.** If the remote provides atomic
  create-if-absent, every active device can safely retry against the one
  established subject and remote. A separate capability adds state that can be
  lost without narrowing who may write the eventual repository.
- **One generic JSON settings document.** It centralizes unrelated merge,
  migration, privacy, and size policies. Typed facts let each feature define
  those properties explicitly.

## Invariants

1. The account repository subject is an immutable account subject. In version
   1 it is the genesis root DID; rotating signing authority does not move the
   repository.
2. Version 1 establishes exactly one immutable, root-signed repository
   descriptor containing exactly one remote.
3. A device never selects a new remote merely because the configured remote is
   unavailable.
4. A device may create the initial remote history only through atomic
   create-if-absent at the established remote. Failure to pull never authorizes
   an ordinary local initialization.
5. A mounted local replica is not writable account state until it has acquired
   a trusted base from a successful pull or successful create-if-absent.
6. The account repository is a system replica, never a user space.
7. `AccountDisplayName` is globally authoritative in version 1. The device
   profile branch and space rosters are projections; per-space aliases are not
   supported.
8. Projection checks every target independently and is retried after boot,
   account-state writes, and successful pulls.

## Design

### 1. Join-time name guard (standalone first PR)

`record_claim_on_content` stamps `MemberName` only when the membership entity
has no name row. A never-named joiner still gets named; a later, sequential join
does not clobber an existing rename.

This is intentionally described as a guard rather than strict first-wins.
The read and write are not one atomic remote transaction: two devices can both
observe an absent row and assert concurrently. The account-state projection is
the eventual repair mechanism.

This fix is correct under any account-state propagation design and lands alone.

### 2. Account repository identity

The account repository has:

- **Subject:** the immutable account subject. Version 1 uses the root DID that
  originally created the account.
- **Routing key:** derived from the account subject by the normal `repo_key`
  convention.
- **Authorization:** the existing root→device→operator chain. The local
  repository credential is verifier-only, as for joined/restored spaces; the
  operator's chain supplies mutation authority.
- **Branch:** `main`.
- **Contents:** `AccountDisplayName` only in version 1. No standard-library seed
  or space roster is required.

The stable subject solves identity, not discovery. A `did:key` does not encode a
network location. The repository therefore also needs an authenticated
locator.

The account roadmap permits root-key rotation. Rotation changes signing
authority and the device delegation chain, not the account subject or repository
routing key. Version 1 starts with `account subject == root DID`; a future
succession ceremony must preserve authorization to that subject.

### 3. Root-signed account repository descriptor

`AccountRepositoryDescriptorV1` is a durable, root-signed artifact carried by
every account link handoff and stored locally beside the root→device
delegation. It contains:

```text
version = 1
account subject
one canonical remote address
root signature envelope
```

The passkey ceremony already derives the root signer to mint a root→device
delegation. Account creation, or the one-time establishment ceremony for an
existing account, signs the descriptor while that signer is available.

The descriptor is not the current five-minute account-service invocation. It is
a non-expiring, canonically encoded signed artifact with its own validation
contract. Its content hash identifies the exact descriptor locally.

The descriptor separates authenticity from coordination:

- the account service stores and relays the exact signed bytes but cannot alter
  them;
- a device verifies those bytes without trusting the service for their
  contents;
- the service is still a version 1 coordination dependency for remembering
  which signed descriptor was established and returning it to new devices;
- an export or direct device-to-device handoff may carry the same bytes later,
  but neither is implemented in version 1.

For a new account, the descriptor is stored atomically with account creation.
For an existing account, the service accepts it only while no descriptor is
established. If two root ceremonies submit different valid candidates, one
set-if-absent operation wins and every device adopts those exact bytes. The
establishment response returns the stored winner; a caller never persists its
candidate before the service has established it.

The current deployment's `<tonk-default-remote>` result proposes the single
remote during creation or establishment. It is never a discovery rule for a
device linking to an existing account. Browser and CLI handoffs both return the
established descriptor with the new device delegation.

The descriptor is immutable in version 1. It has no generation, endpoint list,
failover, or movement semantics. Moving the repository requires a future
descriptor version and a separately specified data-transfer protocol.

### 4. System replica

The mounted account repository is stamped with a distinct replica kind:
`tonk:account`.

It must be:

- hidden from the Hub and FAB space switcher;
- excluded from `profile_space_keys`, roster migration, space removal,
  templates, invitations, and space restore enumeration;
- ineligible for user pause or removal through space controls;
- opened on every linked-device boot and included in the sync population;
- synced without requiring a page to render it.

The local replica records the descriptor content hash for which it has acquired
a trusted base. Merely creating or mounting the local replica does not set that
marker.

Space enumeration selects `kind == tonk:repository`; it no longer treats every
non-profile replica as a space.

### 5. Lifecycle

`ensure_account_state` runs after a link is persisted and during linked-device
boot. Account configuration, local hydration, and current remote availability
are separate concerns:

- **Unconfigured:** no valid local descriptor.
- **Unhydrated:** the descriptor is valid, but this local replica has never
  acquired the established remote history.
- **Ready:** the local replica has a trusted base for the current descriptor.

Remote reachability is transient status, not another durable lifecycle state.
A ready replica remains ready while offline.

The lifecycle is:

1. Load and verify the local account link and
   `AccountRepositoryDescriptorV1`.
2. Derive the account repository subject and routing key.
3. Mount or load the local verifier-only `tonk:account` replica.
4. Attach the descriptor's remote as `origin`; `main` tracks
   `origin/main`.
5. If the local trusted-base marker matches the descriptor hash, the repository
   is ready and may accept offline writes. Attempt normal pull/sync without
   clearing readiness on availability failure.
6. Otherwise pull before permitting any account-state write.
7. A successful pull of the established history records the trusted-base
   marker and runs convergence.
8. If the remote explicitly reports that the repository is absent, attempt an
   atomic create-if-absent of an empty root-owned repository with its `main`
   branch:
   - the winner records the trusted-base marker;
   - a loser pulls the winning history and then records the marker.
9. Timeout, offline, 401/403, 5xx, malformed responses, and unknown errors leave
   the replica unhydrated. They neither record readiness nor create local
   account history.

Every active linked device may retry step 8. A separate, one-shot provisioning
capability is unnecessary if the remote supplies the required compare-and-set:
all devices use the same established subject and remote, and already possess
account write authority. If the live remote cannot provide that primitive,
implementation stops and revisits this decision rather than weakening the
no-fork invariant.

The account-service transaction and remote repository creation cannot be
atomic across systems. Safety comes from durable descriptor establishment plus
idempotent remote creation, so a browser failure after the service accepts a
ceremony cannot strand ephemeral provisioning authority.

#### Existing-account migration

Existing linked accounts have no descriptor and may have several devices with
different local petnames. Boot cannot safely elect whichever device happens to
run first: simultaneous devices and an unavailable remote are indistinguishable
from an empty account.

Migration therefore has a distinct, one-time root/passkey ceremony that
signs and establishes one descriptor through service set-if-absent. The device
running that ceremony chooses its current `ProfileName` as the initial
`AccountDisplayName`; after the repository becomes ready, it writes that fact as
an ordinary account-state mutation. Other devices do not seed a name merely
because the fact is absent.

If local persistence fails after the service accepts the descriptor, a later
login receives the same stored bytes and any active linked device can retry
repository creation. If the establishing device fails before writing the
initial name, the account remains valid but unnamed until the next explicit
rename; boot does not guess among device-local names.

New-account creation uses the same initial-name rule after its repository
becomes ready. The account service's `device_name` is a label for the device,
not the user's account display name, and is never used as the seed.

### 6. Account fact schema

Version 1 introduces a separate authoritative fact:

```text
AccountDisplayName
  this: immutable account subject
  name: text, cardinality one
```

`ProfileName` remains device-local projection state keyed by the device profile
DID because the existing FAB reads it from `main@profile:tonk`. Keeping a
separate account fact avoids redefining the current profile-local schema.

No other account facts or generic account-state registry are introduced in
version 1. A later fact earns inclusion only if it is small, bounded,
user-owned, safe for the repository provider to inspect, and genuinely
account-wide. It must:

- use named, typed concepts rather than arbitrary string keys or one JSON blob;
- define cardinality and concurrent merge behavior;
- define projection and migration behavior;
- state whether the existing active-device writer set is appropriate;
- receive a separate confidentiality review.

Device-local settings, provider-metered usage, billing, entitlements, secrets,
large blobs, and unbounded event logs do not belong here by default.

### 7. Convergence and projections

The worker owns an explicit `converge_account_state` hook. Reactor subscription
re-polling may deliver UI frames, but it is not relied on to mutate another
branch and it does not survive worker replacement.

The hook runs:

- after an account repository first becomes ready;
- after a local account-state commit;
- after a successful background pull of the account repository;
- during linked-device boot.

Version 1 calls `adopt_account_display_name` directly. It does not introduce a
projector registry or plugin abstraction.

Adoption:

1. Read `AccountDisplayName` from `main` of the account repository.
2. If it differs from the device-local `ProfileName`, write the local cache.
3. Inspect `MemberName` independently on every real space in this device's
   replica index. Commit only targets whose root-keyed value is stale or which
   still contain the obsolete device-keyed row.
4. Refresh each stale real space's self-identity overlay.

Convergence never short-circuits merely because the local `ProfileName`
already matches. Another space may be stale or a previous per-space write may
have failed. Repeated convergence performs no durable writes to targets that
are already correct, while failures remain eligible for the next boot or
successful account sync even when the account fact itself has not changed.

### 8. Rename flow

For an unlinked device, rename remains local exactly as today.

For a linked device with a ready account repository:

1. Assert `AccountDisplayName(account_subject, name)` on the local account
   branch.
2. Run `converge_account_state`, which updates the local cache, real-space
   rosters, and overlays.
3. Queue the account repository and changed spaces for sync.

The local commit does not wait for the remote. If it is unavailable, the
account branch remains ahead and converges when it returns.

An unconfigured or unhydrated linked device rejects account rename with an
actionable account-state-unavailable error. Version 1 neither commits to the
blank local branch, falls back to a misleading device-local rename, nor stores
a separate pending intent.

### 9. Conflict semantics

Cardinality one guarantees a single resolved display-name value after merge,
but it still has a deterministic conflict rule. It does not by itself promise
human "latest write wins".

Before implementation relies on it, a two-replica divergence test must verify:

- both devices rename from the same base without seeing the other;
- pull/merge/push in both orders resolves to the same value;
- repeated sync does not oscillate;
- a later rename after convergence supersedes the resolved value.

Version 1 accepts Dialog's verified deterministic winner. If product semantics
later require causal or user-visible conflict resolution, introduce an
explicit versioned register rather than inferring recency from wall clocks.
Future account facts choose their own merge rules; the display-name register is
not a repository-wide policy.

### 10. Availability and provider boundary

The guarantees are:

- Unlinked devices retain today's local behavior.
- Already-linked devices retain local account state and spaces if the account
  service is unavailable.
- A ready account repository remains writable while its remote is
  unavailable; cross-device convergence pauses and later resumes.
- An unhydrated linked device continues to use its existing local profile and
  spaces, but cannot read or write authoritative account state until it
  acquires a trusted base.
- Cross-device convergence requires the configured remote to be reachable.
- New-device enrollment still requires the current account service or a future
  alternate handoff.
- Permanently moving to another provider is unsupported in version 1.

"Compatible remote" means more than accepting Dialog objects. It must enforce
the authorization properties the account expects. Existing root→device grants
are unexpiring and Tonk's access service screens them against the account
revocation registry. A provider that omits equivalent revocation semantics is
available but weaker.

Portable revocation is not solved by this repository. Until it is, the design
claims provider-independent data authority and local continuity for already
hydrated devices, not identical security from every Dialog endpoint or complete
independence from Tonk services.

### 11. Browser and CLI scope

The descriptor is part of the shared account-link contract, not browser-only
state:

- browser creation and link persist the descriptor beside the delegation;
- CLI handoff consumption returns both values in one response, and local
  persistence stores them in one account-link record so it cannot retain a
  delegation without its descriptor;
- both validate the same signed container and descriptor-to-delegation account
  subject;
- the worker ensures account state on boot; the CLI ensures it during link and
  on demand before any account-state operation. Version 1 requires no native
  background daemon.

The CLI has no display-name editing surface today. It still preserves the
descriptor and mounts the account system replica so adding a later account
operation does not require a new linking protocol.

### 12. Privacy

The remote stores repository artifacts. Unless the repository layer adds
end-to-end encryption, a storage operator may be able to inspect account facts.
Version 1 therefore stores only non-secret metadata already suitable for a
remote provider. Adding any other account fact requires the inclusion and
confidentiality review described above.

## Delivery

Keep the work in focused changes:

1. Join-time `MemberName` guard.
2. Live spike for typed remote absence and atomic create-if-absent.
3. Durable descriptor container plus account-service, browser, worker, and CLI
   transport/storage.
4. `tonk:account`, trusted-base persistence, lifecycle, and sync integration.
5. `AccountDisplayName`, ready-only rename, convergence, and per-target
   projections.
6. Existing-account descriptor establishment and initial-name migration.

The descriptor/lifecycle change and display-name change may share a milestone
but should remain reviewable as separate commits. No future account fact,
provider migration, or generic projection abstraction belongs in these changes.

## Testing

Worker wasm tests (`#[dialog_common::test]`, BDD names) where the harness
allows, plus native tests for pure lifecycle and merge decisions:

- join guard preserves an existing name and names an unnamed membership;
- a concurrent join test documents that the guard is not a linearizable
  first-writer lock;
- descriptor validation rejects a wrong signer, account subject, signature,
  remote encoding, or unsupported version;
- the descriptor is a durable artifact, not the expiring account-service
  invocation;
- account creation and existing-account establishment accept exactly one
  descriptor; simultaneous valid candidates converge on the stored winner;
- browser and CLI linking use and persist the established descriptor rather
  than the current page's default;
- mounting alone does not permit an account-state write;
- timeout, offline, 401/403, 5xx, and malformed responses leave an unhydrated
  replica unready and never initialize local history;
- confirmed absence attempts atomic empty-repository create-if-absent;
- the create winner becomes ready, while a loser pulls the winner before
  becoming ready;
- another authorized device can finish repository creation after the
  establishing browser loses local state;
- a ready replica remains writable offline;
- the account replica is `tonk:account`, absent from Hub/space enumeration,
  roster restamps, migration, pause, and removal;
- boot mounts and opens the account repository so background sync includes it;
- a ready linked rename writes `AccountDisplayName`, projects `ProfileName`,
  restamps every stale real local space, and queues sync;
- an unconfigured or unhydrated linked rename fails without writing the account
  branch or local fallback;
- account creation and existing-account establishment copy only the initiating
  device's current name; ordinary boot never guesses an initial name from
  another device;
- adoption after pull updates the cache and restamps spaces known only to the
  adopting device;
- unchanged targets receive no durable writes, while projection retries heal
  one failed space without changing account state;
- two divergent display-name writes converge deterministically in both sync
  orders and a subsequent rename supersedes the winner;
- an unavailable remote leaves ready local account state usable and never
  changes the configured descriptor;
- the repository routing key remains derived from the immutable account subject
  when signing authority later rotates.

Live integration gates before implementation is called complete:

- classify the access service's exact absent/unauthorized/unavailable errors;
- verify atomic create-if-absent/non-fast-forward behavior against a real
  remote;
- verify root→device→operator authorization for an account-subject repository;
- verify the actual cardinality-one divergence winner;
- state which revocation semantics a non-Tonk compatible provider must
  implement.

## Implementation gates

The implementation plan must resolve these before the corresponding code lands:

1. The non-expiring `AccountRepositoryDescriptorV1` signed container,
   canonical encoding, validation limits, and content hash.
2. Account-service set-if-absent storage and exact descriptor transport through
   browser and CLI handoffs.
3. The remote's typed absence and atomic create-if-absent behavior. If it cannot
   supply the required compare-and-set, stop and revisit provisioning authority.
4. Durable trusted-base storage keyed by descriptor hash and the exact events
   that may set it.
5. The one-time root/passkey establishment ceremony for existing linked
   accounts.

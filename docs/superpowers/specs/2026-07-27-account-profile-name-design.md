# Account-repo display name

2026-07-27

## Problem

After creating an account and linking several devices, each device shows a
different profile name, and renaming on one device does not update the
others. This is not a sync-convergence failure. Two design gaps cause it:

1. The name each device displays (`ProfileName`,
   `xyz.tonk.profile/display-name`) lives on that device's local profile
   meta branch and never syncs. Bootstrap stamps `petname(device_did)` —
   a different DID, hence a different petname, per device. Nothing in the
   account link/restore flow carries a name across devices.
2. The rename effect only pushes outward (local `ProfileName` + root-keyed
   `MemberName` restamps on space rosters), and
   `record_claim_on_content` (join) unconditionally asserts
   `MemberName::new(root_membership, local_display_name)` — so any later
   join from a linked device overwrites the chosen roster name with that
   device's petname. `MemberRole` and `InvitedVia` already have first-wins
   guards; `MemberName` does not.

## Constraint

Durable user state lives inside the tonk system — synced repositories and
facts — not in tonk-run services. Everything must keep working if all tonk
services die, and a competing provider must be able to offer equivalent
services without coordinating with tonk's. This rules out storing the name
in the account service (D1/R2), and rules out treating space rosters as
the *home* of identity (an account with no spaces would have no name, and
cross-space disagreement has no principled tie-break).

## Design

### 1. Join-time first-wins guard (standalone first PR)

`record_claim_on_content` stamps `MemberName` only when the membership
entity has no name row yet — the same first-wins discipline the role and
provenance stamps already use. A never-named joiner still gets named; a
rename is never clobbered by a later join. Correct under any propagation
design; lands alone.

### 2. The account repo

A repository whose **subject is the root DID**:

- **Derivable, coordination-free identity.** Any linked device computes
  the subject from the chains it already holds; any provider can host the
  remote.
- **Remote:** the device's default remote convention (the
  `<tonk-default-remote>` resolution), attached as `origin`, `main`
  tracking `origin/main`. A device on a different provider fails soft to
  its local name until the repo's remote is reachable.
- **Authorization:** nothing new. Root→device delegations are
  subject-open; a repo whose subject is the root is root-owned, and the
  existing chains compose.
- **Contents:** exactly one fact for now — `ProfileName` keyed by the
  root entity, on `main`. (This repo is the natural future home for the
  claim index, which would let the R2 claim backup retire; out of scope.)

The device-local `ProfileName` on the profile meta branch remains as a
cache: the FAB's sealed read (`tonk:profile/name` scoped to
`main@profile:tonk`) is untouched.

### 3. Bootstrap: `ensure_account_repo`

Called from the worker's `link` handler and from the boot-time
convergence hook where `restore_spaces` already runs. When linked and no
account-repo replica exists:

1. Try to clone from the default remote (`open_with`, never `init_with` —
   the discipline join's pull-on-claim established).
2. If the remote has nothing, init locally, seed the current local
   display name, push.

First-link is registration, so creator-inits/linkers-clone falls out
without a separate ceremony. Migration for existing linked accounts is
this same boot path — no separate machinery.

**Known risk to verify during implementation:** two devices linking
near-simultaneously to a pre-existing account could both init. The design
assumes a diverged push fails (non-fast-forward) and the loser re-clones
and adopts. If the remote accepts diverged pushes silently, this needs a
different guard — verify against a live remote before relying on it.

### 4. Rename flow

The rename handler writes the authoritative `ProfileName` (root-keyed) to
the account repo, mirrors it into the local profile-meta cache row, and
restamps space rosters exactly as today (space peers cannot read the
account repo, so the roster projection stays).

### 5. Adoption flow

When a sync pull updates the account repo (reactor pulls already re-poll
subscriptions), the worker adopts the authoritative name into the local
profile-meta cache when it differs. No tie-breaks: one authoritative row
in one repo. Concurrent renames from two devices converge by the CRDT's
cardinality-one merge — consistent everywhere, even if not "latest wins".
Adopting devices do not restamp rosters (the renamer did); a later join
stamps the adopted name, which is idempotent.

### 6. Degradation

- Unlinked device: today's behavior, untouched (local petname, local
  rename).
- All remotes dead: renames stay local, everything keeps working,
  convergence resumes when any remote returns.
- No tonk service is in any path; a dialog remote from any provider
  suffices.

### 7. Testing

Worker wasm tests (`#[dialog_common::test]`, BDD names):

- join guard: first-wins, and a new joiner with no prior row still gets
  named
- `ensure_account_repo`: clones when the remote has the repo; inits and
  seeds when it does not
- rename: writes the authoritative row and mirrors the local cache
- adoption: a pulled account-repo name lands in the local cache
- boot path: an existing linked account without an account repo gains one

Native tests where the harness allows, per existing patterns.

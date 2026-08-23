# One account, and ownership read from the space itself

**Goal:** Give the native CLI a space model in which the account is a hosting
relationship, not an access boundary. Editing a local replica is always
unrestricted. Enforcement happens where it is real — at the service boundary,
against the space's own delegation chain. Which account owns a space is read
from the space, never recorded beside it.

**Approach:** Build on `staging` after merged PR #726 (`53821ebe3`), using its
account-as-profile-main upstream, signed account directory, membership roles,
and `/provider/add` protocol. One installation store (`spots.json`), one
Dialog identity, one account slot. This revises the 2026-08-20 direction: the
per-space account tag and the resolution-time account comparison are removed
(see Rejected alternatives). Ownership derives from the roster the space
itself carries on its content branch, verified against the retained
`subject → … → account root` delegation chain when a decision rides on it.

**The model, in four rules:**

- **Editing is unrestricted.** Any replica registered on the device opens,
  reads, and writes — signed in, signed out, or signed into an account other
  than the space's owner. No command refuses a replica over account state.
- **Enforcement lives at the service boundary.** Push and pull present the
  space's own chain; the access service accepts or rejects; revocation kills
  a chain from any of the account's devices. The CLI relays a rejection with
  copy naming the fix — it never pre-judges a sync the service would allow.
- **The signed-in account parameterizes only account-service operations** —
  creating a hosted space, linking, pulling the directory, listing and
  revoking devices, deletion. These refuse without an account; nothing else
  consults it.
- **Ownership is read, not recorded.** The founder row of the space's roster,
  on the content branch and keyed on the account root, names the owner. The
  registry holds no per-space account fact.

**Precedent:** This is the git-plus-forge shape: clones are unowned local
data, commits work offline unconditionally, enforcement happens once at
push/fetch against per-remote credentials, and a forge session (`gh auth`)
gates only forge-level operations — logging out does not touch clones or
standing key access. SSH is the same shape one level down (per-target
credential, no session). The grounding is local-first (Kleppmann et al.,
2019): hosts are relays and the data outlives the service relationship —
tonk's survivability rule. The contrast cases are the modal designs (Dropbox
unlink, OS account-bound app data, profile switching), where sign-out must
"do something" to local data and produces the hide/delete/re-tag trilemma
this plan exists to avoid.

**Constraints:**

- One account at a time. `tonk account link` (alias `login`) refuses while
  another account is signed in; `tonk account logout` clears the slot,
  notifies the provider best-effort, and touches nothing else — not replicas,
  not the profile, not retained delegations. Signing out is not disowning;
  disowning a device is revocation, from the account page or another device.
- Spaces are never deleted, hidden, refused, or re-tagged by an account
  switch. Every registered replica stays listed and usable throughout.
- The roster on the content branch (`main`) is the ownership record: a
  founder-role membership row keyed on the account root, matching the
  worker's convention, so the row converges across devices and clients. The
  meta branch stays local-only and carries no shared fact.
- `tonk space new` creates local-only when signed out (no roster yet), and
  hosted account-owned when signed in (founder row written at creation).
  Linking an account never scans, provisions, or enrolls existing spaces.
- The only ownership transition is local → the signed-in account, one space
  at a time, via `tonk space link`. The already-owned refusal reads the
  roster, not the registry. Linking is non-destructive, every step idempotent
  or guarded, so an interrupted or pre-revision link converges on re-run.
- A space with a foreign content upstream, another durable member, or
  recorded invitations is not eligible to link. Eligibility reads the content
  branch (plus the meta branch for invitation rows older CLI releases wrote
  there).
- Sharing never changes ownership: `tonk invite` / `tonk join` add members.
- Local removal, account ownership, provider hosting, membership, authority,
  and remote bytes stay separate. `tonk space rm` removes a local replica and
  nothing else.
- `spots.json` records name → site bindings and the one signed-in account,
  nothing per-space. A registry written by another tool round-trips
  byte-for-byte.
- Canonical vocabulary `space`, `--space`, `TONK_SPACE`, `account spaces`,
  with visible `spot`, `--spot`, `TONK_SPOT`, `account spots` aliases.
- Out of scope: browser profile UX, account-to-account ownership transfer,
  delegation-chain rebasing, rotating existing bearer links, provider billing
  transfer, multiple simultaneous accounts, and warnings on writes to a
  space owned by an account other than the signed-in one — the owner column
  is the mitigation; refusal was rejected, and nagging is refusal's residue.

## Product contract

```text
$ tonk space list
NAME                 OWNER                     ROLE
scratch (z6Mkq7vp)   -                         local
garden (z6Mk4e2b)    you (z6Mkccc1)            owner
roadmap (z6Mkf0aa)   Ada Lovelace (z6Mkbbb9)   member
```

Every human name in the table is paired with an abbreviation of its stable
identifier — git's `Name <email>` discipline. `NAME` carries the space's
subject, so the same space is recognizable across devices whose local names
differ; `OWNER` carries the founder's account root beside the `MemberName`
display name from the roster (the worker writes one beside every
membership) — `you` when the founder root is the signed-in account, the
abbreviation alone when no name row exists, and `-` until a space is linked
or hosted. The abbreviation is a true prefix of the DID's method-specific
identifier, eight characters by default (`z6Mkf0aa`): the first four are
the shared ed25519 multibase prefix, so anything shorter would render every
row identical, and, like git's short hashes, it lengthens when a listing
contains an ambiguous prefix. Full DIDs stay available via `--json`.
Registry names are unique keys and registration refuses a taken name, so
`NAME` cannot collide locally; owner names are self-chosen and can, which
the always-present root abbreviation absorbs — including a roster name of
literal `you`. Names orient, chains decide — a misleading name can confuse
a reader, but no check in this plan consumes one. `ROLE` is the roster row
this installation can claim, matched by the signed-in account root first and
the device profile DID second: founder → `owner`, member → `member`, no
roster at all → `local`, a roster with no row for us → `-`, and a replica or
roster that cannot be read → `unknown`, listed with a diagnostic rather than
hidden.

There is no ACCESS column and no resolution-time refusal. A sync the
service rejects fails at the boundary where that is true, and the copy is
composed from the roster the replica already holds, because the most likely
cause differs by state. When the owner is not the signed-in account, the
likely fix is signing in, and the error leads with that:

```text
$ tonk push
error: could not sync 'roadmap': this device holds no authority its access
service accepts
'roadmap' is owned by Ada Lovelace (z6Mkbbb9); you are signed in as Alice
(z6Mkccc1). sign into the owning account with `tonk account login`, or ask
a member for an invite and claim it with `tonk join <URL>`
```

When the owner is the signed-in account (or the roster names no one else),
the honest explanation is revocation, and the copy points at the device
list instead:

```text
$ tonk push
error: could not sync 'garden': the access service rejected this device's
authority
this device may have been revoked; check `tonk account devices`, or ask a
member for a new invite and claim it with `tonk join <URL>`
```

Linking is one word and no target, because there is only one account:

```text
$ tonk space link garden
linked	garden	did:key:...
account: did:key:aa
site: /…/spots/garden
```

An account-owned space explains itself rather than moving, from its own
roster:

```text
$ tonk space link garden
error: "garden" already belongs to an account, so it stays there.

Once a space is synced with an account, it stays owned by that account.
This keeps existing shares working.

Share it instead:
  tonk invite

owner account: did:key:aa
```

## Durable data model

`spots.json` shrinks to bindings plus the one signed-in account:

```rust
pub struct SpotEntry {
    pub site: PathBuf,
}

pub struct AccountRecord {
    pub root: String,
    pub ceremony_origin: Option<String>,
    pub access_remote: Option<String>,
    pub revocation_relay: Option<String>,
}

pub struct Registry {
    pub spots: BTreeMap<String, SpotEntry>,
    pub bindings: BTreeMap<PathBuf, String>,
    /// The account signed in here, if any.
    pub account: Option<AccountRecord>,
}
```

Ownership and role are derived per space from `Membership`/`MemberRole` rows
on the content branch. When a decision rides on ownership — the link
refusal, provisioning — the roster is confirmed against the stored
`subject → … → account root` prefix credential (`space_root_site`); the
chain is the truth and the roster is its legible, synced mirror, so a
mismatch is a diagnostic, not silently trusted. The chain, not the roster,
is what the provider validates; the roster, not the chain, is what a member
device can cheaply read — including the owner of a space it merely joined,
which the removed registry tag never knew.

Legacy reads: CLI releases through this branch's first build wrote
`Invitation`/`InvitationExecution` to the meta branch (the worker writes
them to `main`); link eligibility reads both. Founder membership rows the
pre-revision build wrote to meta, keyed on the device DID, are ignored —
re-running `tonk space link` writes the canonical row on `main` and
converges.

## Rejected alternatives

- **Registry account tags plus resolution-time comparison** (the 2026-08-20
  revision of this plan, built through `e67302492`). The comparison invented
  a third kind of access that was neither local possession nor a
  service-validated chain, and it inverted on logout: signing out strictly
  increased what the device could open. Enforcement the provider does not
  apply is copy, not policy, and the tag it required could drift from the
  chains with nothing checking it.
- Labeled profiles with `account add`/`account use`. Multiple simultaneous
  accounts multiplied identity, session, registry, and storage state for a
  case the product does not have, and made every command answer "which
  profile?" first.
- Deleting or hiding a previous account's replicas on switch — the
  hide/delete/re-tag trilemma; data the user can read is not data they can
  sync, and losing the former to explain the latter is a bad trade.
- Removing keys or delegations at logout. The account secret is never on the
  device (PR #726); deleting the device delegation locally is theater unless
  the service stops honoring it, which is revocation and already exists; and
  no key deletion un-reads plaintext replicas on disk.
- Blocking every command while signed out. It breaks offline editing of
  spaces this device demonstrably holds authority over.
- Account-to-account ownership transfer. Revoking the old shared authority
  prefix can invalidate downstream users and existing invite chains.
- Silently reinterpreting a link as a share; that hides the owner/member
  distinction.

## File map

- `plan/cli-space-parity.md`: this contract and its verification state.
- `rust/tonk-cli/src/spot.rs`: the bindings-only registry and account slot;
  the account gate and per-space tag removed.
- `rust/tonk-cli/src/inventory.rs`: the listing; owner and role derived from
  the content-branch roster.
- `rust/tonk-cli/src/site.rs`: founder membership written to the content
  branch keyed on the account root; the adopt-vs-load prefix boundary.
- `rust/tonk-cli/src/space_link.rs`: linking; eligibility and the
  already-owned refusal read from the roster.
- `rust/tonk-cli/src/sync.rs`: the relayed service-rejection copy,
  contextualized from the roster (owner name vs signed-in account).
- `rust/tonk-cli/src/account.rs`, `account_spots.rs`: link/logout/status and
  the signed directory, unchanged in role.
- `rust/tonk-cli/src/bin/tonk.rs`: command surface and the copy above.
- `rust/tonk-cli/tests/`: `space_inventory.rs`, `space_link.rs`,
  `cli_spot.rs` updated; `space_access.rs` replaced by coverage that every
  registered replica opens regardless of account state, and that a revoked
  chain fails at sync with the copy above.

## Upgrading

A `spots.json` written by PR #726 has no per-space fields and reads
unchanged. One written by the pre-revision build of this branch (never
released) carries `account` tags: they are ignored on read and dropped on
the next registry write; its meta-branch founder rows are ignored, and
re-running `tonk space link` converges the roster onto `main`. The
byte-for-byte round-trip promise for third-party registries holds because
this plan no longer adds fields at all.

## Status — 2026-08-23

Built. The 2026-08-20 checkpoint (tag model, verified through `e67302492`,
including the pre-existing `tests/site.rs` profile-lock flake recorded there)
stands as history in git; the branch now implements the model above.

1. **Done.** Founder membership goes onto the content branch through the
   reactor's cached `main` handle, keyed by `site::member_did` — the account
   root when one is signed in, the device profile otherwise, matching the
   worker's `member_did`. The meta-branch write is gone.
2. **Done.** `role_for_site`, `read_roster`, the listing, and link
   eligibility all read `main`. Invitation rows are still read from `meta`
   as well, for the shares older CLI releases recorded there.
3. **Done.** `SpotEntry` is `{ site }` and nothing else; `set_space_account`,
   `SpotError::ForeignAccount`, the resolution-time `access` check, the
   ACCESS column, and the sign-in "unavailable until their own account signs
   in" notice are all removed. `SpaceRole` gained `Unlisted` (rendered `-`)
   for a roster with no row for us. The listing is NAME/OWNER/ROLE with
   `MemberName` display names, `you` for the signed-in account, and one
   ambiguity-lengthened abbreviation shared across every DID in the listing.
4. **Done.** The already-owned refusal reads the founder row, and it is
   settled before any of this account's endpoints are consulted. A founder
   row naming the signed-in account only counts as already-linked when the
   stored `space_root_site` prefix is here too — a non-minting read, so it
   cannot answer yes by creating the ownership it was asked to confirm; an
   interrupted link therefore resumes instead of reporting a finished one.
5. **Done.** `SyncError::Rejected` is classified by downcasting to
   `AuthorizeError` through dialog's layered errors rather than by matching
   text. The three variants the capability crate marks as *not* decisions
   (`Unavailable`, `UnavailableProof`, `Malformed`) stay `Io`, so a service
   that could not answer is never reported as one that said no. The error's
   own Display carries the service's reason; `tonk push` / `tonk pull` print
   `sync::rejection_report`, which branches on the roster as specified.
6. **Done.** `cargo test -p tonk-cli --features integration-tests` — 26 test
   binaries, all green, including the live access-service coverage in
   `space_access.rs` and `space_link.rs`. Workspace clippy
   (`--all-targets --all-features`) and `cargo fmt --all --check` clean.

Two decisions the delta did not spell out, both settled toward the plan's
own logic:

- The rejection copy names the fix and drops the service's raw reason, as
  the contract shows. The reason stays on `SyncError::Rejected`, so any
  other caller that prints the error still gets it.
- Both roots in the wrong-account message are abbreviated against each
  other, for the same reason the listing lengthens: two identical-looking
  prefixes in one sentence tell the reader nothing.

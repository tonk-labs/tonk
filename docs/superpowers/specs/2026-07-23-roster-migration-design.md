# Roster migration (stage 3B)

Design for converging a user's **existing** device-keyed state onto their root
DID when they get an account. Stacks on stage 3A (root-DID rosters, merged) and
the cross-device restore branch (reuses its backup helpers). Companion specs:
`docs/superpowers/specs/2026-07-23-root-did-rosters-design.md`,
`docs/superpowers/specs/2026-07-23-cross-device-restore-design.md`.

## Problem

Stage 3A keys *new* claims and roster writes on the root DID; restore carries
account spaces to new devices. But a user who was a member of spaces *before*
getting an account still has device-keyed roster rows and claim chains that
terminate at their device DID. Those spaces don't converge across devices, and
profile rename is a silent no-op on them (the known 3A limitation). 3B migrates
them.

## Scope

**In:** on device link, sweep the profile's existing spaces and, for each one
still keyed on the device DID: re-key its roster rows to the root DID (retract
the old, re-stamp first-wins), re-anchor its capability chain to the root
(`device -> root` for claimed, `space -> root` for created), back up the
re-anchored chain so other devices restore it, and fix the profile-rename
no-op.

**Out:** revocation-list awareness (access-service concern, pairs with the
billing/access stage); live propagation (migration runs on link, like restore).

## Trigger

The `link` handler (`rust/tonk-worker/src/router/account.rs:147`) — where
account-creation-first-device and browser self-link both persist the
`root -> device` chain, and where `restore_spaces` is already wired. Migration
hooks the same post-persist tail, fire-and-forget (wasm `spawn_local`; native
inline), so a slow account service never stalls the link response. It runs
alongside restore: migration converges *existing local* spaces, restore pulls
*backed-up remote* ones — disjoint sets, order-independent.

Enumeration reuses `profile_space_keys` (`rust/tonk-worker/src/router/profile_name.rs:129`),
which queries every `Replica` and yields each space's routing key.

## Roster re-key (facts) — one atomic transaction per space

For each space, read the content-branch roster (the `build_repository_info`
query pattern, `repository.rs:3305`). A space is *unmigrated* iff it has a
`Membership` whose `member == profile.did()` (the device DID). Post-link,
`member_did` resolves to root, so migration matches on the device DID
explicitly.

For an unmigrated space, in **one** content-branch transaction:

- **Assert** the root-keyed rows: `Membership::new(root_did, subject)`, and —
  stamped on that new entity (`root_membership.this()`) — the copied
  `MemberRole` (same URI, so a founder stays a founder), `MemberName` (same
  name), and `InvitedVia` (same invitation entity).
- **Retract** the four device-keyed facts. Retraction is by *full fact*
  (`transaction.retract(concept)`, `dialog-reactor/src/transaction.rs:83`), so
  each device-keyed concept is reconstructed from the values just read and
  retracted.

Atomic assert+retract means no half-migrated row ever syncs out. First-wins
stamps are copied explicitly, never assumed to carry across the entity change
(`Membership`'s entity is derived from `(subject, member)`, so the root row is
a *different* entity than the device row).

The device-local `Replica` index (meta branch, never syncs) stays device-keyed
— it is the profile's own bookkeeping, not shared roster; untouched.

## Re-anchor (capability) + backup

Re-keying the fact gives the roster; the account also needs a *capability* into
the space, and other devices need it. Per space, mint by ownership using
`try_access()` (the discriminator restore already uses):

- **Created / owned (`Some`):** `access.claim(repo).delegate(root)` ->
  `space -> root`. This is exactly `back_up_owned_space`
  (`account_backup.rs:330`, from the restore branch) — reuse it.
- **Claimed (`None`):** `profile.access().claim(&repo).delegate(root)` -> the
  composed `space -> eph -> device -> root` (the invite-mint path proves this
  call shape for a delegated capability, `repository.rs:931`). Save it to the
  access store so this device's own presign BFS composes it.

Then **back up** the re-anchored chain (`{chain, remote_url}` -> `/chains/put`,
reusing 3A's backup client) so the account's *other* devices restore these
spaces. **Trade-off (from the identity design):** a claimed re-anchor flows
through the old device DID, so revoking that device later severs it; the
clean-up is a fresh invite claim or founder re-delegation.

### The `remote_url` recovery risk (verify-first)

Backup needs each existing space's sync URL, stored as a `SiteAddress` in its
remote config — the same URL-recovery friction as the `put_repository` gap.
The plan's **first step probes** whether the URL is cleanly recoverable from
the stored config:

- **If yes:** migration backs up re-anchored chains — full cross-device.
- **If not:** migration re-keys + re-anchors *locally* (roster converges, the
  account gains the capability on this device), and cross-device backup of
  migrated spaces is a documented follow-up. No guessing; the fact/rename half
  (PR 1) is unaffected either way.

## Rename fix

`restamp_member_name` (`rust/tonk-worker/src/router/profile_name.rs:217`)
currently builds `Membership::new(profile.did(), repo_did)` — the device DID —
so an account-holder's rename writes a `MemberName` against a device-keyed
entity that no roster row uses (the 3A no-op). Fix: key on the resolved member
DID (root when linked) **and** retract the orphaned device-keyed `MemberName`
(cardinality-one on a *different* entity, so it isn't otherwise overwritten).
This reuses PR 1's read-then-retract primitive.

This assumes migration has converged the space (both run on link, migration
first), so by the time a user renames, the space is root-keyed and the rename
lands on the root row. A rename during a still-pending migration is a rare
transient race — it writes a root `MemberName` on a not-yet-migrated space, and
migration then re-stamps the copied (older) device name over it; it self-heals
on the next rename. Bounded and low-impact given migration runs first on link.

## Idempotency

Re-running migration is safe: a migrated space has no device-keyed `Membership`
row, so it is skipped. The per-space transaction is atomic; a crash mid-sweep
leaves already-migrated spaces done and the rest untouched, and the next link
finishes them.

## Testing

- **Native / service-worker:** a device-keyed roster (member/founder + name +
  provenance) migrates to root — device rows gone, root rows present with role,
  name, and provenance preserved; a second run is a no-op; rename updates the
  root row and drops the device one.
- **Re-anchor + backup:** reuse the restore branch's machinery
  (`back_up_owned_space`, the device-signed `/chains/put` client), already
  proven against the real `AccountServer`; add the claimed-space
  `device -> root` mint + backup.

## Build order

Two PRs, off the restore branch (`feat/cross-device-restore`), rebased onto
`staging` as 3A/restore land:

1. **Facts:** the read-then-retract/re-stamp primitive, the migration sweep
   (roster re-key), and the rename fix. No capability minting — mergeable on
   its own, closes the rename no-op immediately.
2. **Capabilities:** re-anchor mint (both cases) + backup of migrated chains,
   with the `remote_url` verify-first probe up front.

## Risks

- **`remote_url` recovery** (above) — handled verify-first; worst case defers
  the backup half, not the re-key/rename.
- **Claimed re-anchors flow through the device DID** — revoking that device
  severs them until a fresh invite/founder re-delegation. Inherent to the
  no-ceremony re-anchor; the identity design already accepts it.
- **Sweep cost at scale** — an account linked with many pre-account spaces does
  one content transaction + one mint each on link; fire-and-forget keeps it off
  the link's critical path, but a very large history trickles in. Acceptable.
- **Partial-migration visibility** — because each space commits atomically and
  the roster is authoritative on the content branch, a partially-swept account
  shows a mix of migrated and not-yet-migrated spaces briefly; both are valid
  rosters, and the next link converges the rest.

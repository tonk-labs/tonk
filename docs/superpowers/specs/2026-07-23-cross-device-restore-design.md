# Cross-device restore (+ created-space backup)

Design for the second half of the cross-device story: a linked device pulls the
account's backed-up space delegations and auto-mounts each space, so a user's
spaces follow them to every device. Stacks on stage 3A
(`docs/superpowers/specs/2026-07-23-root-did-rosters-design.md`), which added
the root-DID rosters and the claim-side backup.

## Problem

Stage 3A backs up a claimed space's `space -> eph -> root` delegation to the
account service, but nothing pulls it back — so a second device gains nothing
yet. And created spaces aren't backed up at all: a creator holds the space
signing key, which no other device has, so created spaces are single-device.

## Scope

**In:** a linked device restores both **claimed** and **created** spaces from
the account service and mounts them locally. To make created spaces
restorable, the create path also backs up a `space -> root` delegation.

**Out:** live cross-device propagation (a space claimed on device A appears on
device B only after B's next link or restart — see Triggers); migration of
pre-account spaces and the rename switch (stage 3B); revocation.

## Shape

Two sides, sharing the 3A artifact and the `/chains/*` endpoints unchanged:

- **Backup (producers):** claim backs up `space -> eph -> root` (3A, done). New:
  the create path backs up `space -> root`.
- **Restore (one consumer):** pull all backed-up artifacts, mount each space,
  let sync bring the rest.

Both producers emit the existing `ClaimBackup { chain_hex, remote_url }`
(`rust/tonk-worker/src/router/account_backup.rs:15`): a `... -> root` chain
whose subject is the space, plus the space's sync URL. No artifact or
account-service change.

## Created-space backup (the new producer)

A created space is a full signer: `create_repository`
(`rust/tonk-worker/src/router/repository.rs:2361`) mints an `Ed25519Signer` and
returns `Repository<SignerCredential>`. It already issues a `space -> profile`
delegation via `repository.access().claim(&repository).delegate(profile_did)`
(`repository.rs:2397`). We mint `space -> root` the same way —
`.delegate(root_did)` — which is subject-specific (the space DID) and carries
full space authority, so a restored device acts with the founder's rights
through the composed `space -> root -> device` chain.

Create is local-only; the remote is attached later. So created-space backup
fires **when a created space gains a remote** — hooked in `enable_sync_inner`
(`repository.rs:1988`), which is where the account-holder create flow attaches
its remote (`CreateSpaceHandler` runs `create_space_inner` local-only, then
`enable_sync_inner`). When the profile is account-linked: mint `space -> root`,
push `{chain, remote_url}` to `/chains/put`. Best-effort and fire-and-forget,
exactly like the 3A claim backup (`account_backup.rs` `back_up_claim`). A
local-only space with no remote is not backed up — there is nothing for another
device to sync.

**Not hooked (deliberate follow-up):** the one-shot `PUT /api/repository/{name}`
with a remote in the body (`put_repository`, `repository.rs:216`) attaches a
remote without going through `enable_sync_inner`, so a space created that way is
not backed up. The account-holder UI never uses this path (it creates
local-only then enables sync), so the exposure is limited to non-UI /
programmatic callers; it fails open (the space works locally, it just does not
restore on another device). Hooking it needs the raw sync URL recovered from
the parsed `SiteAddress` in the request configuration; tracked as a follow-up
rather than done here.

## Restore consumer + a shared mount helper

The claim path's replica-mount is reused. Extract `mount_replica(tonk, subject,
remote_url) -> key` from `join.rs:242-291` — the verifier-only credential
(`Credential::from(Ed25519Verifier)`), the `Subject::from(profile).attenuate(
Space::new(key)).create(...)` local replica, the remote/branch configuration,
the **local replica-meta index** entry, and `mark_replica_initialized`.

Crucially, `mount_replica` must **not** write the content-branch roster.
Today `record_repository_meta` (`repository.rs:2448`) bundles two things: the
local replica-meta index (device-local, needed by both claim and restore) and
`record_membership_on_content` (a content-branch `Membership`/`MemberRole`/
`MemberName` write). The extraction separates them — `mount_replica` keeps the
local-meta half; the content-roster half stays a claim-only step. See Roster
via sync for why restore must not write it. **Claim** = `mount_replica` +
`record_membership_on_content` + `record_claim_on_content`; **restore** =
`mount_replica` + save-delegation + sync.

Restore, best-effort per item:

1. Resolve `account_link` (the `root -> device` chain), the account-service URL,
   and the device signer — the same three `back_up_claim` resolves.
2. `/chains/list` -> keys; `/chains/get` per key -> `ClaimBackup` bytes.
3. Per artifact: parse the chain, take `subject = chain.subject()`. If a replica
   for that subject already exists (`find_replica_for_subject`), skip. Else save
   the chain to the access store (presign's BFS composes it with the local
   `root -> device`), `mount_replica(subject, remote_url)`, and trigger a sync.

Two new client functions, `list` and `get`, parallel the 3A `put` client in
`account_backup.rs`: the same `tonk_identity::request::build_device_invocation`
with commands `["account","chain","list"]` (no args, JSON array of keys) and
`["account","chain","get"]` (arg `key`, raw octet-stream body).

## Roster comes via sync, not re-written

Restore writes no roster rows, and this is a correctness requirement, not just
leanness. `Membership`/`MemberRole`/`MemberName` already live on each space's
content branch keyed on the **root DID** (written by whichever device claimed
or created the space). Once the restored replica syncs, those rows arrive, and
`is_self` matches (mount and reader both resolve through `member_did` -> root).

If restore instead re-ran the claim's content write, it would stamp
`MemberRole::member` — but `MemberRole` is cardinality-one, and
`record_membership_on_content` asserts it **unconditionally** (unlike the
claim's `record_claim_on_content`, which guards on `already_roled`). On a space
the account *created*, that member stamp would overwrite the `founder` role
(last-write-wins) — a self-inflicted demotion that then syncs out. So restore
mounts and installs the delegation only; the roster is authoritative on the
content branch and arrives over sync.

## Triggers and the staleness tradeoff

Restore runs best-effort and fire-and-forget at two points:

- **On device link** — the `link` handler (`rust/tonk-worker/src/router/account.rs:147`),
  after it persists the `root -> device` chain.
- **On startup for a linked profile** — after `bootstrap_profile` in
  `TonkServiceWorker::new` (`rust/tonk-worker/src/worker.rs:1678`), gated on
  `account_link` being present.

Both spawn detached (wasm `spawn_local`; native bounded), so a slow account
service never blocks link or boot — the 3A posture.

Tradeoff: a space claimed on device A appears on device B only after B's next
link or restart; there is no live push. Accepted for this round. A live refresh
(periodic, or on a sync signal) is a deliberate follow-up.

## Testing

- **Native:** the `list`/`get` client and the `space -> root` mint against the
  real `AccountServer` (extend the account-service HTTP test); the
  `mount_replica` extraction (existing claim tests stay green).
- **Wasm/service-worker (CI web leg):** a restore run mounts a backed-up space
  and its roster syncs in; a created space with sync enabled is backed up.
- **Idempotency:** a second restore over the same account is a no-op (replicas
  already mounted).

## Build order

Two PRs off the 3A branch (`feat/root-did-rosters`), rebased onto `staging`
once 3A merges:

1. Extract `mount_replica` (claim refactored onto it, behavior unchanged);
   created-space backup — mint `space -> root` and push on sync-enable /
   create-with-remote.
2. The restore consumer — `list`/`get` client, the mount loop, and the link +
   startup triggers.

## Risks

- **Staleness** (above) — restore is trigger-based, not live. Bounded and
  documented; the space still arrives on the next link/restart.
- **Full authority via `space -> root` for created spaces.** The delegation is
  unattenuated, so every linked device is a full co-owner of spaces the account
  created. That is the intent (your devices are you), but it means revoking a
  device (stage 3B/revocation) is the only way to cut a lost device off from
  created spaces — same trade-off the identity design already accepts for
  `root -> device`.
- **Backup without a remote.** A created, never-synced space is not backed up;
  enabling sync later triggers the backup. If a space is shared before sync is
  enabled (not a current flow), it would be missed — the backup hook must sit
  wherever a remote is first attached.
- **Mount/pull latency at scale.** Restoring an account with many spaces mounts
  and syncs each on link/boot; the fire-and-forget dispatch keeps it off the
  critical path, but a very large account could see a slow trickle of spaces
  appearing. Acceptable for the expected space counts.

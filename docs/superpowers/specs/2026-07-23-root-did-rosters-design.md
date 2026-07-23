# Root-DID rosters (stage 3A)

Design for stage 3 of the cross-device identity system: keying rosters on the
user's root DID. Parent spec:
`docs/superpowers/specs/2026-07-17-identity-accounts-design.md`. This spec
covers **stage 3A only** — new claims and roster writes for account-holders.
Migrating existing device-keyed members is stage 3B (deferred).

## Problem

Stage 2 gives an account-holder a root DID and a subject-open `root -> device`
delegation, persisted locally. But invite claims and roster rows still key on
the **device** DID:

- `rust/tonk-worker/src/router/join.rs:176` — `invite.claim(&tonk.profile.did())`
  audiences the device DID.
- `rust/tonk-worker/src/router/join.rs:336` — `Membership::new(tonk.profile.did(), repo.did())`.
- `rust/tonk-worker/src/router/repository.rs:2678` — founder membership, same shape.

So the two devices of one account land as two unrelated members, and the
membership schema's content-derived `(subject, member)` convergence — designed
for exactly this — never fires. Nothing hangs on the root DID yet.

## Scope

**In:** account-holders' claim and founder/create writes key on the root DID,
and each claimed space's chain is backed up to the account service so a later
device can recover it.

**Out — immediate follow-up PR (restore):** a second device pulling the
backed-up chains and auto-mounting the spaces. Restore is nearly a re-run of the
join path (mount a replica per subject, configure the remote, record roster
rows) and is deferred to keep this PR reviewable. Backing up now means nothing
is lost in the interim.

**Out (stage 3B):** migrating existing device-keyed members, `device -> root`
re-anchoring of pre-account chains, the profile-rename root switch, and
revocation-list awareness.

Device-only users are unchanged in every path.

## Key finding: composition is automatic

The presign path composes delegations from the access store on the fly. At
presign, `CertificateStore::prove`
(`rust/dialog-capability/src/access.rs:461-526`) walks breadth-first from the
requesting device DID back toward the space subject, at each hop consulting
both the subject-specific and the subject-open (powerline) index and splicing
matches into one chain (`MAX_DEPTH = 10`). The gateway
(`rust/dialog-remote-ucan-s3/src/authorizer.rs:289-307`) requires only that the
presented chain terminate at whoever signed the invocation — the same device
signer the walk starts from — so the two agree by construction.

Consequence: a claim can audience the root DID and save `space -> eph -> root`
as-is. The device's own subject-open `root -> device` delegation — already saved
into the **access store** at stage-2 link time
(`rust/tonk-worker/src/router/account.rs:124-132`) — is stitched in
automatically, yielding `space -> eph -> root -> device` at presign. No
pre-composition, no new storage artifact. The composition primitive is already
unit-tested at `rust/tonk-identity/src/delegation.rs:72-93`.

The one hard requirement this rests on: the `root -> device` delegation must
live in the access store (via `profile.access().save(...)`), not merely at the
credential site `tonk-account-link-v1`. Stage 2 already saves it to both.

## Design

### The member-DID resolver

One helper, reused by every writer:

```
account_root_did(profile) -> Option<Did>
```

It reads the stored `root -> device` chain (credential site
`tonk-account-link-v1`, mirroring the existing `load_link`/`get` in
`rust/tonk-worker/src/router/account.rs:61-101`) and returns `chain.issuer()` —
the root DID. Returns `None` when no account is linked or the stored link is
malformed. Every call site resolves its member DID as:

```
let member = account_root_did(&profile).unwrap_or_else(|| profile.did());
```

Fail-safe by design: a missing or broken link falls back to the device DID, so
the device keeps working exactly as an unlinked device would.

### Claim path (`join.rs`)

1. Resolve `member` (root DID if linked, else device DID).
2. `invite.claim(&member)` — audiences `member`. The ephemeral invite signer
   can delegate to any DID; the audience never signs, so no root key is needed
   at claim time.
3. Save the claimed `space -> eph -> root` chain to the access store, as today.
   Presign's BFS composes it with the local `root -> device` — no change needed
   to the save.
4. `Membership::new(member, repo.did())`, `MemberName`, and the first-wins
   `MemberRole`/`InvitedVia` stamps all key on `member` (they already derive
   from `membership.this()`, which is content-derived from `(subject, member)`).

For account-holders, claim and founder/create always operate on a **fresh**
`(subject, member)` pair, so root-keying introduces no duplicate rows.

### Founder / create path (`repository.rs`)

Same resolver: `Membership::new(member, repo.did())` and the founder
`MemberRole` stamp key on the root DID. The founder's own access to the created
space is unaffected — it flows through the separate `repo -> operator`
delegation, not the membership fact.

### Rename is out (stage 3B)

`rust/tonk-worker/src/router/profile_name.rs:233` sweeps *every existing*
membership. Switching it to root-keying before migration exists would write a
new root-keyed row alongside the surviving device-keyed row in every space —
duplicate rosters. Rename's root switch is entangled with the retract-old-row
primitive that stage 3B introduces, so it rides with 3B. Until then rename
continues to key on the device DID.

**Known 3A limitation (until 3B):** because rename still keys on the device
DID, an account-holder who *renames* does not update their displayed name in
spaces they joined **as an account-holder**. Those rows are root-keyed (claim
stamps the initial name correctly at claim time), so a later rename writes a
`MemberName` against a device-keyed membership entity that has no `Membership`
row — `build_repository_info`'s `names_by_membership` lookup never matches it,
and the name silently does not change. This is a no-op, not data loss or a
crash; the initial name is correct, only subsequent renames are affected, and
only for the new account-holder cohort. `publish_self_identity`'s device-DID
overlay sigil is a lesser cosmetic sibling of the same gap. Both are resolved
by 3B's migration (which re-keys and retracts). Tracked, not a blocker for 3A.

### Cross-device access

**Backup on claim.** After a successful account-holder claim, push the claimed
space's delegation to the account-service `/chains/put` endpoint. The backup
artifact is a small struct — the `space -> eph -> root` chain **and** the
invite's `remote_url` — because the chain alone does not carry where the space
syncs from, and restore needs it to mount the replica. Best-effort: a backup
failure logs and does not fail the claim (the local device already works).

The call is a **device-signed** UCAN invocation (issuer = device, subject =
root, `root -> device` attached as a proof), the shape the `/chains/*` handlers'
`authorize` requires. The device signer comes from
`profile.signer().signer().clone()`; it drops straight into
`InvocationBuilder::issuer(...)`, the same shape `tonk-identity`'s root builder
already uses, and signs fine even with the non-extractable WebCrypto key on web.
No dialog dependency change is needed. The account-service base URL is resolved
from the worker's own host (mirroring the page's refuse-by-default map); an
unresolvable host skips backup rather than failing.

**Restore is the immediate follow-up PR.** A later device pulls the backed-up
artifacts (`/chains/list` + `/chains/get`), and for each one re-runs the join's
replica-mount using the recovered `remote_url`, saves the delegation to the
access store (the local `root -> device` completes it via BFS), and records the
roster rows under the root DID. Deferred here to keep this PR reviewable;
because backup runs now, the artifacts are waiting when restore ships.

### Coexistence

The device-only claim path is byte-for-byte unchanged (`account_root_did`
returns `None`). An account-holder who re-touches a space they joined *before*
getting an account still sees the old device-keyed row until stage 3B migrates
it; this is the coexistence the parent design accepts on purpose.

## Testing

- **Unit:** `account_root_did` returns the root when linked, the device DID
  when unlinked, and the device DID when the stored link is malformed.
- **Integration (extends the stage-2 CDP harness):** an account-holder claims
  an invite; the roster row keys on the root DID; the claimed chain lands in the
  account service's `chains` store (a `/chains/list` shows the key).
- **Regression:** the device-only claim path (no account) produces the same
  chain and roster row as today.

## Risks

- **Duplicate rows before 3B.** An account-holder re-touching a pre-account
  space creates a coexisting root-keyed row. Bounded and expected; 3B's
  migration retracts the device-keyed rows.
- **Malformed link fallback masks account state.** If the stored `root ->
  device` link is unreadable, the device silently behaves as unlinked. This is
  the safe failure, but it must be logged so a broken link is diagnosable.
- **Backup without restore (interim).** Until the restore follow-up ships, a
  claimed space is backed up but a second device does not auto-mount it. No data
  is lost — the artifacts accumulate server-side — and the claiming device works
  fully.

## Build order

One PR off `origin/staging`:

1. `account_root_did` / `member_did` resolver.
2. Claim and founder/create key on the resolved member DID.
3. Device-signed account-service invocation builder (`tonk-identity`).
4. Chain backup on claim (`/chains/put`), best-effort, worker-side URL resolver.
5. Unit + integration tests; device-only regression.

Restore (`/chains/list` + `/chains/get`, replica mounting, roster re-record,
link/startup hooks) is the immediate follow-up PR.

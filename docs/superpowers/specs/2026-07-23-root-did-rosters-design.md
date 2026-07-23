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

**In:** account-holders' claim and founder/create writes key on the root DID;
a newly claimed space converges across the user's devices (chain backed up on
claim, restored on other devices).

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

### Cross-device access

**Backup on claim.** After a successful account-holder claim, push the
`space -> eph -> root` chain to the account-service `chains` table. The put/get
API exists from stage 1
(`rust/tonk-account-service/src/handlers/chains.rs`, `store.rs`); stage 2 wired
the client authentication. Best-effort: a backup failure logs and does not fail
the claim (the local device already works).

**Restore.** On device link, and on startup for an already-linked device, pull
the account's backed-up chains and save each to the local access store. The
local `root -> device` completes each one via BFS, so the second device can
sync the space. Best-effort and fail-open, matching the account-service posture
— account-service downtime never blocks local work or sync.

### Coexistence

The device-only claim path is byte-for-byte unchanged (`account_root_did`
returns `None`). An account-holder who re-touches a space they joined *before*
getting an account still sees the old device-keyed row until stage 3B migrates
it; this is the coexistence the parent design accepts on purpose.

## Testing

- **Unit:** `account_root_did` returns the root when linked, the device DID
  when unlinked, and the device DID when the stored link is malformed.
- **Integration (extends the stage-2 CDP harness):** an account-holder claims
  an invite; the roster row keys on the root DID; a second linked device
  restores the backed-up chain and syncs the space.
- **Regression:** the device-only claim path (no account) produces the same
  chain and roster row as today.

## Risks

- **Duplicate rows before 3B.** An account-holder re-touching a pre-account
  space creates a coexisting root-keyed row. Bounded and expected; 3B's
  migration retracts the device-keyed rows.
- **Restore staleness.** A space claimed on device A appears on device B only
  after B's next restore (link or startup). Acceptable for 3A; a live follow
  can come later.
- **Malformed link fallback masks account state.** If the stored `root ->
  device` link is unreadable, the device silently behaves as unlinked. This is
  the safe failure, but it must be logged so a broken link is diagnosable.

## Build order

One PR off `origin/staging`:

1. `account_root_did` resolver.
2. Claim and founder/create key on the resolved member DID.
3. Chain backup on claim.
4. Restore on link/startup.
5. Unit + integration tests; device-only regression.

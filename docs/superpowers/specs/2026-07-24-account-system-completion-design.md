# Account system completion

Program design for finishing the cross-device identity and account system.
Written 2026-07-24 from an audit of everything landed or in flight against
the approved design (`2026-07-17-identity-accounts-design.md`, PR #618).
Each remaining stage below is one PR with its own implementation plan;
plans for stages H, R, D, and C exist alongside this spec.

## Where the program stands

The master design named four build stages. As built:

| Stage | Delivered by | Status |
|---|---|---|
| 1 — account service skeleton | #625 (worker + D1 + R2 + Resend), config folded in via #637 | merged |
| 2 — client ceremonies | #623 (identity crate), #624 (rp id), #627 (browser ceremonies), #628 (CLI handoff) | merged |
| 3A — root-DID rosters + claim backup | #637 | merged |
| 3A' — cross-device restore + created-space backup | #638 | merged |
| 3B — roster migration, re-anchor, rename fix | #639 | in CI |
| 4 — billing and entitlements | — | not started |

Also relevant in flight:

- **#618** — the master design doc PR. Draft, behind staging, zero review
  threads. Needs a light as-built refresh (stage table above, restore as its
  own PR, revocation re-sequenced per this spec) then undraft and merge.
- **#635** — dialog bump with the `dialog.* → db.*` attribute rename sweep.
  Its sweep predates #637–#639, so the roster/migration fact writes in
  `tonk-worker` (roster rows, member name/role/provenance, any
  `dialog.origin/*` reads) must be re-swept by whichever side merges second.
  `schema::Origin → schema::Replica` also touches `tonk-worker`.
  `rust/tonk-account-service` is unaffected (HTTP/D1/R2, no dialog
  attributes).

## What is done and verified

Account creation (email code → passkey/PRF → root-signed `account/create`),
browser self-link, CLI browser handoff, device register/list/revoke
endpoints, chain backup (`/chains/put|list|get`), claim/created-space
backup, cross-device restore, roster re-key + capability re-anchor on link,
and the rename fix. Native test coverage is solid (unit + one full
HTTP-ceremony integration test); a real CDP virtual-authenticator passkey
e2e exists (`tonk-ui/src/identity.rs`, gated on `web-integration-tests`).

## The gap map

Ordered by risk, not by the original stage numbering.

### R — Revocation enforcement (security gap, highest priority)

`root → device` delegations are subject-open (`Subject::Any`) and carry
**no expiration** (`tonk-identity/src/delegation.rs:12`). `POST
/devices/revoke` flips a D1 status flag that only the account service
itself consults (`tonk-account-service/src/auth.rs`). The access-service
presign path (`tonk-access-service/src/handlers/ucan.rs`) does purely
cryptographic chain verification — **a revoked device keeps full storage
access forever**. Until this lands, revocation is cosmetic and a lost
device is a permanent co-owner of every space it could reach.

Decision — pulled forward and decoupled from billing (the master spec
paired them; the registry check is the only kill-switch we have, and
device-management UX is meaningless without it):

- The access-service gains a **read-only D1 binding to the accounts
  database** (same Cloudflare account; `tonk-accounts` /
  `tonk-accounts-staging`). No HTTP hop to the account service on the hot
  path, exactly the "one indexed lookup" shape the master spec prescribes.
- **Dual match**: parse the presented invocation container and collect
  both the CID of every proof delegation and every issuer DID (delegation
  issuers + the invocation issuer); reject with 403 if any matches a
  `devices` row with `status = 'revoked'`. The DID match is the
  security-bearing one — re-anchored chains flow through delegations
  *issued by* the revoked device whose CIDs the registry never saw, and a
  revoked device can mint fresh delegations. Verified feasible with public
  dialog APIs against the pinned dialog tag (no upstream change needed);
  unregistered device-only users produce no matches and are untouched.
- **Fail-closed** on D1 error/outage, softened by the per-isolate verdict
  cache: a verdict is authoritative for 60 s and then usable for a further
  10 minutes *only* to cover an unreachable registry. Credentials the
  registry cleared recently ride out a blip; anything it has not cleared
  inside the grace window gets `503 REVOCATION_UNAVAILABLE` (retryable,
  distinct from the 403). A failed query is never cached, so grace is
  anchored to the last successful lookup and repeated failures cannot walk
  it forward. Unparseable container and missing `ACCOUNTS_DB` binding are
  also 503 — a misconfigured binding now stops presigns loudly instead of
  silently disabling enforcement.
- The lookup seam is written so billing later adds its entitlement join
  without touching the handler again.

Superseded decision, recorded because the reasoning matters: R shipped
(#641) with a **fail-open** posture, on the master spec's availability
grounds. That trades the security property for uptime in exactly the
scenario where it is load-bearing, and makes enforcement a function of
config correctness — a renamed binding disabled it with only a log line.
The grace window buys back the availability that fail-open was protecting
without making the guarantee conditional.

### S — Signed revocation artifacts (R follow-on)

R's registry check works, but a `devices` row is not a revocation — it is
a status flag whose authority is "whoever can write this database". Two
consequences. There is no cryptographic trail: a D1 write (ours, or an
attacker's) can revoke or un-revoke with nothing to audit and nothing to
verify against. And there is nothing portable: the day enforcement has to
happen anywhere but this one worker — peer sync, the FS remote, the CLI,
a second service — there is no artifact to hand it, only a database
credential to spread around.

UCAN's answer is a signed revocation: an invocation with command
`ucan/revoke` naming the CID of the revoked delegation, optionally with a
`path` witness, issuable by any issuer in that delegation's proof chain.
The set is monotone and append-only, which is what makes it safe to
gossip and to cache: you can only ever learn more revocations, so no
distribution failure produces a wrongly-permissive answer.

Decision — mint the artifact, keep the index:

- `POST /devices/revoke` additionally records a signed `ucan/revoke`
  invocation in the existing R2 chains store. D1 `status` stays exactly as
  it is and remains what the access-service hot path reads: this adds an
  auditable artifact, it does not add a hop to the presign path.
- The access-service is unchanged for now. The artifact's value is that a
  *second* enforcement point becomes possible without extending the D1
  binding to it.

The open question that decides the shape, and the reason this is a stage
rather than a patch: **only the root can sign it.** The issuer of
`root → device` is the root, so under UCAN semantics only the root (or a
principal holding a delegated `ucan/revoke` capability) may revoke it. The
account service does not hold the root key — it is derived from the
passkey PRF on the user's own device and never leaves it. So either
revocation becomes a root ceremony (passkey prompt, and a device without
the passkey can no longer revoke — a UX regression that is also a real
tightening), or the root delegates `ucan/revoke` to each device at link
time (keeps today's UX, and means a stolen device can revoke its
siblings). Pick before writing code. Plan:
`2026-07-24-signed-revocation-artifacts.md`.

Verified against the pinned dialog tag: `dialog-ucan-core` has no
revocation type at all. Nothing upstream is needed — a revocation is an
ordinary `Invocation` with command `["ucan", "revoke"]` and the CID in
its arguments — but nothing upstream *verifies* it either, so the
semantics live in this repo until dialog grows them.

### H — Service hardening (tracked debt from #625/#628)

The first batch shipped in #640 (expiry enforcement, sanitized error text,
the foreign-key pragma, the two negative tests). One item remains, added
after R landed:

- **Session delegations.** `root → device` is subject-open, command-open
  and unexpiring — the most powerful thing the root can sign, held
  indefinitely by every linked device. That shape is what forces
  revocation to be a lookup: with no expiry there is nothing to *not*
  renew. Split it in two. The `root → device` grant stays as it is (it is
  what survives offline and what the registry records), but it is used to
  mint a short-lived `device → session` delegation, and the session key is
  what signs presign invocations. Revocation then degrades gracefully:
  the registry check stops new sessions immediately, and an unreachable
  registry costs at most one session lifetime rather than unbounded
  access. This is the UCAN spec's own preference — *"Revocation SHOULD be
  considered the last line of defense against abuse. Proactive expiry
  through time bounds or other constraints SHOULD be preferred."*
  Open questions for the plan: session TTL (hours, not minutes — it must
  survive an offline stretch), whether renewal is a new account-service
  endpoint or the device self-mints from the stored `root → device` grant
  (self-minting needs no network and is preferred if the access-service
  can enforce the expiry), and whether the access-service should *require*
  a bounded invocation rather than merely accepting one, which is the
  breaking half and wants a soak first.

Shipped in #640:

- Enforce **expiry on device-signed invocations** in `authorize` (the root
  path already enforces a 5-minute window; the device path checks nothing).
  Clients must stamp expirations first — verify `build_device_invocation`
  and add one if missing. Decision: **no nonce/replay table** — every
  device-authorized endpoint is an idempotent upsert or read, so replay
  within the window is harmless; recorded as an accepted risk.
- `PRAGMA foreign_keys = ON` in the sqlite test store (D1 parity — absent
  today, confirmed).
- Error-text sanitization: `ServiceError::to_response` serializes internal
  error strings (store/R2 details) to the wire on 500s. Log the detail,
  return a generic message; 4xx ceremony messages stay.
- The two missing negative tests from the #625 review: the
  device-belongs-to-a-different-account **filter** branch in `authorize`
  (valid chain, registered device, mismatched account), and cross-account
  `/chains/get` isolation.
- Ops (runbook, not code): confirm `0002_link_requests.sql` is applied to
  staging and production D1, and extend the existing `/codes` rate rule to
  exact-path `POST /links` (deploy prerequisite stated in #628). Decision:
  Cloudflare zone rate rules only; **Turnstile deferred** until abuse is
  observed.

### D — Device management surface

`/devices/list` and `/devices/revoke` have **no consumer** — no UI panel,
no CLI verbs. Ships after R so revoke is real:

- `tonk-ui` account element: a devices panel (list with name, created,
  status, "this device" marker; revoke with confirm). Reuses the existing
  device-signed invocation plumbing via a new worker route.
- Worker: `GET /api/account/devices`, `POST /api/account/devices/revoke`
  (device-signed invocations built from the stored link), plus a local
  **unlink** (`DELETE /api/account`) that clears the stored `root → device`
  link — self-service "sign out of the account on this device".
- CLI: `tonk account devices`, `tonk account revoke <device-did>`.

### C — Recovery and rotation ceremonies

Specified in the master design ("ride with stage 2 or immediately after")
but never built. None of the three exist in code. Build order within the
stage:

1. **Deliberate rotation / succession** — derive old root, mint
   subject-open `oldRoot → newRoot`, re-assert rosters under the new DID.
   The 3B migration sweep (`migrate.rs`) already re-keys rosters
   device→root; succession generalizes the same machinery to
   oldRoot→newRoot. Account service: root-signed flip of `root_did` +
   `credential_id` on the account row.
2. **Surviving-device recovery** (passkey lost, linked device in hand) —
   new passkey on the tonk origin, surviving device mints
   `device → newRoot`, two-signature ceremony (device-signed under the old
   root + root-signed by the new root) flips the account row and revokes
   the old credential's devices.
3. **Total-loss re-anchor** — email code + support contact re-points the
   account row at a fresh root DID; billing/entitlements carry, space
   access does not. Rate-limited, logged. Needs a support posture more than
   code; last.

### B — Billing and entitlements (master stage 4)

Not started; nothing in any crate references Stripe, plans, or
entitlements. Scope per the master spec: Checkout Session + Customer
Portal, webhook reducer into an `entitlements` table, requester-based
gateway lookup (join on the same D1 binding stage R introduces), free tier
for unknown DIDs, lapsed = free-tier limits, **log-only soak before
enforcement**.

**Blocked on user inputs, deliberately unplanned tonight:** Stripe account
and mode, product/price catalogue, concrete free/paid limits (space count,
storage bytes, bandwidth?), and whether staging gets its own Stripe
sandbox. Write the plan (repo format) once those exist; the enforcement
seam is already reserved in stage R.

### F — Functional follow-ups carried by the 3A/3B/restore specs

Small, independent, each its own commit-or-PR; none block the stages
above:

1. `put_repository` one-shot create-with-remote bypasses
   `enable_sync_inner`, so such spaces are never backed up or restorable.
   Hook the backup (recover the raw sync URL from the parsed
   `SiteAddress`).
2. Native/CLI created-space backup if the `ensure_remote_config` hook
   remains wasm-only (CLI-created spaces currently un-backed-up).
3. Live restore/migration propagation — today both run on link/startup
   only; add a periodic or sync-signal refresh so a space claimed on
   device A appears on device B without a restart.
4. `InvitedVia` self-claim detection is device-scoped (flagged in #637;
   cross-device provenance once restore is live).
5. Account-service URL resolution hardcoded to
   `tonk.spot`/`staging.tonk.xyz` in `account_backup.rs` — fine until a
   third environment appears; fold into config when one does.
6. Deferred by explicit earlier decision, unchanged tonight: public
   root-signed device manifest (privacy semantics unsettled); adopting the
   root DID as the local profile DID.

Accepted, not to be fixed (documented behavior): rename-during-pending-
migration race (bounded, self-heals on next rename); re-anchored chains
flowing through the old device DID until fresh invites (inherent to the
design; revoking that device severs them — R makes this visible, D's UI
copy should say so).

### T — Test and verification debt

- The CDP passkey e2e and the wasm service-worker suites have **never run
  locally** (macOS 27 libffi blocker; wasm tests hang locally) — they rely
  on the CI web leg. Verify the `web-integration-tests` feature is actually
  exercised in CI; wire it if not.
- Manual staging smokes still owed: worker → service `/chains/put` (flagged
  in #637), migration sweep + restore on a real two-device account
  (flagged in #639).
- Bench scenarios from the master spec's testing section (creation,
  self-link, surviving-device recovery) once C lands.

## Sequencing

```
H (#640, merged) ∥ R (#641, merged)
  → R' fail-closed + grace window (access-service)      [immediate]
  → H' session delegations (identity + both services)   [R' ∥ H' after R']
  → D (device management: worker + UI + CLI)
  → S (signed revocation artifacts; needs D's revoke UX decision)
  → C (recovery ceremonies, three sub-stages)
  → B (billing; blocked on user inputs)
F follow-ups and T debt interleave as small PRs whenever convenient.
```

R' is the correction to R's availability posture and carries no
dependencies. H' (session delegations) should land before D so the device
panel documents a bounded credential rather than an unexpiring one. S
follows D because the root-vs-delegated-`ucan/revoke` decision is really a
question about the revoke UX, and D is where that UX gets designed.

H and R are planned in full (`2026-07-24-account-service-hardening.md`,
`2026-07-24-revocation-enforcement.md`); S is planned in
`2026-07-24-signed-revocation-artifacts.md`. H' needs a plan before
execution — the open questions in its bullet decide the task list. D and C
have plans at the same paths pattern (`2026-07-24-device-management.md`,
`2026-07-24-recovery-ceremonies.md`) with verify-first gates where dialog
APIs need confirmation. B gets its plan after the Stripe decisions.

## Risks

- **Revocation matching depends on parsing the presign container** —
  resolved: verified against the pinned dialog tag that the `/ucan/` body
  re-parses with public APIs (`Container::from_bytes` → invocation +
  delegations, `to_cid()`/`issuer()` reachable, CID string parity with the
  registry). The plan keeps a re-runnable verify task so a future dialog
  pin drift surfaces as a STOP, not a silent break.
- **Enforcing device-invocation expiry can strand old clients.** All
  senders of device-signed invocations live in the worker (deployed with
  the service) — verified, not assumed, in the hardening plan. CLI link
  flow uses bearer-token endpoints and is unaffected.
- **Fail-closed revocation can refuse legitimate presigns.** A D1 outage
  longer than the 10-minute grace window, or one that starts before a
  client's credentials have ever been screened by the isolate serving it,
  returns 503 and stops sync for those requests. This is the deliberate
  inversion of R's original posture: the grace window is sized so a blip
  costs nothing and a sustained outage degrades rather than silently
  unenforces. Two things must hold for that trade to stay sound — clients
  must treat 503 as retryable (verify before relying on it), and the
  grace window must never be extended by failed queries (pinned by
  `it_does_not_slide_the_grace_window_on_repeated_failures`).
- **Revocation authority is a database write, not a signature**, until S
  lands. Anyone who can write `ACCOUNTS_DB` can revoke or un-revoke with
  no audit trail, and no other enforcement point can check revocation
  without that same credential.
- **Stage B scope creep.** The entitlement seam in R must stay a seam;
  billing lands only after the log-only soak the master spec requires.

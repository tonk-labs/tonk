# Renewable guest sessions implementation plan

**Goal:** Keep an accountless guest's remote pull, push, and status operations authorized for as long as the retained open invite remains valid, without weakening the one-hour bound on any individual guest delegation.

**Approach:** Treat the retained open-invite URL as the renewable bearer authority and persist which ephemeral operator each guest delegation targets plus its effective expiry. Before any sync path presigns, rotate to a fresh operator when either the normal signing session or any renewable guest delegation is due, replay every current guest invite onto that operator, persist the replacement chains and metadata, and only then swap the worker state to the new operator.

**Constraints:**

- Guest access remains accountless and full read/write; this fix must not add a passkey, root membership, account-service request, or account attachment.
- `Invite::visit` remains bounded to `VISIT_TTL_SECONDS` (one hour). Renewal mints another bounded delegation from the retained bearer invite; it must not turn a guest into durable membership or extend beyond an expiry already present in the original invite chain.
- The access service remains the enforcement boundary for expiry and revocation. No account-service, access-service, invite URL, or remote wire-protocol change is required.
- Never save a replacement guest delegation under the same operator audience. The certificate store retains old chains and its proof walk does not reject expired candidates, so same-audience renewal would intermittently present the stale chain.
- One operator is shared by all mounted repositories. Any operator rotation must therefore replay every still-valid guest invite, not only the guest that triggered renewal; durable spaces continue through their existing `space -> root -> device -> operator` proof path.
- A service-worker restart creates a new operator. A retained guest record whose recorded audience differs from the current operator is due immediately, regardless of its recorded expiry.
- Existing `tonk-guest-invite-v1:<repo-key>` records must remain readable. Missing audience/expiry metadata is interpreted as legacy state and forces one immediate refresh; successful refresh rewrites the record in the new format.
- Refresh is local: parsing the retained URL, importing its embedded invite signer, minting the delegation, and retaining it must not contact the account or access service. The next ordinary remote operation still performs the normal expiry and revocation checks.
- Rotation must be state-safe. Re-read the guest set while holding the exclusive `TonkState` lock so a guest join that completed after the initial due check is included before the operator swap. Prepare and retain every replacement chain before swapping `tonk.operator`.
- If opening the candidate session or retaining any replacement chain fails, leave the current operator and guest metadata unchanged and retry on a later sync trigger. Partially retained chains for an unused candidate audience are harmless. If metadata persistence fails partway, do not swap; the candidate-audience mismatch in any written record keeps the next attempt due.
- A malformed retained record is existing local corruption, not a reason to delete credentials or invent authority. Report it through the existing error/logging path and preserve it; do not let it authorize a remote request.
- Do not add sleeps to tests. Drive the pure expiry predicate with explicit timestamps and force stored metadata into the renewal margin for integration coverage.
- Do not change `Cargo.lock`; the required signing, parsing, storage, and query APIs are already present.
- The current filesystem is at 100% usage and the access-service check already failed with `No space left on device`. Reclaim space with the user's approval before running the full native/WASM verification matrix; do not silently delete build artifacts.

## File map

- `rust/tonk-worker/src/router/join.rs`: versioned retained-guest record, guest enumeration, centralized bounded grant mint/retain helpers, and initial-visit metadata persistence.
- `rust/tonk-worker/src/router/sync.rs`: due detection, batch operator rotation, synchronous renewal at every remote sync/status boundary, and renewal integration tests.
- `rust/tonk-worker/README.md`: document that guest grants are locally renewed from the retained open invite and remain independently expiry/revocation checked.
- `rust/tonk-invite/src/lib.rs`: unchanged source of the one-hour `VISIT_TTL_SECONDS` bound and `Invite::visit` implementation.
- `rust/tonk-access-service/src/expiry.rs`: unchanged expiry enforcement; its existing tests remain part of broader verification.

### Task 1: Persist enough guest-session metadata to renew safely

**Files:**

- Modify: `rust/tonk-worker/src/router/join.rs:GuestRecord, save_guest, guest_url, save_authority`
- Test: `rust/tonk-worker/src/router/join.rs:tests`

**Interfaces:**

- Consumes: the existing `tonk-guest-invite-v1:<repo-key>` credential site, `PreparedJoin.url`, `PreparedJoin.invite`, `Invite::visit`, and the current `tonk.operator.did()`.
- Produces:

```rust
pub(crate) struct GuestLease {
    pub subject: Did,
    pub url: String,
    pub audience: Option<Did>,
    pub expires_at: Option<u64>,
}

pub(crate) struct GuestGrant {
    pub chain: DelegationChain,
    pub audience: Did,
    pub expires_at: u64,
}

pub(crate) async fn guest_leases(
    tonk: &TonkState,
) -> Result<Vec<GuestLease>, TonkWorkerError>;

pub(crate) async fn mint_guest_grant(
    invite: Invite,
    audience: &Did,
) -> Result<GuestGrant, JoinFailure>;

pub(crate) async fn retain_guest_grant(
    tonk: &TonkState,
    operator: &DefaultOperator,
    grant: &GuestGrant,
) -> Result<(), JoinFailure>;

pub(crate) async fn save_guest(
    tonk: &TonkState,
    operator: &DefaultOperator,
    subject: &Did,
    url: &str,
    grant: &GuestGrant,
) -> Result<(), JoinFailure>;
```

- [ ] Add `it_reads_a_v1_guest_record_as_a_legacy_lease`. Save the current `{version: 1, url}` bytes, load them through the production decoder, and assert the URL survives while `audience` and `expires_at` are `None`; unsupported versions and a v2 record missing either field must return a typed internal/claim failure rather than panic.
- [ ] Add `it_persists_the_guest_grants_actual_audience_and_expiry`. Visit an open invite, commit it through the existing guest path, reload the credential record, and assert version 2, the exact current operator DID, and the delegation chain's effective expiration rather than an independently calculated `now + 3600` value.
- [ ] Add `it_enumerates_only_repository_replicas_with_guest_records`. Seed two guest replicas, one durable replica, the profile replica, and the account replica; assert `guest_leases` returns the two guest subjects in stable subject order and never treats an absent/cleared guest site as a guest.
- [ ] Run `nix develop . -c test:web:debug`; expect the new tests to fail because the v2 metadata and helper interfaces do not exist. If disk pressure prevents the run, record `No space left on device` as an environmental block rather than treating it as the expected red result.
- [ ] Replace the write-only `GuestRecord { version: 1, url }` shape with a decoder that accepts version 1 as missing metadata and writes version 2 as `{version, url, audience, expires_at}`. Parse the audience string as a `Did` on load and reject unsupported versions or incomplete v2 records.
- [ ] Refactor `guest_url` to project from the shared record loader so membership status and promotion preserve their current behavior. `clear_guest` continues to write empty bytes and remains the only way promotion removes guest state.
- [ ] Implement `guest_leases` by querying the profile `main` branch for `Replica::repository_kind()` rows belonging to `tonk.profile.did()`, loading each subject's guest credential site, dropping absent/cleared sites, and sorting by subject DID for deterministic batch behavior.
- [ ] Centralize guest minting in `mint_guest_grant`: call `Invite::visit(audience)`, require an effective chain expiration, and return the exact audience and expiration alongside the chain. Initial visit staging and durable commit must use the same helper so tests and renewal cannot drift in TTL or chain construction.
- [ ] Change `save_authority` to return `Option<GuestGrant>` after the chain is retained. Pass that result to `save_guest`, which writes the matching v2 record only for `JoinMode::GuestVisit`; durable join/promotion behavior remains unchanged.
- [ ] Implement `retain_guest_grant` as the chain-retention half and `save_guest` as the metadata-persistence half. Initial visit calls them in that order. Renewal can retain the complete batch first and write the complete metadata batch second, avoiding a state swap while any guest still lacks a chain for the candidate audience. Do not log or return the bearer URL.
- [ ] Run `cargo test -p tonk-invite it_visits_with_bounded_session_authority_without_changing_the_invite`; expect the unchanged one-hour bound test to pass.
- [ ] Run `nix develop . -c test:web:debug`; expect all guest join, promotion, and new metadata tests to pass.

### Task 2: Renew all guest authority before any remote sync operation

**Files:**

- Modify: `rust/tonk-worker/src/router/sync.rs:drain_sync, renew_session, pull, push, sync, sync_status`
- Test: `rust/tonk-worker/src/router/sync.rs:tests and overlay_tests`
- Modify: `rust/tonk-worker/README.md:Browser contracts or a new Guest authority subsection`

**Interfaces:**

- Consumes: `session::needs_renewal`, `session::open`, `session::now`, `join::guest_leases`, `join::mint_guest_grant`, `join::retain_guest_grant`, and `join::save_guest` from Task 1.
- Produces:

```rust
pub(crate) const GUEST_RENEWAL_MARGIN_SECONDS: u64 = 5 * 60;

pub(crate) fn guest_needs_renewal(
    lease: &GuestLease,
    current_audience: &Did,
    now: u64,
) -> bool;

pub(crate) async fn ensure_session_authority(
    state: &AppState,
) -> Result<(), TonkWorkerError>;
```

- [ ] Add pure predicate tests: `it_keeps_a_guest_outside_the_five_minute_margin`, `it_renews_a_guest_inside_the_margin`, `it_renews_legacy_guest_metadata`, and `it_renews_an_audience_mismatch_after_worker_restart`. Use fixed timestamps and assert the exact inclusive boundary `now + GUEST_RENEWAL_MARGIN_SECONDS >= expires_at`.
- [ ] Add `it_rotates_and_rebinds_a_guest_before_expiry` in the service-worker test module. Create a local open-invite guest, rewrite only its stored v2 expiry into the margin, call `ensure_session_authority`, and assert: the operator DID changed; the guest record names the new DID; its expiry moved forward; and `profile.access().prove(subject).audience(new_operator)` succeeds with a chain whose effective expiry equals the rewritten record.
- [ ] Add `it_rebinds_a_fresh_guest_after_operator_restart`. Create a guest, open a new session over the same profile/storage and install that operator into the test state without touching the guest record, call `ensure_session_authority`, and assert the audience mismatch forces replay even though the recorded guest expiry is still far away.
- [ ] Add `it_rebinds_every_guest_when_one_guest_forces_rotation`. Create two guests plus one durable space, put only the first guest inside the margin, renew once, and assert both guest records target the same new operator and all three subjects can produce proofs for it. Assert exactly one operator rotation, not one per guest.
- [ ] Add `it_does_not_rotate_a_healthy_operator_or_guest`. Call the ensure function twice with all expiries outside their margins and assert the operator DID and guest record bytes remain identical.
- [ ] Add `it_keeps_the_current_operator_when_guest_replay_fails`. Create a valid guest, rewrite its retained v2 record with a syntactically invalid invite URL and an expiry inside the margin, call `ensure_session_authority`, and assert it returns an error without changing `tonk.operator`, `session_expires_at`, or the stored guest bytes. Preserve the malformed record for diagnosis; do not add a generic storage abstraction solely to inject a later write failure.
- [ ] Run `nix develop . -c test:web:debug`; expect the renewal tests to fail because current `renew_session` only considers `session_expires_at` and never replays retained guest invites.
- [ ] Implement `guest_needs_renewal`: return true for v1 metadata, an audience different from the current operator, or an expiry inside the five-minute margin. When deciding whether a valid invite can trigger automatic renewal, respect an earlier expiration in the original invite chain; an already expired parent invite cannot be made valid by another guest hop.
- [ ] Replace `renew_session` with `ensure_session_authority`. Its initial read phase loads all guest leases and returns immediately unless the normal session or at least one renewable guest is due. Open the candidate `Session` outside the state lock.
- [ ] Acquire the exclusive state lock, verify `session_expires_at` still matches the snapshot, and re-read the full current guest set under that lock. Parse and mint a `GuestGrant` for every still-valid guest against the candidate operator, including guests that were not individually due, because the operator is global.
- [ ] Retain every candidate chain first. Then persist every matching v2 record. Swap `tonk.operator` and `tonk.session_expires_at` only after both phases complete successfully. On any storage failure, keep the old operator; do not delete the old chains or guest records.
- [ ] Keep the current best-effort drain behavior: `drain_sync` logs a renewal failure and continues with the still-current session, allowing a later trigger to retry. Also call `ensure_session_authority` synchronously before the first presign in each direct remote boundary: `pull`, `push`, `sync`, and `sync_status`. This closes the service-worker-restart race instead of relying on a debounced drain to win against the request.
- [ ] Preserve offline and paused behavior. Local renewal may run while offline, but no new network request is introduced; a paused branch still performs no remote fetch/pull/push.
- [ ] Document the resulting chain and lifecycle in `rust/tonk-worker/README.md`: each guest hop is one-hour bounded, the retained open invite is replayed locally onto rotated operators, expiry/revocation remain enforced remotely, and explicit promotion is still the only transition to durable membership.
- [ ] Run `cargo fmt --all -- --check`; expect success.
- [ ] Run `cargo test -p tonk-invite`; expect success.
- [ ] Run `cargo test -p tonk-access-service --features helpers expiry`; expect the existing expired-chain rejection tests to pass.
- [ ] Run `cargo test -p tonk-worker`; expect native worker tests to pass.
- [ ] Run `nix develop . -c test:web:debug`; expect all WASM worker tests, including guest join/promotion and renewal coverage, to pass.
- [ ] Run `nix flake check '.'`; expect the repository-defined formatting, lint, native, and build checks to pass. Report any remaining disk-, browser-, Nix-daemon-, or sandbox-bound check separately from code failures.

## Acceptance evidence

- A guest proof is never presented after its recorded one-hour delegation expires: before a remote operation reaches presign, it either uses a freshly rotated operator with a freshly bounded guest chain or reports a renewal/storage failure while retaining the previous state.
- A worker restart followed immediately by `sync`, `pull`, `push`, or `sync_status` replays the retained invite before that operation presigns.
- Rotating for one guest preserves every other guest and every durable space under the single new operator.
- Promotion still clears the guest record only after durable authority commits; a promoted space is not replayed as a guest on later rotations.
- No account-service request, account attachment, new root, permanent guest grant, wire-format change, or lock-file change appears in the diff.

# Durable browser account-setup recovery implementation plan

**Goal:** Recover browser account creation after reload, tab loss, worker restart, and provider response loss from every technically recoverable boundary, while starting at most one WebAuthn creation attempt and describing the one irrecoverable browser boundary honestly.
**Approach:** Put account-setup ordering behind one deep UI module and one worker saga module. The top document remains the WebAuthn adapter, but the worker is the sole persistence and validation authority. It persists a secret-free versioned checkpoint and a separate credential-store-protected recovery bundle containing a passkey-sealed envelope, bounded authorizations, and a durable root-signed anti-mix manifest. It serializes ownership with a profile-scoped Web Lock plus revision checks, reconciles each durable effect, and uses the provider's proof-bound status and exact-replay contract before attaching local account state and queuing customer/custody work.
**Base:** branch `fix/audit-account-setup-recovery`, worktree `/Users/jackdouglas/tonk/tonk/.wt/fix/audit-account-setup-recovery`, exact lower-base head `305a60f610d221bd6c9c65cc9eaf227bbb0a8162`.

**Lower-base adoption:** provider PR #835 was reviewed and adopted at exact head `305a60f610d221bd6c9c65cc9eaf227bbb0a8162`; it supplies `GET /capabilities` v1 plus the setup-status policy correction.

## Lower foundation checkpoint (2026-08-31)

This first stacked slice intentionally stops before production route/effect/UI wiring. It contains the canonical manifest, versioned public wire contract (including the worker-selected ceremony context), private checkpoint/recovery encodings, envelope-first decoding, complete-record reducer, and the single bounded semantic `ValidatedRecoveryBundle` constructor. The constructor independently verifies every signed artifact and classifies current expiry separately from canonical/signature/original-window validity.

A bounded foundation review additionally made version classification depend only on a minimal future-safe version envelope before parsing current tags, made `Arm` compare the worker's re-resolved configuration hash inside the reducer, and gave `Stage` a distinct command that must match the one armed attempt before the hash is cleared.

Focused evidence at this checkpoint:

- `CARGO_INCREMENTAL=0 cargo test -p tonk-account recovery::tests`: 4 passed.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker-api account_setup::tests`: 4 passed.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker router::account_setup::tests`: 12 passed.

Still deliberately absent from this lower commit: credential-site reads/writes, Web Lock and live-`ClientId` ownership enforcement, provider HTTP effects, identity/customer/custody effects, top-document orchestration, UI copy, browser tests, and Storybook changes. Those remain the upper tasks below and must not be inferred from the foundation types alone.

## Production saga checkpoint (2026-08-31)

The next stacked slice starts from the reviewed foundation follow-up `89f9627b3` and wires Tasks 1–4 through the worker boundary without invoking the new flow from `tonk-ui` yet. It adds the pre-submit provider fingerprint and proof-bound status invocation, dedicated credential sites, profile-scoped Web Lock plus per-worker serialization, live-`ClientId`/token/revision fencing, a re-resolved canonical-configuration fence on every protected recovery validation, the bounded `/api/account/setup` route, recovery-before-root staging, status-first exact provider replay, exact local observations, ordered custody work, and complete-before-tombstone cleanup. Test-only store and provider ports cover unauthorized stage attempts, configuration drift, recovery/checkpoint loss, provider response loss, unknown provider outcomes, and tombstone-write loss without exposing protected payloads through `Debug` or logs.

Deliberately deferred to the UI/WebAuthn slice: passing the worker-minted ceremony context into identity, producing the final root-signed manifest from the real browser ceremony, performing the browser same-credential assertion and invoking the worker's phase-specific replacement command, top-document owner/attempt-token orchestration, recovery copy, browser Web Lock races, service-worker critical-section composition, and Storybook. The production route is therefore wired and compile-checked but remains unreachable from existing UI entrypoints in this slice.

Focused evidence after the final parent integration:

- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker router::account_setup::tests --no-fail-fast`: 17 passed, 90 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-identity --lib`: 69 passed.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-account pending::tests --no-fail-fast`: 4 passed, 36 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker-api account_setup::tests --no-fail-fast`: 4 passed, 30 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker router::route_table:: --no-fail-fast`: 2 passed, 105 filtered.
- `CARGO_INCREMENTAL=0 cargo clippy -p tonk-account -p tonk-identity -p tonk-worker --all-targets -- -D warnings` and `CARGO_INCREMENTAL=0 cargo check -p tonk-worker --target wasm32-unknown-unknown`: passed.

## Independent production-saga review remediation (2026-08-31)

The first production wiring commit was held for independent review rather than published. The review found nine recovery gaps, all addressed in this stacked follow-up before UI invocation:

- `Inspect` and `Begin` now classify the checkpoint and recovery site together. A surviving bundle or tombstone behind a missing checkpoint is corrupt and cannot authorize another ceremony; phase/record combinations impossible for the current checkpoint also fail closed.
- Replacement create and publish invocations carry a worker-authored immutable receipt-time reference. Canonical scope, exact semantic arguments, signature, and the original expiry window are validated against that stored reference; current expiry remains the recoverable `NeedsRefresh` state.
- Enrollment advances only after both the device-local `CustomerRecord` and the exact profile-main `AccountCustomer` projection are durable. Projection failures propagate and idempotently replay without moving past `Attached`.
- Every pending-work mutation holds one profile-scoped browser Web Lock and the matching per-worker mutex across load/append-or-drain/save. The ordered custody `[Provision, PublishCustody]` pair is appended in one serialized queue write, and a concurrency contract test permits either lock-acquisition order but no lost or crossed pair.
- Repeated `Begin` by the exact live owner and `Acquire` after a dead pre-arm owner return the original worker-selected non-secret ceremony. A different live client still receives only the redacted in-progress view.
- Provider status remains authoritative and transport-unknown. After exact status `Absent`, an account-create HTTP 409 is classified as the typed terminal provider conflict rather than retried forever; upstream bodies never enter the public response.
- The coordinator rejects a request whose origin is not exactly the worker-global service origin before configuration resolution or any provider fetch. All setup context is derived from that trusted origin.
- `NeedsPasskey` uses a tagged, phase-minimal protected input. `RootSaved` may receive only the fields needed to refresh create; `CustomerEnrolled` receives only operation/credential/root/custody/sealed fields for the custody publish invocation. Provider-accepted and later phases never disclose create-resume material merely because the already-consumed create invocation expired.
- Test clocks plus in-memory store/provider/effect ports now execute the production `handle_locked` and `reconcile_with_provider` seams, including lost-checkpoint, configuration/origin, ownership/takeover, provider-conflict, projection-failure, phase-specific-expiry, and completion/tombstone paths.

This remediation remains deliberately production-uninvoked from `tonk-ui`: the UI/WebAuthn adapter, copy, browser journey tests, service-worker critical-section composition, and Storybook behavior changes remain Tasks 5–7. Native queue tests prove the shared mutation seam and Wasm compilation proves its Web Lock adapter; a two-live-worker browser race remains browser-only evidence for Task 6.

Focused review-remediation evidence:

- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib router::account_setup::tests -- --nocapture`: 28 passed, 91 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib router::customer::tests::concurrent_custody_appends_never_lose_or_cross_the_ordered_pair -- --exact --nocapture`: 1 passed, 118 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker-api --lib account_setup::tests -- --nocapture`: 4 passed, 30 filtered.
- An earlier complete native `tonk-worker --lib` sweep in this remediation passed 111 of 119 inside the restricted sandbox; all eight failures stopped at loopback setup with `Operation not permitted`. The unchanged failing families then passed with loopback access: `router::account_state::tests` 11 passed and `router::http::tests` 3 passed. The final phase-minimal protected-response refinement was rerun through the 28 saga and 4 wire tests above.
- `CARGO_INCREMENTAL=0 cargo clippy -p tonk-account -p tonk-worker-api -p tonk-identity -p tonk-worker --all-targets -- -D warnings`: passed. `AccountSetupResponse::Protected` is boxed to keep the wire JSON unchanged while satisfying the all-target enum-size lint.
- `CARGO_INCREMENTAL=0 cargo check -p tonk-worker --target wasm32-unknown-unknown`: passed, including the shared Web Lock adapter.

## Second production-saga re-review remediation (2026-08-31)

A second independent review held the follow-up because queue durability was
being mistaken for custody execution, replacement receipt times were not
explicitly idempotent, and two claimed production fault tests still exercised
test-only shortcuts. This follow-up tightens those boundaries:

- `CustodyQueued` remains a protected, owner-recoverable phase. Saving the
  ordered pending pair no longer records `Complete` or tombstones recovery.
  `Continue` and an authenticated `Inspect` inspect the exact serialized pair;
  only absence after the durable queued checkpoint (the state produced when a
  successful drain removes the pair) may advance to `Complete` and then the
  tombstone. Unreadable queue bytes fail closed rather than being replaced by
  an empty queue.
- If a deferred publish expires, the queued pair stays in place and the saga
  returns the phase-minimal `ReplacePublishInvocation` input. The replacement
  atomically substitutes only the matching publish entry in its existing
  position behind Provision. If activation already drained the pair, it is not
  re-queued. Recovery is retained until the refreshed publish actually drains.
- Exact lost-response retries of create and publish replacements compare the
  already accepted artifact before assigning a receipt time. Identical bytes
  keep the original immutable receipt and continue idempotent reconciliation;
  only a distinct, independently validated artifact receives the new worker
  receipt time.
- Customer enrollment ordering and exact observation now live behind the same
  production projection port used by `enroll_customer` and the setup saga.
  The fault test lets `CustomerRecord` succeed, fails the profile-main
  `AccountCustomer` write, proves the checkpoint remains `Attached`, and then
  proves the idempotent retry converges both projections before advancing.
- Native concurrency coverage now calls the production pending-queue append
  wrapper with two distinct per-worker mutexes and the same canonical named
  lock. The native test adapter models the browser named-lock registry; no
  test-only external mutex hides a missing or mismatched production lock.

The production assumptions remain narrow: the `CustodyQueued` checkpoint is
proof that the exact pair was saved once; thereafter an absent exact subject is
treated as successful idempotent drain removal. A conflicting same-subject
entry is not absence and fails closed. Browser Web Locks remain mandatory in
production; a real two-service-worker browser race is still deferred to Task 6.

Focused second-review evidence:

- RED: the custody-only-queued reconciliation test observed `Complete` before
  execution; the corrected test retains `CustodyQueued` and the bundle.
- RED: temporarily restoring sliding receipt assignment made the identical
  create retry stop before provider reconciliation and the identical publish
  retry stop before the queue replacement seam.
- RED: treating a surviving device-local customer projection as exact after the
  profile-main projection failed skipped the production retry; the final effect
  order was `customer, custody` rather than `customer, customer, custody`.
- RED: changing the production pending-work lock name to include each worker's
  local mutex identity allowed two concurrent appends to overwrite one ordered
  pair. Restoring the canonical profile-derived name retained both pairs.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib
  router::account_setup::tests -- --nocapture`: 32 passed, 92 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib
  router::customer::tests::production_ -- --nocapture`: 2 passed, 122 filtered;
  these execute the production projection and shared pending-queue mutation
  seams, including the partial-projection fault and two-worker lock race.
- `CARGO_INCREMENTAL=0 cargo clippy -p tonk-worker --all-targets -- -D
  warnings`: passed.
- `CARGO_INCREMENTAL=0 cargo check -p tonk-worker --target
  wasm32-unknown-unknown`: passed in 1m 08s.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

## Final production-saga review remediation (2026-08-31)

The final independent review concentrated on checkpoint-loss and inter-write
states at the production queue, recovery, and customer-projection seams. Four
remaining recovery gaps were closed without changing the public wire schema:

- Retrying the full custody batch after a partial drain now restores any
  missing prerequisite before a surviving later batch entry. In particular,
  `[PublishCustody]` plus a replayed `[Provision, PublishCustody]` converges to
  the exact original order rather than appending Provision behind Publish.
- A queued publish refresh uses a private, bounded replacement intent. The
  worker first saves the fresh exact artifact plus the exact prior invocation,
  then performs the production named-lock queue replacement, and only then
  clears the intent. Failure of the first write leaves the queue untouched;
  failure of the queue or final recovery write leaves enough validated state
  for reload to idempotently repair either the previous or already-current
  queue entry. Intent metadata is excluded from the immutable recovery hash,
  is never sent on the public wire, and carries no new secret material.
- Customer enrollment observation now compares immutable customer identity,
  email, recognized lifecycle statuses, and a syntactically safe HTTP(S)
  provider address. Independently advanced valid statuses are accepted as the
  same enrollment, including the normal local `Registered` plus profile-main
  `Active` state after activation; wrong subjects, unknown statuses, and unsafe
  provider values remain mismatches.
- A re-resolved configuration hash that differs after staging now returns the
  same redacted `UpdateRequired`/`Reload` response used by Begin, Stage, and
  Acquire. The fence runs before reconciliation or replacement persistence, so
  the protected recovery bundle and queued checkpoint remain byte-for-byte
  unchanged instead of becoming a terminal recovery conflict.

Focused final-review evidence:

- RED: the production partial-drain test observed `[PublishCustody, Provision]`
  instead of the original `[Provision, PublishCustody]`; it now passes through
  the production append wrapper.
- RED: injected recovery-save failure previously advanced the pending queue;
  injected queue replacement failure previously left the fresh recovery
  receipt unable to reconcile on reload. Both exact crash-window tests now
  converge while retaining the original receipt time and exact invocation
  bytes.
- RED: local `Registered` plus profile-main `Active` entered a terminal
  mismatch, while a wrong projected customer was incorrectly classified
  `Exact`; the production projection tests now advance the valid state and
  reject invalid identity/provider data.
- The configuration-drift test first exposed an invalid test clock as
  `InProgressElsewhere`; after correcting that precondition it passes through
  the production Continue fence and proves checkpoint and recovery bytes are
  unchanged. The original conflict path was established by the review trace
  from failed recovery validation to `persist_conflict`.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib
  router::account_setup::tests -- --nocapture`: 37 passed, 93 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib
  router::customer::tests -- --nocapture`: 3 passed, 127 filtered.
- `CARGO_INCREMENTAL=0 cargo test -p tonk-account pending::tests --
  --nocapture`: 4 passed, 36 filtered.
- `CARGO_INCREMENTAL=0 cargo clippy -p tonk-worker --all-targets -- -D
  warnings`: passed after boxing the large repair-result variant identified by
  the first lint run.
- `CARGO_INCREMENTAL=0 cargo check -p tonk-worker --target
  wasm32-unknown-unknown`: passed.

**Constraints:**

- Do not persist or log an account secret, PRF result, passkey-derived KEK, root private key, owner token, or attempt token. A recovery record may contain the already passkey-sealed envelope and narrowly scoped signed artifacts already held by the pending-custody queue.
- Keep `LocalRootRecord` provider-neutral. Store unfinished setup in dedicated credential sites `tonk-account-setup-v2` and `tonk-account-setup-recovery-v1`. After `Complete` is durable, overwrite the recovery record with a versioned tombstone rather than assuming credential-storage deletion support.
- Compute and retain the provider's canonical version-1 creation fingerprint before the first `POST /accounts`. Exact replay never means “same root is success”; provider `Mismatch` remains terminal.
- Add canonical `AccountSetupRecoveryManifestV1`: a versioned, domain-separated DAG-CBOR payload plus Ed25519 root signature and exact re-encode validation, modeled on `AccountRepositoryDescriptorV1`. It is durable and non-expiring, and binds the worker-minted operation, an immutable `ceremony_created_at`, canonical worker-derived deployment identity, expected credential/root/device and fingerprint, passkey/encryption facts, and domain-separated hashes (including an ordered-list hash) of every staged artifact. It is cross-record anti-mix proof, not provider/customer/custody effect authorization; every referenced artifact is still independently decoded and verified before use.
- `Arm` writes a separate immutable `armed_at`; later transitions never derive the ceremony reference from mutable `last_transition_at`. At first `Stage`, require `armed_at <= staged_at <= current worker time`, `ceremony_created_at >= armed_at - 60s`, `ceremony_created_at <= staged_at + 60s`, and `staged_at <= ceremony_created_at + 1h`, using checked arithmetic so rollback/overflow fails closed. Historically validate account-create at the signed ceremony time and require expiration within `[created_at + 240s, created_at + 360s]`; require deferred-publish expiration within `[created_at + 30d - 60s, created_at + 30d + 360s]`. Exact boundary values pass. Canonical/signature/original-window violations are invalid; an otherwise valid artifact expired relative to current worker time is `NeedsRefresh`, never corrupt.
- Query `POST /accounts/setup-status` with a fresh device-signed invocation carrying the exact stable root-to-device proof. Never infer `Absent` from a transport failure, timeout, 404, or malformed body.
- Order durable effects as sealed recovery bundle, local root, provider acceptance, exact local provider/account state, customer enrollment, and ordered custody `Provision` then `PublishCustody` work.
- Back or Escape is a true cancellation only before `Armed`. Once armed, Tonk does not claim to cancel `navigator.credentials.create()`; it keeps or restores a recovery surface.
- The random `AccountSecret` is generated before WebAuthn returns, while it can be sealed only after the credential response supplies/evaluates PRF output. A renderer crash after the authenticator creates the credential but before `RecoveryBundleV1` commits cannot be recovered without forbidden plaintext persistence or a cryptographic redesign. Tests and copy must name this boundary.
- Before any `navigator.credentials.create()`, new UI must complete an explicit version-2 worker handshake and the worker must confirm the provider advertises version-1 setup-status/exact-replay capability. An old worker, missing route, 404, timeout, malformed capability, or unavailable provider fails closed with “Tonk needs an update before it can safely create this account. Reload before approving a passkey.” New workers coordinate through Web Locks; arbitrary overlap with an already-deployed old UI/worker cannot be made mutually exclusive by new code alone and must remain an explicit deployment constraint.
- `Handshake`, `Begin`, and `Arm` accept no provider/config URL from the page. The worker canonicalizes the same-origin deployment configuration, derives the exact `account_service_url`, repository remote, and optional service DID, checks the provider capability itself, and binds a configuration hash into the checkpoint. The page receives only the worker-selected ceremony context. `Stage` may echo the selected provider as part of the protected bundle, but the worker requires exact equality and derives remote/service identity from independently verified signed artifacts, never page strings. The worker is never an arbitrary cross-origin fetch proxy.
- Compose rollout behavior with the service-worker upgrade work in #800/#816: its update prompt must not auto-reload an `Armed` flow. This branch guarantees pre-WebAuthn refusal for incompatible controllers and durable recovery after staging; a page critical-section signal is a dependency unless #816 leaves a non-duplicative integration seam.
- Preserve existing profiles, spaces, passkeys, local/offline data, and unrelated work. Do not clear storage or unregister workers during testing.
- Keep Cargo commands serialized with `CARGO_INCREMENTAL=0`; inspect free disk space before and after large Wasm/browser builds.
- Update Storybook source documentation, regenerate `app/data.json` and `app/data.js`, check links, and run the committed base-impact check before completion.
- Approval gate satisfied 2026-08-31 with the v2 checkpoint, tombstone, post-stage takeover, capability-handshake, and residual-boundary corrections below. Begin with focused wire/reducer RED tests; do not run broad Cargo until those protocol tests are green.

## Irreducible crash boundary

`tonk_identity::ceremony::create_custody_account` currently executes:

1. `AccountSecret::generate()`;
2. derive the intended root DID;
3. `navigator.credentials.create()` through `create_custody_passkey`;
4. obtain/evaluate PRF output;
5. seal the random secret under the derived KEK; and
6. return the sealed ciphertext and signed recovery artifacts to the UI.

A crash after step 3 but before the worker durably saves step 5 may have completed passkey approval, but Tonk cannot know or recover that attempt because a later assertion can reproduce the KEK but cannot reproduce the discarded random account secret. The implementation must stage the returned ciphertext before reporting ceremony success, but it cannot call this interval recoverable. No provider request occurs before staging. Recovery copy must neither assert that a passkey exists nor assert that none was created; it tells the user how to review an unused Tonk passkey in device settings and then explicitly start over.

## Deep module and interfaces

The public UI module is intentionally small:

```rust
pub(crate) async fn begin_or_resume(
    email: &str,
    narrate: impl Fn(&str) + Clone + 'static,
) -> Result<AccountSetupOutcome, AccountSetupError>;

pub(crate) async fn resume_pending(
    narrate: impl Fn(&str) + Clone + 'static,
) -> Result<ResumeDisposition, AccountSetupError>;

pub(crate) async fn cancel() -> Result<CancelDisposition, AccountSetupError>;
```

Both registration entrypoints use this interface. They do not directly save a root, submit an account invocation, persist an account link, enroll a customer, provision custody, or queue a custody publish.

The worker exposes one tagged command route, `POST /api/account/setup`, through `tonk_worker_api::AccountSetupRequest`. Its variants are internal steps used only by the UI module:

```rust
pub enum AccountSetupRequest {
    Handshake(AccountSetupHandshake),
    Inspect { owner_token: Option<String> },
    Begin(AccountSetupBegin),
    Acquire(AccountSetupAcquire),
    Arm(AccountSetupArm),
    Stage(Box<AccountSetupStage>),
    Continue(AccountSetupMutation),
    ReplaceInvocation(AccountSetupInvocation),
    ReplacePublishInvocation(AccountSetupInvocation),
    Cancel(AccountSetupMutation),
}
```

`Handshake` requires worker protocol version 2 and makes the worker resolve canonical deployment configuration and probe the explicit provider capability contract before setup can be armed. `Begin` re-resolves that configuration and returns an owner-bound `AccountSetupLease { view, ceremony }`, where `ceremony` is the worker-selected canonical provider, remote, and optional service DID the page must sign into its manifest. `Arm` re-resolves the configuration under the saga lock and compares its stored hash before passkey creation. Every mutation names `operation_id`, `owner_token`, and `expected_revision`; `Arm` and `Stage` additionally carry the document-memory `attempt_token`. General inspection returns only a closed redacted disposition/next-action state: no email, owner or attempt hashes, ciphertext, signed artifacts, provider diagnostic bodies, or raw durable phase. Sealed ciphertext and the minimum fields needed to reopen the same semantic creation may cross back only in a non-`Debug`, owner-authenticated `NeedsPasskey` response, and only when the request's `ClientId` is still the checkpoint's live bound owner; a matching token string alone is insufficient.

The worker saga phase is monotonic:

```text
Leased
  -> Armed
  -> RecoveryStaged
  -> RootSaved
  -> ProviderAccepted
  -> Attached
  -> CustomerEnrolled
  -> CustodyQueued
  -> Complete

Leased -> Cancelled
Armed without a bundle after its owning document disappears -> InterruptedBeforeRecovery
malformed/unsupported durable data -> Corrupt/Unsupported (fail closed)
provider exact-state mismatch -> Conflict (fail closed)
```

`Continue` owns all post-stage advancement and first reconciles observable durable effects. Callers cannot declare a later phase themselves. A private reducer consumes a fully validated `StoredCheckpointV2` plus a typed command/effect observation and returns the complete next private record, required durable action, and closed next action. It owns revision increments, timestamps, owner/client/attempt clearing, staged/accepted fields, and revalidates every result before returning it. `Leased` may be acquired by a new live `ClientId` only after the previous client is absent. From `RecoveryStaged` onward, a new live `ClientId` may acquire and resume under the same Web Lock and expected revision after the previous client is absent and the exact validated bundle remains present. `Armed` with a bundle repairs the lost checkpoint write to `RecoveryStaged`; `Armed` without one becomes terminal `InterruptedBeforeRecovery`, clears its attempt and owner fields, and is never silently rebound.

## Stored records

`StoredCheckpointV2` at credential site `tonk-account-setup-v2` contains no cryptographic secret or user-facing identity data:

```text
version, operation_id, revision, owner_hash, bound_client_id,
configuration_hash, phase, armed_at?, staged_at?, attempt_hash?, root_did?, create_fingerprint?, recovery_hash?,
accepted_descriptor_hash?, last_transition_at
```

`RecoveryBundleV1` at credential site `tonk-account-setup-recovery-v1` is deliberately not `Debug`. Credential storage protects it, but it still contains PII and bounded signed authorizations in addition to recoverable ciphertext, so it is never described as secret-free:

```text
version, operation_id, ceremony_created_at, worker-authored staged_at,
normalized_email, worker-selected provider, credential_id, expected_root_did,
expected_device_did, device_name,
delegation_cid, delegation_hex, passkey metadata, encryption recipient,
canonical descriptor_hex, create_fingerprint, original create invocation,
optional replacement create invocation plus immutable receipt reference,
account-signed customer deposits, custody DID and consent,
passkey-sealed envelope, bounded publish invocation,
optional replacement publish invocation plus immutable receipt reference,
optional private pending-publish replacement source retained only across the
two durable replacement writes,
recovery_manifest_hex
```

`RecoveryTombstoneV1 { version, operation_id, completed_at, recovery_hash }` replaces the recovery bundle only after the `Complete` checkpoint is durable. Inspection retries an interrupted tombstone write; code never relies on a delete/retract API.

`StoredPhaseV2` and `StoredRecoveryBundleV1` are private durable encodings. Wire DTOs convert explicitly into them and public views convert explicitly out; changing a page response enum must not silently migrate credential data. Decode first parses only the bounded version/tag envelope. Unknown versions, record tags, or phase tags are `Unsupported`, never `Corrupt`; only a recognized current shape that fails strict decoding or validation is corrupt.

### Required API/schema migrations

The provider base exposes proof-bound `POST /accounts/setup-status` and exact replay, but current branch base SHA `6923a9b16f9f528795d18589c58f601820e005fa` predates its explicit versioned capability. A plain `OPTIONS`/404 inference is forbidden. Reviewed provider PR #835 at `305a60f610d221bd6c9c65cc9eaf227bbb0a8162` adds the CORS-readable `GET /capabilities` v1 contract; this branch will rebase onto that exact lower head after its foundation review and make `Handshake` require the exact supported value. This is a provider API response migration, not a database migration. No account-service database schema change is required.

The new `AccountSetupRecoveryManifestV1` is an internal signed-artifact schema/API migration shared by `tonk-account`, `tonk-identity`, the top-document bridge, and `tonk-worker`. The creation input gains the worker-minted operation/config context and the output gains the canonical manifest bytes. Existing accounts and provider records need no migration because the artifact exists only for new v2 setup operations.

Loading applies overall-body, per-string, per-hex/blob, deposit-count, and decoded-total limits before expensive parsing. One `ValidatedRecoveryBundle::new` constructor then verifies the manifest signature and every manifest reference; canonical hex/CBOR/UCAN encodings; root-to-device grant and CID; descriptor root/remote; create invocation subject/audience/command/exact facts and historical signature; passkey normalization; recomputed fingerprint; X25519 recipient; exact service-deposit scopes; custody consent; sealed-envelope structure; and deferred-publish scope/checksum. Expired create/publish invocations are valid historical artifacts with a typed `NeedsRefresh` disposition, not corruption. Effects accept only `ValidatedRecoveryBundle`, never raw stored or wire DTOs. Corruption never becomes `Missing` and never permits another passkey ceremony.

## File map

- `plan/audit-account-setup-recovery.md`: durable scope, interfaces, phase invariants, TDD sequence, and evidence.
- `rust/tonk-account/src/recovery.rs`: canonical root-signed `AccountSetupRecoveryManifestV1`, bounded artifact-hash inputs, exact verification, and mutation/cross-bundle vectors.
- `rust/tonk-account/src/lib.rs`: export the recovery-manifest contract.
- `rust/tonk-account/src/pending.rs`: append an exact ordered batch in one queue serialization while retaining duplicate suppression.
- Provider lower-branch dependency: expose the explicit `GET /capabilities` v1 account-setup recovery capability consistently from the account-service Worker, native helper, and HTTP tests before this branch is publishable.
- `rust/tonk-identity/src/ceremony.rs`: retain the root through deferred-publish creation, compute the pre-POST canonical fingerprint, sign the durable recovery manifest after every artifact exists, and rebuild an expired create invocation from the same passkey-sealed local envelope after a same-credential assertion.
- `rust/tonk-identity/src/install.rs`: top-document bridge output for fingerprint/descriptor/delegation CID and the assertion-based resume ceremony.
- `rust/tonk-identity/src/request.rs`: build the fresh device-signed `account/setup/status` invocation from the stable root-to-device grant.
- `rust/tonk-worker-api/src/account_setup.rs`: versioned account-setup commands, redacted views, phases, outcomes, and copy-state discriminants.
- `rust/tonk-worker-api/src/lib.rs`: export the account-setup wire interface.
- `rust/tonk-worker/src/router/account_setup.rs`: deep saga implementation, record validation, reducer, credential adapter, Web Lock/revision fencing, provider adapter, effect reconciliation, and focused fault tests.
- `rust/tonk-worker/src/router.rs`: module registration and the single `/api/account/setup` route.
- `rust/tonk-worker/src/worker.rs`: per-worker account-setup mutex used with the cross-worker Web Lock and initialized in production/test state constructors.
- `rust/tonk-worker/Cargo.toml`: `web-sys` `Lock`/`LockManager` features needed by the service-worker Web Lock adapter.
- `rust/tonk-worker/src/router/identity.rs`: exact local-root probe and existing `persist_root` reuse; reject or reconcile mismatches rather than replacing them.
- `rust/tonk-worker/src/router/account.rs`: exact provider-link probe and idempotent reuse of `persist_link`.
- `rust/tonk-worker/src/router/customer.rs`: customer-enrollment probe/reuse and one ordered custody-cell plus pending-work promotion operation.
- `rust/tonk-ui/src/account_setup.rs`: sole top-document orchestration module, session/document tokens, same-passkey resume gesture, progress mapping, and honest copy.
- `rust/tonk-ui/src/lib.rs`: export the internal account-setup module.
- `rust/tonk-ui/src/identity_bridge.rs`: typed creation and same-passkey-resume bridge shapes; no secret-bearing debug/log path.
- `rust/tonk-ui/src/api.rs`: typed client for the single account-setup route.
- `rust/tonk-ui/src/register_dialog.rs`: render/reopen setup phases and make Back/Escape cross the worker cancellation barrier before closing.
- `rust/tonk-ui/src/account.rs`: replace the legacy duplicate creation block with the shared module and retain login-only orchestration.
- `rust/tonk-ui/src/bin/ui.rs`: inspect pending setup after the identity bridge and account UI are installed, reopening only the appropriate recovery surface.
- `rust/tonk-ui/src/user_error.rs`: map typed setup outcomes to actionable user copy without exposing diagnostics.
- `rust/tonk-ui/src/account_flow.rs`: representative real-browser cancellation, response-loss, reload, stale-worker, and same-credential recovery coverage.
- `docs/storybook/accounts/lifecycle.md`: account-creation durability phases and honest interrupt behavior.
- `docs/storybook/cross-cutting/failure-and-recovery.md`: WebAuthn irreducible boundary and response-loss/reload contract.
- `docs/storybook/journey-catalog.md`: update `ACCT-B02` and `ACCT-B06` recovery evidence/gaps.
- `docs/storybook/verification/accounts.md`: stable P1 checks for cancel/arm races, phase reloads, response loss, and incompatible workers.
- `docs/storybook/screens.json`: update the `WEB-08` behavior summary/source ownership if rendered copy changes.
- `docs/storybook/app/data.json`, `docs/storybook/app/data.js`: generated product map.

### Task 1: Produce and reconstruct the exact creation operation

**Files:**

- Modify: `rust/tonk-identity/src/ceremony.rs:AccountCeremony, create_account, create_custody_account`
- Create: `rust/tonk-account/src/recovery.rs:AccountSetupRecoveryManifestV1`
- Modify: `rust/tonk-account/src/lib.rs:recovery export`
- Modify: `rust/tonk-identity/src/install.rs:create_account, installed method table`
- Modify: `rust/tonk-identity/src/request.rs:setup-status builder`
- Modify: `rust/tonk-ui/src/identity_bridge.rs:CreateAccountOutput and resume bridge`
- Test: `rust/tonk-identity/src/ceremony.rs:tests`
- Test: `rust/tonk-account/src/recovery.rs:tests`
- Test: `rust/tonk-identity/src/request.rs:tests`
- Test: `rust/tonk-ui/src/identity_bridge.rs:tests`

**Interfaces:**

- Produces: `AccountSetupRecoveryManifestV1::sign(root, RecoveryManifestInput)` only after the delegation, descriptor, create invocation/fingerprint, deposits, consent, sealed envelope, and publish invocation exist. Its exact canonical payload binds a fixed domain, root subject, device audience, operation/config identity, immutable ceremony timestamp, expected credential/fingerprint, passkey/encryption facts, and domain-separated hashes/list-hash of those exact artifact bytes. `validate(bytes)` verifies bounds, exact canonical re-encoding, Ed25519 root signature, domain/audience/subject, and exact expected bindings; it grants no effect authority.
- Produces: `AccountCeremony { descriptor_hex, create_fingerprint, invocation_hex, .. }` before provider submission, and `CustodyAccountCeremony { recovery_manifest_hex, .. }` before returning to the page.
- Produces: `resume_custody_account(ResumeCustodyAccount)` which asserts exactly `credential_id`, opens `sealed_hex`, verifies `expected_root_did`, and returns only a freshly signed invocation plus the same canonical fingerprint.
- Produces: `build_account_setup_status_invocation(device, stable_grant, fingerprint) -> Vec<u8>` with command `account/setup/status`, root subject/audience, one exact proof, and five-minute expiration.

- [x] RED: add deterministic recovery-manifest tests proving canonical bytes and exact validation. Mutate operation, `ceremony_created_at`, canonical deployment URL/config identity, credential, root/device, fingerprint, passkey/encryption facts, each artifact hash, deposit order, subject/audience/domain, signature, version, and canonical encoding; cross two otherwise-valid ceremony bundles and require rejection. Add exact envelope/payload/string/list/body bounds before GREEN.
- [x] GREEN: implement the durable domain-separated DAG-CBOR manifest in `tonk-account`, modeled on the repository descriptor but with a distinct signing domain. The manifest hashes bytes/facts and never contains an account secret, PRF result, KEK, or private key. `CARGO_INCREMENTAL=0 cargo test -p tonk-account recovery::tests` passes 4 tests.
- [x] RED: add `it_computes_the_provider_fingerprint_before_submission` with fixed root/device/delegation/descriptor facts and the independent provider test vector; run `CARGO_INCREMENTAL=0 cargo test -p tonk-identity it_computes_the_provider_fingerprint_before_submission`; expect a compile failure because `AccountCeremony` has no fingerprint.
- [ ] GREEN: compute through `tonk_account::AccountCreationFingerprintInput`; retain `root.clone()` until the publish artifact exists; sign the manifest with the worker-minted operation and worker-selected configuration context; expose fingerprint, descriptor, device DID, delegation CID, and manifest through the installed bridge; rerun the focused tests successfully.
- [ ] RED: add `it_rebuilds_only_the_same_semantic_creation_from_a_recovered_secret` using a fixed secret and delegation; mutate credential ID, root DID, device DID, descriptor remote, passkey metadata, and sealed envelope independently; expect exact recovery only for the original tuple. Run the focused filter and observe the missing resume seam.
- [ ] GREEN: separate assertion/unwrap from a native-testable `create_account_from_recovered_secret` helper; require the same root and canonical fingerprint; return no secret/PRF/KEK. Rerun the focused and adjacent `tonk-identity` tests.
- [x] RED/GREEN: add a request-container test proving status uses the exact stable proof, root audience/subject, fingerprint argument, and fresh expiration; malformed or alternate grants must not build a request accepted by the provider verifier.
- [ ] Run `CARGO_INCREMENTAL=0 cargo test -p tonk-identity ceremony::tests` and `CARGO_INCREMENTAL=0 cargo test -p tonk-identity request::tests`; expect success.

### Task 2: Persist and reduce one exclusively owned setup operation

**Files:**

- Create: `rust/tonk-worker-api/src/account_setup.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Create: `rust/tonk-worker/src/router/account_setup.rs`
- Modify: `rust/tonk-worker/src/router.rs:module and route table`
- Modify: `rust/tonk-worker/src/worker.rs:TonkState and test constructors`
- Modify: `rust/tonk-worker/Cargo.toml:web-sys features`
- Test: `rust/tonk-worker-api/src/account_setup.rs:tests`
- Test: `rust/tonk-worker/src/router/account_setup.rs:record/reducer/ownership tests`
- Test: `rust/tonk-worker/src/router/route_table.rs`
- Test: `rust/tonk-worker/src/router/wire_compat.rs`

**Interfaces:**

- Consumes: tagged `AccountSetupRequest` through one route and the originating `ClientId` request extension.
- Produces: redacted `AccountSetupView` with revision, phase, ownership/cancellation disposition, and one explicit next action.
- Internal seam: private `StoredPhaseV2`/`StoredCheckpointV2`/`StoredRecoveryBundleV1`; envelope-first decoders; one deep reducer returning `{ checkpoint, durable_action, next_action }`; `SetupStore` with production credential-storage and in-memory fault adapters; and `SetupLock` with Web Lock plus per-worker mutex in production and deterministic mutex in tests.

- [ ] RED: add strict serialization tests for every command/view, private v2 checkpoint, private v1 bundle, and v1 tombstone round trips, unsupported versions, truly future record/phase tags, malformed JSON, missing fields, invalid phase data, and corrupted hashes. Parse the minimal bounded version/tag envelope before dispatch so future shapes are `Unsupported`; corrupt or unsupported input must return typed fail-closed state rather than `Missing`.
- [ ] GREEN: add the wire and stored types, validation, and bounded/redacted error presentation; do not derive `Debug` for recovery payloads.
- [ ] RED: add table-driven reducer tests for every legal phase edge and every backward/skip transition, including `Leased -> Cancelled`, `Armed -> Cancel` returning `TooLate`, immutable `armed_at`, Armed owner loss with/without a validated recovery record, post-stage takeover with an exact bundle, conflict provenance, stale revision, wrong operation, token/client mismatch, time regression, and arbitrary public phase advancement being impossible.
- [ ] GREEN: implement a pure monotonic reducer whose only post-stage advance input is a typed verified effect observation. The reducer consumes/returns complete validated private records, increments revision/timestamp itself, clears every transient field at its boundary, and revalidates all no-write and write results.
- [ ] RED: race two distinct ClientIds and owner tokens against `Begin`/`Arm`; require one owner, one revision winner, and one `Arm`. Cover same-client worker restart, copied session token in another live tab, pre-arm reload after the old client dies, `Armed` owner loss becoming terminal without a bundle, and post-`RecoveryStaged` takeover by a new live ClientId only after the old client is absent.
- [x] GREEN: hash tokens with domain-separated BLAKE3, bind the live ClientId, and wrap load/reduce/save in the named Web Lock and expected-revision check.
- [x] Add a production Web Lock adapter based on `navigator.locks`; unavailable Web Locks fail setup closed before `Arm` rather than silently falling back to per-worker exclusion.
- [ ] RED/GREEN: add `Handshake` tests for exact worker protocol 2 and provider recovery capability 1. Missing worker route, worker version mismatch, provider 404/timeout/malformed capability, and provider capability absence all refuse before `Arm`; no test adapter may record a WebAuthn request.
- [ ] Run `CARGO_INCREMENTAL=0 cargo test -p tonk-worker-api account_setup` and the native `tonk-worker` account-setup reducer filters; expect success.
- [ ] Run `CARGO_INCREMENTAL=0 nix develop path:. -c test:web:debug -E 'test(account_setup)'`; expect the service-worker Web Lock/storage tests to pass.

### Task 3: Stage ciphertext before root persistence and reconcile every local write

**Files:**

- Modify: `rust/tonk-worker/src/router/account_setup.rs:Stage and recovery reconciliation`
- Modify: `rust/tonk-worker/src/router/identity.rs:exact local-root observation`
- Modify: `rust/tonk-worker/src/router/account.rs:exact attachment observation`
- Test: `rust/tonk-worker/src/router/account_setup.rs:fault-injected store/effect tests`

**Interfaces:**

- `Stage` converts the wire DTO into private `StoredRecoveryBundleV1`, applies cheap bounds, constructs the sole `ValidatedRecoveryBundle`, saves it, reads it back through the same constructor, validates its hash/manifest/fingerprint, advances `RecoveryStaged`, then calls the existing local-root persistence implementation.
- `observe_local_effects` returns exact/missing/mismatch facts for root and provider link; mismatch is terminal and no stored authority is replaced.

- [ ] RED: add fault tests after recovery save, checkpoint save, root record save, root access projection, and root-phase checkpoint save. Recreate the saga adapter after each injected failure and require convergence without another WebAuthn create.
- [x] GREEN: implement save-first staging and root reconciliation. A bundle present behind an `Armed` checkpoint repairs it to `RecoveryStaged`; an exact local root repairs to `RootSaved`; mismatched root/grant fails closed.
- [ ] RED/GREEN: mutate and oversize every bundle field independently; cross records from two fully valid bundles; alter deposit ordering; and require no root/provider/customer/custody effect. Cover overall input, every string/blob/hex, deposit count/decoded total, canonical encoding, signatures/scopes, manifest binding, fingerprint, and sealed-envelope structure before expensive parsing where possible. Test `ceremony_created_at` at exactly both Stage skew bounds and the one-hour age bound, one second outside each, worker clock rollback, checked-arithmetic overflow, exact create/publish original-expiry bounds, and current `Usable`/`NeedsRefresh`. The constructor structurally validates the envelope without opening it; the manifest proves cross-record origin, while a later same-credential assertion proves the ciphertext opens to the expected root.
- [x] Add the explicit `InterruptedBeforeRecovery` observation for `Armed` with a dead owning client and no bundle; it must never offer automatic retry or claim no passkey was created.
- [ ] Run the focused native fault filters and Wasm credential-storage filters serially; expect success.

### Task 4: Recover provider response loss and finish ordered local/customer/custody effects

**Files:**

- Modify: `rust/tonk-worker/src/router/account_setup.rs:Continue, provider adapter, effect probes`
- Modify: `rust/tonk-worker/src/router/account.rs:exact link probe`
- Modify: `rust/tonk-worker/src/router/customer.rs:enrollment probe and persist_custody_setup`
- Modify: `rust/tonk-account/src/pending.rs:push_all`
- Test: `rust/tonk-worker/src/router/account_setup.rs:provider/effect fault matrix`
- Test: `rust/tonk-account/src/pending.rs:ordered batch tests`

**Interfaces:**

- Internal owned-remote port: `AccountSetupProvider::status` and `AccountSetupProvider::create`; production uses the existing bounded worker HTTP adapter and tests use a scripted in-memory adapter.
- Produces: `Continue` outcome `Complete`, `NeedsPasskey`, `RetryLater`, or terminal `Conflict`; transport uncertainty never becomes provider `Absent`.
- Produces: `customer::persist_custody_setup` which records the local custody cell, appends `[Provision, PublishCustody]` in one queue save, and then drains best-effort.

- [x] RED: script provider `Accepted`, `Absent`, `Mismatch`, timeout, malformed body, and lost response after commit. Require status before create on every resume; Accepted advances with the canonical descriptor, Absent submits the exact stored invocation, Mismatch writes nothing, and unknown outcomes remain retryable without WebAuthn.
- [x] GREEN: implement the status invocation/HTTP adapter and strict response/fingerprint/descriptor checks. Exact create replay accepts either initial 201 or reused 200 but verifies the returned fingerprint and descriptor.
- [x] RED/GREEN: when Absent and the stored invocation is expired, return `NeedsPasskey` with owner-authorized sealed recovery input. `ReplaceInvocation` accepts only a fresh invocation whose decoded semantic facts reproduce the stored fingerprint. The same phase-specific contract now refreshes an expired custody publish authorization through `ReplacePublishInvocation` without exposing create material after provider acceptance.
- [ ] RED: inject failure after provider acceptance, provider-phase checkpoint, provider record save, account-state initialization, enrollment response, customer record, custody cell, ordered queue save, drain, and completion save. Recreate the worker between failures; require monotonic convergence and no duplicate account/device/custody ordering. The production-seam matrix now covers provider 409, customer projection failure, custody completion, and tombstone loss; the remaining exhaustive crash-point/browser restart matrix stays open.
- [x] GREEN: reconcile exact provider link/customer/custody effects before replaying. Treat enrollment and pending work as at-least-once idempotent operations; never let `PublishCustody` overtake `Provision`.
- [ ] RED/GREEN: make malformed pending work recoverable from the still-retained bundle, and prove a drained queue followed by checkpoint loss can safely reappend/replay the exact pair. Concurrent append loss/crossing is now covered under the shared mutation seam; the drained-queue/checkpoint-loss browser restart case remains open.
- [x] Save `Complete` before overwriting the recovery bundle with `RecoveryTombstoneV1`. A failed tombstone write leaves credential-store-protected sealed/bounded material and is retried on later inspection; do not assume delete/retract support.
- [ ] Run `CARGO_INCREMENTAL=0 cargo test -p tonk-account pending::tests` and focused native/wasm worker saga filters; expect success.

### Task 5: Consolidate entrypoints and render truthful cancel/recovery states

**Files:**

- Create: `rust/tonk-ui/src/account_setup.rs`
- Modify: `rust/tonk-ui/src/lib.rs`
- Modify: `rust/tonk-ui/src/api.rs:account-setup client`
- Modify: `rust/tonk-ui/src/identity_bridge.rs:resume ceremony`
- Modify: `rust/tonk-ui/src/register_dialog.rs:setup presenter, cancel, reload recovery`
- Modify: `rust/tonk-ui/src/account.rs:run_account_ceremony and duplicate create handler`
- Modify: `rust/tonk-ui/src/bin/ui.rs:resume inspection after install`
- Modify: `rust/tonk-ui/src/user_error.rs:typed setup copy`
- Test: `rust/tonk-ui/src/account_setup.rs:tests`
- Test: `rust/tonk-ui/src/register_dialog.rs:tests`
- Test: `rust/tonk-ui/src/account.rs:tests`

**Interfaces:**

- Both account creation surfaces call only `account_setup::begin_or_resume`; the legacy account element cannot perform a separate ordering.
- `register_dialog` renders a stable view model derived from `AccountSetupView`; it never interprets raw transport text as phase state.
- Back/Escape awaits `cancel`: close only for `Cancelled`; keep the modal for `TooLate` and explain the recovery state.

- [ ] RED: add an orchestration spy test showing both creation entrypoints produce the same `Begin -> Arm -> WebAuthn once -> Stage -> Continue` sequence and neither directly calls old save/submit/enroll/custody clients.
- [ ] GREEN: move creation orchestration into `account_setup.rs`, replace the duplicate account-element block, and leave login-only behavior in `complete_remote`.
- [ ] RED/GREEN: persist the owner token in `sessionStorage`, keep the attempt token document-local, and prove raw tokens never enter diagnostics or stored records. Only domain-separated hashes are stored in the checkpoint, and owner/email/artifacts are absent from general status and logs.
- [ ] RED: race Back/Escape with `Arm` in both orders. Before arm, expect close plus “Account setup cancelled. No passkey was created.” After arm, expect the dialog to remain with “Your device may still be creating the passkey. Finish or dismiss the device prompt. Tonk won’t start another one.”
- [ ] GREEN: call `preventDefault` synchronously, then await worker cancellation. Do not wire Back/Escape to a WebAuthn abort claim.
- [ ] RED/GREEN: render `InProgressElsewhere`, `RecoveryStaged/RetryLater`, `NeedsPasskey`, provider `Conflict`, local-link retry, and `InterruptedBeforeRecovery` with actionable copy and the correct enabled action. The interrupted copy must say: “Passkey approval may have completed, but Tonk cannot know or recover that attempt. Check your device’s passkey settings for an unused Tonk passkey, remove it if present, then choose Start over.” It must not imply the user definitely has or lacks an orphan passkey.
- [ ] On top-document boot, inspect pending setup. Resume post-stage work automatically; reopen a user-action surface for `Leased`, `Armed`, `NeedsPasskey`, conflict, or interruption. Never invoke WebAuthn without the fresh action click.
- [ ] Review the touched interface using the interface-polish checklist. Preserve existing geometry/motion unless the new state needs a control; maintain at least 40x40 hit targets, balanced/pretty wrapping, interruptible transitions, and no `transition: all`.
- [ ] Run focused `tonk-ui` Wasm tests, then `CARGO_INCREMENTAL=0 nix develop path:. -c test:web:debug -E 'package(tonk-ui)'`; expect success.

### Task 6: Prove representative browser faults and incompatible-worker refusal

**Files:**

- Modify: `rust/tonk-ui/src/account_flow.rs:account creation recovery tests and helpers`
- Test: existing local account/access services and virtual authenticator only; never a live browser profile.

**Interfaces:**

- Whole-journey evidence complements the exhaustive reducer/fault-adapter matrix; it does not duplicate every lower-layer phase.

- [ ] RED/GREEN: add a real-browser test where the provider commits account creation and the response is lost, reload the page, require setup status to recover the canonical descriptor, and require exactly one provider account/device and one virtual credential.
- [ ] RED/GREEN: reload after `RecoveryStaged`, after provider acceptance, after local link, and after custody queue persistence using deterministic test barriers exposed through the existing test harness rather than production-only methods.
- [ ] RED/GREEN: two tabs race the same profile; one displays `InProgressElsewhere` and the virtual authenticator records one creation.
- [ ] RED/GREEN: Escape before arm closes with no credential; Escape after arm does not close or start another ceremony.
- [ ] RED/GREEN: emulate a worker without `/api/account/setup`, a protocol-v1 response, and a worker whose provider probe returns 404/timeout/malformed/missing capability. New UI must show “Tonk needs an update before it can safely create this account. Reload before approving a passkey.” before invoking the virtual authenticator.
- [ ] Record the untestable legacy-old-UI/new-worker overlap as a deployment constraint unless the service-worker lifecycle branch is explicitly added as a prerequisite; do not mark exactly-one mixed-version operation verified without that composition test.
- [ ] Compose with #800/#816 in rollout evidence: an update prompt must not auto-reload an `Armed` flow. If this branch does not own the page critical-section signal, record the exact dependent interface/test rather than duplicating it.
- [ ] Run each new browser test alone with `CARGO_INCREMENTAL=0 nix develop path:. -c cargo test -p tonk-ui --features integration-tests <exact-test-name> -- --test-threads=1 --nocapture`; expect success. Loopback permission failures are infrastructure failures and must be retried unchanged with permission.

### Task 7: Update the outside-in account recovery contract

**Files:**

- Modify: `docs/storybook/accounts/lifecycle.md`
- Modify: `docs/storybook/cross-cutting/failure-and-recovery.md`
- Modify: `docs/storybook/journey-catalog.md`
- Modify: `docs/storybook/verification/accounts.md`
- Modify if rendered behavior changes: `docs/storybook/screens.json`
- Regenerate: `docs/storybook/app/data.json`
- Regenerate: `docs/storybook/app/data.js`

**Interfaces:**

- Documents: `ACCT-B02` normal, cancel, concurrent-tab, response-loss, reload, corruption, stale-worker, and irreducible WebAuthn boundary behavior without claiming executed evidence before tests run.

- [ ] Update lifecycle/failure source with the exact `Leased -> Armed -> RecoveryStaged` boundary, provider status-first recovery, ordered postconditions, and the possibly-completed-but-unknowable passkey interval.
- [ ] Add P1 verification rows for cancel-vs-arm, same-profile tab exclusion, every durable-phase reload, provider response loss, corrupt/unsupported recovery state, and incompatible worker refusal.
- [ ] Update evidence labels only for tests actually executed; leave arbitrary old-worker coexistence as an explicit gap.
- [ ] Run `python3 docs/storybook/scripts/build.py` to regenerate data.
- [ ] From `docs/storybook`, run `python3 scripts/build.py --check` and `python3 scripts/check-links.py .`; expect success.
- [ ] After the documentation/generated files are committed locally, run `python3 docs/storybook/scripts/build.py --check --base 6923a9b16f9f528795d18589c58f601820e005fa`; expect the impact check to pass.

### Task 8: Verify the complete recovery slice without publishing it

**Files:**

- Modify: `plan/audit-account-setup-recovery.md:verification evidence and remaining limits`

- [ ] Run `df -h .` and record available space before broad builds. Do not start overlapping Cargo/Nix processes.
- [ ] Run `CARGO_INCREMENTAL=0 cargo fmt --all -- --check` and `git diff --check`; expect success.
- [ ] Run the focused native packages serially: `tonk-account`, `tonk-worker-api`, `tonk-identity`, `tonk-worker`, and `tonk-ui`; expect success.
- [ ] Run `CARGO_INCREMENTAL=0 nix develop path:. -c test:web:debug -E 'test(account_setup)'`; expect all focused Wasm worker/UI tests to pass.
- [ ] Run each new real-browser regression serially with one test thread; expect success without touching a live browser profile.
- [ ] Run the Storybook generation, freshness, link, and committed base-impact checks; expect success.
- [ ] Run `CARGO_INCREMENTAL=0 cargo clippy -p tonk-account -p tonk-worker-api -p tonk-identity -p tonk-worker -p tonk-ui --all-targets -- -D warnings`; if target-specific code requires the repository Wasm wrapper, run the equivalent scoped Wasm clippy/check and record the distinction.
- [ ] Inspect `git diff --stat`, `git diff --check`, `git status --short`, free disk, and every changed file against this plan. Preserve unrelated work and report any unverified service-worker composition explicitly.
- [ ] Request independent review of the local branch. Do not push or open a PR until the parent confirms provider review/publication and approves this branch's review outcome.

## Requirement coverage self-review

- Reload/tab/worker recovery: Tasks 2–6.
- Versioning, serialization, and corruption: Task 2 and Task 3.
- Exclusive ownership/revision and concurrent ClientIds: Task 2.
- Canonical fingerprint before POST and provider status/replay: Tasks 1 and 4.
- Same-passkey recovery after invocation expiry: Tasks 1, 4, and 5.
- Provider acceptance before local link/customer/custody: Task 4.
- Ordered, idempotent custody promotion and checkpoint-write loss: Task 4.
- Consolidated creation entrypoints: Task 5.
- Honest Back/Escape semantics and copy: Task 5.
- Irreducible credential-created-before-stage interval: constraints, Task 3, Task 5, and Storybook Task 7.
- Outdated service worker behavior: constraints, Task 2, and Task 6.
- Provider capability/API migration and provider-first rollout: required migration, Task 2, and Task 6.
- #800/#816 armed-flow reload dependency: constraints and Task 6.
- Storybook source/generated/impact checks: Task 7.
- Serialized Cargo, disk monitoring, no live-profile destruction, no premature publication: constraints and Task 8.

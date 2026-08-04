# Account-bound CLI sessions and offline logout implementation plan

**Goal:** Make the native CLI locally usable while logged out, prevent every access-service request while logged out, bind every logged-in remote request to exactly one freshly passkey-approved account grant, and eventually remove an offline-logged-out device from that account's visible device list without revoking the grant.

**Approach:** Replace the current `LocalRoot` plus provider tombstone as the login authority with one durable account-session record whose `active` field is either absent or names exactly one provider/root/delegation from the latest handoff. Keep historical UCAN certificates for local repository authority, but put an owned `AccountBoundOperator` in front of every repository network fork; it constructs the outgoing chain from the active grant and an account-root-specific space prefix and never forwards a chain selected from historical authority. Logout performs one local state transition from `active` to a signed, narrowly scoped detach outbox entry, so it succeeds offline and immediately closes the network boundary; later online account operations flush that intent to the account service.

**Constraints:**
- Every `tonk account link` must complete a browser/passkey handoff. A previously used grant must never reactivate an account locally.
- The CLI has exactly zero or one active remote account. `account link` refuses while one is active; switching accounts is logout followed by a fresh handoff.
- Logged out means no UCAN invocation can reach a UCAN-S3 access service from spot sync, account-state hydration, status fetches, auto-sync, invite sync, or any other `dialog_repository` network path.
- Historical root-to-device and space-to-root certificates may remain in local storage because existing spots can require them for local writes. They must be unreachable as outgoing remote authorization unless they are also the latest active grant.
- Reading, querying, editing, and committing existing local spots must continue while logged out. Remote configuration metadata may remain intact.
- Remote access under account B requires a reusable space prefix rooted in B plus B's latest root-to-device grant. Authority rooted in A must not satisfy or be sent for a B session.
- Logout is not revocation. Detachment hides an attachment and releases the device DID for a later handoff; it does not publish a revocation artifact. Revoked delegation CIDs remain revoked permanently.
- Logout must succeed without network access. Provider/device-list staleness until an outbox retry is acceptable.
- A queued detach item must not contain a reusable UCAN delegation. It is a canonical device-signed statement capable only of detaching the exact attachment generation it names.
- A stale detach must never detach a newer login, including a later login to the same account and device DID. Every registration path therefore receives a service-generated random `attachment_id`; it is not derived from the device DID or delegation CID.
- Account-session mutation and access-service dispatch must be serialized across CLI processes with a native shared/exclusive file lock. Network forks hold a shared lock from active-state read through HTTP dispatch; login/logout/outbox writes hold the exclusive lock. Once logout returns, no invocation authorized before that logout may still be dispatched.
- An interrupted handoff must be resumable and must not strand an active service attachment unknown to local state. Browser completion records grant material but does not activate the device; activation is a separate idempotent CLI step performed only after `PendingLogin::Activating` is durable.
- Keep the native SQLite helper and Cloudflare D1 behavior identical and driven by the ordered account-service migrations.
- Preserve the current CLI profile DID, spots, account repository bytes, remote/upstream metadata, trusted markers, and unpushed revisions across logout and account switching.
- The current uncommitted `0006_account_scoped_devices.sql` and associated account-scoped lookup changes are superseded. Account-scoped history is useful, but the final schema must additionally enforce at most one active attachment globally for a device DID.
- Do not change `Cargo.lock` unless implementation proves a dependency change is unavoidable. The required Dialog provider, UCAN authorization, and network-fork APIs are already dependencies.

## State and protocol contracts

The canonical CLI credential record is one application-level transition boundary:

```rust
pub const ACCOUNT_SESSION_SITE: &str = "tonk-account-session-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSessionState {
    pub version: u8,
    pub active: Option<ActiveAccount>,
    pub pending_login: Option<PendingLogin>,
    pub pending_detaches: Vec<PendingDetach>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingLogin {
    Waiting { provider: String, secret: String, token_hash: String },
    Activating { provider: String, secret: String, account: ActiveAccount },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDetach {
    pub provider: String,
    pub intent: SignedDetachIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveAccount {
    pub provider: String,
    pub credential_id: String,
    pub root_did: String,
    pub delegation_cid: String,
    pub delegation_hex: String,
    pub descriptor_hex: Option<String>,
    pub attachment_id: String,
    pub attached_at: u64,
}
```

A detach intent is signed directly by the persistent CLI profile key, not authorized by or packaged with the account delegation:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachPayloadV1 {
    pub version: u8,
    pub account_root: String,
    pub device_did: String,
    pub attachment_id: String,
    pub delegation_cid: String,
    pub issued_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDetachIntent {
    pub payload: Vec<u8>,       // canonical DAG-CBOR DetachPayloadV1
    pub signature: Vec<u8>,     // Ed25519 over a domain-separated payload
}
```

`SignedDetachIntent::validate` must require canonical encoding, a canonical Ed25519 `did:key`, a valid device signature, non-empty canonical IDs, and the domain separator `tonk/account-device-detach/v1`. The account service then binds the verified payload to its stored attachment row; the signature alone does not claim account membership.

The final device table represents attachment generations, not one mutable device identity:

```sql
CREATE TABLE devices_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    device_did TEXT NOT NULL,
    attachment_id TEXT NOT NULL UNIQUE,
    delegation_cid TEXT NOT NULL UNIQUE,
    delegation_hex TEXT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL
);
CREATE INDEX devices_account ON devices_next(account_id);
CREATE UNIQUE INDEX devices_one_active_did
    ON devices_next(device_did) WHERE status = 'active';
```

Migration of existing rows uses `attachment_id = delegation_cid` because no older detach intent can name those rows. Every new account creation, direct registration, and CLI handoff receives a service-generated random 32-byte hex `attachment_id`. Statuses are `active`, `detached`, and `revoked`.

## File map

- `rust/tonk-account/src/detach.rs`: canonical signed detach-intent contract shared by CLI and service.
- `rust/tonk-account/src/handoff.rs`: carry the recoverable handoff and unique attachment generation through consumption/activation.
- `rust/tonk-account/src/backup.rs`: account-root-specific space-prefix credential key helper.
- `rust/tonk-account/src/lib.rs`: export the new shared contracts.
- `rust/tonk-identity/src/ceremony.rs`: ensure each passkey handoff mints a fresh delegation and returns the handoff attachment generation unchanged.
- `rust/tonk-account-service/migrations/0006_device_attachment_lifecycle.sql`: preserve attachment history while enforcing one globally active attachment per device DID.
- `rust/tonk-account-service/src/store.rs`: attachment-aware store types, queries, and transition interfaces.
- `rust/tonk-account-service/src/store/sqlite.rs`: transactional native implementation.
- `rust/tonk-account-service/src/store/d1.rs`: transactional D1 implementation.
- `rust/tonk-account-service/src/core/devices.rs`: detach verification, visible-device filtering, and lifecycle rules.
- `rust/tonk-account-service/src/core/accounts.rs`: assign attachment generations during first-device account creation.
- `rust/tonk-account-service/src/core/links.rs`: persist recoverable completed handoffs, then idempotently activate a fresh attachment only when no active attachment owns the DID.
- `rust/tonk-account-service/src/auth.rs`: authenticate only the active attachment whose delegation CID is the invocation's exact proof.
- `rust/tonk-account-service/src/handlers/links.rs`: recoverable consume and idempotent `POST /links/activate` adapters.
- `rust/tonk-account-service/src/handlers/devices.rs`: `POST /devices/detach` adapter.
- `rust/tonk-account-service/src/helpers/server.rs`: native helper route with matching behavior.
- `rust/tonk-account-service/src/lib.rs`: worker route registration.
- `rust/tonk-account-service/tests/service.rs`: end-to-end detach, reattach, stale-intent, and revocation coverage.
- `rust/tonk-ui/src/account.rs`: select and revoke the exact visible attachment generation.
- `rust/tonk-ui/src/identity_bridge.rs`: bind browser-generated link/revoke ceremonies to attachment IDs.
- `rust/tonk-worker-api/src/account.rs`: carry attachment IDs through browser/worker device and revoke contracts.
- `rust/tonk-worker/src/router/account_devices.rs`: preserve and sign the selected attachment generation.
- `rust/tonk-cli/src/account_session.rs`: canonical active/pending account state, cross-process lifecycle lock, detach-outbox persistence, legacy migration, and retry/recovery logic.
- `rust/tonk-cli/src/account_authority.rs`: owned operator wrapper that is the only UCAN-S3 authorization boundary.
- `rust/tonk-cli/src/account.rs`: handoff activation, logout transition, status, and account-service connection adapters.
- `rust/tonk-cli/src/identity.rs`: retain historical grants but allow the latest handoff record to change roots.
- `rust/tonk-cli/src/site.rs`: account-root-specific prefixes and replacement of raw site operators with the guarded operator.
- `rust/tonk-cli/src/account_state.rs`: use the guarded operator for account repository hydration/push/pull.
- `rust/tonk-cli/src/invite.rs`: save reusable prefixes under the account-root-specific key.
- `rust/tonk-cli/src/lib.rs`: register the new private modules and expose only test helpers that integration coverage needs.
- `rust/tonk-cli/tests/account_session.rs`: cross-module offline logout and account-switch behavior.
- `rust/tonk-cli/README.md`: user-visible login, logout, switching, delayed detachment, and revocation semantics.
- `rust/tonk-account-service/README.md`: attachment lifecycle and detach endpoint contract.

### Task 1: Define canonical detach and handoff-generation contracts

**Files:**
- Create: `rust/tonk-account/src/detach.rs`
- Modify: `rust/tonk-account/src/handoff.rs:ConsumedLink`
- Modify: `rust/tonk-account/src/lib.rs:module exports`
- Test: `rust/tonk-account/src/detach.rs:tests`
- Test: `rust/tonk-account/src/handoff.rs:tests`

**Interfaces:**
- Consumes: `dialog_credentials::{Signer, Ed25519KeyResolver}`, `dialog_varsig` Ed25519 signatures, and canonical DAG-CBOR conventions already used by `AccountRepositoryDescriptorV1`.
- Produces:

```rust
impl SignedDetachIntent {
    pub async fn sign(
        signer: &dialog_credentials::SignerCredential,
        account_root: &Did,
        attachment_id: &str,
        delegation_cid: &str,
        issued_at: u64,
    ) -> Result<Self, DetachIntentError>;

    pub async fn validate(&self) -> Result<DetachPayloadV1, DetachIntentError>;
}

pub struct ConsumedLink {
    // existing fields
    pub attachment_id: String,
}
```

- [ ] Add `it_round_trips_a_canonical_device_signed_detach_intent`. Sign fixed payload fields with a fixed Ed25519 device key, validate it, and assert all decoded fields including the signer-derived `device_did` and exact `attachment_id`.
- [ ] Add rejection tests for a changed account root, device DID, attachment ID, delegation CID, payload byte, signature byte, non-canonical DAG-CBOR spelling, non-Ed25519 DID, and unsupported version. Each must return a typed `DetachIntentError` without panicking.
- [ ] Run `cargo test -p tonk-account detach`; expect failure because the module and types do not exist.
- [ ] Implement domain-separated signing and canonical decode/re-encode comparison following `rust/tonk-account/src/descriptor.rs`; do not include provider URLs or delegation bytes in the signed object.
- [ ] Extend `ConsumedLink` serialization tests to require camelCase `attachmentId` and reject a response missing it rather than silently inventing an attachment generation.
- [ ] Run `cargo test -p tonk-account`; expect success.

### Task 2: Model active, detached, and revoked attachment generations in the account service

**Files:**
- Delete: `rust/tonk-account-service/migrations/0006_account_scoped_devices.sql` (the superseded untracked migration)
- Create: `rust/tonk-account-service/migrations/0006_device_attachment_lifecycle.sql`
- Modify: `rust/tonk-account-service/src/store.rs:Device, DeviceStatus, Store, device SQL constants`
- Modify: `rust/tonk-account-service/src/store/sqlite.rs:device and link methods`
- Modify: `rust/tonk-account-service/src/store/d1.rs:device and link methods`
- Modify: `rust/tonk-account-service/src/core/accounts.rs:first-device creation`
- Modify: `rust/tonk-account-service/src/core/devices.rs:register_device`
- Modify: `rust/tonk-account-service/src/core/links.rs:complete_link, consume_link, activate_link`
- Modify: `rust/tonk-account-service/src/auth.rs:authorize`
- Modify: `rust/tonk-account-service/src/handlers/links.rs:consume and activate adapters`
- Modify: `rust/tonk-account-service/src/helpers/server.rs:link activation route`
- Modify: `rust/tonk-account-service/src/lib.rs:link activation Worker route`
- Test: `rust/tonk-account-service/src/core/links.rs:tests`
- Test: `rust/tonk-account-service/src/auth.rs:tests`

**Interfaces:**
- Consumes: service-generated random attachment IDs and the completed handoff's recoverable root grant.
- Produces:

```rust
pub enum DeviceStatus { Active, Detached, Revoked }

pub struct Device {
    pub id: i64,
    pub attachment_id: String,
    // existing fields
}

async fn active_device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError>;
async fn attachment(&self, attachment_id: &str) -> Result<Option<Device>, StoreError>;
async fn completed_link_by_attachment(&self, attachment_id: &str) -> Result<Option<LinkRequest>, StoreError>;
async fn activate_completed_link(&self, token_hash: &str, attachment_id: &str, now: u64) -> Result<ActivateOutcome, StoreError>;
async fn detach_attachment(&self, attachment_id: &str) -> Result<DetachStoreOutcome, StoreError>;
```

- [ ] Add a migration test that applies migrations `0001` through `0006` to a fixture containing active and revoked `0005` rows. Assert `attachment_id` is backfilled from each delegation CID, statuses and bytes are unchanged, and the partial unique index rejects two active rows for one DID while permitting detached/revoked history.
- [ ] Add `it_replays_each_crash_safe_link_phase_idempotently`. Recreating the same token hash/device/name returns the existing pending request, while mismatched metadata conflicts. Browser completion stores the delegation, descriptor, and one random attachment ID but creates no device row. Repeated consume with the same 256-bit secret returns the same completed payload during the recovery window. A device-signed `/links/activate` request then inserts the active row; replaying activation returns the same success.
- [ ] Add `it_registers_a_fresh_generation_after_detachment`. Activate generation A, mark it detached, complete and activate generation B for the same DID and either the same or another account, and assert A remains detached while only B is active.
- [ ] Add `it_rejects_activation_while_the_device_has_an_active_attachment` and assert the account-service conflict is actionable rather than leaking a SQL constraint.
- [ ] Add `it_never_reactivates_a_revoked_delegation`. Activation carrying a previously revoked delegation CID must conflict even when no attachment is active; a genuinely fresh root-signed delegation from a later passkey handoff may create a new generation.
- [ ] Add auth tests proving detached and revoked generations cannot authenticate account-service commands. Specifically seed detached A and active B for the same account/device DID, invoke with A's proof, and require rejection because the invocation's sole root→device proof CID does not equal B's `delegation_cid`.
- [ ] Add registration tests for all non-handoff paths: first-device account creation and `/devices/register` each receive a random attachment ID, return/expose it where the caller needs it, and obey the one-active-DID constraint.
- [ ] Run `cargo test -p tonk-account-service --features helpers core::links`; expect at least the missing schema/interface failures.
- [ ] Replace the current account-scoped migration with the attachment-generation schema above. Implement equivalent SQLite and D1 transactions: activation checks for an active DID and revoked delegation CID before inserting; it never updates an old row in place.
- [ ] Make link creation idempotent for an exact existing token hash/device/name tuple so `PendingLogin::Waiting` can retry after a lost HTTP response; retain conflicts for any mismatch. Make completed handoff consumption replayable for 24 hours or until successful activation, whichever comes first. `consume_link` must not erase the only copy before the CLI durably records `PendingLogin::Activating`.
- [ ] Add a special activation verifier for command `account/link/activate`: cryptographically verify the returned root→device proof and device signature, then bind token hash, attachment ID, root, device DID, and delegation CID to the completed link. Do not call normal `authorize`, because no active device row exists yet.
- [ ] Generate attachment IDs with 32 random bytes for account creation, direct registration, and handoff completion; return the stored handoff attachment ID in `ConsumedLink` and update all fixtures/handlers.
- [ ] In `authorize`, require exactly one invocation proof and require its CID to equal the selected active row's `delegation_cid`; matching only account plus issuer DID is insufficient.
- [ ] Run `cargo test -p tonk-account-service --features helpers`; expect success on lifecycle, recovery, account creation, authorization, and revocation tests.

### Task 3: Add idempotent, generation-bound remote detachment

**Files:**
- Modify: `rust/tonk-account-service/src/core/devices.rs:list_devices, new detach_device`
- Modify: `rust/tonk-account-service/src/handlers/devices.rs:new handle_detach`
- Modify: `rust/tonk-account-service/src/helpers/server.rs:route and adapter`
- Modify: `rust/tonk-account-service/src/lib.rs:worker routes`
- Modify: `rust/tonk-account-service/tests/service.rs`
- Modify: `rust/tonk-ui/src/account.rs:device list and revoke selection`
- Modify: `rust/tonk-ui/src/identity_bridge.rs:revoke ceremony arguments`
- Modify: `rust/tonk-worker-api/src/account.rs:AccountDevice and RevokeDeviceRequest`
- Modify: `rust/tonk-worker/src/router/account_devices.rs:list projection and signed revoke arguments`
- Modify: `rust/tonk-cli/src/account.rs:DeviceRow and revoke polling`
- Modify: `rust/tonk-account-service/README.md`

**Interfaces:**
- Consumes: a JSON-encoded `SignedDetachIntent`; no UCAN invocation or bearer account credential.
- Produces: `POST /devices/detach` with a typed JSON outcome: `detached`, `alreadyDetached`, `cancelledPendingActivation`, `superseded`, or `revoked` are terminal success outcomes; `unknownAttachment` is `404`; `payloadMismatch` is `409`; malformed/forged intent remains `400`/`403`.

- [ ] Add core tests `it_detaches_only_the_exact_signed_generation`, `it_accepts_replayed_detach_idempotently`, `it_cancels_a_completed_but_not_yet_active_generation`, and `it_does_not_detach_a_newer_generation`. Cover invalid signatures, payload DID differing from its signer, unknown attachment IDs, account-root mismatch, delegation-CID mismatch, revoked rows, and a stale intent submitted after a newer generation became active. Assert the exact typed outcome and HTTP status for every case.
- [ ] Add a device-list test proving detached generations are omitted. Include `attachmentId` in each visible row. When history contains revoked A plus active B for the same DID, return the active row as the actionable device; when no active row exists, return only the newest revoked row for that DID.
- [ ] Run `cargo test -p tonk-account-service --features helpers detach`; expect failure because the ceremony and route are absent.
- [ ] Implement `detach_device`: validate the signed intent, load by `attachment_id`, bind all payload fields to the stored row or completed link and account root, and conditionally update only `status = 'active'`. If the generation is completed but not active, permanently cancel that completed link so `/links/activate` rejects it. Treat the same already-detached row as success; report revoked/superseded without changing them; never change another generation.
- [ ] Make revocation generation-specific. Include `attachmentId` in device views and signed revoke arguments, verify the artifact first, select the stored row by the artifact's exact target delegation CID plus account, then require its attachment ID and device DID to match the request. Update CLI/browser/worker contracts and polling to match `delegation_cid`/`attachment_id`, never the first row sharing a DID; add serialization tests in `tonk-worker-api` and router tests proving the ID survives list→revoke.
- [ ] Register identical native-helper and Worker routes and use the normal bounded error envelope.
- [ ] Add service-level HTTP coverage for valid, replayed, forged, and stale requests and confirm CORS preflight advertises the route.
- [ ] Run `cargo test -p tonk-account-service --features helpers`; expect success.

### Task 4: Introduce one durable CLI account-session transition boundary

**Files:**
- Create: `rust/tonk-cli/src/account_session.rs`
- Modify: `rust/tonk-cli/src/lib.rs:module declarations`
- Modify: `rust/tonk-cli/src/account.rs:status, stored provider/connection helpers, persist, test fixture attachment`
- Modify: `rust/tonk-cli/src/identity.rs:save_local_root`
- Test: `rust/tonk-cli/src/account_session.rs:tests`
- Test: `rust/tonk-cli/src/account.rs:tests`

**Interfaces:**
- Consumes: `ConsumedLink.attachment_id`, `LocalRoot`, `AccountProviderRecord`, and the current empty-byte provider tombstone for legacy migration.
- Produces:

```rust
pub fn shared_remote_guard(store: &SpotStore) -> Result<AccountSessionReadGuard>;
pub fn exclusive_transition_guard(store: &SpotStore) -> Result<AccountSessionWriteGuard>;
pub async fn ensure_initialized(profile: &Profile, operator: &AccountBoundOperator, guard: &AccountSessionWriteGuard) -> Result<()>;
pub async fn load_guarded(profile: &Profile, operator: &AccountBoundOperator, guard: &AccountSessionReadGuard) -> Result<AccountSessionState>;
pub async fn active_guarded(profile: &Profile, operator: &AccountBoundOperator, guard: &AccountSessionReadGuard) -> Result<Option<ActiveAccount>>;
pub async fn begin_login(profile: &Profile, operator: &AccountBoundOperator, pending: PendingLogin) -> Result<()>;
pub async fn finish_activation(profile: &Profile, operator: &AccountBoundOperator) -> Result<()>;
pub async fn logout_transition(profile: &Profile, operator: &AccountBoundOperator, now: u64) -> Result<bool>;
pub async fn flush_pending(profile: &Profile, operator: &AccountBoundOperator) -> Result<FlushOutcome>;
```

`logout_transition` runs under the exclusive transition guard. For `active`, it signs and queues the exact detach. For `PendingLogin::Waiting`, it clears the unactivated ceremony. For `PendingLogin::Activating`, it signs and queues a detach that either cancels the completed link or detaches an activation whose response was lost. It then saves `active: None` and `pending_login: None`; `false` means neither active nor pending login existed. State mutation and legacy initialization hold the exclusive native file lock; guarded reads accept an already-held shared guard and never migrate or write. Remote dispatch holds that shared guard through the HTTP response, so no lock upgrade occurs, a stale flush cannot overwrite logout, and logout cannot return while an older invocation is still dispatching.

- [ ] Add `it_migrates_a_legacy_attachment_once`. Seed the existing non-empty `LOCAL_ROOT_SITE` and `ACCOUNT_LINK_SITE` records, load the new state, and assert one matching active session with legacy `attachment_id = delegation_cid`; seed an empty provider tombstone and assert migration produces no active session.
- [ ] Add `it_logs_out_in_one_state_write`. Seed active state and unrelated root, trusted marker, spot/account files, and certificates; perform the transition; assert active is absent, one valid exact detach intent exists with A's provider routing metadata, and every unrelated byte/revision remains unchanged. Repeat logout and assert no duplicate intent. Repeat from `PendingLogin::Waiting` and `PendingLogin::Activating`: waiting is cancelled without an intent, while activating queues its exact generation for cancel/detach.
- [ ] Add `it_serializes_flush_logout_and_activation_across_processes`. Use two independently opened lock handles: hold a stale flush/read, start logout, and prove logout waits; then release and assert no stale save can restore `active`. Repeat with activation versus logout and verify the final state follows lock acquisition order rather than last unguarded write.
- [ ] Add a fault-injection test at the session-store seam: a failed save leaves the prior active state authoritative and returns an error; a successful save makes remote authority absent even if later best-effort legacy tombstone or HTTP work fails.
- [ ] Add `it_replaces_the_latest_root_record_without_deleting_old_certificates`. Install A, then B after logout; assert `save_local_root` no longer returns `this device already has a different local root`, B is the latest record, and a local proof dependent on A remains available.
- [ ] Run `cargo test -p tonk-cli --lib account_session`; expect failure because the module is absent.
- [ ] Implement versioned serialization, migration, and a cross-process shared/exclusive lock file under the CLI account directory using stable `std::fs::File::{lock_shared, lock}` APIs. Run `ensure_initialized` under an exclusive guard before constructing site/account operators. `load_guarded` and `active_guarded` require the caller's existing shared guard and are strictly read-only, avoiding shared→exclusive lock upgrades. Make `ACCOUNT_SESSION_SITE` the sole authority for account status/connections once present; keep `LOCAL_ROOT_SITE` and `ACCOUNT_LINK_SITE` as compatibility projections, not as the remote gate.
- [ ] Implement the recoverable login phases. Persist `PendingLogin::Waiting { provider, secret, token_hash }` before creating the remote link. After replayable consumption, validate/retain the grant and persist `PendingLogin::Activating { account }` before calling idempotent `/links/activate`. Only a confirmed activation may atomically move that account to `active`; startup and the next `account link` resume either pending phase instead of starting another ceremony.
- [ ] Add crash-point tests after pending-link save, browser completion, consume response, activating-state save, service activation, and final active-state save. Every restart must either resume to the same active attachment or remain logged out with enough information to detach/retry; none may leave an untracked active service row.
- [ ] Change `account link` to reject when `session.active.is_some()`, resume when `pending_login.is_some()`, and otherwise always run a new browser handoff. It must never reactivate a remembered historical grant.
- [ ] Run `cargo test -p tonk-cli --lib account::tests` and `cargo test -p tonk-cli --lib account_session`; expect success.

### Task 5: Store each reusable space prefix under its account root

**Files:**
- Modify: `rust/tonk-account/src/backup.rs:SPACE_ROOT_SITE_PREFIX and key helper`
- Modify: `rust/tonk-cli/src/site.rs:bootstrap_repository, mount_delegated_inner, account_root_prefix`
- Modify: `rust/tonk-cli/src/invite.rs:delegated prefix persistence`
- Modify: `rust/tonk-cli/src/account_spots.rs:restored prefix persistence if applicable`
- Test: `rust/tonk-cli/tests/account_spots.rs`
- Test: `rust/tonk-cli/tests/site.rs`

**Interfaces:**
- Produces `space_root_site(repository_did: &Did, account_root: &Did) -> String`, yielding `tonk-space-root-v2/<repository>/<account-root>`.
- Keeps read-only fallback for the legacy `tonk-space-root-v1/<repository>` key; a valid legacy prefix is copied to its validated root-specific v2 key.

- [ ] Add `it_keeps_distinct_reusable_prefixes_for_two_account_roots`. Save valid A→spot and B→spot authority for one repository, then load each by explicit root and assert neither overwrites or satisfies the other.
- [ ] Add `it_migrates_a_legacy_prefix_only_to_the_root_that_validates_it` and assert a B lookup cannot relabel an A prefix.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots prefix`; expect failure because the key is currently repository-only.
- [ ] Introduce the v2 helper and update every prefix write to include the validated account root. Preserve the v1 fallback only for migration; never use generic proof selection to relabel a prefix for another root.
- [ ] Run the focused prefix tests and `cargo test -p tonk-cli --features integration-tests --test account_spots`; expect success.

### Task 6: Make the site operator structurally account-bound for every network fork

**Files:**
- Create: `rust/tonk-cli/src/account_authority.rs`
- Modify: `rust/tonk-cli/src/lib.rs:module declaration`
- Modify: `rust/tonk-cli/src/site.rs:TonkSite.operator, derive_operator_for_profile, build_profile_and_operator`
- Modify: `rust/tonk-cli/src/account_state.rs:operator builders and remote operations`
- Test: `rust/tonk-cli/src/account_authority.rs:tests`
- Test: `rust/tonk-cli/tests/account_session.rs`

**Interfaces:**
- Consumes: the raw `dialog_operator::Operator<NativeSpace>`, exact device→operator session delegation minted during operator construction, current `AccountSessionState.active`, and the active-root v2 space prefix.
- Produces an owned `AccountBoundOperator` that forwards local storage/authority effects but owns `Provider<Authorize<Ucan>>` and `Provider<Fork<RemoteSite, Fx>>`.

The guarded authorization algorithm is:

1. After startup has completed exclusive legacy initialization, acquire the account-session shared remote guard, call the strictly read-only `active_guarded` with that guard, and retain it until network dispatch completes; if `active` is absent, return `AuthorizeError::Denied("log in with `tonk account link` before accessing a remote")`.
2. Ask the inner operator only for the fresh signer, scope, duration, and operator session context; discard its historically selected delegation chain.
3. Decode and validate the exact active `root → profile` grant and require its CID, issuer, and audience to match the active state and profile DID.
4. If the requested UCAN subject is the active account root (the account repository), use the active grant directly. Otherwise load `space_root_site(subject, active_root)`, validate it with `AccountSpotBackup::validate_for(active_root)`, and prepend it.
5. Append the exact profile→operator session delegation captured when the raw operator was built, construct one `DelegationChain`, and put that chain into the returned `dialog_ucan::UcanAuthorization`.
6. Implement `Fork<RemoteSite, Fx>` by acquiring the shared guard, authorizing the fork against `AccountBoundOperator` itself, dispatching through `RemoteSite::default()`, and releasing the guard only after dispatch returns. Forwarding the fork to the raw operator is forbidden because it bypasses this authorization provider. Avoid double-locking by passing one held guard through the wrapper's internal authorization helper.

- [ ] First add a compile-only provider-surface test that builds `TonkSite`, opens/commits locally, and type-checks fetch, pull, push, account hydration, archive/blob reads, and memory resolve/publish with `AccountBoundOperator`. Run `cargo test -p tonk-cli --lib account_authority`; expect missing provider implementations, then forward only the concrete provider traits currently exposed by `Operator`.
- [ ] Add pure chain-construction tests: logged out denies; malformed active grant denies; account A prefix with active B denies; B prefix plus B grant plus current operator session produces a chain containing B's active CID exactly once and no A CID.
- [ ] Add `it_sends_no_request_after_logout` with a counting TCP listener as the UCAN endpoint. Open the site while active, logout without rebuilding it, attempt fetch/pull and push, and assert an authorization error plus zero accepted HTTP requests. This proves the gate is re-read per request rather than snapshotted at site open.
- [ ] Add `it_orders_concurrent_logout_after_in_flight_dispatch`. Pause a remote fork after authorization but before HTTP, start logout in another process/task, and assert logout cannot complete until dispatch releases the shared guard. After logout returns, start another fork and assert the listener receives no additional request.
- [ ] Add `it_sends_only_the_latest_account_chain`. Retain valid A and B histories, activate B, capture the CBOR request at the listener, decode its invocation proofs, and assert B's active CID is present and A's is absent.
- [ ] Add `it_denies_a_spot_not_delegated_to_the_active_account_before_http`. Keep local A authority, activate B without a B prefix for that spot, and assert local commit succeeds while fetch/push accepts zero requests.
- [ ] Run `cargo test -p tonk-cli --lib account_authority`; expect failures before the wrapper exists.
- [ ] Implement the owned wrapper. Replace `TonkSite.operator` rather than keeping a public raw operator beside it. Return the wrapper from account-state operator builders as well, closing direct `branch.pull()`/`branch.push()` paths in `account_state.rs`. Change those builders to mint and retain the exact profile→operator session delegation explicitly so the wrapper never rediscovers a historical operator session.
- [ ] Update isolated providerless UCAN integration fixtures: tests intended to exercise remote access must install an explicit active test account; tests intended to exercise local-only unsafe fixtures must expect remote denial. Do not add an unsafe bypass to production authorization.
- [ ] Run `cargo test -p tonk-cli --lib`, `cargo test -p tonk-cli --test site`, and `cargo test -p tonk-cli --test sync`; expect success.

### Task 7: Queue offline detachment and flush it before later handoffs

**Files:**
- Modify: `rust/tonk-cli/src/account_session.rs:flush_pending`
- Modify: `rust/tonk-cli/src/account.rs:logout, link, account-service request helpers`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:logout output if warnings are surfaced`
- Test: `rust/tonk-cli/tests/account_session.rs`

**Interfaces:**
- `flush_pending` runs under the exclusive transition guard and POSTs each `PendingDetach.intent` to `PendingDetach.provider`. It removes only typed terminal outcomes (`detached`, `alreadyDetached`, `cancelledPendingActivation`, `superseded`, `revoked`). It retains `unknownAttachment`, `payloadMismatch`, malformed responses, timeouts, and `5xx` for retry and surfaces a warning containing the bounded service code.
- `logout` commits local state before any HTTP attempt. A detach retry is best-effort and cannot turn a completed local logout into failure.
- `link` first resumes any durable pending login, then calls `flush_pending` before creating a new handoff on that provider. If the old active generation at the same service cannot be detached, linking stops before opening the browser because the service's one-active-DID invariant would reject activation.

- [ ] Add `it_logs_out_offline_and_flushes_later`. Make `/devices/detach` unreachable, logout, assert immediate remote denial and one queued intent, restore the helper server, run the next account operation, and assert the queue is empty and the old device is absent from its visible list.
- [ ] Add `it_applies_typed_outbox_dispositions`. Remove entries for each terminal success outcome; retain entries for `unknownAttachment`, `payloadMismatch`, connection errors, `5xx`, malformed success bodies, and timeouts. Assert a replayed already-detached item does not remain forever.
- [ ] Add `it_cannot_switch_accounts_without_detaching_the_active_generation`. Account A is active; direct B link is rejected locally. Logout A offline; B link against the same service cannot start while A detach cannot flush. Once flushing succeeds, a fresh B passkey handoff and idempotent activation succeed.
- [ ] Add `it_ignores_a_stale_detach_after_reattachment`. Queue generation A, detach and complete generation B, replay A's intent, and assert B remains active.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_session`; expect failures before retry integration exists.
- [ ] Implement bounded HTTP retry semantics without background daemons. Retry at logout when online and at the beginning of later `account link`, `account status` hydration, device-list, revoke, and account-spots network operations; never retry during purely local spot commands.
- [ ] Keep successful CLI output `logged out` immediate. Print a warning, not an error exit, when remote detachment remains queued.
- [ ] Run the focused integration test; expect success.

### Task 8: Verify local work, account switching, and revocation remain separate

**Files:**
- Modify: `rust/tonk-cli/tests/account_session.rs`
- Modify: `rust/tonk-account-service/tests/service.rs`
- Modify: `rust/tonk-cli/README.md`
- Modify: `rust/tonk-account-service/README.md`

**Interfaces:**
- End-to-end behavior matrix:

```text
logged out + local query/edit/commit       allowed
logged out + fetch/pull/push/account sync  denied before HTTP
active A + A-rooted spot                   allowed using only A chain
active B + A-only spot                     local work allowed; remote denied
active B + B-rooted spot                   allowed using only B chain
offline logout A                           local detach succeeds; remote row temporarily stale
later successful outbox flush              A attachment hidden/detached
return to A                                fresh passkey handoff required
replay old detach after new login          new attachment remains active
revoked grant                              never reactivated; detach publishes no revocation
```

- [ ] Add an end-to-end test that links A, creates and pushes a spot, disconnects the service, logs out, edits and commits locally, and proves the revision and data survive process reopen.
- [ ] Continue that test by linking B with a fresh handoff. Assert B cannot push the A-only spot; grant B independent spot authority, retry, and inspect the received invocation to prove only B's chain was sent.
- [ ] Logout B, return to A, and assert a browser/passkey handoff is required even though A's historical certificates remain. After the handoff, push the offline revision and verify the remote converges without rewriting local history.
- [ ] Add a revocation regression proving detach creates no immutable revocation artifact, while `tonk account revoke` still publishes one and permanently invalidates the exact delegation CID.
- [ ] Update both READMEs with the state matrix, delayed device-list convergence, mandatory handoff on every login, one-active-account rule, and distinction between detach and revoke. Remove current claims that logout only writes a provider tombstone or deliberately leaves the device active indefinitely.
- [ ] Run `cargo test -p tonk-account`, `cargo test -p tonk-account-service --features helpers`, `cargo test -p tonk-cli`, and `cargo test -p tonk-cli --features integration-tests --test account_session --test account_spots`; expect success.
- [ ] Run `cargo build -p tonk-account-service --target wasm32-unknown-unknown` and the repository's existing wasm check for `tonk-ui`; expect success after the shared handoff shape change.
- [ ] Run `cargo fmt --all -- --check`; expect success.

## Handoff verification

- [ ] Run `rg -n "TBD|TODO|similar to|handle errors|write tests" plan/account-logout-cli.md`; expect no unresolved implementation placeholders (the command may match this verification sentence only).
- [ ] Inspect `git diff -- rust/tonk-account-service/migrations` and confirm there is one final `0006` migration, global one-active-DID enforcement, preserved detached/revoked history, and no account-scoped schema that allows simultaneous active memberships.
- [ ] Inspect every `Provider<Fork<RemoteSite, _>>` implementation reachable from `TonkSite` and account-state hydration; none may dispatch through a raw `Operator` without `AccountBoundOperator` authorization.
- [ ] Search `ACCOUNT_LINK_SITE`, `LOCAL_ROOT_SITE`, and `ACCOUNT_SESSION_SITE`. Confirm only the canonical session `active` field decides whether remote authorization exists; legacy records are migration/compatibility data.
- [ ] Capture access-service requests in tests and verify the strongest invariant directly: logged out sends zero requests; logged in sends one chain containing the latest active attachment CID and no historical account attachment CID.
- [ ] Confirm `git diff -- Cargo.lock` is empty unless the final implementation documents and justifies a required dependency change.

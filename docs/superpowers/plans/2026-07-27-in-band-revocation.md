# In-band Revocation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every durable remote chain pass through a stable `root → device` delegation, publish every valid revocation as a witnessed immutable R2 artifact, and make the access service enforce only revoked delegation CIDs from a locally replicated monotone set.

**Architecture:** Split local authority from provider registration. Every browser or CLI profile first stores a provider-neutral local-root record containing its passkey-derived root DID, credential ID, and `root → device` delegation. Space creation delegates `space → root`; durable open-invite claims and targeted invites terminate at the recipient root; the existing bounded `device → operator` session remains the final signing hop. Attaching an account stores provider metadata and backs up existing root-terminated chains, but never mints or rewrites authority. Revocations gain a signed path witness, are verified through one shared `tonk-identity` implementation, and are relayed into a dedicated global R2 bucket under `revocations/<target-cid>/<artifact-cid>`. The account-service D1 status becomes a best-effort projection. The access service lists and verifies the complete R2 prefix every 60 seconds, unions target CIDs into an in-memory set, applies the existing 10-minute stale grace, and deletes all issuer-DID and D1 matching.

**Tech Stack:** Rust 2024, dialog UCAN (`DelegationChain`, `InvocationChain`, canonical `Cid`), WebAuthn PRF, workers-rs R2/D1, axum service-worker routes, custom-elements UI, clap CLI, `dialog_common::test`.

**Design of record:** `docs/superpowers/specs/2026-07-27-in-band-revocation-design.md`.

**Written against:** commit `0499b86fa` (the design document has uncommitted revisions in the planning worktree; do not overwrite or revert them).

## Delivery order and safety

This plan is deliberately ordered so no deploy weakens current enforcement:

1. Ship witnessed artifacts and dual-write them to the new global relay while the existing D1 CID+DID screen remains active.
2. Ship local roots and root-first creation/join paths while the D1 screen still protects old and new chain shapes.
3. Recreate pre-release development accounts/spaces and verify every durable presign chain contains `root → device`.
4. Only then switch the access service from D1 to the replicated CID set and remove issuer matching.

Do not deploy Task 9 (the access-service cutover) before Tasks 2–8 and the Task 9 pre-cutover smoke pass. CID-only screening does not revoke the current direct `space → device` chains.

## Locked decisions

- **Local root and account attachment are separate records.** Use a new local credential site for the root record and a second site for provider metadata. Do not reinterpret `tonk-account-link-v1`; there is no data migration.
- **One stable grant per device.** A local-root record owns the exact unexpiring, subject-open, command-open `root → device` bytes and credential ID. Account creation submits those bytes; it never asks the passkey to mint a replacement.
- **Witness is signed.** Keep the dialog command and argument `['ucan', 'revoke']` / `revoke`. Add a `path` argument containing hex-encoded canonical `DelegationChain` bytes. Because it is an invocation argument, the revoker signs the witness and it cannot be substituted in transit.
- **Shared verification lives in `tonk-identity`.** It already owns dialog-shaped revocation minting and has only target-gated browser dependencies. Both services must call the same verifier; do not copy authority logic into either service.
- **Dedicated global bucket.** Use production/staging buckets `tonk-revocations` / `tonk-revocations-staging`, bound as `REVOCATIONS` to the account relay and access executor. Do not put the global set under account-root namespaces in `CHAINS`.
- **Immutable key shape.** The only write API computes `revocations/<target-cid>/<artifact-cid>` from verified bytes. It exposes no arbitrary key, overwrite, or delete operation. Identical re-publish is idempotent; different bytes necessarily have a different artifact CID.
- **R2 before D1.** A device revoke is accepted once the artifact is verified and durably written to R2. Updating `devices.status` happens afterwards and failure is logged/returned as stale projection metadata, never used to roll back or hide the revocation.
- **Open visit and durable join are different operations.** Opening an open invite extends its bearer key to the current bounded operator and mounts a guest replica without passkey, roster, or provider backup. “Join” extends the original invite to the local root, records membership, and may back up the root-terminated chain.
- **Targeted means root-targeted.** The scoped invite API names `recipient_root`, carries no seed, and can be durably accepted only when the local root DID equals that audience. No account lookup participates.
- **Account sign-out is not device revocation.** Detaching a provider clears provider metadata but keeps the local root grant and every space chain unchanged. Explicit device revoke remains permanent and is the only action that rotates/replaces a revoked device key.
- **No legacy migration.** Delete the roster re-key/re-anchor startup sweep after root-first paths land. Existing development profiles, spaces, account rows, account-chain backups, and old revocation objects are discarded before cutover.

## Global constraints

- Native and wasm tests use `#[dialog_common::test]` and names `it_does_x`.
- Existing wasm test modules retain `run_in_service_worker` / `run_in_browser` configuration.
- No `mod.rs`; use `foo.rs` plus `foo/` submodules.
- No phase/PR/design-doc references in production comments.
- Do not change the dialog rev pin. If Task 1 finds a missing public API, stop instead of bumping or patching dialog.
- No root seed or PRF output crosses the window boundary. The service worker receives only DIDs, credential IDs, and signed delegation/invocation bytes.
- No email/account identifier is passed to WebAuthn registration as the user handle or credential label.
- Never log an invite URL: its fragment is an authority-bearing seed.
- Full lint gate:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

- Full repository gates:

```bash
nix develop -c test:native:debug
nix develop -c test:web:debug
```

The release variants remain CI gates; run `nix develop -c test:native:release` and `nix develop -c test:web:release` before the final PR when resources permit.

---

### Task 1: Verify the dialog, R2, and client seams

**Files:** none modified.

- [ ] **Step 1: Verify canonical CID and path APIs at the pinned rev**

Run:

```bash
rg -n "pub fn (to_cid|proofs|proof_cids|issuer|audience|subject|command|expiration)|pub async fn verify_signature" \
  ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-ucan-core/src/{delegation.rs,invocation.rs,container/delegation.rs,container/invocation.rs}
```

Expected: delegation and invocation CIDs are public; `DelegationChain::proofs()` is ordered root-to-leaf; both token types expose signature verification; `InvocationChain::verify` remains public.

- [ ] **Step 2: Prove a `path` argument round-trips exact chain bytes**

Write a temporary ignored test or a throwaway `/tmp` program using the pinned crates: hex-encode `DelegationChain::to_bytes()`, put it in `Promised::String` under `path`, serialize/parse the invocation, and parse the decoded bytes back as the same chain. Do not commit the probe.

Expected: the path proof CIDs and order are unchanged.

- [ ] **Step 3: Verify the two authority cases the shared verifier will use**

Using temporary fixtures:

1. Root signs a proofless `ucan/revoke` invocation whose path is `root → device`; direct invocation signature verification succeeds and root equals the target delegation issuer.
2. Device signs `ucan/revoke` with `root → device` as invocation proof; full `InvocationChain::verify` succeeds and the target CID is among the invocation proofs.

Expected: both pass without an upstream change.

- [ ] **Step 4: Verify operator signing is available for invite revocation**

Inspect `dialog_operator::Operator` and the current invite mint path. Confirm either:

- the current bounded operator can be passed as an `InvocationBuilder` issuer; or
- `TonkState.profile.signer().signer().clone()` can sign a revocation while the witnessed invitation chain proves that device is in the path.

Expected: one current local signer can issue an invite revocation without retaining the ephemeral invite seed.

- [ ] **Step 5: Verify R2 pagination**

Run:

```bash
rg -n "pub fn (prefix|limit|cursor)|pub fn (truncated|cursor)" \
  ~/.cargo/registry/src/*/worker-0.8.5/src/r2/{builder.rs,mod.rs}
```

Expected: `Bucket::list().prefix(...).cursor(...).execute()`, `truncated()`, and `cursor()` are available.

- [ ] **Step 6: Verify the certificate store accepts root-terminated prefixes**

Add a temporary worker/native fixture that saves `space → root` and `root → device` separately, then asks dialog to prove a presign from the bounded operator. Parse the resulting invocation container with `collect_presented`.

Expected: the presented delegation CIDs contain both hops in root-to-leaf order. If dialog only proves when a pre-composed chain is saved, Task 5 must save `space → root → device` explicitly instead of relying on BFS composition.

**STOP conditions:**

- A `DelegationChain` parse does not guarantee principal-aligned path order: define and verify an explicit ordered witness envelope before continuing.
- Neither the operator nor device signer can sign invite revocations: change invite minting so its revocable hop is issued by the stable device signer; do not retain the space or invite signer as a workaround.
- The certificate store cannot present `root → device` from separately stored chains and cannot accept a composed chain: this requires an upstream dialog capability-store change and blocks root-first cutover.
- R2 list cannot expose a continuation cursor: use a separate append-only feed with detectable gaps; never treat one page as complete.

- [ ] **Step 7: Nothing to commit** — remove all temporary probes.

---

### Task 2: Make revocation artifacts self-contained and provider-independent

**Files:**
- Modify: `rust/tonk-identity/src/revocation.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs`
- Modify: `rust/tonk-identity/src/install.rs`
- Modify: `rust/tonk-identity/src/lib.rs` only if exports change
- Modify callers/tests in `rust/tonk-account-service/` that use the old mint signatures

**Interfaces:**

In `tonk_identity::revocation` produce:

```rust
pub const REVOKE_COMMAND: [&str; 2];
pub const REVOKE_ARGUMENT: &str;
pub const PATH_ARGUMENT: &str;

pub struct VerifiedRevocation {
    pub target_cid: String,
    pub artifact_cid: String,
    pub target_expires_at: Option<u64>,
    pub issuer: Did,
    pub authority: RevocationAuthority,
}

pub enum RevocationAuthority {
    PathIssuer,
    Delegated,
}

pub async fn verify(bytes: &[u8]) -> Result<VerifiedRevocation>;
pub async fn mint_root_revocation(root, path: &DelegationChain, target: &Cid) -> Result<Vec<u8>>;
pub async fn mint_self_revocation(device, grant: &DelegationChain, target: &Cid) -> Result<Vec<u8>>;
pub async fn mint_delegated_revocation(issuer, path: &DelegationChain, target: &Cid, proofs: &DelegationChain) -> Result<Vec<u8>>;
```

Use the concrete signer types the pinned builders accept; do not force dynamic dispatch merely to match this illustrative signature.

Verification algorithm:

1. Parse the invocation container and require exact command `['ucan', 'revoke']`.
2. Parse `revoke` as a canonical CID string and reject a string whose parse/re-render changes it.
3. Hex-decode and parse signed argument `path` as a non-empty `DelegationChain`.
4. Verify every path delegation signature and path principal alignment.
5. Find exactly one path delegation whose `to_cid()` equals `revoke`; reject no match or duplicate ambiguity.
6. Verify the revocation invocation signature.
7. Accept `PathIssuer` when the revocation issuer is an issuer in the witnessed prefix through the target delegation.
8. Otherwise require full `InvocationChain::verify`, require the target CID among its attached proofs, and return `Delegated`.
9. Return the target delegation expiration for future safe eviction and the invocation's canonical artifact CID for storage.

Do not accept “subject equals account root”, a D1 row, an account ID, or caller-supplied attestation as authority.

- [ ] **Step 1: Add failing verifier tests**

Add tests named:

- `it_verifies_a_root_revocation_with_the_target_path`
- `it_verifies_a_device_self_revocation_with_the_target_path`
- `it_verifies_an_invite_revocation_by_an_issuer_in_the_path`
- `it_rejects_a_decoy_path_that_omits_the_named_cid`
- `it_rejects_a_path_changed_after_signing`
- `it_rejects_an_unauthorized_issuer`
- `it_rejects_a_non_canonical_target_cid`
- `it_reports_the_target_delegations_expiration`

The invite fixture must be at least `space → member → invite-key`; revoke the leaf and prove an unrelated key cannot publish it.

- [ ] **Step 2: Run tests to verify failure**

```bash
cargo test -p tonk-identity revocation
```

Expected: compile/test failure because `path`, `verify`, and the new signatures do not exist.

- [ ] **Step 3: Implement minting and verification**

Keep the current dialog wire representation: invocation container, command `ucan/revoke`, target in `revoke`. Add only the signed `path` argument. Do not introduce ucanto `can/with/nb` fields or claim wire compatibility.

For self-revocation, attach the grant as invocation proof as today and also include it in `path`. The duplication is intentional: invocation proofs establish delegated signing authority; `path` gives every consumer one uniform target witness.

- [ ] **Step 4: Update browser ceremony input**

`window.tonkIdentity.signRevocation` must now take both `delegationCid` and `pathHex`. Parse the path before prompting where possible, derive the root, require the derived DID to be an authorized path issuer, and return `revocationHex`.

- [ ] **Step 5: Run crate and caller tests**

```bash
cargo test -p tonk-identity
cargo test -p tonk-account-service --features helpers revocation
cargo check -p tonk-identity --target wasm32-unknown-unknown
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-identity rust/tonk-account-service
git commit -m "feat(tonk-identity): add witnessed revocation artifacts"
```

---

### Task 3: Add the immutable global revocation relay and D1 projection

**Files:**
- Modify: `rust/tonk-account-service/Cargo.toml` (`tonk-identity` becomes a normal dependency)
- Create: `rust/tonk-account-service/src/revocations.rs`
- Create: `rust/tonk-account-service/src/revocations/r2.rs`
- Create: `rust/tonk-account-service/src/handlers/revocations.rs`
- Modify: `rust/tonk-account-service/src/lib.rs`
- Modify: `rust/tonk-account-service/src/handlers.rs`
- Modify: `rust/tonk-account-service/src/helpers/server.rs`
- Modify: `rust/tonk-account-service/src/core/devices.rs`
- Modify: `rust/tonk-account-service/src/core/revocation.rs` (delete account-scoped artifact verification/storage after callers move)
- Modify: `rust/tonk-account-service/src/store.rs`
- Modify: `rust/tonk-account-service/src/store/d1.rs`
- Modify: `rust/tonk-account-service/src/store/sqlite.rs`
- Modify: `rust/tonk-account-service/src/handlers/devices.rs`
- Modify: `rust/tonk-account-service/tests/service.rs`
- Modify: `wrangler.account.toml`

**Interfaces:**

```rust
pub const REVOCATION_PREFIX: &str = "revocations/";

pub trait RevocationStore {
    async fn put(&self, verified: &VerifiedRevocation, bytes: &[u8]) -> Result<PutOutcome, RevocationStoreError>;
}

pub fn object_key(verified: &VerifiedRevocation) -> String;
pub async fn publish<R: RevocationStore>(store: &R, bytes: &[u8]) -> Result<PublishOutcome, PublishError>;
```

`object_key` must return exactly `revocations/<canonical-target-cid>/<canonical-artifact-cid>` and must not accept either component separately from callers.

- [ ] **Step 1: Add failing native storage tests**

Use an in-memory store whose map is private to the implementation. Add:

- `it_keys_an_artifact_by_target_and_content_cids`
- `it_republishes_identical_bytes_idempotently`
- `it_keeps_distinct_valid_artifacts_for_the_same_target`
- `it_rejects_invalid_bytes_before_storage`
- `it_exposes_no_delete_or_arbitrary_key_operation` (compile-time/API review assertion in the test module documentation; the behavioral proof is that only `VerifiedRevocation` reaches `put`)

- [ ] **Step 2: Implement the store and R2 binding**

The R2 implementation wraps `ctx.bucket("REVOCATIONS")`. It only calls `put(object_key(...), bytes)`; no delete/list method belongs in the writer abstraction.

Add production/staging bindings:

```toml
[[r2_buckets]]
binding = "REVOCATIONS"
bucket_name = "tonk-revocations"

[[env.staging.r2_buckets]]
binding = "REVOCATIONS"
bucket_name = "tonk-revocations-staging"
```

Keep `CHAINS` for provider backup data; do not mix namespaces.

- [ ] **Step 3: Add `POST /revocations`**

Accept raw `application/cbor` artifact bytes. The endpoint has no account/device authorization: validity is self-certifying and publishing a valid denial needs no extra authority. Bound request size to a small explicit maximum (64 KiB is sufficient for the current chains); reject larger bodies before parsing.

Response on valid first or repeat publish: HTTP 202 JSON with `targetCid`, `artifactCid`, and `stored` (`false` for a known identical object if the backend can distinguish it). Invalid artifact: 400/403 according to malformed versus unauthorized. Never echo artifact bytes.

Mirror the route in the native helper so integration tests exercise the same verifier and store abstraction.

- [ ] **Step 4: Make device revoke publish first**

Change `/devices/revoke` to require a signed artifact for self and cross-device revocation. Keep the product authority rule:

- self: verified issuer is the target device and delegated authority verifies through that device's exact registered grant;
- cross-device: verified issuer is the account root;
- target CID equals the registered device grant CID.

Call global `publish` before changing D1. Delete the unsigned-self branch and its test.

- [ ] **Step 5: Make D1 a best-effort projection**

Add a store operation that marks the row matching `(account_id, delegation_cid)` revoked. Invoke it after R2 success. If it fails, log the internal error and still return success with `projection: "stale"`; if it succeeds return `projection: "updated"`. A subsequent reconciliation can replay R2, but reconciliation implementation is out of scope for this plan.

The sqlite/native test store gets the same method so ordering is testable.

Add tests:

- `it_writes_r2_before_projecting_device_status`
- `it_accepts_a_revocation_when_the_projection_fails`
- `it_never_projects_an_artifact_that_failed_verification`
- `it_requires_an_artifact_for_self_revocation`

Use a spy store to assert call order; do not infer order from final state.

- [ ] **Step 6: Retire the account-scoped artifact endpoint**

Delete `POST /devices/revocations` and account-root namespaced `revocations/root|device/...` storage. The global relay replaces it. Keep general chain backup endpoints unchanged.

- [ ] **Step 7: Run tests and wasm check**

```bash
cargo test -p tonk-account-service --features helpers
cargo check -p tonk-account-service --target wasm32-unknown-unknown
python3 -c "import tomllib; tomllib.load(open('wrangler.account.toml','rb')); print('ok')"
```

Expected: all pass and `ok`.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-account-service wrangler.account.toml Cargo.toml Cargo.lock
git commit -m "feat(tonk-account-service): publish revocations to an immutable relay"
```

---

### Task 4: Persist a provider-neutral local root before authority-bearing work

**Files:**
- Modify: `rust/tonk-identity/src/passkey.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs`
- Modify: `rust/tonk-identity/src/install.rs`
- Create: `rust/tonk-worker-api/src/identity.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Create: `rust/tonk-worker/src/router/identity.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/router/account.rs`
- Create: `rust/tonk-ui/src/identity_gate.rs`
- Modify: `rust/tonk-ui/src/lib.rs`
- Modify: `rust/tonk-ui/src/bin/ui.rs`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify crate `Cargo.toml` files only for required web-sys features

**Local storage shape:**

Use credential site `tonk-local-root-v1`. Store a versioned serialized record:

```rust
pub struct LocalRootRecord {
    pub version: u8,              // exactly 1
    pub credential_id: String,    // opaque WebAuthn credential id
    pub delegation: Vec<u8>,      // exact root → device chain bytes
}
```

The root DID and device DID are derived from the verified chain on every read; do not duplicate them in storage.

**Wire shape:**

```rust
pub enum RootStatus {
    Missing { device_did: String },
    Ready {
        root_did: String,
        device_did: String,
        credential_id: String,
        delegation_cid: String,
        delegation_hex: String,
    },
}

pub struct SaveRootRequest {
    pub credential_id: String,
    pub delegation_hex: String,
}
```

Routes: `GET /api/identity/root`, `POST /api/identity/root`.

- [ ] **Step 1: Make passkey registration provider-neutral**

Change `creation_options` / `create_passkey` so callers no longer supply email or account name. Generate a random opaque user name and user handle locally; use a fixed provider-neutral display label such as `Tonk identity`. Preserve discoverable credential, required user verification, PRF, and RP behavior.

Tests must assert the WebAuthn user entity does not contain a supplied email/provider string and remains opaque across two registrations.

- [ ] **Step 2: Add root ceremony functions**

Expose:

- `createRoot({ deviceDid })`: create passkey, reuse PRF-at-create when available, derive root, mint one `root → device`, return credential/delegation metadata.
- `evaluateRoot({ deviceDid })`: evaluate an existing discoverable passkey, derive root, mint `root → device`, return the same shape.

Neither function contacts an account provider or returns PRF/root secret material.

- [ ] **Step 3: Add failing worker API/state tests**

Add:

- `it_reports_a_missing_local_root`
- `it_persists_and_reloads_a_local_root`
- `it_rejects_a_grant_for_another_device`
- `it_rejects_replacing_a_ready_root_with_another_root`
- `it_accepts_an_idempotent_repeat_of_the_same_record`

- [ ] **Step 4: Implement root validation and persistence**

Reuse the exact shape checks currently in `account.rs::validate_link`: one proof, issuer root, audience current profile, subject-open, command-open, valid signature. Also save `UcanDelegation(chain)` into the profile access store so dialog can compose it into every proof.

Move generic grant validation out of `account.rs` into `identity.rs`. `account.rs` consumes local identity rather than owning it.

- [ ] **Step 5: Install the top-document identity gate**

The service worker will post messages shaped:

```json
{ "type": "identity-required", "intent": { "kind": "...", "...": "..." } }
```

`tonk-ui/src/identity_gate.rs` installs a top-document `navigator.serviceWorker` message listener and an accessible modal overlay. It offers two explicit actions: create a new passkey or use an existing passkey. On success it POSTs `/api/identity/root`, then replays the supplied intent through a normal worker API call. Keep the intent only in JS/Rust memory; never put an open invite URL into history, storage, logs, or analytics.

The initial intent variants are reserved here and implemented in Tasks 5 and 7:

- `createSpace { name, remote, template }`
- `durableJoin { url }`

A second message while one prompt is active is refused with visible “finish the current identity request” copy; never run concurrent passkey ceremonies.

- [ ] **Step 6: Add parsing/UI tests**

Native/pure tests cover only accepted message variants and omission of invite URLs from debug formatting. Browser tests mount the gate and assert create/use-existing buttons. CDP coverage is added in Task 11.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-worker-api -p tonk-identity
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-identity rust/tonk-worker-api rust/tonk-worker rust/tonk-ui
git commit -m "feat(identity): persist a provider-neutral local passkey root"
```

---

### Task 5: Make space creation root-first

**Files:**
- Modify: `rust/tonk-worker/src/router/repository.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-worker/src/router/navigate.rs` or create a sibling worker→page message helper
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-fab/src/element.rs` only for pending/error presentation if needed
- Modify: `rust/tonk-ui/src/identity_gate.rs`
- Modify: `rust/tonk-ui/src/api.rs`

**Interfaces:**

- `identity::local_root(&TonkState) -> Result<LocalRoot, TonkWorkerError>` (error when missing/malformed; no device fallback).
- `identity::root_did(&TonkState) -> Result<Did, TonkWorkerError>`.
- `POST /api/spaces` with `{ name, remote?, template? }`, returning the created repository key. This is the replayable API used by the identity gate; route and command paths call one `create_space` core.
- Worker helper `notify_identity_required(client, IdentityIntent)` matching `notify_navigate`'s client resolution but never logging the payload.

- [ ] **Step 1: Add failing root-first chain tests**

Cover both direct HTTP and command cores:

- `it_refuses_space_creation_without_a_local_root`
- `it_delegates_a_new_space_to_the_local_root`
- `it_never_mints_a_direct_space_to_device_grant`
- `it_presents_space_root_device_and_session_cids_for_presign`
- `it_keeps_the_root_device_grant_cid_stable_across_spaces`

The presign test must inspect the actual dialog-produced invocation container, not only chains constructed in the test.

- [ ] **Step 2: Replace `member_did` fallback**

Delete “root when linked, device when unlinked.” Membership identity is now always the local root for durable spaces. All durable callers handle missing root explicitly.

Do not change device-local `Replica(profile, subject)` metadata; the profile/device DID remains the local replica key. Only authority and shared roster identity become root-first.

- [ ] **Step 3: Delegate `space → root`**

In `create_repository`, load the verified local root before generating the repository signer. Mint a subject-specific, command-open `space → root` delegation. Save either:

- the root-terminated prefix plus the independent `root → device` grant if Task 1 proved BFS composition; or
- an explicitly composed `space → root → device` chain.

Never mint `space → device`. Drop the repository signer after the prefix is safely persisted.

- [ ] **Step 4: Persist the root prefix for later provider backup**

Store exact `space → root` bytes under a versioned per-space credential site such as `tonk-space-root-v1/<subject-cid-or-did>`. This is local provider-neutral state, not account state. `account_backup` reads this prefix; it must not ask the discarded space signer to mint another delegation.

Add `it_reads_the_same_space_root_cid_when_sync_is_enabled_later`.

- [ ] **Step 5: Gate both create entry points**

- `POST /api/spaces`: missing root returns HTTP 409 with machine code `ROOT_REQUIRED`.
- `CreateSpaceHandler`: missing root posts `identity-required/createSpace` to the originating client and creates nothing.
- Once the gate persists a root, it replays through `POST /api/spaces`; the response key is navigated to through `tonk_host::navigate_to`.

Do not commit a direct device grant while waiting for the ceremony and “upgrade” it later.

- [ ] **Step 6: Change backup to consume stored prefixes**

`back_up_owned_space` reads the stored `space → root` prefix. It remains a no-op without an attached provider or remote URL. Account attachment in Task 6 can sweep these prefixes without changing any authority CID.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-worker --features helpers repository
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo test -p tonk-fab
```

Expected: root-first tests pass; existing creation tests use a seeded local-root fixture.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-worker rust/tonk-fab rust/tonk-ui
git commit -m "feat(tonk-worker): create every space through the local root"
```

---

### Task 6: Turn account creation/linking into provider attachment

**Files:**
- Modify: `rust/tonk-worker-api/src/account.rs`
- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router/restore.rs`
- Modify: `rust/tonk-worker/src/worker.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs`
- Modify: `rust/tonk-identity/src/install.rs`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/account.html`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify: `rust/tonk-cli/src/account.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Modify: account-service tests as required; account-service wire fields may remain the same

**Local provider record:** credential site `tonk-account-provider-v1`, versioned JSON containing at least provider base URL and attachment time. It contains no authority bytes.

**Account status:**

```rust
pub enum AccountStatus {
    RootMissing { device_did: String },
    Unregistered { root_did: String, device_did: String },
    Registered { root_did: String, device_did: String, provider: String },
}
```

- [ ] **Step 1: Add failing separation tests**

- `it_reports_an_unregistered_local_root_without_an_account`
- `it_attaches_a_provider_without_replacing_the_root_grant`
- `it_leaves_all_space_delegation_cids_unchanged_on_attachment`
- `it_detaches_a_provider_without_revoking_or_rotating_the_device`
- `it_creates_and_edits_a_space_when_the_provider_is_unreachable`

Capture all local root and space proof CIDs before and after attachment and compare exact sorted vectors.

- [ ] **Step 2: Rework account creation ceremony**

`createAccount` no longer calls `create_passkey` or `mint_device_delegation`. Inputs include the expected local root DID, credential ID, and existing delegation hex from `GET /api/identity/root`. It evaluates the passkey, derives the root, requires exact DID equality, and signs `account/create` with those existing fields.

The account service continues verifying root signature, email code, credential ID, and the exact `root → device` delegation. Add a negative test for a signed account-create invocation whose delegation belongs to a different device/root.

- [ ] **Step 3: Keep new-device login as one combined ceremony**

When a new browser has no local root and selects “Log in”, evaluate the discoverable passkey, derive the existing root, mint that root's one stable grant to the new device, submit `/devices/link`, persist the local-root record, then persist the provider record. Remote success precedes provider marking; local-root persistence can be retried without repeating the remote mutation.

- [ ] **Step 4: Replace local account link persistence**

`POST /api/account/attach` validates that the provider ceremony root/device/delegation exactly match the already stored local-root record, then stores only provider metadata. It must not call `profile.access().save` for a newly minted grant because no new grant exists.

Delete post-link roster migration/re-anchor dispatch. Keep restore and backup dispatch, now provider-service operations over already root-shaped chains.

- [ ] **Step 5: Back up existing root prefixes after attachment**

Enumerate profile spaces, read each `space → root` / invite→root prefix, and best-effort upload those with configured remote URLs. Then run restore. Neither operation writes roster identity or mints authority.

- [ ] **Step 6: Change sign-out semantics**

`DELETE /api/account` clears only `tonk-account-provider-v1` and returns the `Unregistered` state. Remove self-revoke and profile rotation from account sign-out. Keep explicit device revocation in device management; if a later UI adds “revoke this device,” it must run the witnessed self-revoke and then provision a fresh `root → new-device` grant.

Update copy from “Sign out and revoke this device” to “Disconnect account services on this device,” with a separate explanation that space authority remains local.

- [ ] **Step 7: Update CLI account handoff**

The existing provider-backed browser handoff still mints `root → CLI-device`, but persistence writes the local-root record first and provider metadata second. `tonk account status` distinguishes unregistered root from registered provider. `tonk account link` refuses only when that provider is already attached, not merely because a root grant exists.

- [ ] **Step 8: Run tests**

```bash
cargo test -p tonk-identity
cargo test -p tonk-worker-api -p tonk-cli
cargo test -p tonk-account-service --features helpers
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add rust/tonk-identity rust/tonk-worker-api rust/tonk-worker rust/tonk-ui rust/tonk-cli rust/tonk-account-service
git commit -m "refactor(account): attach provider services to the local root"
```

---

### Task 7: Split open-invite visits from durable root joins

**Files:**
- Modify: `rust/tonk-invite/src/lib.rs`
- Modify: `rust/tonk-worker-api/src/join.rs`
- Modify: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-schema/src/invitation.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-fab/src/markup.rs`
- Modify: `rust/tonk-fab/src/element.rs` or add a focused membership element
- Modify: `rust/tonk-ui/src/identity_gate.rs`
- Modify: `rust/tonk-ui/src/api.rs`

**Interfaces:**

- `POST /api/profile/visit` with full invite URL: open invites only, no root required.
- `POST /api/profile/join`: durable operation, root required.
- `GET /api/repository/{repo}/membership`: returns `guest` or `durable` for the active local profile.
- A versioned local guest record stores the original open invite URL only as long as needed to offer durable join. It is never sent to account backup or shared roster data.

- [ ] **Step 1: Add failing `tonk-invite` tests**

- `it_visits_an_open_invite_by_delegating_to_a_bounded_guest_session`
- `it_visits_without_a_passkey_root`
- `it_refuses_to_visit_a_targeted_invite_as_a_guest`
- `it_joins_an_open_invite_to_the_recipient_root`
- `it_accepts_a_targeted_invite_only_for_its_root_audience`

The visit chain must be `... → invite-key → operator` and bounded no later than the current operator session. The durable chain must be `... → invite-key → root`; dialog later composes `root → device → operator`.

- [ ] **Step 2: Implement guest visit**

Parse the open invite, import its seed, delegate subject-specific authority to the current bounded operator, save the resulting chain for proof, mount/pull the replica, and record a local guest marker. Do not write `Membership`, `MemberRole`, `MemberName`, `InvitedVia`, or account backup.

Persisting the original URL locally is allowed so the user can later join, but treat it as credential material: versioned credential site, never branch facts, logs, errors, analytics, or response JSON.

- [ ] **Step 3: Change `/join` open-link behavior**

The on-mount join handler calls `visit` for `InviteAudience::Open`, then navigates into the space. No identity message or passkey prompt occurs. A targeted invite goes directly to durable acceptance and, if no root is present, emits `identity-required/durableJoin` instead of falling back to the device DID.

- [ ] **Step 4: Implement explicit durable join**

The in-space “Join this spot” action loads the stored original invite, requires the local root, calls `Invite::claim(root_did)`, saves the root-terminated chain, records root-keyed roster/provenance, removes the local guest marker/URL, and backs up only if a provider is attached.

If root is missing, post the full URL only in the in-memory `identity-required` message. The identity gate provisions the root and replays `POST /api/profile/join`.

- [ ] **Step 5: Preserve open bearer semantics in tests**

Add an integration fixture that:

1. visits and writes with no root;
2. mints/revokes a descendant grant and proves the holder of the original invite seed can mint another descendant;
3. revokes the delegation to the invite key and proves both the guest and durable descendant chain contain the revoked target CID.

This can use an in-memory revocation set; no R2 server is needed.

- [ ] **Step 6: Pin targeted invite vocabulary**

Rename public request field `audience` to `recipient_root` (camelCase `recipientRoot`) while keeping `InviteAudience::Scoped` internally if useful. Require no seed/fragment and exact root audience equality. The API accepts a DID directly; account discovery UX remains out of scope.

Add:

- `it_mints_a_targeted_invite_to_an_unregistered_root`
- `it_rejects_a_targeted_invite_claimed_by_another_root`
- `it_does_not_put_a_bearer_seed_in_a_targeted_url`

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-invite -p tonk-schema -p tonk-worker-api
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo test -p tonk-fab
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-invite rust/tonk-worker-api rust/tonk-worker rust/tonk-schema rust/tonk-core rust/tonk-fab rust/tonk-ui
git commit -m "feat(invite): separate open visits from durable root joins"
```

---

### Task 8: Publish invite revocations through configured remote relays

**Files:**
- Modify: `rust/tonk-invite/src/lib.rs`
- Modify: `rust/tonk-schema/src/remote.rs`
- Modify: `rust/tonk-schema/src/invitation.rs`
- Modify: `rust/tonk-worker/src/router/repository.rs`
- Modify: `rust/tonk-worker/src/router/create_invite.rs`
- Create: `rust/tonk-worker/src/router/revoke_invite.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-cli/src/remote.rs`
- Modify: `rust/tonk-cli/src/invite.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`

**Remote metadata:** add optional explicit revocation submission URL alongside the access address. For Tonk defaults, configure `https://accounts.tonk.xyz/revocations` and staging equivalent at remote creation; do not derive authority from that hostname. Carry the URL in invite data so visitors can publish against the same executor's relay. This is configuration, not a signed authority claim.

- [ ] **Step 1: Extend invite/remote round-trip tests**

- `it_round_trips_the_revocation_submission_url`
- `it_keeps_existing_invites_without_relay_metadata_parseable`
- `it_records_the_invitation_target_cid_and_public_path`

Extend `Invitation` with the canonical target delegation CID and public chain/path bytes needed to revoke it. The chain contains no open invite seed and is safe to replicate.

- [ ] **Step 2: Add invite revoke core**

Given a recorded invitation, load its path, target its leaf delegation CID, sign with the current authorized path principal established in Task 1, call `tonk_identity::revocation::verify` locally as a preflight, then POST raw bytes to the configured relay.

Add route `POST /api/repository/{repo}/invites/{target_cid}/revoke`. Refuse a target CID not recorded for that repository. The executor may deny any request, but the client must not accidentally publish a valid revocation for an unrelated space through a free-form CID parameter.

- [ ] **Step 3: Add tests for revocation granularity**

- Revoking an open invite hop invalidates every route descending from it.
- Revoking one durable descendant does not close the open invite.
- Revoking a targeted invite leaves unrelated targeted/open branches untouched.
- Relay outage does not claim success; the invitation remains visibly open/retryable.

- [ ] **Step 4: Add CLI parity**

Remote configuration accepts `--revocation-url`. `tonk invite` carries it. Add a focused invite-revoke command only if the CLI already retains the invitation path after minting; otherwise print a clear unsupported message and leave CLI revoke UI to a follow-up. Do not retain an open seed merely to implement revoke—the public path is sufficient.

- [ ] **Step 5: Run tests**

```bash
cargo test -p tonk-invite -p tonk-schema -p tonk-cli
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-invite rust/tonk-schema rust/tonk-worker rust/tonk-cli
git commit -m "feat(revocation): publish invite revocations through remote relays"
```

---

### Task 9: Replace D1 screening with the replicated monotone CID set

**Files:**
- Modify: `rust/tonk-access-service/Cargo.toml`
- Rewrite: `rust/tonk-access-service/src/revocation.rs`
- Delete: `rust/tonk-access-service/src/revocation/d1.rs`
- Create: `rust/tonk-access-service/src/revocation/r2.rs`
- Modify: `rust/tonk-access-service/src/handlers/ucan.rs`
- Modify: `rust/tonk-access-service/src/lib.rs`
- Modify: `rust/tonk-access-service/tests/ucan_integration.rs`
- Modify: `wrangler.toml`

**Core state:**

```rust
pub struct RevocationSnapshot {
    revoked: HashSet<String>,
    seen_artifacts: HashSet<String>,
    refreshed_at_ms: Option<u64>,
}

pub trait RevocationSource {
    async fn complete_listing(&self) -> Result<Vec<StoredArtifact>, SourceError>;
}

pub enum SetVerdict {
    Allowed,
    AllowedStale(String),
    Revoked,
    Unavailable(String),
}
```

The production R2 source may stream pages internally rather than allocate all bytes, but the pure decision seam must make “complete listing succeeded” explicit.

- [ ] **Step 1: Add failing native set tests**

- `it_unions_verified_target_cids_without_removing_old_entries`
- `it_republishes_an_identical_artifact_idempotently`
- `it_paginates_until_r2_reports_complete`
- `it_does_not_advance_freshness_after_a_partial_listing`
- `it_does_not_advance_freshness_after_an_invalid_artifact`
- `it_rejects_a_known_revoked_cid_even_during_an_outage`
- `it_serves_a_complete_stale_set_inside_the_grace_window`
- `it_fails_closed_without_a_complete_set_past_grace`
- `it_checks_only_presented_delegation_cids`

Use a scripted source with page/fetch failures; do not require wasm or R2.

- [ ] **Step 2: Simplify presented credentials**

Keep `collect_presented`'s invocation proof CIDs, token delegation CIDs, and time window. Delete `invocation_issuer`, `delegation_issuers`, `keys()`, `revoked_query`, per-key verdicts, and the registry trait.

A matching test asserts that a revoked issuer DID string not present as a delegation CID has no effect, while the same chain's revoked `root → device` CID is rejected.

- [ ] **Step 3: Implement monotone refresh semantics**

On stale/missing snapshot:

1. list every `revocations/` page;
2. fetch unseen object bytes;
3. verify each with `tonk_identity::revocation::verify`;
4. require verified target/artifact CIDs to match both path components of the object key;
5. union target and artifact CIDs into state;
6. update `refreshed_at_ms` only after all pages, fetches, and verifications succeed.

It is safe to union valid objects observed before a later page failure because the set is monotone, but that partial attempt must not refresh the authoritative timestamp.

Known revoked CIDs remain rejected indefinitely. Clean chains use the existing 60-second fresh / additional 10-minute stale grace. A source failure with no complete snapshot returns `REVOCATION_UNAVAILABLE`.

- [ ] **Step 4: Implement paginated R2 source**

Bind `REVOCATIONS`, list prefix `revocations/`, follow every cursor, and get each unseen object. Reject malformed key shapes before fetch where possible. Do not use a mutable index object or D1 snapshot.

- [ ] **Step 5: Change handler injection**

Keep window checking first. Then call the plain-data set screen with the presented `delegation_cids`. The handler obtains only `ctx.bucket("REVOCATIONS")`; no database binding exists.

Native tests instantiate the screen with a memory/scripted source. Do not cfg-gate the decision core to wasm.

- [ ] **Step 6: Change configuration**

In `wrangler.toml`:

- delete both `ACCOUNTS_DB` bindings and comments;
- add production/staging `REVOCATIONS` R2 bindings to the dedicated buckets.

Drop workers-rs `d1` feature from `tonk-access-service` if nothing else uses it.

- [ ] **Step 7: Pre-cutover staging gate — mandatory**

Before deploying this commit:

1. Recreate staging profiles/account/spaces under Tasks 4–8.
2. Capture presign containers for created, durable-open-join, and targeted chains.
3. Assert every durable container includes that device's exact registered `root → device` CID.
4. Publish a witnessed device revocation to staging R2 and confirm its key shape.
5. Confirm the current D1 screen rejects the device before proceeding.

If any durable route omits the grant CID, stop. Do not deploy CID-only screening.

- [ ] **Step 8: Run tests**

```bash
cargo test -p tonk-access-service --features helpers
cargo check -p tonk-access-service --target wasm32-unknown-unknown
python3 -c "import tomllib; tomllib.load(open('wrangler.toml','rb')); print('ok')"
cargo clippy -p tonk-access-service --all-targets --all-features -- -D warnings
cargo fmt --check
```

Expected: all pass and `ok`.

- [ ] **Step 9: Commit**

```bash
git add rust/tonk-access-service wrangler.toml Cargo.toml Cargo.lock
git commit -m "feat(tonk-access-service): enforce a replicated revocation set"
```

---

### Task 10: Bring native CLI creation and joining onto local roots

**Files:**
- Modify: `rust/tonk-cli/src/identity.rs`
- Modify: `rust/tonk-cli/src/account.rs`
- Modify: `rust/tonk-cli/src/site.rs`
- Modify: `rust/tonk-cli/src/spot.rs`
- Modify: `rust/tonk-cli/src/invite.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Modify: `rust/tonk-ui/src/identity_gate.rs` / top-document route support for browser handoff

**Ceremony:** Native platforms do not receive PRF output. Add `tonk identity link` using a provider-free browser handoff: open a top-document URL whose fragment names the CLI device DID and a random display challenge; create/use the passkey in the browser; mint `root → CLI-device`; display a base58/hex one-time response for the user to paste back into the CLI. The delegation audience cryptographically binds the response to that CLI profile. No account-service endpoint participates.

Do not put the response in a query sent to the server and do not use email/account metadata. If an acceptable local callback can be proven across supported browsers without mixed-content/CORS exceptions, it may replace copy/paste, but copy/paste is the baseline and must remain available.

- [ ] **Step 1: Add local-root persistence shared with CLI account state**

Refactor `tonk-cli/src/account.rs`'s `ACCOUNT_LINK_SITE` handling into a provider-neutral local-root module plus provider metadata, matching browser record validation. Existing account handoff calls the same persistence helper.

- [ ] **Step 2: Refuse native creation without a root**

`TonkSite::init*` / `bootstrap_repository` must load the local root and mint `space → root`, never `space → profile`. `tonk spot new` reports: “A local passkey root is required; run `tonk identity link`.”

Add:

- `it_refuses_spot_creation_without_a_local_root`
- `it_creates_a_spot_with_space_root_device_authority`
- `it_does_not_change_the_space_chain_when_an_account_is_attached`

- [ ] **Step 3: Make native durable join root-targeted**

`tonk join` is the durable verb and requires a local root. It claims open invites to root and requires targeted invite audience equality. Persist the root-terminated chain and root-keyed roster.

If a passkey-free native visitor mode is product-required now, add a separate `tonk visit <url>` command that delegates the open invite to the bounded operator and does not write durable membership. Do not silently retain the old `join → profile DID` behavior.

- [ ] **Step 4: Make native invite minting root-chain aware**

Existing open invite minting continues from the site's valid authority path; targeted mint accepts `--recipient-root`. Include configured revocation relay metadata and persist the public invitation path/CID for later revoke.

- [ ] **Step 5: Run CLI tests**

```bash
cargo test -p tonk-cli
cargo clippy -p tonk-cli --all-targets --all-features -- -D warnings
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-cli rust/tonk-ui
git commit -m "feat(cli): require local roots for durable spaces and joins"
```

---

### Task 11: Delete superseded migration paths and complete end-to-end verification

**Files:**
- Delete or reduce: `rust/tonk-worker/src/router/migrate.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/worker.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router/restore.rs`
- Modify: `rust/tonk-ui/src/identity.rs`
- Modify: relevant READMEs (`tonk-identity`, account service, access service, invite, UI)
- Modify: `docs/superpowers/plans/implementation-notes.md` only if execution uncovers deviations worth preserving

- [ ] **Step 1: Delete authority migration/re-anchor code**

Remove startup/link calls to `migrate_rosters`, `migrate_space_roster`, `reanchor_space`, and backup helpers for `space → ... → device → root`. Restore accepts only root-terminated backup chains and composes the local root grant.

Delete comments/copy that say unlinked users are device identities or that pre-account spaces may need fresh invites. “Unregistered” means a local root without provider services.

- [ ] **Step 2: Add offline/provider boundary tests**

- An unregistered local root creates, edits, invites, and (with a configured remote fixture) syncs without account-service calls.
- Account attachment changes no authority chain/CID.
- The same chain authorizes against two access-service fixtures sharing verified revocation artifacts but no D1 database.
- A device revocation projected stale in D1 is nevertheless enforced from R2.
- A D1 status flip with no valid artifact has no effect on access.

- [ ] **Step 3: Add CDP browser scenario**

Extend `rust/tonk-ui/src/identity.rs` with one serial scenario:

1. install PRF virtual authenticator;
2. create a local root without requesting an email code;
3. create a synced space and capture its presented delegation CIDs;
4. attach an account using the same passkey/root/grant;
5. assert the chain and CIDs are unchanged;
6. create a second device grant;
7. publish a root-signed witnessed revocation;
8. refresh the access set and assert the second device receives 403 while the first still syncs.

Use a controllable test clock/source rather than sleeping 60 seconds.

- [ ] **Step 4: Add browser open-invite scenario**

In a fresh browser/profile with no passkey:

1. open an open invite;
2. pull and write successfully;
3. assert no root record and no durable roster row;
4. choose Join, create/evaluate passkey, and assert root-keyed membership plus `root → device` in presign;
5. revoke the invite hop and assert both guest and joined descendant routes are denied.

- [ ] **Step 5: Recreate staging data**

Because migration is explicitly out of scope, remove development staging accounts, account-chain backups, revocation objects, and spaces created under the old chain shape. Do not write migration code or import old direct grants into the new system.

Record the exact operational reset commands in the private deployment runbook/PR body, not in production source comments.

- [ ] **Step 6: Full gates**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
nix develop -c test:native:debug
nix develop -c test:web:debug
nix develop -c test:native:release
nix develop -c test:web:release
```

Expected: all green.

- [ ] **Step 7: Manual staging smoke**

- Unregistered passkey root creates a local-only and synced spot with account service unavailable.
- Account attachment leaves captured CIDs unchanged and backup/restore works on a second device.
- Open link reads/writes without passkey; Join makes root-keyed durable membership.
- Targeted invite to an unregistered root succeeds only for that root.
- Root-signed device revoke writes `revocations/<grant-cid>/<artifact-cid>` and is enforced within one refresh interval.
- Self-revoke writes the same global shape and requires no root prompt.
- Open-invite revoke kills all descendants; descendant revoke does not close the bearer link.
- Rename/remove `ACCOUNTS_DB` from the access worker has no effect because the binding no longer exists.
- Make `REVOCATIONS` unavailable: fresh snapshot works for 60 seconds and stale grace for 10 further minutes; unseen/too-stale clean chains receive retryable 503.
- Inspect logs to confirm invalid artifacts and incomplete pagination never advance freshness.

- [ ] **Step 8: Update documentation**

Document:

- local root versus optional provider account;
- open visit versus durable join;
- targeted root invites;
- dialog artifact fields `revoke` and `path` and explicit non-compatibility with ucanto bytes;
- relay submission and R2 object shape;
- executor freshness/grace behavior;
- permanent growth for unexpiring grant revocations;
- current `tonk.spot` RP-origin dependency.

- [ ] **Step 9: Commit**

```bash
git add rust docs wrangler.toml wrangler.account.toml Cargo.toml Cargo.lock
git commit -m "refactor(identity): remove device-first authority migration"
```

---

## Out of scope

- Migrating old device-first profiles, spaces, account backups, or old account-scoped revocation objects.
- Root rotation and passkey-loss recovery.
- Proving WebAuthn provenance in UCAN.
- Read-only, single-use, or attenuated open links.
- Targeted-invite discovery/request UI; the implementation exposes direct root-DID targeting only.
- A portable standard for discovering relay/mirror URLs. This plan stores explicit configured URLs.
- Fixing dialog's unbounded `prove` request or session-rotation workaround.
- Revocation-set compaction beyond retaining target expiration metadata and testing the safe predicate. Unexpiring device/invite/member grants remain forever.
- A D1 reconciliation worker. The relay response and logs expose stale projection; reconciliation can replay the global set later without changing enforcement.

## Review checklist

- [ ] No durable creation/join path calls `delegate(profile.did())` or falls back to a device DID.
- [ ] Every captured durable presign contains the exact local `root → device` CID.
- [ ] Open visit is the only identity-bearing exception and contains the invite hop that closes it.
- [ ] Account attach/detach changes no delegation bytes or CIDs.
- [ ] Every R2 object was accepted by the shared verifier and its key matches verified CIDs.
- [ ] No write failure can produce D1 `revoked` without a durable artifact.
- [ ] No D1 state or issuer DID participates in access authorization.
- [ ] Partial/invalid R2 refresh cannot advance snapshot freshness.
- [ ] Failed refresh cannot slide stale grace.
- [ ] Root PRF/seed never leaves the top document and invite seeds never enter logs/history/analytics.

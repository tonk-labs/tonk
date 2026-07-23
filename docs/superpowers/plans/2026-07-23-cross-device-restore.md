# Cross-device Restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A linked device restores both claimed and created spaces from the account service and mounts them locally, so a user's spaces follow them across devices. Created spaces gain a backed-up `space -> root` delegation so they are restorable.

**Architecture:** Two backup producers (claim — already done in 3A — and the create/remote-attach path, new) escrow a `{chain, remote_url}` artifact to the account service. One restore consumer pulls all artifacts and mounts each space through a shared `mount_replica` helper extracted from the claim path. Restore writes no content roster — the roster arrives over sync. Triggers are best-effort/fire-and-forget on device link and startup.

**Tech Stack:** Rust (edition 2024). `dialog-*` pinned at `tonk-2026-07-17`. Builds on 3A: `tonk-worker/router/account_backup.rs`, `tonk_identity::request::build_device_invocation`, `crate::router::account::{account_link, member_did}`.

## Global Constraints

- Work on branch `feat/cross-device-restore` (already cut from `feat/root-did-rosters`; carries the design doc). Rebase onto `staging` once 3A (#637) merges.
- Do NOT bump the pinned dialog tag `tonk-2026-07-17`.
- `Subject::Any` is only ever the `root -> device` link. The `space -> root` delegation is subject-SPECIFIC (subject = the space DID). Never mint `Subject::Any` for a space.
- Backup and restore are BEST-EFFORT and FAIL-OPEN: they never fail a claim/create/link/boot or block local work. Dispatch detached so a slow account service can't stall.
- Restore writes NO content-branch roster. `record_membership_on_content` asserts `MemberRole` unconditionally + cardinality-one; a restore-time `member` stamp would demote a founder on a created space. The roster is authoritative on the content branch and arrives over sync.
- Tests: `#[dialog_common::test]`; `tonk-worker` wasm test mods carry `wasm_bindgen_test_configure!(run_in_service_worker)`. No `mod.rs`. Conventional Commits, no emojis. Self-contained comments (no "3b"/spec references).
- **Wasm/service-worker tests HANG in the local sandbox** (no browser automation). Verify wasm-gated code by COMPILING (`cargo clippy -p tonk-worker --all-targets` + `cargo build -p tonk-worker --target wasm32-unknown-unknown`), never `test:web:debug`. Native tests (`cargo test`) run normally. CI's `web` matrix leg executes the wasm tests.
- Lint gate before each PR: `cargo clippy --workspace --all-targets --all-features` + `cargo fmt --check`.

## File Structure

- `rust/tonk-worker/src/router/repository.rs` — split `record_repository_meta` into `record_replica_meta` (meta + profile index) + the content-roster call; `ensure_remote_config` gains the created-space backup hook.
- `rust/tonk-worker/src/router/join.rs` — extract `mount_replica`; rewire `claim_invite` onto it.
- `rust/tonk-worker/src/router/account_backup.rs` — add `back_up_owned_space` (create producer) and the `list`/`get` restore client; grows the shared device-signed request plumbing.
- `rust/tonk-worker/src/router/restore.rs` — **new**: the restore consumer (`restore_spaces`).
- `rust/tonk-worker/src/router.rs` — declare `mod restore;`.
- `rust/tonk-worker/src/router/account.rs` — link handler triggers restore.
- `rust/tonk-worker/src/worker.rs` — startup triggers restore for a linked profile.

---

## PR 1 — Mount extraction + created-space backup (Tasks 1-3)

### Task 1: Split `record_repository_meta` — carve out the content-roster write

**Files:**
- Modify: `rust/tonk-worker/src/router/repository.rs`

**Interfaces:**
- Produces: `pub(crate) async fn record_replica_meta<C: Principal + Clone>(tonk: &TonkState, repository: &Repository<C>, display_name: &str, configuration: &RepositoryConfiguration) -> Result<(), RepositoryError>` — everything `record_repository_meta` did EXCEPT the `record_membership_on_content` call. Consumed by Task 2.
- `record_repository_meta` keeps its exact signature and behavior (now = `record_replica_meta` + the content-roster call).

- [ ] **Step 1: Rename the body and re-express `record_repository_meta` as a thin wrapper**

In `repository.rs`, rename the existing `record_repository_meta` function body to `record_replica_meta` by:
1. Changing the signature at line 2448 to drop `role_uri`:
```rust
pub(crate) async fn record_replica_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    display_name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
```
2. Deleting the content-roster call (the `record_membership_on_content(tonk, repository, key, role_uri).await?;` at line 2635 and its preceding comment block at 2630-2634).

Then add a new wrapper that preserves the old surface:
```rust
/// Lay down the meta-branch facts and profile index, then record the
/// opening profile's membership on the content branch. The two halves
/// are split so the join/restore mount can reuse the meta half without
/// the roster write (restore must not stamp a role — see the restore
/// path).
pub(crate) async fn record_repository_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    display_name: &str,
    configuration: &RepositoryConfiguration,
    role_uri: &str,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    record_replica_meta(tonk, repository, display_name, configuration).await?;
    record_membership_on_content(tonk, repository, &repository.did().repo_key(), role_uri).await
}
```

(Check the exact `key` expression `record_membership_on_content` expects — the original computed `let key = did.repo_key();` at 2461 from `repository.did()`. Reuse that form.)

- [ ] **Step 2: Verify the workspace compiles and existing tests pass**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -30`
Expected: clean. `create_repository` (2420) and the claim path still call `record_repository_meta` with a role and get identical behavior.
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-worker/src/router/repository.rs
git commit -m "refactor(tonk-worker): split replica meta from the content roster write"
```

---

### Task 2: Extract `mount_replica` and rewire the claim path

**Files:**
- Modify: `rust/tonk-worker/src/router/join.rs`

**Interfaces:**
- Consumes: `record_replica_meta` (Task 1), `find_replica_for_subject`, `mark_replica_initialized`, `record_membership_on_content`, `record_claim_on_content`.
- Produces: `pub(crate) async fn mount_replica(tonk: &TonkState, subject: &Did, remote_url: Option<&str>) -> Result<(String, Repository<Credential>), TonkWorkerError>` — creates the verifier-only replica, configures the remote/branch, records the local replica meta (NO content roster). Returns the routing key AND the mounted repository handle (so the claim path can write the roster without a reload). `Credential` is the verifier-credential type from `Repository::from(space_credential)` — confirm the concrete type name and use it in the signature. Consumed by Task 5 (restore, which ignores both return values) and by the claim rewire below.

- [ ] **Step 1: Add `mount_replica`, lifting the claim's mount block**

In `join.rs`, add (mirrors `claim_invite`'s lines ~246-292, minus the roster writes):
```rust
/// Mount a local verifier-only replica for a space and configure its
/// remote, without touching the content roster. Shared by the invite
/// claim and by cross-device restore. Idempotent-safe callers should
/// check `find_replica_for_subject` first.
pub(crate) async fn mount_replica(
    tonk: &TonkState,
    subject: &Did,
    remote_url: Option<&str>,
) -> Result<String, TonkWorkerError> {
    let key = subject.repo_key().to_owned();

    let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|e| {
        TonkWorkerError::Router(format!("subject is not a valid Ed25519 did:key: {e:?}"))
    })?;
    let credential = Credential::from(verifier);
    let space_capability = Subject::from(tonk.profile.did()).attenuate(Space::new(&key));
    let space_credential = space_capability
        .create(credential)
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to create local replica '{key}': {e}"))
        })?;
    let repository = Repository::from(space_credential);

    let mut configuration = RepositoryConfiguration::default();
    if let Some(url) = remote_url {
        let address = SiteAddress::from(UcanAddress::new(url));
        configuration = configuration
            .remote(
                DEFAULT_REMOTE,
                RemoteConfiguration::new(address).subject(subject.clone()),
            )
            .branch(
                DEFAULT_BRANCH,
                BranchConfiguration {
                    upstream: Some(UpstreamConfiguration::new(DEFAULT_REMOTE, DEFAULT_BRANCH)),
                    revision: None,
                },
            );
    } else {
        configuration = configuration.branch(DEFAULT_BRANCH, BranchConfiguration::default());
    }

    crate::router::repository::record_replica_meta(tonk, &repository, &key, &configuration)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to record replica meta: {e}")))?;

    Ok((key, repository))
}
```

(Confirm the `RepositoryError -> TonkWorkerError` conversion form the file already uses; if `record_replica_meta` returns `RepositoryError`, map it as shown. `mount_replica` returns the mounted `repository` so the claim path can write the roster off it without a reload.)

- [ ] **Step 2: Rewire `claim_invite`'s new-replica branch onto `mount_replica`**

In `claim_invite` (the block currently at ~242-299 that builds the verifier, configures the remote, and calls `record_repository_meta`), replace the inline mount + `record_repository_meta` with a `mount_replica` call, then write the roster explicitly off the returned repository so behavior is unchanged (same net writes as the old `record_repository_meta(MEMBER)` + `record_claim_on_content`):
```rust
    let (key, repository) = mount_replica(tonk, &subject, remote_url.as_deref()).await?;

    // Roster on the content branch: the claimer is a member. (mount_replica
    // wrote only the device-local meta; the content roster is claim-only.)
    crate::router::repository::record_membership_on_content(
        tonk,
        &repository,
        &key,
        tonk_schema::MemberRole::MEMBER,
    )
    .await?;
    record_claim_on_content(tonk, &repository, &key, &invitation, &member).await?;
    mark_replica_initialized(tonk, &subject).await?;
```
The old code called `record_repository_meta(&repository, MEMBER)` (meta + content MEMBER) then `record_claim_on_content`; this reproduces exactly that — `mount_replica`'s `record_replica_meta` is the meta half, and `record_membership_on_content(MEMBER)` is the content half — with `mark_replica_initialized` last, preserving ordering. `repository` is the value `mount_replica` returns (the verifier-credential `Repository`), which satisfies `C: Principal + Clone` as the pre-refactor code proved.

- [ ] **Step 3: Add the failing regression assertion is unnecessary — reuse the existing claim tests**

The existing `it_records_membership_and_provenance_on_join`, `it_records_the_claimer_name_on_join`, `it_reports_provenance_in_members`, and the 3A `it_keys_membership_on_the_root_did_for_an_account_holder` all exercise the claim mount + roster. They must stay green — they are the behavior-preservation guard for this refactor.

- [ ] **Step 4: Verify compile (wasm tests can't execute locally)**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -30` (clean)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
The claim tests are `run_in_service_worker`; they compile here and execute in CI's web leg. Note in the report that they were compiled, not executed.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/join.rs
git commit -m "refactor(tonk-worker): extract mount_replica shared by claim and restore"
```

---

### Task 3: Back up a created space's `space -> root` delegation

**Files:**
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router/repository.rs` (hook in `ensure_remote_config`)

**Interfaces:**
- Consumes: `crate::router::account::{account_link, account_root_did}`, `tonk.profile.signer()`, `ClaimBackup`, `run_backup`/`build_device_invocation` plumbing.
- Produces: `pub(crate) async fn back_up_owned_space<C: Principal + Clone>(tonk: &TonkState, repository: &Repository<C>, remote_url: &str)` — best-effort; mints `space -> root` and pushes `{chain, remote_url}` to `/chains/put`. Fire-and-forget.

- [ ] **Step 1: Verify the mint API works on the repository handle in scope**

Before writing the backup, confirm — in a scratch native unit test in `account_backup.rs` OR by reading `create_repository:2397` — that `repository.access().claim(repository).delegate(root_did).perform(&tonk.operator).await` yields a one-hop `space -> root` delegation with subject = `repository.did()`. This is the exact call `create_repository` already makes to delegate to the profile (`repository.rs:2397-2405`), so the API is proven for a repository that holds a signer. The open risk is whether the `repository` handed to `ensure_remote_config` (loaded via `profile.repository(key).load()`) still holds signing capability for an OWNED space. If `.access().claim()` is unavailable on the loaded handle, STOP and report — the fallback is to mint at `create_repository` time and thread the chain to the remote-attach hook, which is a larger change worth flagging before proceeding.

- [ ] **Step 2: Implement `back_up_owned_space`**

In `account_backup.rs`, add (reuses the existing `account_service_url`, `ClaimBackup`, `run_backup`/dispatch pattern — factor the common "resolve link+service+device, then dispatch `run_backup`" out of `back_up_claim` if it isn't already, so both callers share it):
```rust
/// Back up a created space's `space -> root` delegation so another of
/// the account's devices can restore it. Best-effort and fire-and-forget;
/// a no-op when the profile is unlinked. The space must hold its signer
/// (an owned/created space) to issue the delegation.
pub(crate) async fn back_up_owned_space<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    remote_url: &str,
) where
    C: dialog_varsig::Principal + Clone,
{
    if let Err(error) = try_back_up_owned_space(tonk, repository, remote_url).await {
        log!("created-space backup skipped: {error}");
    }
}

async fn try_back_up_owned_space<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    remote_url: &str,
) -> Result<(), TonkWorkerError>
where
    C: dialog_varsig::Principal + Clone,
{
    let Some(root_did) = crate::router::account::account_root_did(tonk).await else {
        return Ok(());
    };
    // space -> root, subject-specific (subject = the space DID), full authority.
    let delegation = repository
        .access()
        .claim(repository)
        .delegate(root_did)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to mint space->root: {e}")))?;
    let chain = /* the DelegationChain from `delegation` — match create_repository's `save(delegation)` value shape */;
    dispatch_owned_backup(tonk, chain, remote_url.to_owned()).await;
    Ok(())
}
```
The exact type returned by `.access().claim(repository).delegate(root_did).perform(...)` must be resolved against the dialog API (create_repository saves it directly via `tonk.profile.access().save(delegation)` at 2409). It needs to become the hex-encoded chain in a `ClaimBackup`. If it is not already a `DelegationChain`, convert/extract its chain bytes. Then reuse the same fire-and-forget dispatch as `back_up_claim` (device signer from `tonk.profile.signer().signer().clone()`, the `root -> device` link from `account_link`, command `["account","chain","put"]`, artifact `ClaimBackup { chain_hex, remote_url: Some(remote_url) }`).

- [ ] **Step 3: Hook the backup after a remote is attached to an owned space**

In `repository.rs` `ensure_remote_config` (3429) — or, if `ensure_remote_config` is cross-target and the account plumbing is wasm-only, in its wasm callers `enable_sync_inner` (2023) right after `ensure_remote_config` returns `Ok` — add, after the remote is durably configured:
```rust
    // Escrow this owned space's delegation so the account's other devices
    // can restore it. Best-effort; no-op for unlinked profiles.
    crate::router::account_backup::back_up_owned_space(tonk, repository, remote).await;
```
Place it only where the space is OWNED (created by this profile) and `remote` is the just-attached URL. `enable_sync_inner` is the natural single hook (its doc: both the create-with-sync and enable-sync forms converge here). Confirm `repository` there holds a signer; if the loaded handle is verifier-only, fall back per Step 1.

- [ ] **Step 4: Verify compile + native tests**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -30` (clean)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
Run: `cargo test -p tonk-worker back_up 2>&1 | tail -20` (any native backup tests stay green)

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/account_backup.rs rust/tonk-worker/src/router/repository.rs
git commit -m "feat(tonk-worker): back up created spaces for cross-device restore"
```

**End of PR 1.** Open a PR: `git push -u origin feat/cross-device-restore` then `gh pr create --base <feat/root-did-rosters or staging> ...`. If 3A hasn't merged, base on `feat/root-did-rosters` (stacked) and note it.

---

## PR 2 — Restore consumer + triggers (Tasks 4-6)

### Task 4: The `/chains/list` and `/chains/get` client

**Files:**
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-account-service/tests/service.rs` (extend the round-trip proof)

**Interfaces:**
- Produces:
  - `pub(crate) async fn list_backed_up_chains(device: &Ed25519Signer, link: &DelegationChain, service: &str) -> Result<Vec<String>, TonkWorkerError>` — device-signed `/chains/list`, parses the JSON array.
  - `pub(crate) async fn get_backed_up_chain(device: &Ed25519Signer, link: &DelegationChain, service: &str, key: &str) -> Result<Vec<u8>, TonkWorkerError>` — device-signed `/chains/get`, returns the raw artifact bytes.
  - Consumed by Task 5.

- [ ] **Step 1: Implement the two client functions**

In `account_backup.rs`, parallel to the existing put client (`run_backup` builds a `["account","chain","put"]` container via `build_device_invocation` and POSTs). Add:
```rust
pub(crate) async fn list_backed_up_chains(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
) -> Result<Vec<String>, TonkWorkerError> {
    let body = tonk_identity::request::build_device_invocation(
        device.clone(),
        link,
        vec!["account".into(), "chain".into(), "list".into()],
        std::collections::BTreeMap::new(),
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build list invocation: {e}")))?;
    let endpoint = format!("{}/chains/list", service.trim_end_matches('/'));
    let bytes = post_for_bytes(&endpoint, body).await?;
    serde_json::from_slice(&bytes)
        .map_err(|e| TonkWorkerError::Internal(format!("parse chain keys: {e}")))
}

pub(crate) async fn get_backed_up_chain(
    device: &Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<Vec<u8>, TonkWorkerError> {
    let arguments = [("key".to_owned(), Promised::String(key.to_owned()))]
        .into_iter()
        .collect();
    let body = tonk_identity::request::build_device_invocation(
        device.clone(),
        link,
        vec!["account".into(), "chain".into(), "get".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build get invocation: {e}")))?;
    let endpoint = format!("{}/chains/get", service.trim_end_matches('/'));
    post_for_bytes(&endpoint, body).await
}
```
Add a `post_for_bytes(endpoint, body) -> Result<Vec<u8>, _>` cfg-split helper mirroring the existing `post_chains_put`, but returning the response body bytes (wasm: `Response.array_buffer()` → `Uint8Array` → `Vec<u8>`; native: `reqwest ... .bytes()`), with the same non-2xx → error handling. `Ed25519Signer`, `DelegationChain`, `Promised` are already imported for the put path or come from the same crates (`dialog_credentials`, `dialog_ucan_core::{DelegationChain, promise::Promised}`).

- [ ] **Step 2: Extend the account-service HTTP test to prove list+get via the production client**

The account-service test (`rust/tonk-account-service/tests/service.rs`) already round-trips `/chains/put` + `/chains/get` using the production `build_device_invocation` (via its `container` helper). It cannot call the tonk-worker client (different crate), but it CAN assert the wire contract the client depends on: confirm `/chains/list` returns a JSON array containing the key that `/chains/put` returned, and `/chains/get` returns the exact bytes. If the existing test only exercises put+get, add the list assertion:
```rust
    // /chains/list should surface the key we just put.
    let body = container(vec!["account".into(), "chain".into(), "list".into()], BTreeMap::new()).await;
    let response = client.post(format!("{base}/chains/list")).body(body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let keys: Vec<String> = response.json().await.unwrap();
    assert!(keys.contains(&key));
```

- [ ] **Step 3: Verify**

Run: `cargo test -p tonk-account-service --features helpers 2>&1 | tail -15` (green, incl. the list assertion)
Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -30` and `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-worker/src/router/account_backup.rs rust/tonk-account-service/tests/service.rs
git commit -m "feat(tonk-worker): client for listing and fetching backed-up chains"
```

---

### Task 5: The restore consumer

**Files:**
- Create: `rust/tonk-worker/src/router/restore.rs`
- Modify: `rust/tonk-worker/src/router.rs` (`mod restore;`)

**Interfaces:**
- Consumes: `list_backed_up_chains`, `get_backed_up_chain` (Task 4), `ClaimBackup`, `account_link`, `account_service_url` (make `account_service_url` `pub(crate)` in Task 4/5 if not already), `crate::router::join::mount_replica` (Task 2), `find_replica_for_subject`, `mark_replica_initialized`.
- Produces: `pub(crate) async fn restore_spaces(tonk: &TonkState)` — best-effort; pulls every backed-up artifact and mounts any space not already present. Consumed by Task 6.

- [ ] **Step 1: Implement `restore_spaces`**

Create `restore.rs`:
```rust
//! Pull the account's backed-up space delegations and mount any space
//! this device does not already have. Best-effort: failures log and are
//! swallowed; nothing here blocks link or boot.

use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;

use crate::router::account_backup::{
    account_service_url, get_backed_up_chain, list_backed_up_chains, ClaimBackup,
};
use crate::worker::TonkState;

/// Restore all backed-up spaces for the linked account. No-op when
/// unlinked or when the account service is unreachable.
pub(crate) async fn restore_spaces(tonk: &TonkState) {
    if let Err(error) = try_restore_spaces(tonk).await {
        log!("restore skipped: {error}");
    }
}

async fn try_restore_spaces(tonk: &TonkState) -> Result<(), crate::TonkWorkerError> {
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return Ok(());
    };
    let Some(service) = account_service_url() else {
        return Ok(());
    };
    let device = tonk.profile.signer().signer().clone();

    let keys = list_backed_up_chains(&device, &link, &service).await?;
    for key in keys {
        if let Err(error) = restore_one(tonk, &device, &link, &service, &key).await {
            // One bad artifact must not stop the rest.
            log!("restore of chain '{key}' skipped: {error}");
        }
    }
    Ok(())
}

async fn restore_one(
    tonk: &TonkState,
    device: &dialog_credentials::Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    key: &str,
) -> Result<(), crate::TonkWorkerError> {
    let bytes = get_backed_up_chain(device, link, service, key).await?;
    let artifact: ClaimBackup = serde_json::from_slice(&bytes)
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad backup artifact: {e}")))?;
    let chain_bytes = hex::decode(&artifact.chain_hex)
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad chain hex: {e}")))?;
    let chain = DelegationChain::try_from(chain_bytes.as_slice())
        .map_err(|e| crate::TonkWorkerError::Internal(format!("bad chain: {e}")))?;

    let subject = chain
        .subject()
        .ok_or_else(|| crate::TonkWorkerError::Internal("backup chain has no subject".into()))?
        .clone();

    // Already have it? Nothing to do.
    if crate::router::join::find_replica_for_subject(tonk, &subject).await? {
        return Ok(());
    }

    // Install the delegation so presign's BFS can compose it with the
    // local root -> device link, then mount and let sync bring the roster.
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|e| crate::TonkWorkerError::Internal(format!("save restored delegation: {e}")))?;

    crate::router::join::mount_replica(tonk, &subject, artifact.remote_url.as_deref()).await?;
    crate::router::repository::mark_replica_initialized(tonk, &subject).await?;
    Ok(())
}
```
Notes for the implementer:
- `find_replica_for_subject` and `mark_replica_initialized` are `pub(crate)` (make them so if not — `find_replica_for_subject` is at `join.rs:435`, `mark_replica_initialized` at `repository.rs:2828`). `mount_replica` returns `(key, repository)` — `restore_one` uses neither (it drives everything off `subject`), so `let _ = mount_replica(...).await?;`.
- `account_service_url` must be `pub(crate)`.
- `log!` = `use tonk_common::log;`.

- [ ] **Step 2: Declare the module**

In `rust/tonk-worker/src/router.rs`, add `mod restore;` alongside the other submodules.

- [ ] **Step 3: Verify compile**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -40` (clean — expect a `restore_spaces` dead_code warning until Task 6 wires the triggers)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-worker/src/router/restore.rs rust/tonk-worker/src/router.rs rust/tonk-worker/src/router/join.rs rust/tonk-worker/src/router/repository.rs
git commit -m "feat(tonk-worker): restore backed-up spaces onto a device"
```

---

### Task 6: Trigger restore on link and on startup

**Files:**
- Modify: `rust/tonk-worker/src/router/account.rs` (link handler)
- Modify: `rust/tonk-worker/src/worker.rs` (startup)

**Interfaces:**
- Consumes: `crate::router::restore::restore_spaces` (Task 5).

- [ ] **Step 1: Trigger after a successful link**

In `account.rs` `link` (147-190), after the `root -> device` chain is persisted and before returning `AccountStatus::Linked`, dispatch restore fire-and-forget so a slow account service can't stall the link response:
```rust
    // A freshly linked device pulls the account's spaces in the
    // background — never block the link response on it.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let state = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let tonk = state.read().await;
            crate::router::restore::restore_spaces(&tonk).await;
        });
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        crate::router::restore::restore_spaces(&state.read().await).await;
    }
```
(Confirm the `State`/`AppState` handle type and how to clone it for the spawned task — mirror the 3A `back_up_claim` dispatch pattern in `account_backup.rs`.)

- [ ] **Step 2: Trigger on startup for a linked profile**

In `worker.rs` `TonkServiceWorker::new`, after `bootstrap_profile(&state)` (1678-1680), dispatch restore for an already-linked profile, fire-and-forget so boot isn't blocked:
```rust
    // Catch up on spaces claimed/created on other devices since last boot.
    // Fire-and-forget; account-service latency must not delay startup.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let state = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let tonk = state.read().await;
            crate::router::restore::restore_spaces(&tonk).await;
        });
    }
```
(Use the same `state` handle `bootstrap_profile` received. If startup isn't inside an async context that allows `spawn_local`, place the trigger at the first activation/fetch hook instead — `worker.rs`'s `on_activate` — and note it.)

- [ ] **Step 3: Verify compile (both targets) and the full gate**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -30` (clean — `restore_spaces` dead_code warning now gone)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-worker/src/router/account.rs rust/tonk-worker/src/worker.rs
git commit -m "feat(tonk-worker): restore spaces on device link and startup"
```

---

### Task 7: Full gates

**Files:** none (verification only).

- [ ] **Step 1: Workspace gate**

```bash
cargo clippy --workspace --all-targets --all-features
cargo fmt --check
cargo test -p tonk-account-service --features helpers
```
Expected: all green. Fix any finding in the crates this plan touched.

- [ ] **Step 2: Note the deferred wasm + staging verification in the PR body**

The restore mount, the created-space backup hook, and the triggers are `run_in_service_worker`/wasm and are executed by CI's `web` matrix leg, not locally. In the PR body, list for a human/staging pass: (a) claim a space on device A (account-holder), then link device B → the space auto-mounts on B; (b) create a space with sync on device A → it appears on device B after link/restart.

- [ ] **Step 3: Push and open PR 2**

```bash
git push
gh pr create --base <feat/cross-device-restore's PR1 base> --title "feat(account): restore backed-up spaces across devices" --body "<summary + verification checklist + design link>"
```

---

## Out of scope (later)

Live cross-device propagation (restore is trigger-based, not push); migrating pre-account device-keyed spaces and the rename root-switch (stage 3B); revocation-list awareness. Native (CLI) created-space backup if the `ensure_remote_config` hook lands wasm-only — track as a follow-up.

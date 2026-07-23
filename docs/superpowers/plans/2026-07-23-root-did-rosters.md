# Root-DID Rosters (stage 3A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** For account-holders, invite claims and roster writes key on the passkey-derived **root DID** instead of the device DID, and each claimed space's delegation is backed up to the account service so a later device can recover it. Device-only users are unchanged.

**Architecture:** A single resolver — "my member DID is the account root if I'm linked, else my device DID" — feeds the three membership-writing sites and the invite-claim audience. Presign already composes `space → eph → root` with the device's local `root → device` link automatically (`dialog-capability` BFS), so a root-audienced claim needs no new storage shape. Backup is a best-effort, device-signed UCAN call to the account service's `/chains/put`; restore is a deferred follow-up PR.

**Tech Stack:** Rust (edition 2024). `dialog-credentials` / `dialog-ucan-core` / `dialog-operator` / `dialog-varsig` (git tag `tonk-2026-07-17`). `tonk-identity` (workspace path dep). `serde_json`, `reqwest`, `web-sys` (already `tonk-worker` deps).

## Global Constraints

- PRs target `staging`, not `main`. Work on branch `feat/root-did-rosters` (already cut from `origin/staging`; carries the design doc + a wrangler config commit).
- Do **not** bump the pinned dialog tag `tonk-2026-07-17`.
- The subject-open (`Subject::Any`) shape is only ever the `root → device` link. Never mint `Subject::Any` in any invite path — invites stay subject-specific.
- Tests: always `#[dialog_common::test]`, never `#[test]`/`#[tokio::test]`/`#[wasm_bindgen_test]` directly. BDD names `it_does_x`. `tonk-worker` tests run in the service worker — every test mod already carries `wasm_bindgen_test_configure!(run_in_service_worker)`.
- No `mod.rs` — use `foo.rs` + `foo/` form.
- Conventional Commits, imperative lowercase subject under 72 chars, no emojis.
- Code comments stand on their own — no "stage 3a", "per the spec", or design-doc references in code or tests.
- Full lint gate before the PR: `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --check` must both pass (the flake check runs exactly this).
- Design of record: `docs/superpowers/specs/2026-07-23-root-did-rosters-design.md`.

## File Structure

- `rust/tonk-worker/src/router/account.rs` — **modify**: add the `account_link` / `account_root_did` / `member_did` resolver next to the existing `load_link`.
- `rust/tonk-worker/src/router/join.rs` — **modify**: claim audiences the member DID; `record_claim_on_content` keys membership on it; call the backup after persisting the chain.
- `rust/tonk-worker/src/router/repository.rs` — **modify**: `record_membership_on_content` keys membership on the member DID.
- `rust/tonk-identity/src/request.rs` — **create**: `build_device_invocation`, the production device-signed account-service invocation builder.
- `rust/tonk-identity/src/lib.rs` — **modify**: `pub mod request;`.
- `rust/tonk-account-service/tests/service.rs` — **modify**: point the test `container()` helper at the production builder (proves the server accepts it).
- `rust/tonk-worker/src/router/account_backup.rs` — **create**: the backup artifact, the worker-side service-URL resolver, the `/chains/put` POST, and the best-effort `back_up_claim` entry.
- `rust/tonk-worker/src/router.rs` — **modify**: declare `mod account_backup;`.
- `rust/tonk-worker/Cargo.toml` — **modify** if needed: enable the `WorkerLocation` web-sys feature.

---

### Task 1: The member-DID resolver

**Files:**
- Modify: `rust/tonk-worker/src/router/account.rs`

**Interfaces:**
- Consumes: the existing `load_link(&TonkState) -> Result<Option<Vec<u8>>, TonkWorkerError>` (account.rs:61), `ACCOUNT_LINK_SITE` (account.rs:15), `DelegationChain::{try_from, issuer}`.
- Produces:
  - `pub(crate) async fn account_link(tonk: &crate::worker::TonkState) -> Option<DelegationChain>`
  - `pub(crate) async fn account_root_did(tonk: &crate::worker::TonkState) -> Option<dialog_varsig::Did>`
  - `pub(crate) async fn member_did(tonk: &crate::worker::TonkState) -> dialog_varsig::Did`
  - Consumed by Tasks 2 and 4.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` in `account.rs` (which already has `wasm_bindgen_test_configure!(run_in_service_worker)`, `test_state`, and the `request_for` helper). Extend the imports with `use dialog_varsig::Principal;` if not already in scope:

```rust
    #[dialog_common::test]
    async fn it_resolves_the_member_did_to_the_root_when_linked() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        let expected_root = request.root_did.clone();
        let _ = link(State(state.clone()), Json(request)).await.unwrap();

        let tonk = state.read().await;
        assert_eq!(member_did(&tonk).await.to_string(), expected_root);
        assert_eq!(
            account_root_did(&tonk).await.map(|did| did.to_string()),
            Some(expected_root),
        );
    }

    #[dialog_common::test]
    async fn it_resolves_the_member_did_to_the_device_when_unlinked() {
        let state = Arc::new(RwLock::new(test_state().await));
        let tonk = state.read().await;
        let device_did = tonk.profile.did();
        assert_eq!(member_did(&tonk).await, device_did);
        assert!(account_root_did(&tonk).await.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop -c test:web:debug` (or the repo's wasm test entrypoint).
Expected: FAIL to compile — `member_did` / `account_root_did` not found.

- [ ] **Step 3: Implement the resolver**

Add below `load_link` in `account.rs`, and add `use tonk_common::log;` to the module imports (the same import `create_invite.rs:29` uses):

```rust
/// The stored `root → device` delegation for this profile, or `None`
/// when the profile is unlinked or the stored link is unreadable.
///
/// Fail-safe: an unreadable or malformed link resolves to `None`, so the
/// device behaves exactly as an unlinked one and keeps working.
pub(crate) async fn account_link(tonk: &crate::worker::TonkState) -> Option<DelegationChain> {
    let bytes = match load_link(tonk).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(error) => {
            log!("account link unreadable; treating profile as unlinked: {error}");
            return None;
        }
    };
    match DelegationChain::try_from(bytes.as_slice()) {
        Ok(chain) => Some(chain),
        Err(error) => {
            log!("account link malformed; treating profile as unlinked: {error}");
            None
        }
    }
}

/// The account root DID for this profile, or `None` when unlinked. A
/// linked device knows its root without holding the root key: the
/// `root → device` delegation names the root as issuer.
pub(crate) async fn account_root_did(
    tonk: &crate::worker::TonkState,
) -> Option<dialog_varsig::Did> {
    account_link(tonk).await.map(|chain| chain.issuer().clone())
}

/// The DID roster writes and invite claims key on: the account root when
/// linked, otherwise this device's own DID.
pub(crate) async fn member_did(tonk: &crate::worker::TonkState) -> dialog_varsig::Did {
    match account_root_did(tonk).await {
        Some(root) => root,
        None => tonk.profile.did(),
    }
}
```

`load_link` currently takes `&crate::worker::TonkState`; the handlers call it after `let state = state.read().await`, so these helpers take the same borrowed `TonkState`. If `load_link` is not already reachable from the new functions (same module — it is), no signature change is needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS — both new tests, plus the existing account-link tests still green.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/account.rs
git commit -m "feat(tonk-worker): resolve the member did from the account link"
```

---

### Task 2: Claim and membership writers key on the member DID

**Files:**
- Modify: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-worker/src/router/repository.rs`

**Interfaces:**
- Consumes: `crate::router::account::member_did` (Task 1); `tonk_invite::Invite::claim(&Did)`; `tonk_schema::Membership::new(Did, Did)`.
- Produces: no new public surface — behavioural change only. Task 4 relies on the claim still persisting the chain before returning.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` in `join.rs` (imports already include `DelegationChain`, `Principal as _`, `DidExt as _`, `test_state`, `content_memberships`, `api_router_with_state`, `post_join`, `handcrafted_invite_url`):

```rust
    /// An account-holder's claim keys the roster row on their root DID,
    /// and no device-keyed row is written.
    #[dialog_common::test]
    async fn it_keys_membership_on_the_root_did_for_an_account_holder() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);

        // Link this profile to an account root.
        let device_did = state.read().await.profile.did();
        let root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let root_did = root.did();
        let delegation =
            tonk_identity::delegation::mint_device_delegation(root, &device_did)
                .await
                .unwrap();
        let request = tonk_worker_api::AccountLinkRequest {
            root_did: root_did.to_string(),
            delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        };
        crate::router::account::link(axum::extract::State(state.clone()), axum::Json(request))
            .await
            .unwrap();

        // Join an invite.
        let (url, key) = handcrafted_invite_url(40, 41).await;
        assert_eq!(post_join(&app, &url).await, StatusCode::CREATED);

        let memberships = content_memberships(&state, &key).await;
        let root_entity = root_did.this();
        let device_entity = device_did.this();
        assert!(
            memberships.iter().any(|m| m.member.0 == root_entity),
            "membership keyed on the root did",
        );
        assert!(
            !memberships.iter().any(|m| m.member.0 == device_entity),
            "no device-keyed row was written",
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c test:web:debug`
Expected: FAIL — membership is keyed on the device DID, so the root assertion fails and the device-keyed row is present.

- [ ] **Step 3: Audience the claim on the member DID**

In `join.rs` `claim_invite` (around line 176), resolve the member DID and claim to it:

```rust
    let member = crate::router::account::member_did(tonk).await;
    let claimed = invite
        .claim(&member)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?;
```

- [ ] **Step 4: Key `record_claim_on_content` on the member DID**

Thread `member` into `record_claim_on_content`. Change its signature (join.rs:327) to accept `member: &dialog_varsig::Did`, and change the membership line (join.rs:336) from `Membership::new(tonk.profile.did(), repository.did())` to:

```rust
    let membership = Membership::new(member.clone(), repository.did());
```

Update both call sites (join.rs:223 and join.rs:289) to pass `&member`. Leave the `self_invite` check (join.rs:386) keyed on `tonk.profile.did().this()` — it answers "did I mint this from this device", which stays device-scoped.

- [ ] **Step 5: Key `record_membership_on_content` on the member DID**

In `repository.rs` `record_membership_on_content` (line 2666), replace the membership line (line 2678):

```rust
    let member = crate::router::account::member_did(tonk).await;
    let membership = Membership::new(member, repository.did());
```

This is the shared founder/create + join meta writer, so a created space's founder row also keys on the root DID.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop -c test:web:debug`
Expected: PASS — the new root-keying test passes, and the existing `it_records_membership_and_provenance_on_join` (which links no account) still asserts the device-keyed row, proving device-only is unchanged.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker/src/router/join.rs rust/tonk-worker/src/router/repository.rs
git commit -m "feat(tonk-worker): key rosters and invite claims on the account root did"
```

---

### Task 3: The device-signed account-service invocation builder

**Files:**
- Create: `rust/tonk-identity/src/request.rs`
- Modify: `rust/tonk-identity/src/lib.rs`
- Modify: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- Consumes: `dialog_ucan_core::{InvocationBuilder, InvocationChain, DelegationChain}`, `dialog_ucan_core::promise::Promised`, `dialog_credentials::Ed25519Signer`.
- Produces: `pub async fn build_device_invocation(device: Ed25519Signer, link: &DelegationChain, command: Vec<String>, arguments: BTreeMap<String, Promised>) -> anyhow::Result<Vec<u8>>` — raw invocation-container bytes (the POST body). Consumed by Task 4 and by the account-service tests.

- [ ] **Step 1: Write the failing unit test**

Create `rust/tonk-identity/src/request.rs` with the test mod first:

```rust
//! Device-signed account-service invocation containers.
//!
//! The account service's `authorize` accepts requests issued by a device
//! key whose `root → device` delegation is attached as a proof, with the
//! account root as subject. This builds exactly that container from a
//! profile's live device signer and its stored `root → device` link — no
//! root key, no raw seed.

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationChain;
    use dialog_varsig::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_builds_a_device_signed_invocation_the_service_verifies() {
        let root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let root_did = root.did();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let device_did = device.did();
        let link = crate::delegation::mint_device_delegation(root, &device_did)
            .await
            .unwrap();

        let arguments = [("chain".to_owned(), Promised::String("deadbeef".to_owned()))]
            .into_iter()
            .collect();
        let bytes = build_device_invocation(
            device,
            &link,
            vec!["account".into(), "chain".into(), "put".into()],
            arguments,
        )
        .await
        .unwrap();

        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(chain.issuer(), &device_did);
        assert_eq!(chain.subject(), &root_did);
        assert_eq!(
            chain.command().0,
            vec!["account".to_string(), "chain".to_string(), "put".to_string()],
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tonk-identity`
Expected: FAIL to compile — `build_device_invocation` not found.

- [ ] **Step 3: Implement `build_device_invocation`**

Add above the test mod in `request.rs` (this mirrors the account-service test helper's exact container shape):

```rust
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};

/// Build a device-signed account-service invocation container.
///
/// `link` is the stored `root → device` delegation: its issuer is the
/// account root (the invocation subject and audience), and its single
/// proof is attached so the service can bind the device to the account.
pub async fn build_device_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
) -> Result<Vec<u8>> {
    let root_did = link.issuer().clone();
    let delegation = link
        .proofs()
        .last()
        .context("account link carries no delegation to prove the device")?
        .clone();
    let cid = delegation.to_cid();

    let invocation = InvocationBuilder::new()
        .issuer(device)
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![cid])
        .try_build()
        .await
        .context("failed to sign the device invocation")?;

    let mut proofs = HashMap::new();
    proofs.insert(cid, Arc::new(delegation));
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .context("failed to serialize the device invocation")
}
```

Add to `rust/tonk-identity/src/lib.rs`:

```rust
pub mod request;
```

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `cargo test -p tonk-identity`
Expected: PASS.

- [ ] **Step 5: Point the account-service test helper at the production builder**

In `rust/tonk-account-service/tests/service.rs`, replace the hand-rolled `container` (lines 21-47) with a thin wrapper over the new builder, so the existing `/chains/put`+`/chains/get` and `/devices/list` round-trip tests now exercise production code:

```rust
/// Build a device-signed invocation container for the account's first
/// device, using the production builder against the `root → device`
/// delegation minted for account creation.
async fn container(command: Vec<String>, args: BTreeMap<String, Promised>) -> Vec<u8> {
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let link = tonk_identity::delegation::mint_device_delegation(root, &device.did())
        .await
        .unwrap();
    tonk_identity::request::build_device_invocation(device, &link, command, args)
        .await
        .unwrap()
}
```

Remove the now-unused imports this leaves behind (`InvocationBuilder`, `Arc`, and the `Principal` import if `device.did()` is its only user — the compiler will flag them).

- [ ] **Step 6: Run the account-service tests to verify they pass**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: PASS — the full-ceremony test (including the chains and devices/list round-trips) is now green against the production builder.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-identity/src/request.rs rust/tonk-identity/src/lib.rs rust/tonk-account-service/tests/service.rs
git commit -m "feat(tonk-identity): build device-signed account-service invocations"
```

---

### Task 4: Back up the claimed chain, best-effort

**Files:**
- Create: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router.rs` (declare the module)
- Modify: `rust/tonk-worker/src/router/join.rs` (call the backup)
- Modify: `rust/tonk-worker/Cargo.toml` (enable `WorkerLocation` if absent)

**Interfaces:**
- Consumes: `crate::router::account::account_link` (Task 1), `tonk_identity::request::build_device_invocation` (Task 3), the `TonkState.profile.signer()` device signer, `DelegationChain::to_bytes`.
- Produces: `pub(crate) async fn back_up_claim(tonk: &TonkState, chain: &DelegationChain, remote_url: Option<&str>)` — best-effort, never returns an error.

- [ ] **Step 1: Write the failing native resolver test**

Create `rust/tonk-worker/src/router/account_backup.rs` with the artifact type, a native-only service-URL resolver, and its test:

```rust
//! Best-effort backup of a claimed space's delegation to the account
//! service, so a later device can recover the space.

use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// What gets backed up per claimed space: the delegation chain plus the
/// invite's sync URL, which the chain itself does not carry. A restoring
/// device needs both to mount and sync the space.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ClaimBackup {
    /// Hex-encoded `space → eph → root` delegation chain.
    pub chain_hex: String,
    /// The invite's remote/sync URL, when it carried one.
    pub remote_url: Option<String>,
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_prefers_the_service_url_override() {
        // SAFETY: single-threaded test; no other reader of this var.
        unsafe { std::env::set_var("TONK_ACCOUNT_SERVICE_URL", "http://127.0.0.1:8787") };
        assert_eq!(
            account_service_url().as_deref(),
            Some("http://127.0.0.1:8787"),
        );
        unsafe { std::env::remove_var("TONK_ACCOUNT_SERVICE_URL") };
        assert_eq!(account_service_url().as_deref(), Some("https://accounts.tonk.xyz"));
    }
}
```

(The `unsafe { set_var }` is required on edition 2024. If the surrounding test harness disallows env mutation, drop this test and assert the default arm only: `assert!(account_service_url().is_some())`.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c test:native:debug` (or `cargo test -p tonk-worker account_backup` — this is a native test, matching the `not(wasm32)` test mods in `repository.rs`)
Expected: FAIL to compile — `account_service_url` not found.

- [ ] **Step 3: Implement the resolver, the POST, and `back_up_claim`**

Add to `account_backup.rs`:

```rust
/// Resolve the account-service base URL for this context. Unknown hosts
/// resolve to `None` so backup is skipped rather than failing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn account_service_url() -> Option<String> {
    use wasm_bindgen::JsCast;
    let scope: web_sys::ServiceWorkerGlobalScope = js_sys::global().dyn_into().ok()?;
    match scope.location().host().as_str() {
        "tonk.spot" => Some("https://accounts.tonk.xyz".to_owned()),
        "staging.tonk.xyz" => Some("https://accounts-staging.tonk.xyz".to_owned()),
        _ => None,
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn account_service_url() -> Option<String> {
    std::env::var("TONK_ACCOUNT_SERVICE_URL")
        .ok()
        .or_else(|| Some("https://accounts.tonk.xyz".to_owned()))
}

/// POST a device-signed invocation container to the account service.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn post_chains_put(endpoint: &str, body: Vec<u8>) -> Result<(), TonkWorkerError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&js_sys::Uint8Array::from(body.as_slice()).into());
    let request = Request::new_with_str_and_init(endpoint, &init)
        .map_err(|e| TonkWorkerError::Internal(format!("chains/put request: {e:?}")))?;
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service-worker scope".to_owned()))?;
    let response: Response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| TonkWorkerError::Internal(format!("chains/put fetch: {e:?}")))?;
    if !response.ok() {
        return Err(TonkWorkerError::Internal(format!(
            "chains/put returned HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn post_chains_put(endpoint: &str, body: Vec<u8>) -> Result<(), TonkWorkerError> {
    let response = reqwest::Client::new()
        .post(endpoint)
        .body(body)
        .send()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("chains/put: {e}")))?;
    if !response.status().is_success() {
        return Err(TonkWorkerError::Internal(format!(
            "chains/put returned HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

async fn try_back_up_claim(
    tonk: &TonkState,
    chain: &DelegationChain,
    remote_url: Option<&str>,
) -> Result<(), TonkWorkerError> {
    // Only account-holders back up; an unlinked device has no account to
    // escrow under and returns early.
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return Ok(());
    };
    let Some(service) = account_service_url() else {
        return Ok(());
    };

    let chain_bytes = chain
        .to_bytes()
        .map_err(|e| TonkWorkerError::Internal(format!("serialize claimed chain: {e}")))?;
    let artifact = ClaimBackup {
        chain_hex: hex::encode(chain_bytes),
        remote_url: remote_url.map(str::to_owned),
    };
    let artifact_bytes = serde_json::to_vec(&artifact)
        .map_err(|e| TonkWorkerError::Internal(format!("serialize backup artifact: {e}")))?;

    let device = tonk.profile.signer().signer().clone();
    let arguments = [(
        "chain".to_owned(),
        Promised::String(hex::encode(artifact_bytes)),
    )]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "chain".into(), "put".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build backup invocation: {e}")))?;

    let endpoint = format!("{}/chains/put", service.trim_end_matches('/'));
    post_chains_put(&endpoint, body).await
}

/// Back up a claimed space's delegation to the account service.
/// Best-effort: any failure logs and is swallowed — the claiming device
/// already works, and the roster keys on the root regardless.
pub(crate) async fn back_up_claim(
    tonk: &TonkState,
    chain: &DelegationChain,
    remote_url: Option<&str>,
) {
    if let Err(error) = try_back_up_claim(tonk, chain, remote_url).await {
        log!("claim backup skipped: {error}");
    }
}
```

Add `use tonk_common::log;` to the `account_backup.rs` imports (as in Task 1, Step 3). Add `mod account_backup;` to `rust/tonk-worker/src/router.rs` alongside the other `mod` declarations.

If `cargo build -p tonk-worker --target wasm32-unknown-unknown` reports `WorkerLocation` unknown, add `"WorkerLocation"` to the `web-sys` features list in `rust/tonk-worker/Cargo.toml`.

- [ ] **Step 4: Run the resolver test to verify it passes**

Run: `nix develop -c test:native:debug` (or `cargo test -p tonk-worker account_backup` — this is a native test, matching the `not(wasm32)` test mods in `repository.rs`)
Expected: PASS.

- [ ] **Step 5: Call the backup from the claim path**

In `join.rs` `claim_invite`, the chain is persisted around lines 190-197 and then `remote_url` is consumed later (line 258). Keep a clone for the save so the original chain can be backed up, and call the backup right after persisting. Change the save block to:

```rust
    tonk.profile
        .access()
        .save(UcanDelegation(chain.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to persist delegation chain: {e}"))
        })?;

    // Escrow the claimed chain (with the invite's sync URL) so another of
    // this account's devices can recover the space. No-op for unlinked
    // devices; best-effort for linked ones.
    crate::router::account_backup::back_up_claim(tonk, &chain, remote_url.as_deref()).await;
```

`remote_url` is `Option<String>` (cloned at join.rs:182) and is not moved until join.rs:258, so `remote_url.as_deref()` borrows it safely here.

- [ ] **Step 6: Verify device-only claims are unaffected**

Run: `nix develop -c test:web:debug`
Expected: PASS — the existing device-only join tests still pass. With no account linked, `back_up_claim` returns early (unlinked), so no network call is attempted and the claim behaves exactly as before.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker/src/router/account_backup.rs rust/tonk-worker/src/router.rs rust/tonk-worker/src/router/join.rs rust/tonk-worker/Cargo.toml
git commit -m "feat(tonk-worker): back up a claimed chain to the account service"
```

---

### Task 5: Full gates

**Files:** none (verification only).

- [ ] **Step 1: Run the full lint + test gate**

```bash
cargo clippy --workspace --all-targets --all-features
cargo fmt --check
cargo test -p tonk-identity
cargo test -p tonk-account-service --features helpers
nix develop -c test:web:debug
```

Expected: all green. Fix any clippy/fmt findings in the crates this plan touched.

- [ ] **Step 2: Manual staging verification (backup e2e)**

The worker→service `/chains/put` POST is not automatically end-to-end tested (tonk-worker tests run in the service worker and cannot host a native account server; Task 3 proves the builder against the real server, and Task 4 tests the resolver). Verify the full path once against staging:

1. Create an account in a browser on `staging.tonk.xyz`.
2. Claim an invite as that account-holder.
3. Confirm the account service's `chains` store gained a key for the account (a `/chains/list` invocation, or inspect the staging R2 bucket).

Note the result in the PR description.

- [ ] **Step 3: Open the PR**

```bash
git push -u origin feat/root-did-rosters
gh pr create --repo tonk-labs/tonk --base staging \
  --title "feat(account): root-did rosters and claim backup (stage 3a)" \
  --body "<summary + the staging verification result + link to the design doc>"
```

---

## Out of scope (immediate follow-up PR: restore)

A restoring device pulling backed-up artifacts (`/chains/list` + `/chains/get`), re-running the join's replica-mount per subject using the recovered `remote_url`, saving the delegation to the access store, and recording roster rows under the root DID — plus the startup/link hooks that trigger it. Deferred to keep this PR reviewable; the `ClaimBackup` artifact and `build_device_invocation` builder are the shared foundation it will consume.

## Out of scope (stage 3B)

Migrating existing device-keyed members, `device → root` re-anchoring of pre-account chains, the profile-rename root switch, and revocation-list awareness.

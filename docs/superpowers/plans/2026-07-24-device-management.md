# Device Management Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the existing account-service device registry its first consumers — a devices panel in the browser account UI, `tonk account devices` / `tonk account revoke` in the CLI, and a local "sign out" — so users can see and revoke their linked devices.

**Architecture:** The worker proxies device-signed invocations to the account service (exactly the plumbing `account_backup.rs` already uses for chain backup), exposing plain JSON routes the UI consumes. The CLI signs the same invocations natively from its stored `root → device` link. Local unlink writes an empty-bytes tombstone over the stored link (the credential effect API has Save/Load only — no delete; verified in `dialog-effects/src/credential.rs`).

**Tech Stack:** axum (`tonk-worker` router), workers-rs account service (unchanged), `tonk-identity::request::build_device_invocation`, custom-elements UI (`tonk-ui`), clap CLI.

**Sequencing:** Ships after revocation enforcement (`2026-07-24-revocation-enforcement.md`) so revoke actually severs storage access. Branch `feat/device-management` off `staging`.

## Global Constraints

- Native tests use `#[dialog_common::test]`, named `it_does_x`.
- Worker tests are wasm-gated behind `run_in_service_worker`, UI tests behind `run_in_browser`; both compile locally but **execute only in CI's web leg** (local wasm runs hang — do not wait on them).
- Lint gate: `cargo clippy --workspace --all-targets --all-features` + `cargo fmt --check` (native), plus `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`.
- No `mod.rs`; sibling files (`foo.rs` + `foo/`).
- No stage/phase/RFC references in code or comments.
- Conventional commits, imperative, lowercase, no trailing period.
- Wire DTOs serialize camelCase (`#[serde(rename_all = "camelCase")]`), matching every existing account DTO.
- User-supplied strings (device names) reach the DOM only via `set_text_content`, never `set_inner_html`.

---

### Task 1: Wire DTOs in `tonk-worker-api`

**Files:**
- Modify: `rust/tonk-worker-api/src/account.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs:26` (re-export)

**Interfaces:**
- Produces: `AccountDevice { did: String, name: String, status: String, created_at: u64, this_device: bool }` and `RevokeDeviceRequest { did: String }`, both `Clone + Debug + Serialize + Deserialize`, camelCase on the wire (`createdAt`, `thisDevice`). Tasks 3, 6, 7 consume these exact names.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `rust/tonk-worker-api/src/account.rs`:

```rust
    #[dialog_common::test]
    fn it_serializes_account_devices_in_camel_case() {
        let json = serde_json::to_value(AccountDevice {
            did: "did:key:device".into(),
            name: "laptop".into(),
            status: "active".into(),
            created_at: 1_753_300_000,
            this_device: true,
        })
        .unwrap();
        assert_eq!(json["did"], "did:key:device");
        assert_eq!(json["createdAt"], 1_753_300_000);
        assert_eq!(json["thisDevice"], true);
        assert!(json.get("created_at").is_none());

        let request: RevokeDeviceRequest =
            serde_json::from_value(serde_json::json!({ "did": "did:key:device" })).unwrap();
        assert_eq!(request.did, "did:key:device");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-worker-api it_serializes_account_devices_in_camel_case`
Expected: FAIL — `cannot find struct AccountDevice`.

- [ ] **Step 3: Write minimal implementation**

Add above the `tests` module in `rust/tonk-worker-api/src/account.rs`:

```rust
/// One device registered under the linked account, as returned by the
/// worker's device-list proxy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDevice {
    /// The device's DID.
    pub did: String,
    /// Display name registered at link time.
    pub name: String,
    /// Registry status: `active` or `revoked`.
    pub status: String,
    /// Registration time, seconds since the epoch.
    pub created_at: u64,
    /// Whether this row is the profile making the request.
    pub this_device: bool,
}

/// Revoke one device under the linked account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// DID of the device to revoke.
    pub did: String,
}
```

In `rust/tonk-worker-api/src/lib.rs` extend the existing re-export:

```rust
pub use account::{AccountDevice, AccountLinkRequest, AccountStatus, RevokeDeviceRequest};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p tonk-worker-api it_serializes_account_devices_in_camel_case`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker-api/src/account.rs rust/tonk-worker-api/src/lib.rs
git commit -m "feat(tonk-worker-api): account device wire types"
```

---

### Task 2: Worker local unlink (`DELETE /api/account`)

**Files:**
- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-worker/src/router.rs:145` (route)

**Interfaces:**
- Consumes: `load_link`, `ACCOUNT_LINK_SITE`, `AccountStatus` — all already in `account.rs`.
- Produces: `pub async fn unlink(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError>`; `load_link` now maps an empty stored value to `Ok(None)`. Task 6's `unlink_account()` calls the route.

**Design note (constraint the code can't show):** the credential effect surface is Save/Load only — there is no delete effect — so unlink persists an empty byte vector and every reader treats empty as absent. The `UcanDelegation` saved into the access store at link time has no removal API either; unlink leaves it behind. That means a signed-out browser still holds a usable `root → device` delegation in its access store until the device is *revoked* — the UI copy in Task 7 says "sign out", and the revoke action is the security boundary. Document this in the handler doc comment.

- [ ] **Step 1: Write the failing tests**

Append to the wasm `tests` module in `rust/tonk-worker/src/router/account.rs`:

```rust
    #[dialog_common::test]
    async fn it_unlinks_and_returns_to_the_unlinked_state() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }

        let Json(after) = unlink(State(state.clone())).await.unwrap();
        assert_eq!(
            after,
            AccountStatus::Unlinked {
                device_did: device_did.to_string()
            }
        );
        let Json(loaded) = get(State(state.clone())).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Unlinked { .. }));

        // The tombstone must read as "no link", not as a malformed link.
        let tonk = state.read().await;
        assert!(account_link(&tonk).await.is_none());
    }

    #[dialog_common::test]
    async fn it_relinks_the_same_root_after_an_unlink() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }
        let _ = unlink(State(state.clone())).await.unwrap();
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }
        let Json(loaded) = get(State(state)).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Linked { .. }));
    }
```

- [ ] **Step 2: Verify the tests fail to compile**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`
Expected: FAIL — `cannot find function unlink`.

- [ ] **Step 3: Implement**

In `rust/tonk-worker/src/router/account.rs`, add the empty-bytes guard at the top of `load_link`'s `Ok` arm:

```rust
        Ok(bytes) => {
            // An empty value is the unlink tombstone: the credential
            // store has no delete, so signing out writes empty bytes.
            if bytes.is_empty() {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
```

Add the handler after `link`:

```rust
/// Clear the stored account link for this profile — local sign-out.
///
/// Writes an empty tombstone over the stored link (the credential store
/// has no delete effect). The `root → device` delegation saved into the
/// access store at link time has no removal API and stays behind: a
/// signed-out device that is not also *revoked* still holds a usable
/// delegation. Revocation, not unlink, is the security boundary.
#[wasm_compat]
pub async fn unlink(
    State(state): State<AppState>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    state
        .profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(Vec::new())
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to clear local account link: {error}"))
        })?;
    Ok(Json(AccountStatus::Unlinked {
        device_did: state.profile.did().to_string(),
    }))
}
```

In `rust/tonk-worker/src/router.rs`, extend the account route (`.delete(...)` chains off the `MethodRouter` returned by `get(...)`, so no new import is needed):

```rust
        .route("/api/account", get(account::get).delete(account::unlink))
```

- [ ] **Step 4: Compile for the real target and run native gate**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests && cargo clippy -p tonk-worker --all-features`
Expected: clean. (The new tests execute in CI's web leg.)

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/account.rs rust/tonk-worker/src/router.rs
git commit -m "feat(tonk-worker): local account unlink via an empty-link tombstone"
```

---

### Task 3: Worker device-list and revoke proxy routes

**Files:**
- Create: `rust/tonk-worker/src/router/account_devices.rs`
- Modify: `rust/tonk-worker/src/router.rs` (module + routes)
- Modify: `rust/tonk-worker/src/router/account_backup.rs:104` (`post_for_bytes` → `pub(crate)`)

**Interfaces:**
- Consumes: `account::account_link`, `account_backup::{account_service_url, post_for_bytes}`, `tonk_identity::request::build_device_invocation(device: Ed25519Signer, link: &DelegationChain, command: Vec<String>, arguments: BTreeMap<String, Promised>) -> Result<Vec<u8>>`, `AccountDevice`/`RevokeDeviceRequest` from Task 1.
- Produces: `pub async fn list(State<AppState>) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError>` at `GET /api/account/devices`; `pub async fn revoke(State<AppState>, Json<RevokeDeviceRequest>) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError>` at `POST /api/account/devices/revoke` (returns the refreshed list so the UI re-renders from one response). Task 6 consumes both routes.

- [ ] **Step 1: Make `post_for_bytes` crate-visible**

In `rust/tonk-worker/src/router/account_backup.rs` change both cfg variants of `async fn post_for_bytes` to `pub(crate) async fn post_for_bytes`.

- [ ] **Step 2: Write the failing tests**

Create `rust/tonk-worker/src/router/account_devices.rs` with the test module first:

```rust
//! Proxy the account service's device registry for the linked profile.

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use std::sync::Arc;

    use axum::Json;
    use axum::extract::State;
    use tokio::sync::RwLock;
    use tonk_worker_api::RevokeDeviceRequest;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    use crate::TonkWorkerError;
    use crate::router::tests::test_state;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_refuses_to_list_devices_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state().await));
        assert!(matches!(
            list(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_to_revoke_the_requesting_device() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request =
            crate::router::account::tests_request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request).await.unwrap();
        }
        assert!(matches!(
            revoke(
                State(state),
                Json(RevokeDeviceRequest {
                    did: device_did.to_string()
                })
            )
            .await,
            Err(TonkWorkerError::Conflict(_))
        ));
    }
}
```

Note the second test needs the existing test helper `request_for` from `account.rs`'s test module. That helper is currently private to `account::tests`; promote it to a shared test-support function in `account.rs`:

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn tests_request_for(
    root_seed: &[u8; 32],
    audience: dialog_varsig::Did,
) -> tonk_worker_api::AccountLinkRequest {
    let root = tonk_identity::derive::derive_root_signer(root_seed)
        .await
        .unwrap();
    let root_did = root.did().to_string();
    let delegation = tonk_identity::delegation::mint_device_delegation(root, &audience)
        .await
        .unwrap();
    tonk_worker_api::AccountLinkRequest {
        root_did,
        delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
    }
}
```

and have the existing `tests::request_for` delegate to it (`tests_request_for(root_seed, audience).await`).

- [ ] **Step 3: Verify the tests fail to compile**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`
Expected: FAIL — `cannot find function list`.

- [ ] **Step 4: Implement the handlers**

Fill in `rust/tonk-worker/src/router/account_devices.rs` above the test module:

```rust
use std::collections::BTreeMap;

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::promise::Promised;
use serde::Deserialize;
use tonk_worker_api::{AccountDevice, RevokeDeviceRequest};

use super::AppState;
use super::account_backup::{account_service_url, post_for_bytes};
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// A device row as the account service serializes it. `delegationCid` is
/// deliberately not modeled: the UI has no use for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDevice {
    did: String,
    name: String,
    status: String,
    created_at: u64,
}

/// Resolve the stored link and service URL, or explain what's missing.
async fn linked_service(
    state: &TonkState,
) -> Result<(dialog_ucan_core::DelegationChain, String), TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let service = account_service_url().ok_or_else(|| {
        TonkWorkerError::NotFound("no account service is configured for this host".to_string())
    })?;
    Ok((link, service))
}

async fn fetch_devices(
    state: &TonkState,
    link: &dialog_ucan_core::DelegationChain,
    service: &str,
) -> Result<Vec<AccountDevice>, TonkWorkerError> {
    let device = state.profile.signer().signer().clone();
    let body = tonk_identity::request::build_device_invocation(
        device,
        link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-list invocation: {e}")))?;
    let endpoint = format!("{}/devices/list", service.trim_end_matches('/'));
    let bytes = post_for_bytes(&endpoint, body).await?;
    let rows: Vec<ServiceDevice> = serde_json::from_slice(&bytes)
        .map_err(|e| TonkWorkerError::Internal(format!("parse device list: {e}")))?;
    let this_did = state.profile.did().to_string();
    Ok(rows
        .into_iter()
        .map(|row| AccountDevice {
            this_device: row.did == this_did,
            did: row.did,
            name: row.name,
            status: row.status,
            created_at: row.created_at,
        })
        .collect())
}

/// List the devices registered under this profile's account.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    let state = state.read().await;
    let (link, service) = linked_service(&state).await?;
    Ok(Json(fetch_devices(&state, &link, &service).await?))
}

/// Revoke another of the account's devices, then return the fresh list.
///
/// Revoking the requesting device is refused: cutting a device off is an
/// action taken *about* a lost or untrusted device from a surviving one.
/// The local analogue on this device is unlink (sign out).
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    let state = state.read().await;
    if request.did == state.profile.did().to_string() {
        return Err(TonkWorkerError::Conflict(
            "cannot revoke the device you are using; sign out instead".to_string(),
        ));
    }
    let (link, service) = linked_service(&state).await?;
    let device = state.profile.signer().signer().clone();
    let arguments = [("did".to_owned(), Promised::String(request.did))]
        .into_iter()
        .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-revoke invocation: {e}")))?;
    let endpoint = format!("{}/devices/revoke", service.trim_end_matches('/'));
    let _ = post_for_bytes(&endpoint, body).await?;
    Ok(Json(fetch_devices(&state, &link, &service).await?))
}
```

Register the module and routes. In `rust/tonk-worker/src/router.rs` add `pub(crate) mod account_devices;` beside the existing `pub(crate) mod migrate;` declaration, and in `api_router_from_state`:

```rust
        .route("/api/account/devices", get(account_devices::list))
        .route(
            "/api/account/devices/revoke",
            post(account_devices::revoke),
        )
```

- [ ] **Step 5: Compile both targets, run native gate**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests && cargo clippy -p tonk-worker --all-features && cargo fmt --check`
Expected: clean. The two wasm tests execute in CI's web leg; they cover the unlinked and revoke-self guards, which never touch the network. The happy path's wire contract is proven natively in Task 4.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker/src/router/account_devices.rs rust/tonk-worker/src/router/account_backup.rs rust/tonk-worker/src/router/account.rs rust/tonk-worker/src/router.rs
git commit -m "feat(tonk-worker): proxy account device list and revoke"
```

---

### Task 4: Wire-contract test in the account service

The worker (Task 3) and CLI (Task 8) both parse `/devices/list` JSON and both send `did` as the revoke argument. The service's integration test already drives `devices/list`; pin the exact field names and the revoke round trip so camelCase drift breaks a native test, not production.

**Files:**
- Modify: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- Consumes: the existing `it_drives_the_full_ceremony_over_http` scaffolding (local HTTP server + `build_device_invocation` helpers already used there — reuse its container-building code verbatim; read the test before editing).

- [ ] **Step 1: Extend the integration test**

In `it_drives_the_full_ceremony_over_http`, directly after the existing `/devices/list` assertions (the block ending `assert_eq!(devices[1]["name"], "phone");` around `tests/service.rs:120`), add:

```rust
    // The worker and CLI parse exactly these keys; renaming one is a
    // breaking wire change.
    for key in ["did", "name", "status", "delegationCid", "createdAt"] {
        assert!(
            devices[0].get(key).is_some(),
            "device list row is missing `{key}`"
        );
    }

    // POST /devices/revoke -> the first device cuts off the second.
    let body = container(
        vec!["account".into(), "device".into(), "revoke".into()],
        [(
            "did".to_owned(),
            Promised::String(second_did.clone()),
        )]
        .into_iter()
        .collect(),
    )
    .await;
    let response = client
        .post(format!("{base}/devices/revoke"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let body = container(
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{base}/devices/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    let devices: serde_json::Value = response.json().await.unwrap();
    let devices = devices.as_array().unwrap();
    assert_eq!(devices[0]["status"], "active");
    assert_eq!(devices[1]["status"], "revoked");
```

Note `second_did` is already in scope from the self-link leg; the later handoff leg re-uses none of these bindings, so the shadowed `body`/`response`/`devices` names follow the test's existing style.

- [ ] **Step 2: Run the test**

Run: `cargo test -p tonk-account-service --features helpers --test service`
Expected: PASS. If the revoke leg fails on the arguments shape, the service handler is the authority (`string_argument(&caller, "did")` in `handlers/devices.rs:178`) — fix the test, not the handler.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-account-service/tests/service.rs
git commit -m "test(tonk-account-service): pin the device list and revoke wire contract"
```

---

### Task 5: UI API wrappers

**Files:**
- Modify: `rust/tonk-ui/src/api.rs` (after `save_account_link`, `api.rs:473`)

**Interfaces:**
- Consumes: worker routes from Tasks 2–3; `AccountDevice`, `RevokeDeviceRequest`, `AccountStatus` (add the two new names to the existing `tonk_worker_api` import in `api.rs`).
- Produces: `pub async fn account_devices() -> Result<Vec<AccountDevice>, TonkUiError>`, `pub async fn revoke_account_device(did: String) -> Result<Vec<AccountDevice>, TonkUiError>`, `pub async fn unlink_account() -> Result<AccountStatus, TonkUiError>`. Task 6 consumes all three.

- [ ] **Step 1: Implement the wrappers**

```rust
/// List the devices registered under the linked account.
pub async fn account_devices() -> Result<Vec<AccountDevice>, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/account/devices", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/account/devices returned {status}: {text}"
        )))
    }
}

/// Revoke one of the account's devices; returns the refreshed list.
pub async fn revoke_account_device(did: String) -> Result<Vec<AccountDevice>, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/devices/revoke", origin()))
        .json(&RevokeDeviceRequest { did })
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/account/devices/revoke returned {status}: {text}"
        )))
    }
}

/// Clear this browser's stored account link (local sign-out).
pub async fn unlink_account() -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .delete(format!("{}/api/account", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "DELETE /api/account returned {status}: {text}"
        )))
    }
}
```

- [ ] **Step 2: Compile**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-ui/src/api.rs
git commit -m "feat(tonk-ui): device list, revoke, and unlink api wrappers"
```

---

### Task 6: UI devices panel

**Files:**
- Modify: `rust/tonk-ui/src/account.html`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/account.css`

**Interfaces:**
- Consumes: Task 5 wrappers; existing helpers `set_mode`, `set_busy`, `show_error`, `clear_error`, `on_click` in `account.rs`.
- Produces: a `devices` mode reachable from the success panel; `fn render_devices(host: &HtmlElement, devices: &[AccountDevice])` (module-private).

- [ ] **Step 1: Markup**

In `account.html`, add to the success panel (before the "Back to Tonk" link):

```html
    <button id="account-manage-devices" class="account__button account__button--secondary" type="button">Manage devices</button>
```

and a new panel before the `#account-working` line:

```html
  <section id="account-devices" class="account__panel" hidden>
    <h2>Your devices</h2>
    <p class="account__hint">Revoking a device cuts off its sync access. Spaces that device joined before it was linked may need a fresh invite afterwards.</p>
    <ul id="account-device-list" class="account__devices"></ul>
    <div class="account__actions">
      <button id="account-unlink" class="account__button account__button--quiet" type="button">Sign out on this device</button>
      <button id="account-devices-back" class="account__button account__button--secondary" type="button">Back</button>
    </div>
  </section>
```

- [ ] **Step 2: Failing wasm test**

Append to the existing `run_in_browser` test module in `rust/tonk-ui/src/account.rs` (follow the module's existing element-construction helpers — read the test module first and reuse its way of mounting a `<tonk-account>` host):

```rust
    #[dialog_common::test]
    async fn it_renders_the_device_list_with_a_this_device_marker() {
        let host = mounted_account_host().await;
        let devices = vec![
            tonk_worker_api::AccountDevice {
                did: "did:key:zThis".into(),
                name: "This browser".into(),
                status: "active".into(),
                created_at: 1_753_300_000,
                this_device: true,
            },
            tonk_worker_api::AccountDevice {
                did: "did:key:zOther".into(),
                name: "Old laptop".into(),
                status: "revoked".into(),
                created_at: 1_753_200_000,
                this_device: false,
            },
        ];
        render_devices(&host, &devices);

        let list = host
            .query_selector("#account-device-list")
            .unwrap()
            .unwrap();
        let items = list.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 2);
        let text = list.text_content().unwrap();
        assert!(text.contains("This browser"));
        assert!(text.contains("this device"));
        assert!(text.contains("revoked"));
        // Only the active, non-self row gets a revoke button.
        assert_eq!(
            list.query_selector_all("button[data-revoke]").unwrap().length(),
            1
        );
    }
```

If the module has no `mounted_account_host` helper, add one mirroring how its existing tests create the element (`document.create_element("tonk-account")`, append to body, await a tick). Adapt the helper name to whatever the module already uses if one exists.

- [ ] **Step 3: Verify compile failure**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown --tests`
Expected: FAIL — `cannot find function render_devices`.

- [ ] **Step 4: Implement rendering and bindings**

In `account.rs`:

1. Add `("devices", "#account-devices")` to the panel array in `set_mode`.

2. Add the renderer (all user text via `set_text_content`):

```rust
fn render_devices(host: &HtmlElement, devices: &[tonk_worker_api::AccountDevice]) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(Some(list)) = host.query_selector("#account-device-list") else {
        return;
    };
    list.set_inner_html("");
    for device in devices {
        let Ok(item) = document.create_element("li") else {
            continue;
        };
        let _ = item.set_attribute("class", "account__device-row");

        let Ok(name) = document.create_element("span") else {
            continue;
        };
        name.set_text_content(Some(&device.name));

        let Ok(meta) = document.create_element("span") else {
            continue;
        };
        let _ = meta.set_attribute("class", "account__device-meta");
        let registered = js_sys::Date::new(&JsValue::from_f64(device.created_at as f64 * 1000.0))
            .to_locale_date_string("default", &JsValue::UNDEFINED);
        let mut details = format!("{} · {}", device.status, String::from(registered));
        if device.this_device {
            details.push_str(" · this device");
        }
        meta.set_text_content(Some(&details));

        let _ = item.append_child(&name);
        let _ = item.append_child(&meta);

        if device.status == "active" && !device.this_device {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("class", "account__button account__button--quiet");
            let _ = button.set_attribute("data-revoke", &device.did);
            button.set_text_content(Some("Revoke"));
            let _ = item.append_child(&button);
        }
        let _ = list.append_child(&item);
    }
}
```

3. Add a loader and wire the clicks in `bind`:

```rust
fn load_devices(host: HtmlElement) {
    set_busy(&host, true, "Loading devices…");
    spawn_local(async move {
        match crate::api::account_devices().await {
            Ok(devices) => {
                set_busy(&host, false, "");
                render_devices(&host, &devices);
                set_mode(&host, "devices");
            }
            Err(error) => {
                set_busy(&host, false, "");
                show_error(&host, error.to_string());
            }
        }
    });
}
```

In `bind`, add:

```rust
    on_click(host, "#account-manage-devices", |host| {
        clear_error(&host);
        load_devices(host);
    });
    on_click(host, "#account-devices-back", |host| {
        clear_error(&host);
        set_mode(&host, "success");
    });
    on_click(host, "#account-unlink", |host| {
        clear_error(&host);
        let confirmed = window()
            .map(|window| {
                window
                    .confirm_with_message(
                        "Sign out of your account on this device? Your data stays; \
                         this browser stops acting as the account until you log in again.",
                    )
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        set_busy(&host, true, "Signing out…");
        spawn_local(async move {
            match crate::api::unlink_account().await {
                Ok(_) => {
                    set_busy(&host, false, "");
                    set_mode(&host, "choice");
                }
                Err(error) => {
                    set_busy(&host, false, "");
                    show_error(&host, error.to_string());
                }
            }
        });
    });
```

4. Revoke clicks are per-row dynamic buttons, so bind one delegated listener on the list container (in `bind`, after the handlers above):

```rust
    if let Ok(Some(list)) = host.query_selector("#account-device-list") {
        let host_for_revoke = host.clone();
        let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Some(did) = target.get_attribute("data-revoke") else {
                return;
            };
            let host = host_for_revoke.clone();
            let confirmed = window()
                .map(|window| {
                    window
                        .confirm_with_message(
                            "Revoke this device? It immediately loses account and sync \
                             access. Spaces it joined before it was linked may need a \
                             fresh invite.",
                        )
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if !confirmed {
                return;
            }
            clear_error(&host);
            set_busy(&host, true, "Revoking device…");
            spawn_local(async move {
                match crate::api::revoke_account_device(did).await {
                    Ok(devices) => {
                        set_busy(&host, false, "");
                        render_devices(&host, &devices);
                    }
                    Err(error) => {
                        set_busy(&host, false, "");
                        show_error(&host, error.to_string());
                    }
                }
            });
        });
        let _ = list.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
        closure.forget();
    }
```

5. `account.css` additions:

```css
.account__devices {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 0.5rem;
}

.account__device-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--account-border, rgba(0, 0, 0, 0.12));
  border-radius: 0.5rem;
}

.account__device-meta {
  font-size: 0.85em;
  opacity: 0.7;
}
```

(Adopt the file's existing custom-property names if it already defines a border variable — read the file and match.)

- [ ] **Step 5: Compile both targets**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown --tests && cargo clippy -p tonk-ui --all-features && cargo fmt --check`
Expected: clean; the wasm test runs in CI's web leg.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-ui/src/account.html rust/tonk-ui/src/account.rs rust/tonk-ui/src/account.css
git commit -m "feat(tonk-ui): account devices panel with revoke and sign-out"
```

---

### Task 7: CLI `tonk account devices` / `tonk account revoke`

**Files:**
- Modify: `rust/tonk-cli/Cargo.toml` (add `tonk-identity = { workspace = true }`)
- Modify: `rust/tonk-cli/src/account.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs` (`AccountCommand` at :417, dispatch in `account_op` at :877, telemetry match at :700)

**Interfaces:**
- Consumes: `stored_link` (already in `account.rs`), `profile.signer().signer().clone() -> Ed25519Signer` (verified public: `dialog-credentials/src/credential/signer.rs:36`), `build_device_invocation` (same signature as Task 3), the service wire shape pinned in Task 4.
- Produces: `pub struct DeviceRow { pub did: String, pub name: String, pub status: String, pub created_at: u64 }`; `pub async fn devices(profile: &Profile, service_url: &str) -> Result<Vec<DeviceRow>>`; `pub async fn revoke(profile: &Profile, service_url: &str, did: &str) -> Result<()>`.

- [ ] **Step 1: Write the failing native tests**

Append to the `tests` module in `rust/tonk-cli/src/account.rs`:

```rust
    #[test]
    fn it_parses_a_service_device_row() {
        let rows: Vec<DeviceRow> = serde_json::from_str(
            r#"[{"did":"did:key:z1","name":"laptop","status":"active",
                 "delegationCid":"bafy","createdAt":1753300000}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].did, "did:key:z1");
        assert_eq!(rows[0].created_at, 1_753_300_000);
    }

    #[test]
    fn it_refuses_to_revoke_the_own_device_did() {
        assert!(revoke_target_guard("did:key:same", "did:key:same").is_err());
        assert!(revoke_target_guard("did:key:same", "did:key:other").is_ok());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tonk-cli it_parses_a_service_device_row it_refuses_to_revoke_the_own_device_did`
Expected: FAIL — missing `DeviceRow` / `revoke_target_guard`.

- [ ] **Step 3: Implement in `account.rs`**

Add `tonk-identity = { workspace = true }` to `rust/tonk-cli/Cargo.toml` (dependencies section, alphabetical position).

```rust
/// One registry row from `POST /devices/list`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    /// The device's DID.
    pub did: String,
    /// Display name registered at link time.
    pub name: String,
    /// Registry status: `active` or `revoked`.
    pub status: String,
    /// Registration time, seconds since the epoch.
    pub created_at: u64,
}

fn revoke_target_guard(own_did: &str, target_did: &str) -> Result<()> {
    if own_did == target_did {
        bail!("refusing to revoke the device you are using");
    }
    Ok(())
}

async fn linked_chain(profile: &Profile) -> Result<DelegationChain> {
    let bytes = stored_link(profile)
        .await?
        .context("this profile is not linked to an account; run `tonk account link`")?;
    DelegationChain::try_from(bytes.as_slice()).context("stored account delegation is invalid")
}

async fn post_invocation(
    service_url: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response> {
    let response = reqwest::Client::new()
        .post(format!(
            "{}/{}",
            service_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/cbor")
        .body(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("failed to reach the account service at {path}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("account service rejected {path} ({status}): {text}");
    }
    Ok(response)
}

/// List the devices registered under this profile's account.
pub async fn devices(profile: &Profile, service_url: &str) -> Result<Vec<DeviceRow>> {
    let link = linked_chain(profile).await?;
    let body = tonk_identity::request::build_device_invocation(
        profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "device".into(), "list".into()],
        std::collections::BTreeMap::new(),
    )
    .await
    .context("failed to sign the device-list request")?;
    let response = post_invocation(service_url, "devices/list", body).await?;
    response
        .json()
        .await
        .context("account service returned an invalid device list")
}

/// Revoke another of the account's devices.
pub async fn revoke(profile: &Profile, service_url: &str, did: &str) -> Result<()> {
    revoke_target_guard(&profile.did().to_string(), did)?;
    let link = linked_chain(profile).await?;
    let arguments = [(
        "did".to_owned(),
        dialog_ucan_core::promise::Promised::String(did.to_owned()),
    )]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    .context("failed to sign the revoke request")?;
    post_invocation(service_url, "devices/revoke", body).await?;
    Ok(())
}
```

Add `use anyhow::Context;`-compatible imports as needed (`Context` is already imported; add `dialog_ucan_core::promise::Promised` only at the use site as written above to keep the import list stable).

- [ ] **Step 4: Wire the subcommands in `bin/tonk.rs`**

Extend `AccountCommand`:

```rust
    /// List the devices linked to this profile's account
    Devices {
        /// Account service base URL (for staging or local development).
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_SERVICE_URL,
            hide = true
        )]
        service_url: String,
    },

    /// Revoke one of the account's devices by DID
    #[command(after_help = "Examples:\n  tonk account revoke did:key:z6Mk...")]
    Revoke {
        /// DID of the device to revoke (see `tonk account devices`).
        #[arg(value_name = "DID")]
        did: String,
        /// Account service base URL (for staging or local development).
        #[arg(
            long,
            value_name = "URL",
            default_value = account::DEFAULT_SERVICE_URL,
            hide = true
        )]
        service_url: String,
    },
```

Extend the telemetry match at `bin/tonk.rs:703`:

```rust
                AccountCommand::Status => "status",
                AccountCommand::Link { .. } => "link",
                AccountCommand::Devices { .. } => "devices",
                AccountCommand::Revoke { .. } => "revoke",
```

Extend `account_op`:

```rust
        AccountCommand::Devices { service_url } => {
            match account::devices(&profile, &service_url).await {
                Ok(rows) => {
                    let own = profile.did().to_string();
                    for row in rows {
                        let marker = if row.did == own { " (this device)" } else { "" };
                        println!("{}\t{}\t{}{}", row.status, row.name, row.did, marker);
                    }
                    ExitCode::Success
                }
                Err(error) => print_error(error.to_string()),
            }
        }
        AccountCommand::Revoke { did, service_url } => {
            match account::revoke(&profile, &service_url, &did).await {
                Ok(()) => {
                    println!("revoked\ndevice: {did}");
                    ExitCode::Success
                }
                Err(error) => print_error(error.to_string()),
            }
        }
```

- [ ] **Step 5: Run tests and the workspace gate**

Run: `cargo test -p tonk-cli && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: PASS / clean.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-cli/Cargo.toml rust/tonk-cli/src/account.rs rust/tonk-cli/src/bin/tonk.rs
git commit -m "feat(cli): tonk account devices and revoke"
```

---

### Task 8: Final verification and PR

- [ ] **Step 1: Full gates**

Run, expecting all clean:

```bash
cargo clippy --workspace --all-targets --all-features
cargo fmt --check
cargo test -p tonk-worker-api -p tonk-cli
cargo test -p tonk-account-service --features helpers
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

- [ ] **Step 2: Manual staging smoke (needs a human or a browser session)**

On `staging.tonk.xyz` with a linked account: open `/account`, Manage devices → list renders with "this device"; revoke a second linked browser → its next sync presign is refused (requires the revocation-enforcement PR to be deployed); sign out → `/account` shows the choice panel; `tonk account link --service-url https://accounts-staging.tonk.xyz --account-url https://staging.tonk.xyz/account/link`, then `tonk account devices --service-url …` lists the CLI row.

- [ ] **Step 3: PR**

Base `staging`, title `feat(account): device management surface`. Body notes: wasm test legs run in CI only; unlink leaves the access-store delegation behind (revoke is the security boundary — see Task 2's doc comment); UI copy warns about re-anchored chains dying with a revoked device.

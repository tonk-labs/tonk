//! Best-effort backup of a delegation chain to the account service, so a
//! later device can recover the space. Covers both a claimed space's
//! `space -> eph -> root` chain and a created space's one-hop
//! `space -> root` chain.

use dialog_credentials::Ed25519Signer;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_repository::Repository;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;
use tonk_common::log;

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// What gets backed up per space: the delegation chain plus the invite's
/// sync URL, which the chain itself does not carry. A restoring device
/// needs both to mount and sync the space. The chain is either a claimed
/// space's `space -> eph -> root` chain or a created space's one-hop
/// `space -> root` chain.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ClaimBackup {
    /// Hex-encoded delegation chain: `space -> eph -> root` for a claimed
    /// space, or the one-hop `space -> root` for a created space.
    pub chain_hex: String,
    /// The invite's remote/sync URL, when it carried one.
    pub remote_url: Option<String>,
}

/// Resolve the account-service base URL for this context. Unknown hosts
/// resolve to `None` so backup is skipped rather than failing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn account_service_url() -> Option<String> {
    use wasm_bindgen::JsCast;
    let scope: web_sys::ServiceWorkerGlobalScope = js_sys::global().dyn_into().ok()?;
    match scope.location().host().as_str() {
        "tonk.spot" => Some("https://accounts.tonk.xyz".to_owned()),
        "staging.tonk.xyz" => Some("https://accounts-staging.tonk.xyz".to_owned()),
        _ => None,
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) fn account_service_url() -> Option<String> {
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
        // Native awaits this inline (there's no UI to stall), but it must
        // never hang indefinitely — bound it so a wedged account service
        // can't wedge whatever native caller is doing the claim.
        .timeout(std::time::Duration::from_secs(10))
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

/// POST a device-signed invocation container to the account service and
/// return the raw response body bytes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn post_for_bytes(
    endpoint: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, TonkWorkerError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&js_sys::Uint8Array::from(body.as_slice()).into());
    let request = Request::new_with_str_and_init(endpoint, &init)
        .map_err(|e| TonkWorkerError::Internal(format!("account-service request: {e:?}")))?;
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service-worker scope".to_owned()))?;
    let response: Response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| TonkWorkerError::Internal(format!("account-service fetch: {e:?}")))?;
    if !response.ok() {
        return Err(TonkWorkerError::Internal(format!(
            "account-service returned HTTP {}",
            response.status()
        )));
    }
    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| TonkWorkerError::Internal(format!("account-service body: {e:?}")))?,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("account-service body: {e:?}")))?;
    let array = js_sys::Uint8Array::new(&buffer);
    Ok(array.to_vec())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn post_for_bytes(
    endpoint: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, TonkWorkerError> {
    let response = reqwest::Client::new()
        .post(endpoint)
        .body(body)
        // Same reasoning as `post_chains_put`: bound the wait so a wedged
        // account service can't wedge the native caller.
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("account-service: {e}")))?;
    if !response.status().is_success() {
        return Err(TonkWorkerError::Internal(format!(
            "account-service returned HTTP {}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("account-service body: {e}")))?;
    Ok(bytes.to_vec())
}

/// List the keys of every chain this account has backed up. Used by
/// restore to discover what can be pulled from the account service.
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

/// Fetch one backed-up chain's raw artifact bytes by key. Used by restore
/// to pull down a chain discovered via [`list_backed_up_chains`].
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

/// Build the backup artifact, sign the device invocation, and POST it to
/// the account service. Takes only owned data so it can run detached from
/// the caller (see [`back_up_claim`]).
async fn run_backup(
    device: Ed25519Signer,
    link: DelegationChain,
    service: String,
    chain: DelegationChain,
    remote_url: Option<String>,
) -> Result<(), TonkWorkerError> {
    let chain_bytes = chain
        .to_bytes()
        .map_err(|e| TonkWorkerError::Internal(format!("serialize claimed chain: {e}")))?;
    let artifact = ClaimBackup {
        chain_hex: hex::encode(chain_bytes),
        remote_url,
    };
    let artifact_bytes = serde_json::to_vec(&artifact)
        .map_err(|e| TonkWorkerError::Internal(format!("serialize backup artifact: {e}")))?;

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

/// Resolve the account link, service URL, and device signer, then hand the
/// backup off to [`run_backup`]. Shared by every backup caller
/// ([`back_up_claim`] and [`back_up_owned_space`]): a no-op when the
/// profile is unlinked or the account service is unknown for this host.
///
/// The lookups here (account link, service URL, device signer) are cheap
/// local reads, so they run inline. The actual network POST is handed off
/// to run detached: on wasm via `spawn_local`, so a slow/hung account
/// service can never stall the caller's `.await`; on native the caller has
/// no UI to stall, so it awaits inline, bounded by `post_chains_put`'s
/// request timeout.
async fn dispatch_backup(
    tonk: &TonkState,
    context: &'static str,
    chain: DelegationChain,
    remote_url: Option<String>,
) {
    // Only account-holders back up; an unlinked device has no account to
    // escrow under and returns early.
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return;
    };
    let Some(service) = account_service_url() else {
        return;
    };
    let device = tonk.profile.signer().signer().clone();

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = run_backup(device, link, service, chain, remote_url).await {
                log!("{context} backup failed: {error}");
            }
        });
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        if let Err(error) = run_backup(device, link, service, chain, remote_url).await {
            log!("{context} backup failed: {error}");
        }
    }
}

/// Back up a claimed space's delegation to the account service.
/// Best-effort: any failure logs and is swallowed — the claiming device
/// already works, and the roster keys on the root regardless.
pub(crate) async fn back_up_claim(
    tonk: &TonkState,
    chain: &DelegationChain,
    remote_url: Option<&str>,
) {
    dispatch_backup(tonk, "claim", chain.clone(), remote_url.map(str::to_owned)).await;
}

/// Back up a re-anchored space's delegation to the account service. Used
/// by roster migration: a claimed space's held capability re-delegated
/// from the device to the account root, composing `space -> eph -> device
/// -> root`. Best-effort: any failure logs and is swallowed, same as
/// [`back_up_claim`].
///
/// Only called from the wasm worker's roster-migration sweep today, so
/// this is worker-only rather than carrying dead code on native, mirroring
/// [`back_up_owned_space`].
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn back_up_reanchored(tonk: &TonkState, chain: DelegationChain, remote_url: &str) {
    dispatch_backup(tonk, "reanchor", chain, Some(remote_url.to_owned())).await;
}

/// Back up a created space's `space -> root` delegation so another of the
/// account's devices can restore it. Best-effort and fire-and-forget; a
/// no-op when the profile is unlinked, or when `repository` doesn't hold a
/// signer (a joined/verifier-only space has nothing to delegate from —
/// only the space that created it can mint this).
///
/// Only called from the wasm worker today (its one hook is
/// `enable_sync_inner`, which is itself worker-only), so this — like that
/// hook — is worker-only rather than carrying dead code on native.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn back_up_owned_space(
    tonk: &TonkState,
    repository: &Repository,
    remote_url: &str,
) {
    if let Err(error) = try_back_up_owned_space(tonk, repository, remote_url).await {
        log!("created-space backup skipped: {error}");
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn try_back_up_owned_space(
    tonk: &TonkState,
    repository: &Repository,
    remote_url: &str,
) -> Result<(), TonkWorkerError> {
    let prefix = crate::router::repository::space_root_prefix(tonk, &repository.did()).await?;
    dispatch_backup(tonk, "created-space", prefix, Some(remote_url.to_owned())).await;
    Ok(())
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
        assert_eq!(
            account_service_url().as_deref(),
            Some("https://accounts.tonk.xyz")
        );
    }
}

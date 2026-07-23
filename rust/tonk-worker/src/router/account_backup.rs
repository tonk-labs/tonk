//! Best-effort backup of a claimed space's delegation to the account
//! service, so a later device can recover the space.

use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;
use tonk_common::log;

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

//! Account setup protocol negotiation with the deployed access service.

use axum::Json;
use axum_wasm_macros::wasm_compat;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use crate::TonkWorkerError;

const ACCOUNT_SETUP_LIFECYCLE_VERSION: u8 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capabilities {
    account_setup_lifecycle: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityResponse {
    service: String,
    capabilities: Capabilities,
}

fn validates(body: &[u8]) -> bool {
    serde_json::from_slice::<CapabilityResponse>(body).is_ok_and(|response| {
        response.service == "tonk-access-service"
            && response.capabilities.account_setup_lifecycle == ACCOUNT_SETUP_LIFECYCLE_VERSION
    })
}

fn unavailable() -> TonkWorkerError {
    TonkWorkerError::Upstream {
        status: 503,
        code: Some("ACCOUNT_SETUP_PROTOCOL_UNAVAILABLE".to_string()),
        message: "the account service does not support this Tonk setup protocol".to_string(),
    }
}

/// Confirm that this worker and its independently deployed account provider
/// implement the same pre-WebAuthn setup protocol.
#[wasm_compat]
pub async fn get() -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let endpoint = super::customer::service_origin()?
        .join("capabilities")
        .map_err(|error| TonkWorkerError::Internal(format!("capability endpoint: {error}")))?;
    let response = super::http::get(&endpoint).await?;
    if !validates(&response.body) {
        return Err(unavailable());
    }
    Ok(Json(serde_json::json!({
        "service": "tonk-access-service",
        "capabilities": {
            "accountSetupLifecycle": ACCOUNT_SETUP_LIFECYCLE_VERSION,
        },
    })))
}

#[cfg(test)]
mod tests {
    use super::validates;

    #[test]
    fn it_accepts_only_the_exact_account_setup_protocol() {
        assert!(validates(
            br#"{"service":"tonk-access-service","capabilities":{"accountSetupLifecycle":1}}"#
        ));
        for rejected in [
            &br#"{"service":"tonk-access-service","capabilities":{"accountSetupLifecycle":0}}"#[..],
            &br#"{"service":"tonk-access-service","capabilities":{"accountSetupLifecycle":2}}"#[..],
            &br#"{"service":"other","capabilities":{"accountSetupLifecycle":1}}"#[..],
            &br#"{"service":"tonk-access-service","capabilities":{}}"#[..],
            &br#"{"service":"tonk-access-service","capabilities":{"accountSetupLifecycle":1},"extra":true}"#[..],
            &b"not json"[..],
        ] {
            assert!(!validates(rejected), "accepted {rejected:?}");
        }
    }
}

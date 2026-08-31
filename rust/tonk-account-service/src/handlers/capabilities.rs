//! Public, privacy-neutral service capability discovery.

use serde::Serialize;
use worker::*;

use crate::handlers::with_cors_headers;

/// Stable capability versions advertised by the account service.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    account_setup_recovery: u8,
}

/// Exact additive response body shared by the Worker and native helper.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CapabilitiesResponse {
    service: &'static str,
    capabilities: Capabilities,
}

/// Build the stable capability response without consulting account state.
pub(crate) const fn response_body() -> CapabilitiesResponse {
    CapabilitiesResponse {
        service: "tonk-account-service",
        capabilities: Capabilities {
            account_setup_recovery: 1,
        },
    }
}

/// Return the public account-service capability marker with CORS headers.
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_json(&response_body()).map(with_cors_headers)
}

/// Answer the exact capability-route CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

#[cfg(test)]
mod tests {
    use super::response_body;
    use crate::handlers::{CORS_ALLOW_METHODS, CORS_ALLOW_ORIGIN};

    #[test]
    fn it_pins_the_worker_and_native_capability_payload() {
        assert_eq!(
            serde_json::to_string(&response_body()).unwrap(),
            r#"{"service":"tonk-account-service","capabilities":{"accountSetupRecovery":1}}"#
        );
        assert_eq!(CORS_ALLOW_ORIGIN, "*");
        assert_eq!(CORS_ALLOW_METHODS, "GET, POST, OPTIONS");
    }
}

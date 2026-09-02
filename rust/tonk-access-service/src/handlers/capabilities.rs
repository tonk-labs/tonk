//! Privacy-neutral setup protocol discovery.

use serde::Serialize;
use worker::*;

/// Version required by the account producer before it can create a passkey.
pub(crate) const ACCOUNT_SETUP_LIFECYCLE_VERSION: u8 = 1;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    account_setup_lifecycle: u8,
}

/// Exact additive response shared by the Worker and native test server.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CapabilitiesResponse {
    service: &'static str,
    capabilities: Capabilities,
}

/// Build the stable marker without consulting account or customer state.
pub(crate) const fn response_body() -> CapabilitiesResponse {
    CapabilitiesResponse {
        service: "tonk-access-service",
        capabilities: Capabilities {
            account_setup_lifecycle: ACCOUNT_SETUP_LIFECYCLE_VERSION,
        },
    }
}

fn with_cors(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "GET, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Access-Control-Expose-Headers", "Content-Type");
    response.with_headers(headers)
}

/// Advertise the setup protocol the deployed access service implements.
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_json(&response_body()).map(with_cors)
}

/// Answer capability discovery preflight without reading account state.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors(Response::empty()?.with_status(204)))
}

#[cfg(test)]
mod tests {
    use super::response_body;

    #[test]
    fn it_pins_the_account_setup_lifecycle_marker() {
        assert_eq!(
            serde_json::to_string(&response_body()).unwrap(),
            r#"{"service":"tonk-access-service","capabilities":{"accountSetupLifecycle":1}}"#
        );
    }
}

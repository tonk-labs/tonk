//! Service info endpoint.

use worker::*;

/// `GET /` → what this service is, and the anchor DID an account genesis
/// delegates to if it wants email-gated enrollment here.
pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    #[allow(unused_mut)]
    let mut info = serde_json::json!({
        "service": "tonk-account-service",
        "version": env!("CARGO_PKG_VERSION"),
    });

    // Absent when no anchor is configured. Genesis reads this to address
    // `root → recovery`, so publishing a DID the service cannot sign for
    // would strand every account created against it.
    #[cfg(target_arch = "wasm32")]
    if let Ok(anchor) = crate::handlers::build_anchor(&_ctx).await {
        use dialog_varsig::Principal;
        info["recoveryAnchor"] = serde_json::Value::String(anchor.did().to_string());
    }

    Response::from_json(&info)
}

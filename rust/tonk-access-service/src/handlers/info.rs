//! Service info endpoint.

use crate::identity::ServiceIdentity;
use worker::*;

pub async fn handle(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    // Get KV binding
    let kv = ctx.kv("CONFIG")?;

    // Get or create service identity
    let identity = ServiceIdentity::get_or_create(&kv)
        .await
        .map_err(|e| worker::Error::RustError(e.to_string()))?;

    // Return service info as JSON
    let info = serde_json::json!({
        "service": "tonk-access-service",
        "version": env!("CARGO_PKG_VERSION"),
        "did": identity.did_string(),
    });

    Response::from_json(&info)
}

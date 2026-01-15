//! Service info endpoint.

use worker::*;

pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    // Return service info as JSON
    let info = serde_json::json!({
        "service": "tonk-access-service",
        "version": env!("CARGO_PKG_VERSION"),
    });

    Response::from_json(&info)
}

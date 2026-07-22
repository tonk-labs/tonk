//! Health check endpoint.

use worker::*;

pub async fn handle(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::ok("OK")
}

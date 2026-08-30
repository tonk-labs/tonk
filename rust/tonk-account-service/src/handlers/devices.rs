//! `POST /devices/{list,register,revoke}` over UCAN-authorized invocations.

use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::devices::link_device;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};
use crate::store::Store;

/// `OPTIONS /devices/*` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

//! Same-origin browser deployment configuration.

use dialog_varsig::Principal;
use tonk_worker_api::DeploymentConfig;
use worker::*;

use crate::service::signer_from_hex;

/// Return the service endpoints belonging to this page deployment.
pub async fn handle(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    // Enrollment addresses the service by DID, so discovery carries it
    // when the identity is configured. Its absence is not an error: the
    // rest of the config still serves deployments without one.
    let service_did = ctx
        .secret("SERVICE_SECRET_KEY")
        .ok()
        .and_then(|seed| signer_from_hex(&seed.to_string()).ok())
        .map(|signer| signer.did().to_string());
    Response::from_json(&DeploymentConfig {
        service_did,
        account_service_url: None,
    })
}

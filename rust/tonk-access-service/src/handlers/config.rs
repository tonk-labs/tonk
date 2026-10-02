//! Same-origin browser deployment configuration.

use dialog_varsig::Principal;
use tonk_worker_api::{DeploymentConfig, SiteOrigins};
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
    // Sites render on origins of their own only where the deployment routes a
    // wildcard host to itself; elsewhere both vars are unset and sites stay in
    // sealed frames.
    let var = |name| {
        ctx.var(name)
            .ok()
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty())
    };
    let sites = var("SITE_HOST")
        .zip(var("APP_ORIGIN"))
        .map(|(host, app)| SiteOrigins {
            host,
            suffix: var("SITE_SUFFIX"),
            app,
        });
    Response::from_json(&DeploymentConfig {
        service_did,
        account_service_url: None,
        sites,
    })
}

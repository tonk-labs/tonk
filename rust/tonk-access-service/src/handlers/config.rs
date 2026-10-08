//! Same-origin browser deployment configuration.

use dialog_varsig::Principal;
use tonk_worker_api::{DeploymentConfig, SiteOrigins};
use worker::*;

use crate::service::signer_from_hex;

/// Return the service endpoints belonging to this page deployment.
pub async fn handle(req: Request, ctx: RouteContext<()>) -> Result<Response> {
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
        })
        // Only under the app's own name or a site's. A preview is also
        // reached at its `workers.dev` alias, where its sites stay sealed.
        .filter(|sites| requested_host(&req).is_some_and(|host| sites.serves(&host)));
    Response::from_json(&DeploymentConfig {
        service_did,
        account_service_url: None,
        sites,
    })
}

/// The host the browser asked for: the one a router in front of this worker
/// forwarded, or this request's own.
fn requested_host(req: &Request) -> Option<String> {
    if let Ok(Some(forwarded)) = req.headers().get("x-forwarded-host") {
        return Some(forwarded);
    }
    let url = req.url().ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

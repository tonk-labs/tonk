//! Same-origin browser deployment configuration.

use tonk_worker_api::DeploymentConfig;
use url::Url;
use worker::*;

fn configured_url(ctx: &RouteContext<()>, name: &str) -> Result<Url> {
    let value = ctx.env.var(name).map_err(|error| {
        console_error!("deployment configuration {name} is missing: {error}");
        Error::RustError("deployment configuration is unavailable".into())
    })?;
    let url = Url::parse(&value.to_string()).map_err(|error| {
        console_error!("deployment configuration {name} is invalid: {error}");
        Error::RustError("deployment configuration is unavailable".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        console_error!("deployment configuration {name} is not an absolute HTTP URL");
        return Err(Error::RustError(
            "deployment configuration is unavailable".into(),
        ));
    }
    Ok(url)
}

/// Return the service endpoints belonging to this page deployment.
pub async fn handle(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let config = (|| {
        Ok::<_, Error>(DeploymentConfig {
            account_service_url: configured_url(&ctx, "ACCOUNT_SERVICE_URL")?,
            revocation_relay_url: configured_url(&ctx, "REVOCATION_RELAY_URL")?,
        })
    })();
    match config {
        Ok(config) => Response::from_json(&config),
        Err(_) => Response::error("Deployment configuration is unavailable", 500),
    }
}

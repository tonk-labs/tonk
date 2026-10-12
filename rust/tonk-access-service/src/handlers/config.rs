//! Same-origin browser deployment configuration.

use dialog_varsig::Principal;
use tonk_worker_api::{DeploymentConfig, DiscoverConfig};
use worker::*;

use crate::service::signer_from_hex;

/// The wrangler var naming the template catalog the Discover tab lists
/// by default. Set per environment in `wrangler.toml`.
pub const TEMPLATE_CATALOG_URL: &str = "TEMPLATE_CATALOG_URL";

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

/// Return the Discover tab's deployment default:
/// `GET /.well-known/tonk/discover`.
///
/// Served beside `/.well-known/tonk` rather than inside it, because that
/// document is `deny_unknown_fields` and released CLIs parse it.
pub async fn handle_discover(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let setting = ctx
        .var(TEMPLATE_CATALOG_URL)
        .ok()
        .map(|var| var.to_string());
    Response::from_json(&discover_config(setting.as_deref()))
}

/// The answer for a deployment whose `TEMPLATE_CATALOG_URL` is `setting`:
/// absent, blank, or anything but https (or loopback http) is
/// `catalog: null`, so a misconfigured deployment names no default
/// rather than one every page would refuse.
pub fn discover_config(setting: Option<&str>) -> DiscoverConfig {
    DiscoverConfig::from_setting(setting)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serves_the_configured_catalog() {
        let url = "https://goblinoats.github.io/honky-tonks/catalog.json";
        assert_eq!(discover_config(Some(url)).catalog.as_deref(), Some(url));
    }

    #[dialog_common::test]
    fn it_serves_null_when_the_var_is_unset() {
        let value = serde_json::to_value(discover_config(None)).unwrap();
        assert_eq!(value, serde_json::json!({ "catalog": null }));
    }

    #[dialog_common::test]
    fn it_serves_null_for_an_invalid_var() {
        for invalid in [
            "",
            "not a url",
            "http://example.com/catalog.json",
            "ftp://example.com/c.json",
        ] {
            assert_eq!(
                discover_config(Some(invalid)).catalog,
                None,
                "`{invalid}` must not be advertised"
            );
        }
    }
}

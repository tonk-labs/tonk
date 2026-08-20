//! Provider-matched deployment-default discovery for native profiles.

use anyhow::{Context, Result, bail};
use tonk_worker_api::DeploymentConfig;
use url::Url;

/// Content-service endpoints advertised by the deployment that hosted the
/// account ceremony.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentDefaults {
    /// Normalized origin that hosted the ceremony.
    pub ceremony_origin: Url,
    /// Same-origin UCAN content endpoint.
    pub access_remote: Url,
    /// Where this deployment accepts revocations.
    ///
    /// The same endpoint as [`Self::access_remote`], and derived rather than
    /// advertised: a revocation is an ordinary `ucan/revoke` invocation now,
    /// so it goes where every other invocation goes. The deployment stopped
    /// advertising a separate relay when the standalone revocation registry
    /// was deleted.
    pub revocation_relay: Url,
}

/// Discover content defaults from the exact deployment used for account
/// linking, refusing configs that point at a different account provider.
pub async fn discover(
    account_url: &str,
    expected_account_service: &str,
) -> Result<DeploymentDefaults> {
    let ceremony_origin = ceremony_origin(account_url)?;
    let expected = validated_http_url(expected_account_service, "account service URL")?;
    let endpoint = ceremony_origin
        .join("/.well-known/tonk")
        .context("failed to form deployment discovery URL")?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("failed to build deployment discovery client")?;
    let config = client
        .get(endpoint)
        .send()
        .await
        .context("deployment discovery is unavailable")?
        .error_for_status()
        .context("deployment discovery returned an error")?
        .json::<DeploymentConfig>()
        .await
        .context("deployment discovery returned malformed configuration")?;
    let advertised = validated_http_url(
        config.account_service_url.as_str(),
        "advertised account service URL",
    )?;
    if normalized_service(&advertised) != normalized_service(&expected) {
        bail!(
            "deployment advertises account service {}, expected {}",
            advertised,
            expected
        );
    }
    let access_remote = ceremony_origin
        .join("/ucan/")
        .context("failed to form deployment access URL")?;
    Ok(DeploymentDefaults {
        ceremony_origin,
        revocation_relay: access_remote.clone(),
        access_remote,
    })
}

/// Validate a ceremony URL and return only its origin for safe persistence.
pub fn ceremony_origin(account_url: &str) -> Result<Url> {
    let ceremony = validated_http_url(account_url, "account ceremony URL")?;
    origin_url(&ceremony)
}

fn validated_http_url(value: &str, label: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("invalid {label}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        bail!("{label} must use https (or loopback http)");
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        bail!("{label} must be an absolute URL without userinfo");
    }
    if url.scheme() == "http" && !url.host().is_some_and(is_loopback) {
        bail!("{label} must use https unless its host is loopback");
    }
    Ok(url)
}

fn is_loopback(host: url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    }
}

fn origin_url(url: &Url) -> Result<Url> {
    Url::parse(&format!("{}/", url.origin().ascii_serialization()))
        .context("account ceremony URL has no usable origin")
}

fn normalized_service(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    let path = normalized.path().trim_end_matches('/').to_owned();
    normalized.set_path(if path.is_empty() { "/" } else { &path });
    normalized.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};

    async fn deployment_server(
        account_path: &str,
    ) -> Result<(String, tokio::task::JoinHandle<()>)> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let account: Url = format!("{origin}{account_path}").parse()?;
        let app = Router::new().route(
            "/.well-known/tonk",
            get(move || {
                let account = account.clone();
                async move {
                    Json(DeploymentConfig {
                        account_service_url: account,
                        service_did: None,
                    })
                }
            }),
        );
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((origin, handle))
    }

    #[tokio::test]
    async fn it_discovers_the_access_remote_from_the_ceremony_deployment() -> Result<()> {
        let (origin, server) = deployment_server("/accounts/").await?;
        let defaults = discover(
            &format!("{origin}/account/link?intent=login"),
            &format!("{origin}/accounts"),
        )
        .await?;
        assert_eq!(defaults.ceremony_origin.as_str(), format!("{origin}/"));
        assert_eq!(defaults.access_remote.as_str(), format!("{origin}/ucan/"));
        // A revocation is an ordinary invocation, so it is addressed to the
        // same endpoint rather than to a relay of its own.
        assert_eq!(
            defaults.revocation_relay.as_str(),
            defaults.access_remote.as_str()
        );
        server.abort();
        Ok(())
    }

    #[test]
    fn it_rejects_unsafe_ceremony_urls_before_network_access() {
        for url in [
            "relative",
            "ftp://deployment.example/account/link",
            "https://user@deployment.example/account/link",
            "http://deployment.example/account/link",
        ] {
            let error = validated_http_url(url, "account ceremony URL")
                .expect_err("unsafe URL must be rejected");
            assert!(!error.to_string().is_empty());
        }
    }

    #[tokio::test]
    async fn it_rejects_a_different_advertised_account_service() -> Result<()> {
        let (origin, server) = deployment_server("/other/").await?;
        let error = discover(
            &format!("{origin}/account/link"),
            &format!("{origin}/accounts"),
        )
        .await
        .expect_err("provider mismatch must be rejected");
        assert!(error.to_string().contains("expected"), "{error:#}");
        server.abort();
        Ok(())
    }
}

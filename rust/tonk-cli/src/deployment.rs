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
    /// Typed revocation relay advertised by the deployment.
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
    let revocation_relay = validated_http_url(
        config.revocation_relay_url.as_str(),
        "advertised revocation relay URL",
    )?;
    let access_remote = match config.access_remote_url {
        Some(url) => validated_http_url(url.as_str(), "advertised access remote URL")?,
        None => ceremony_origin
            .join("/ucan/")
            .context("failed to form deployment access URL")?,
    };
    Ok(DeploymentDefaults {
        ceremony_origin,
        access_remote,
        revocation_relay,
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
        access_remote_url: Option<Url>,
    ) -> Result<(String, tokio::task::JoinHandle<()>)> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let account: Url = format!("{origin}{account_path}").parse()?;
        let relay: Url = format!("{origin}/revocations/").parse()?;
        let app = Router::new().route(
            "/.well-known/tonk",
            get(move || {
                let account = account.clone();
                let relay = relay.clone();
                let access_remote_url = access_remote_url.clone();
                async move {
                    Json(DeploymentConfig {
                        access_remote_url,
                        account_service_url: account,
                        revocation_relay_url: relay,
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
        let (origin, server) = deployment_server("/accounts/", None).await?;
        let defaults = discover(
            &format!("{origin}/account/link?intent=login"),
            &format!("{origin}/accounts"),
        )
        .await?;
        assert_eq!(defaults.ceremony_origin.as_str(), format!("{origin}/"));
        assert_eq!(defaults.access_remote.as_str(), format!("{origin}/ucan/"));
        assert_eq!(
            defaults.revocation_relay.as_str(),
            format!("{origin}/revocations/")
        );
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn it_prefers_an_advertised_loopback_access_remote() -> Result<()> {
        let remote: Url = "http://127.0.0.1:4200/ucan/".parse()?;
        let (origin, server) = deployment_server("/accounts/", Some(remote.clone())).await?;
        let defaults = discover(
            &format!("{origin}/account/link"),
            &format!("{origin}/accounts"),
        )
        .await?;
        assert_eq!(defaults.access_remote, remote);
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
        let (origin, server) = deployment_server("/other/", None).await?;
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

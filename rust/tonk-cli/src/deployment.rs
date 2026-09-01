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
    ///
    /// Revocations are addressed here too. A revocation is an ordinary
    /// `ucan/revoke` invocation, so it goes where every other invocation
    /// goes; the deployment stopped advertising a relay of its own when the
    /// standalone revocation registry was deleted, and nothing here records
    /// one.
    pub access_remote: Url,
}

/// Discover content defaults from the exact deployment used for account
/// linking.
///
/// The config is fetched to confirm the deployment answers at all —
/// a 404 there is what "deployment configuration is invalid" means —
/// and the access remote comes from the origin that served it.
pub async fn discover(account_url: &str) -> Result<DeploymentDefaults> {
    let ceremony_origin = ceremony_origin(account_url)?;
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
    let _ = config;
    let access_remote = ceremony_origin
        .join("/ucan/")
        .context("failed to form deployment access URL")?;
    Ok(DeploymentDefaults {
        ceremony_origin,
        access_remote,
    })
}

/// The registry record for a freshly linked account.
///
/// Discovery enriches this record; it does not gate it. The link is already
/// complete by the time discovery runs — the grant is installed, the
/// provider attachment persisted, the session active — so a deployment that
/// cannot be reached leaves the endpoints unset rather than costing the
/// account its only registry row. Every reader of those endpoints already
/// answers "sign in again" when they are missing, which is a state the
/// device can act on; a missing record is one where `status` and the
/// registry disagree about whether this device is signed in at all.
pub fn account_record(
    root_did: &str,
    ceremony_page: &str,
    defaults: Option<&DeploymentDefaults>,
) -> crate::space::AccountRecord {
    let mut record = crate::space::AccountRecord::new(root_did);
    match defaults {
        Some(defaults) => {
            record.ceremony_origin = Some(defaults.ceremony_origin.to_string());
            record.access_remote = Some(defaults.access_remote.to_string());
        }
        // The origin is the one part discovery never needed the network
        // for, and it is what a later sign-in reads to know where to ask.
        None => {
            record.ceremony_origin = ceremony_origin(ceremony_page)
                .ok()
                .map(|origin| origin.to_string());
        }
    }
    record
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
                    let _ = &account;
                    Json(DeploymentConfig::default())
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
        let defaults = discover(&format!("{origin}/settings/link?intent=login")).await?;
        assert_eq!(defaults.ceremony_origin.as_str(), format!("{origin}/"));
        assert_eq!(defaults.access_remote.as_str(), format!("{origin}/ucan/"));
        server.abort();
        Ok(())
    }

    #[test]
    fn it_rejects_unsafe_ceremony_urls_before_network_access() {
        for url in [
            "relative",
            "ftp://deployment.example/settings/link",
            "https://user@deployment.example/settings/link",
            "http://deployment.example/settings/link",
        ] {
            let error = validated_http_url(url, "account ceremony URL")
                .expect_err("unsafe URL must be rejected");
            assert!(!error.to_string().is_empty());
        }
    }

    #[dialog_common::test]
    async fn it_records_every_endpoint_the_deployment_advertised() -> Result<()> {
        let (origin, server) = deployment_server("/accounts/").await?;
        let page = format!("{origin}/settings/link?intent=login");
        let defaults = discover(&page).await?;
        let record = account_record("did:key:zRoot", &page, Some(&defaults));
        assert_eq!(record.root, "did:key:zRoot");
        assert_eq!(
            record.ceremony_origin.as_deref(),
            Some(&*format!("{origin}/"))
        );
        assert_eq!(
            record.access_remote.as_deref(),
            Some(&*format!("{origin}/ucan/"))
        );
        server.abort();
        Ok(())
    }

    #[dialog_common::test]
    async fn it_records_the_ceremony_origin_when_discovery_fails() -> Result<()> {
        let record = account_record(
            "did:key:zRoot",
            "https://deployment.example/settings/link?intent=login",
            None,
        );
        assert_eq!(record.root, "did:key:zRoot");
        assert_eq!(
            record.ceremony_origin.as_deref(),
            Some("https://deployment.example/")
        );
        assert_eq!(record.access_remote, None);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_records_the_root_alone_when_the_ceremony_url_is_unusable() -> Result<()> {
        let record = account_record("did:key:zRoot", "not-a-url", None);
        assert_eq!(record.root, "did:key:zRoot");
        assert_eq!(record.ceremony_origin, None);
        assert_eq!(record.access_remote, None);
        Ok(())
    }
}

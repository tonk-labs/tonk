//! The Discover tab's deployment default, and the rule every template
//! catalog URL is held to.
//!
//! The Discover tab lists templates from catalogs. Which catalog a
//! deployment offers by default is configuration — prod, staging, dev
//! and preview may each point somewhere else — so it is served at
//! `GET /.well-known/tonk/discover` rather than baked into the library.
//! The account owner may add catalogs of their own on top; those are
//! facts on their profile branch, and they pass the same
//! [`admit_catalog_url`] check before they are recorded.

use serde::{Deserialize, Serialize};
use url::Url;

/// What `GET /.well-known/tonk/discover` answers.
///
/// Deliberately its own document and NOT a field on
/// [`DeploymentConfig`](crate::DeploymentConfig): that one is
/// `deny_unknown_fields` and every released CLI parses it, so a new
/// field there would break every installed CLI against a newer
/// deployment.
///
/// For the same reason this type is deliberately NOT
/// `deny_unknown_fields`: a page from an older build reading a newer
/// deployment's answer must keep working, so a field added later is
/// ignored rather than refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverConfig {
    /// The template catalog this deployment lists by default, or `null`
    /// when it names none (or names one that fails
    /// [`admit_catalog_url`]). A page that reads `null` falls back to
    /// the catalog its own library names.
    #[serde(default)]
    pub catalog: Option<String>,
}

impl DiscoverConfig {
    /// The config for a deployment whose catalog setting is `raw`.
    ///
    /// An unset, blank, or inadmissible setting yields `catalog: None`:
    /// a misconfigured deployment advertises no default rather than one
    /// the page would refuse to fetch.
    pub fn from_setting(raw: Option<&str>) -> Self {
        let catalog = raw
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .and_then(|raw| admit_catalog_url(raw).ok())
            .map(String::from);
        Self { catalog }
    }
}

/// Why a URL cannot name a template catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogRefusal {
    /// The text does not parse as an absolute URL.
    NotAUrl(String),
    /// The URL is neither `https:` nor `http:` to a loopback host.
    Insecure {
        /// The scheme it used.
        scheme: String,
    },
    /// The URL carries a username or password. A catalog is public, and
    /// credentials in a URL would be stored as a fact and replicated.
    Credentials,
}

impl std::fmt::Display for CatalogRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAUrl(raw) => write!(f, "`{raw}` is not a URL"),
            Self::Insecure { scheme } => write!(
                f,
                "a catalog must be an https URL (or http on localhost), not `{scheme}:`"
            ),
            Self::Credentials => write!(f, "a catalog URL cannot carry a username or password"),
        }
    }
}

impl std::error::Error for CatalogRefusal {}

/// Whether `raw` may name a template catalog, and its canonical form.
///
/// A catalog decides what code is installed into new spaces, so it is
/// held to the rule the worker applies to the template sources it
/// fetches: `https:`, or `http:` to `localhost`, `127.0.0.1` or `[::1]`
/// for development. The fragment is dropped, since a template reference
/// is the catalog URL with the slug as its fragment.
pub fn admit_catalog_url(raw: &str) -> Result<Url, CatalogRefusal> {
    let mut url =
        Url::parse(raw.trim()).map_err(|_| CatalogRefusal::NotAUrl(raw.trim().to_owned()))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" if url.host_str().is_some() => {}
        "http" if loopback => {}
        scheme => {
            return Err(CatalogRefusal::Insecure {
                scheme: scheme.to_owned(),
            });
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CatalogRefusal::Credentials);
    }
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const CATALOG: &str = "https://goblinoats.github.io/honky-tonks/catalog.json";

    #[dialog_common::test]
    fn it_serializes_an_absent_catalog_as_null() {
        let value = serde_json::to_value(DiscoverConfig::default()).unwrap();
        assert_eq!(value, serde_json::json!({ "catalog": null }));
    }

    #[dialog_common::test]
    fn it_round_trips_a_catalog() {
        let config = DiscoverConfig {
            catalog: Some(CATALOG.into()),
        };
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(text, format!(r#"{{"catalog":"{CATALOG}"}}"#));
        assert_eq!(
            serde_json::from_str::<DiscoverConfig>(&text).unwrap(),
            config
        );
    }

    /// Unlike `DeploymentConfig`, a field a newer deployment adds is
    /// ignored rather than refused, so an older page keeps working.
    #[dialog_common::test]
    fn it_ignores_fields_it_does_not_know() {
        let config: DiscoverConfig =
            serde_json::from_str(r#"{"catalog":null,"catalogs":["https://x.example/c.json"]}"#)
                .unwrap();
        assert_eq!(config, DiscoverConfig::default());
        let empty: DiscoverConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.catalog, None);
    }

    #[dialog_common::test]
    fn it_builds_the_config_from_the_deployment_setting() {
        assert_eq!(
            DiscoverConfig::from_setting(Some(CATALOG))
                .catalog
                .as_deref(),
            Some(CATALOG)
        );
        assert_eq!(DiscoverConfig::from_setting(None).catalog, None);
        assert_eq!(DiscoverConfig::from_setting(Some("  ")).catalog, None);
        assert_eq!(
            DiscoverConfig::from_setting(Some("http://example.com/catalog.json")).catalog,
            None,
            "an inadmissible setting advertises no default"
        );
    }

    #[dialog_common::test]
    fn it_admits_https_and_loopback_http() {
        for (raw, admitted) in [
            (CATALOG, CATALOG),
            (
                "http://localhost:8777/catalog.json",
                "http://localhost:8777/catalog.json",
            ),
            ("http://127.0.0.1:1/c.json", "http://127.0.0.1:1/c.json"),
            ("http://[::1]:9/c.json", "http://[::1]:9/c.json"),
            (
                " https://example.com/catalog.json#slug ",
                "https://example.com/catalog.json",
            ),
        ] {
            assert_eq!(admit_catalog_url(raw).unwrap().as_str(), admitted);
        }
    }

    #[dialog_common::test]
    fn it_refuses_everything_else_with_the_reason() {
        assert!(matches!(
            admit_catalog_url("http://example.com/catalog.json"),
            Err(CatalogRefusal::Insecure { scheme }) if scheme == "http"
        ));
        assert!(matches!(
            admit_catalog_url("http://localhost.example.com/catalog.json"),
            Err(CatalogRefusal::Insecure { .. })
        ));
        assert!(matches!(
            admit_catalog_url("javascript:alert(1)"),
            Err(CatalogRefusal::Insecure { scheme }) if scheme == "javascript"
        ));
        assert!(matches!(
            admit_catalog_url("file:///etc/passwd"),
            Err(CatalogRefusal::Insecure { .. })
        ));
        assert!(matches!(
            admit_catalog_url("catalog.json"),
            Err(CatalogRefusal::NotAUrl(_))
        ));
        assert!(matches!(
            admit_catalog_url("https://user:secret@example.com/catalog.json"),
            Err(CatalogRefusal::Credentials)
        ));
    }
}

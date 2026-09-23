//! Same-origin deployment configuration exposed to browser clients.

use serde::{Deserialize, Serialize};

/// Service endpoints selected by the deployment serving the current page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentConfig {
    /// The access service's signing DID, which customer enrollment
    /// addresses. Absent on a deployment whose service identity is not
    /// configured, and on configs written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_did: Option<String>,
    /// The account service that used to hold the registry. Accepted so
    /// a config written by an older deployment still parses — the
    /// fields are `deny_unknown_fields` — and ignored: every route it
    /// served is gone, and what they held are facts on the account's
    /// own branch or rows the access service already keeps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_service_url: Option<String>,
    /// Where sites render on origins of their own. Absent on a deployment
    /// with no wildcard host, whose sites stay in sealed frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sites: Option<SiteOrigins>,
}

/// The origins a deployment renders its sites on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SiteOrigins {
    /// The authority every site's origin sits under, `{label}.{host}`, with
    /// the app's scheme: `tonk.spot`, or `localhost:8080` in development.
    pub host: String,
    /// The origin of the app that frames the sites, the only one a site
    /// origin lets frame it besides the profile's own.
    pub app: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_serializes_the_service_identity() {
        let config = DeploymentConfig {
            service_did: Some("did:key:z6Mk".into()),
            ..DeploymentConfig::default()
        };
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["serviceDid"], "did:key:z6Mk");
        // A retired field is never written, so a fresh deployment
        // advertises only what it still serves.
        assert!(value.get("accountServiceUrl").is_none());
    }

    /// A config from a deployment that still names an account service
    /// parses: the field is ignored, not refused.
    #[test]
    fn it_still_reads_a_config_naming_an_account_service() {
        let config: DeploymentConfig =
            serde_json::from_str(r#"{"accountServiceUrl":"https://accounts.example/"}"#).unwrap();
        assert_eq!(
            config.account_service_url.as_deref(),
            Some("https://accounts.example/")
        );
    }

    #[test]
    fn it_names_where_sites_render() {
        let config = DeploymentConfig {
            sites: Some(SiteOrigins {
                host: "tonk.spot".into(),
                app: "https://staging.tonk.xyz".into(),
            }),
            ..DeploymentConfig::default()
        };
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["sites"]["host"], "tonk.spot");
        assert_eq!(value["sites"]["app"], "https://staging.tonk.xyz");
        // A deployment without them writes nothing, so its config reads the
        // same as before sites had origins of their own.
        let value = serde_json::to_value(DeploymentConfig::default()).unwrap();
        assert!(value.get("sites").is_none());
    }

    #[test]
    fn it_rejects_unknown_configuration() {
        assert!(serde_json::from_str::<DeploymentConfig>(r#"{"extra":true}"#).is_err());
    }
}

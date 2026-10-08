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
    /// What follows the label within a site's hostname, for a deployment
    /// that shares its zone with others. A pull request's preview keeps its
    /// sites beside staging's, at `{label}-pr33.tonk.spot`: a level of their
    /// own (`{label}.pr-33.tonk.spot`) is past what a wildcard certificate
    /// covers. Absent where the zone is the deployment's alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// The origin of the app that frames the sites, the only one a site
    /// origin lets frame it besides the profile's own.
    pub app: String,
}

impl SiteOrigins {
    /// The hostname of every site, with `*` where its label goes:
    /// `*.tonk.spot`, or `*-pr33.tonk.spot` with a suffix.
    pub fn pattern(&self) -> String {
        format!("*{}.{}", self.suffix.as_deref().unwrap_or(""), self.host)
    }

    /// Whether `host` is one these origins are for: the app's own, or one
    /// of its sites'.
    ///
    /// A deployment can be reached under other names too. A pull request's
    /// preview answers at its `workers.dev` alias as well as the name routed
    /// to it, and a site framed by a page on the alias would refuse its
    /// frame. On such a name the deployment names no site origins, and its
    /// sites stay in sealed frames.
    pub fn serves(&self, host: &str) -> bool {
        let app = self
            .app
            .split_once("://")
            .map_or(self.app.as_str(), |(_, rest)| rest);
        if host == app.trim_end_matches('/') {
            return true;
        }
        let Some(label) = host.strip_suffix(&format!(".{}", self.host)) else {
            return false;
        };
        !label.contains('.') && label.ends_with(self.suffix.as_deref().unwrap_or(""))
    }
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
                suffix: None,
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
    fn it_places_a_preview_beside_the_sites_it_shares_a_zone_with() {
        let staging = SiteOrigins {
            host: "tonk.spot".into(),
            suffix: None,
            app: "https://staging.tonk.xyz".into(),
        };
        assert_eq!(staging.pattern(), "*.tonk.spot");
        // No suffix is written, so staging's config reads as it did.
        assert!(
            serde_json::to_value(&staging)
                .unwrap()
                .get("suffix")
                .is_none()
        );

        let preview = SiteOrigins {
            host: "tonk.spot".into(),
            suffix: Some("-pr33".into()),
            app: "https://pr-33.tonk.spot".into(),
        };
        assert_eq!(preview.pattern(), "*-pr33.tonk.spot");
        let read: SiteOrigins =
            serde_json::from_value(serde_json::to_value(&preview).unwrap()).unwrap();
        assert_eq!(read, preview);
    }

    #[test]
    fn it_names_site_origins_only_under_its_own_names() {
        let preview = SiteOrigins {
            host: "tonk.foundation".into(),
            suffix: Some("-pr33".into()),
            app: "https://pr-33.tonk.foundation".into(),
        };
        assert!(preview.serves("pr-33.tonk.foundation"));
        assert!(preview.serves("profile-pr33.tonk.foundation"));
        assert!(preview.serves("b5uaspace-pr33.tonk.foundation"));
        // Its alias, another preview's site, and a deeper name are not its own.
        assert!(!preview.serves("pr-33-tonk-access-service-preview.tonk.workers.dev"));
        assert!(!preview.serves("profile-pr34.tonk.foundation"));
        assert!(!preview.serves("a.profile-pr33.tonk.foundation"));

        let dev = SiteOrigins {
            host: "tonk.foundation".into(),
            suffix: None,
            app: "https://tonk.foundation".into(),
        };
        assert!(dev.serves("tonk.foundation"));
        assert!(dev.serves("profile.tonk.foundation"));
        assert!(!dev.serves("tonk-access-service-dev.tonk.workers.dev"));
    }

    #[test]
    fn it_rejects_unknown_configuration() {
        assert!(serde_json::from_str::<DeploymentConfig>(r#"{"extra":true}"#).is_err());
    }
}

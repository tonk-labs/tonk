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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_serializes_the_service_identity() {
        let config = DeploymentConfig {
            service_did: Some("did:key:z6Mk".into()),
            account_service_url: None,
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
    fn it_rejects_unknown_configuration() {
        assert!(serde_json::from_str::<DeploymentConfig>(r#"{"extra":true}"#).is_err());
    }
}

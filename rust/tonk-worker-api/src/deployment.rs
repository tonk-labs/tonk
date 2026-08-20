//! Same-origin deployment configuration exposed to browser clients.

use serde::{Deserialize, Serialize};
use url::Url;

/// Service endpoints selected by the deployment serving the current page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentConfig {
    /// Content repository access remote. Absent on deployments where clients
    /// should derive the same-origin `/ucan/` endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_remote_url: Option<Url>,
    /// Account linking, custody, and device-management service.
    pub account_service_url: Url,
    /// Relay accepting immutable invitation and device revocation artifacts.
    pub revocation_relay_url: Url,
    /// The access service's signing DID, which customer enrollment
    /// addresses. Absent on a deployment whose service identity is not
    /// configured, and on configs written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_did: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_serializes_canonical_camel_case_urls() {
        let config = DeploymentConfig {
            access_remote_url: Some("https://sync.example/ucan/".parse().unwrap()),
            account_service_url: "https://accounts.example/".parse().unwrap(),
            revocation_relay_url: "https://relay.example/revocations".parse().unwrap(),
            service_did: None,
        };
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["accessRemoteUrl"], "https://sync.example/ucan/");
        assert_eq!(value["accountServiceUrl"], "https://accounts.example/");
        assert_eq!(
            value["revocationRelayUrl"],
            "https://relay.example/revocations"
        );
        // Absent identity serializes to nothing, so configs written by a
        // deployment without one keep parsing in strict old clients.
        assert!(value.get("serviceDid").is_none());

        let legacy: DeploymentConfig = serde_json::from_value(serde_json::json!({
            "accountServiceUrl": "https://accounts.example/",
            "revocationRelayUrl": "https://relay.example/revocations"
        }))
        .unwrap();
        assert_eq!(legacy.access_remote_url, None);
    }

    #[test]
    fn it_rejects_malformed_or_unknown_configuration() {
        for json in [
            r#"{"accountServiceUrl":"relative","revocationRelayUrl":"https://relay.example"}"#,
            r#"{"accountServiceUrl":"https://accounts.example","revocationRelayUrl":"https://relay.example","extra":true}"#,
        ] {
            assert!(serde_json::from_str::<DeploymentConfig>(json).is_err());
        }
    }
}

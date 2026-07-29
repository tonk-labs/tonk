//! Same-origin deployment configuration exposed to browser clients.

use serde::{Deserialize, Serialize};
use url::Url;

/// Service endpoints selected by the deployment serving the current page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentConfig {
    /// Account backup, restore, linking, and device-management service.
    pub account_service_url: Url,
    /// Relay accepting immutable invitation and device revocation artifacts.
    pub revocation_relay_url: Url,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_serializes_canonical_camel_case_urls() {
        let config = DeploymentConfig {
            account_service_url: "https://accounts.example/".parse().unwrap(),
            revocation_relay_url: "https://relay.example/revocations".parse().unwrap(),
        };
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["accountServiceUrl"], "https://accounts.example/");
        assert_eq!(
            value["revocationRelayUrl"],
            "https://relay.example/revocations"
        );
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

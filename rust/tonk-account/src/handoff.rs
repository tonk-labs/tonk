//! Shared contracts for one-time native account handoffs.

/// Request to create a pending native account handoff.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCreateRequest {
    /// Hash of the bearer secret bound into the completion invocation.
    pub token_hash: String,
    /// Native device DID that will receive the account delegation.
    pub device_did: String,
    /// Human-readable native device name shown before confirmation.
    pub device_name: String,
}

/// Bearer-secret request used to resolve or consume an account handoff.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkSecretRequest {
    /// Raw one-time bearer secret for the handoff.
    pub secret: String,
}

/// Pending native device metadata resolved for the browser ceremony.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLink {
    /// Hash bound into the root-signed completion invocation.
    pub token_hash: String,
    /// Native device DID that will receive the account delegation.
    pub device_did: String,
    /// Human-readable native device name shown before confirmation.
    pub device_name: String,
}

/// Root-signed browser ceremony result submitted to the account service.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteLinkCeremony {
    /// Hex-encoded signed account-link completion invocation.
    pub invocation_hex: String,
}

/// Provider-neutral local-root material returned by a completed handoff.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumedLink {
    /// Exact root-to-device delegation bytes, hex encoded.
    pub delegation_hex: String,
    /// Opaque credential identifier belonging to the root passkey.
    pub credential_id: String,
    /// Exact established account repository descriptor, hex encoded.
    pub descriptor_hex: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn it_round_trips_each_handoff_phase_in_camel_case() {
        let create = LinkCreateRequest {
            token_hash: "hash".to_string(),
            device_did: "did:key:device".to_string(),
            device_name: "terminal".to_string(),
        };
        let create_json = serde_json::to_value(&create).unwrap();
        assert_eq!(
            sorted_keys(&create_json),
            vec!["deviceDid", "deviceName", "tokenHash"]
        );
        assert_eq!(
            serde_json::from_value::<LinkCreateRequest>(create_json).unwrap(),
            create
        );

        let secret = LinkSecretRequest {
            secret: "secret".to_string(),
        };
        let secret_json = serde_json::to_value(&secret).unwrap();
        assert_eq!(sorted_keys(&secret_json), vec!["secret"]);
        assert_eq!(
            serde_json::from_value::<LinkSecretRequest>(secret_json).unwrap(),
            secret
        );

        let resolved = ResolvedLink {
            token_hash: "hash".to_string(),
            device_did: "did:key:device".to_string(),
            device_name: "terminal".to_string(),
        };
        let resolved_json = serde_json::to_value(&resolved).unwrap();
        assert_eq!(
            sorted_keys(&resolved_json),
            vec!["deviceDid", "deviceName", "tokenHash"]
        );
        assert_eq!(
            serde_json::from_value::<ResolvedLink>(resolved_json).unwrap(),
            resolved
        );

        let ceremony = CompleteLinkCeremony {
            invocation_hex: "invocation".to_string(),
        };
        let ceremony_json = serde_json::to_value(&ceremony).unwrap();
        assert_eq!(sorted_keys(&ceremony_json), vec!["invocationHex"]);
        assert_eq!(
            serde_json::from_value::<CompleteLinkCeremony>(ceremony_json).unwrap(),
            ceremony
        );

        let consumed = ConsumedLink {
            delegation_hex: "delegation".to_string(),
            credential_id: "credential".to_string(),
            descriptor_hex: "descriptor".to_string(),
        };
        let consumed_json = serde_json::to_value(&consumed).unwrap();
        assert_eq!(
            sorted_keys(&consumed_json),
            vec!["credentialId", "delegationHex", "descriptorHex"]
        );
        assert_eq!(
            serde_json::from_value::<ConsumedLink>(consumed_json).unwrap(),
            consumed
        );
    }
}

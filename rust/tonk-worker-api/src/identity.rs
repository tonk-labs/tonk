//! Provider-neutral local-root wire types.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Current local passkey-root state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RootStatus {
    /// This profile has no persisted local root grant.
    Missing {
        /// Current device profile DID.
        device_did: String,
    },
    /// A verified root → device grant is persisted.
    Ready {
        /// Root DID derived from the grant issuer.
        root_did: String,
        /// Current device profile DID.
        device_did: String,
        /// Opaque WebAuthn credential identifier.
        credential_id: String,
        /// CID of the stable root → device delegation.
        delegation_cid: String,
        /// Exact hex-encoded delegation bytes.
        delegation_hex: String,
    },
}

/// Persist a root ceremony result for the current profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRootRequest {
    /// Opaque WebAuthn credential identifier.
    pub credential_id: String,
    /// Exact hex-encoded root → device delegation bytes.
    pub delegation_hex: String,
}

/// Request to create a durable space through the local root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSpaceRequest {
    /// Space display name.
    pub name: String,
    /// Optional sync remote URL.
    pub remote: Option<String>,
    /// Optional immutable-artifact relay stored beside the remote.
    pub revocation_url: Option<String>,
    /// Optional template name.
    pub template: Option<String>,
}

/// Created space routing key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSpaceResponse {
    /// DID-derived repository routing key.
    pub key: String,
}

/// Deferred durable operation that requires a local root.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IdentityIntent {
    /// Create a durable space after provisioning identity.
    CreateSpace {
        /// Space display name.
        name: String,
        /// Optional sync remote.
        remote: Option<String>,
        /// Optional immutable-artifact relay stored beside the remote.
        revocation_url: Option<String>,
        /// Optional template name.
        template: Option<String>,
    },
    /// Turn an invite into durable membership.
    DurableJoin {
        /// Authority-bearing invite URL. Debug output always redacts it.
        url: String,
    },
}

impl fmt::Debug for IdentityIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateSpace {
                name,
                remote,
                revocation_url,
                template,
            } => formatter
                .debug_struct("CreateSpace")
                .field("name", name)
                .field("remote", remote)
                .field("revocation_url", revocation_url)
                .field("template", template)
                .finish(),
            Self::DurableJoin { .. } => formatter
                .debug_struct("DurableJoin")
                .field("url", &"<redacted>")
                .finish(),
        }
    }
}

/// Service-worker message asking the top document to provision identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRequired {
    /// Fixed message discriminator.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Operation to replay after provisioning.
    pub intent: IdentityIntent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_accepts_only_known_identity_intents() {
        assert!(
            serde_json::from_value::<IdentityIntent>(serde_json::json!({
                "kind": "createSpace",
                "name": "Notes",
                "remote": null,
                "revocationUrl": null,
                "template": null,
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<IdentityIntent>(serde_json::json!({
                "kind": "unknown"
            }))
            .is_err()
        );
    }

    #[dialog_common::test]
    fn it_omits_invite_urls_from_debug_output() {
        let secret = "https://tonk.spot/join#authority";
        let debug = format!("{:?}", IdentityIntent::DurableJoin { url: secret.into() });
        assert!(!debug.contains(secret));
        assert!(debug.contains("<redacted>"));
    }
}

//! Provider-neutral local-root wire types.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Informational metadata recorded when Tonk creates a passkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyMetadata {
    /// Browser-reported Unix time immediately after credential creation.
    pub created_at: u64,
    /// Browser and operating-system label where creation ran.
    pub created_on: String,
}

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
        /// Creation details when this Tonk client created the passkey.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passkey: Option<PasskeyMetadata>,
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
    /// Creation details when this request follows passkey creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<PasskeyMetadata>,
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

/// Deferred durable operation that requires an account.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PendingIntent {
    /// Create a durable space once an account exists.
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

impl fmt::Debug for PendingIntent {
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

/// Service-worker message asking the top document to sign the user in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRequired {
    /// Fixed message discriminator.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Operation to replay once an account exists.
    pub intent: PendingIntent,
}

/// The `type` every [`AccountRequired`] message carries.
pub const ACCOUNT_REQUIRED: &str = "account-required";

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_accepts_only_known_pending_intents() {
        assert!(
            serde_json::from_value::<PendingIntent>(serde_json::json!({
                "kind": "createSpace",
                "name": "Notes",
                "remote": null,
                "revocationUrl": null,
                "template": null,
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PendingIntent>(serde_json::json!({
                "kind": "unknown"
            }))
            .is_err()
        );
    }

    #[dialog_common::test]
    fn it_omits_invite_urls_from_debug_output() {
        let secret = "https://tonk.network/join#authority";
        let debug = format!("{:?}", PendingIntent::DurableJoin { url: secret.into() });
        assert!(!debug.contains(secret));
        assert!(debug.contains("<redacted>"));
    }

    /// The page routes on this discriminator, so it is part of the contract
    /// between the service worker and the top document, not a local string.
    #[dialog_common::test]
    fn it_names_the_account_required_message() {
        let message = AccountRequired {
            message_type: ACCOUNT_REQUIRED.to_string(),
            intent: PendingIntent::DurableJoin {
                url: "https://tonk.network/join#authority".into(),
            },
        };
        let value = serde_json::to_value(&message).expect("serializes");
        assert_eq!(value["type"], "account-required");
    }
}

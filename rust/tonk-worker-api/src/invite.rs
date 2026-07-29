//! Browser invitation request and response contracts.

use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use url::Url;

/// Request to mint an open or root-targeted invitation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInviteRequest {
    /// Link base. Browser callers may omit it to use their request origin.
    #[serde(default, alias = "base_url", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Url>,
    /// Recipient root for a targeted invitation; absent means open.
    #[serde(
        default,
        alias = "recipient_root",
        skip_serializing_if = "Option::is_none"
    )]
    pub recipient_root: Option<Did>,
}

/// Minted invitation URL and its audience mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreateInviteResponse {
    /// Only the named root can claim the invitation.
    Scoped {
        /// Public invitation URL.
        url: Url,
        /// Root authorized to claim it.
        #[serde(rename = "recipientRoot", alias = "recipient_root")]
        recipient_root: Did,
    },
    /// Anyone holding the URL can visit or claim it.
    Open {
        /// Public invitation URL.
        url: Url,
    },
}

/// Audience mode stored for a durable invitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationKind {
    /// Anyone holding the URL may claim.
    Open,
    /// Only one named root may claim.
    Scoped,
    /// Legacy invitation without execution metadata.
    Unknown,
}

impl InvitationKind {
    /// Stable stored and wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Scoped => "scoped",
            Self::Unknown => "unknown",
        }
    }
}

/// Safe list projection for a recorded invitation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationSummary {
    /// Exact revocation target CID.
    pub target_cid: String,
    /// Open or root-targeted invitation.
    pub kind: InvitationKind,
    /// Recipient root for a scoped invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_root: Option<Did>,
    /// Local display state; canonical enforcement remains relay-backed.
    pub status: String,
}

/// Canonical relay acknowledgement for an invitation revocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeInvitationAcknowledgement {
    /// Delegation CID named by the revocation.
    pub target_cid: String,
    /// Content CID of the signed revocation artifact.
    pub artifact_cid: String,
    /// Whether canonical storage accepted the artifact.
    #[serde(default = "published_by_success")]
    pub published: bool,
    /// Whether this request created the immutable object.
    pub stored: bool,
}

fn published_by_success() -> bool {
    true
}

impl CreateInviteResponse {
    /// The minted invitation URL, independent of audience mode.
    pub fn url(&self) -> &Url {
        match self {
            Self::Scoped { url, .. } | Self::Open { url } => url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Did {
        "did:key:z6Mktest".parse().unwrap()
    }

    #[test]
    fn it_accepts_canonical_fields_and_documented_aliases() {
        for json in [
            r#"{"baseUrl":"https://local.example/join","recipientRoot":"did:key:z6Mktest"}"#,
            r#"{"base_url":"https://local.example/join","recipient_root":"did:key:z6Mktest"}"#,
        ] {
            let request: CreateInviteRequest = serde_json::from_str(json).unwrap();
            assert_eq!(
                request.base_url.unwrap().as_str(),
                "https://local.example/join"
            );
            assert_eq!(request.recipient_root, Some(root()));
        }
    }

    #[test]
    fn it_rejects_typos_unknown_fields_and_malformed_values() {
        for json in [
            r#"{"baseURL":"https://local.example/join"}"#,
            r#"{"recipientRot":"did:key:z6Mktest"}"#,
            r#"{"baseUrl":"not absolute"}"#,
            r#"{"recipientRoot":"not a did"}"#,
        ] {
            assert!(
                serde_json::from_str::<CreateInviteRequest>(json).is_err(),
                "{json}"
            );
        }
    }

    #[test]
    fn it_serializes_response_fields_in_camel_case_and_reads_the_alias() {
        let response = CreateInviteResponse::Scoped {
            url: "https://local.example/join?invite=x".parse().unwrap(),
            recipient_root: root(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("recipientRoot"));
        assert!(!json.contains("recipient_root"));

        let legacy = json.replace("recipientRoot", "recipient_root");
        assert_eq!(
            serde_json::from_str::<CreateInviteResponse>(&legacy).unwrap(),
            response
        );
    }

    #[test]
    fn it_keeps_invitation_lists_secret_free() {
        let summary = InvitationSummary {
            target_cid: "bafycid".into(),
            kind: InvitationKind::Scoped,
            recipient_root: Some(root()),
            status: "active".into(),
        };
        let json = serde_json::to_value(summary).unwrap();
        assert_eq!(json["targetCid"], "bafycid");
        assert_eq!(json["recipientRoot"], "did:key:z6Mktest");
        assert!(json.get("url").is_none());
        assert!(json.get("pathHex").is_none());
    }
}

//! Local-root and optional account-provider wire DTOs.

use serde::{Deserialize, Serialize};

use crate::PasskeyMetadata;

/// Verified facts shown on the linked account dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    /// Email address verified when the account was created. Absent only when
    /// the account service could not be reached: the address is service-owned
    /// and is never mirrored into the account repository, because it is the
    /// uniqueness key and the enumeration boundary.
    pub email: Option<String>,
    /// Facts Tonk recorded during passkey creation, absent for legacy roots.
    pub passkey: Option<PasskeyMetadata>,
    /// The chosen account display name, when one was ever set. Absent
    /// for an account nobody has named — which is how a sign-up is told
    /// apart from a sign-in to an already-named account, so the name is
    /// asked for once per account rather than once per device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// One hosted space associated with an account deletion review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionSpace {
    /// Repository subject to be permanently purged from Tonk services.
    pub subject: String,
    /// Display name from the account directory, when recorded.
    pub name: Option<String>,
    /// When the access service began purging this space, if it has. A
    /// finished deletion leaves no record, so such a space is absent
    /// from the plan rather than listed as already gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleting_since: Option<u64>,
}

/// Reviewable destructive scope loaded before asking for a passkey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionPlan {
    /// Account root the finalization ceremony must sign for.
    pub root_did: String,
    /// Verified email the user must type exactly.
    pub email: String,
    /// Hosted spaces originally provided by this account.
    pub spaces: Vec<AccountDeletionSpace>,
    /// Directory spaces absent from the owned service inventory; these
    /// are joined spaces and are not deleted.
    pub joined_spaces: usize,
}

/// One reviewed hosted-space deprovision. The worker signs the
/// `/provider/remove` invocation itself — deletion is the account
/// ending its hosting relationship, and any linked device holds that
/// authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpaceDeletionRequest {
    /// Reviewed repository subject.
    pub subject: String,
}

/// The reviewed destructive scope. The worker signs every deletion
/// invocation itself with the device's delegated authority; the UI's
/// passkey assertion is a user-verification gate, not a signing key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionRequest {
    /// Every reviewed owned hosted space, deprovisioned by the worker.
    pub spaces: Vec<AccountSpaceDeletionRequest>,
    /// The account email the person retyped to confirm the deletion.
    pub confirmed_email: String,
}

/// Completed service-account deletion result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionResult {
    /// Number of owned hosted spaces whose purge was confirmed.
    pub deleted_spaces: usize,
    /// Joined spaces deliberately left intact locally and on their owners' services.
    pub retained_joined_spaces: usize,
}

/// Receipt for deleting one owned hosted space without deleting its account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSpaceDeletionResult {
    /// Repository subject removed from Tonk services.
    pub subject: String,
}

/// Attach provider services to an already persisted local root, naming the
/// account repository this root owns.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLinkRequest {
    /// Provider base URL.
    pub provider: String,
    /// Root DID returned by the provider ceremony.
    pub root_did: String,
    /// Opaque credential ID already stored with the local root.
    pub credential_id: String,
    /// Exact existing root → device grant bytes.
    pub delegation_hex: String,
    /// Where the account syncs: the access service's `/ucan/` address.
    ///
    /// Named by whoever links, because they are the party talking to
    /// the service. It replaces a root-signed descriptor whose remote
    /// was the linking browser's own origin, frozen at signup — the
    /// same value, without the signature that made a guess permanent.
    pub remote: String,
    /// Seed the current profile name only for a new-account creation winner.
    #[serde(default)]
    pub initialize_name: bool,
}

/// Local identity and provider attachment state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AccountStatus {
    /// No local passkey root is available.
    RootMissing {
        /// Current device DID.
        device_did: String,
    },
    /// A local root exists without provider services.
    Unregistered {
        /// Local root DID.
        root_did: String,
        /// Current device DID.
        device_did: String,
    },
    /// Provider services are attached to the local root.
    Registered {
        /// Local root DID.
        root_did: String,
        /// Current device DID.
        device_did: String,
        /// Attached provider base URL.
        provider: String,
        /// Configuration/hydration state of the account repository.
        account_state: tonk_account::AccountStateStatus,
    },
}

/// Request to change the authoritative account display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDisplayNameRequest {
    /// New non-blank display name.
    pub name: String,
}

/// Result of an authoritative account display-name write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDisplayNameResponse {
    /// Name committed to the account repository.
    pub name: String,
}

/// One device authorized under this profile's account, read from the
/// account space's own facts.
///
/// Deliberately carries no "this device" flag: the rows are a projection
/// of shared facts and are identical on every device. Which row is the
/// caller is presentation, answered by asking who the caller is and
/// comparing DIDs — the way an active tab is marked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDevice {
    /// The device's DID.
    pub did: String,
    /// Display name described at link time.
    pub name: String,
    /// Link time, seconds since the epoch.
    pub created_at: u64,
}

/// Revoke one device authorized under this profile's account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// DID of the device to revoke.
    pub did: String,
}

/// Canonical acknowledgement returned after revoking an account device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceAcknowledgement {
    /// DID whose grant was revoked.
    pub target_did: String,
    /// CID of the revoked root-to-device delegation.
    pub target_cid: String,
    /// Whether the immutable revocation was accepted by canonical storage.
    pub published: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serializes_account_summary_passkey_facts_in_camel_case() {
        let json = serde_json::to_value(AccountSummary {
            email: Some("person@example.com".into()),
            passkey: Some(PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".into(),
            }),
            display_name: None,
        })
        .unwrap();
        assert_eq!(json["email"], "person@example.com");
        assert_eq!(json["passkey"]["createdAt"], 1_754_380_800_u64);
        assert_eq!(json["passkey"]["createdOn"], "Chrome on macOS");
    }

    #[dialog_common::test]
    fn it_serves_passkey_facts_without_a_reachable_account_service() {
        let json = serde_json::to_value(AccountSummary {
            email: None,
            passkey: Some(PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".into(),
            }),
            display_name: None,
        })
        .unwrap();
        assert!(json["email"].is_null());
        assert_eq!(json["passkey"]["createdOn"], "Chrome on macOS");
    }

    #[dialog_common::test]
    fn it_serializes_repository_setup_requests_in_camel_case() {
        let link = serde_json::to_value(AccountLinkRequest {
            provider: "https://accounts.example".into(),
            root_did: "did:key:root".into(),
            credential_id: "cred".into(),
            delegation_hex: "aa".into(),
            remote: "https://accounts.example/ucan/".into(),
            initialize_name: true,
        })
        .unwrap();
        assert_eq!(link["credentialId"], "cred");
        assert_eq!(link["delegationHex"], "aa");
        assert_eq!(link["remote"], "https://accounts.example/ucan/");
        assert_eq!(link["initializeName"], true);
    }

    #[dialog_common::test]
    fn it_serializes_account_status_in_camel_case() {
        let json = serde_json::to_value(AccountStatus::Registered {
            root_did: "did:key:root".into(),
            device_did: "did:key:device".into(),
            provider: "https://accounts.example".into(),
            account_state: tonk_account::AccountStateStatus::Unhydrated,
        })
        .unwrap();
        assert_eq!(json["status"], "registered");
        assert_eq!(json["rootDid"], "did:key:root");
        assert_eq!(json["deviceDid"], "did:key:device");
        assert_eq!(json["accountState"], "unhydrated");
        assert!(json.get("root_did").is_none());
    }

    #[dialog_common::test]
    fn it_serializes_display_name_results_in_camel_case() {
        let value = serde_json::to_value(AccountDisplayNameResponse {
            name: "Alice".into(),
        })
        .unwrap();
        assert_eq!(value["name"], "Alice");
        assert!(value.get("convergence").is_none());
    }

    #[dialog_common::test]
    fn it_serializes_account_devices_in_camel_case() {
        let json = serde_json::to_value(AccountDevice {
            did: "did:key:device".into(),
            name: "laptop".into(),
            created_at: 1_753_300_000,
        })
        .unwrap();
        assert_eq!(json["did"], "did:key:device");
        assert_eq!(json["name"], "laptop");
        assert_eq!(json["createdAt"], 1_753_300_000u64);
    }

    #[dialog_common::test]
    fn it_serializes_a_canonical_revocation_acknowledgement() {
        let json = serde_json::to_value(RevokeDeviceAcknowledgement {
            target_did: "did:key:device".into(),
            target_cid: "bafycid".into(),
            published: true,
        })
        .unwrap();
        assert_eq!(json["targetDid"], "did:key:device");
        assert_eq!(json["targetCid"], "bafycid");
        assert_eq!(json["published"], true);
    }
}

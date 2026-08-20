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
}

/// One hosted space associated with an account deletion review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionSpace {
    /// Repository subject to be permanently purged from Tonk services.
    pub subject: String,
    /// Display name from the account directory, when recorded.
    pub name: Option<String>,
    /// Access-service lifecycle state.
    pub state: String,
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
    /// Hex-encoded root-signed account repository descriptor.
    pub descriptor_hex: String,
    /// Seed the current profile name only for a new-account creation winner.
    #[serde(default)]
    pub initialize_name: bool,
    /// Default content access remote for local-only spaces, when deployment
    /// discovery supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_remote: Option<String>,
    /// Default invitation-revocation relay paired with `access_remote`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_relay: Option<String>,
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

/// Durable projections changed by account-state convergence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConvergenceReport {
    /// Whether the device-local profile name cache changed.
    pub profile_changed: bool,
    /// Real-space routing keys whose durable roster changed.
    pub changed_keys: Vec<String>,
    /// Real-space routing keys that could not be checked or updated.
    pub failed_keys: Vec<String>,
}

/// Result of an authoritative account display-name write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDisplayNameResponse {
    /// Name committed to the account repository.
    pub name: String,
    /// Idempotent projection work completed after the account commit.
    pub convergence: AccountConvergenceReport,
}

/// Canonical account membership state for one repository subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSpaceMembership {
    /// The account currently discovers this subject.
    Active,
    /// A monotonic account-owned archive marker hides this subject.
    Archived,
}

/// Whether one account space is visible on this browser profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSpaceVisibility {
    /// Not suppressed on this profile.
    Visible,
    /// Explicitly removed from this device/profile.
    HiddenOnThisDevice,
}

/// Durable browser enrollment phase for one locally known space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSpaceEnrollment {
    /// No automatic remote enrollment has begun.
    #[default]
    LocalOnly,
    /// Remote provisioning is in progress or remains retryable.
    Provisioning,
    /// A remote exists but the exact content tree is not yet confirmed.
    PendingPush,
    /// Content, canonical membership, and saved access all converged.
    Connected,
    /// The last bounded enrollment pass failed at a named step.
    Error,
}

/// One browser account-space inventory row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpaceRow {
    /// Wire schema version; exactly 1.
    pub version: u8,
    /// Immutable repository subject DID.
    pub subject: String,
    /// Account-facing display name when known.
    pub name: Option<String>,
    /// Signed account-repository membership state.
    pub membership: AccountSpaceMembership,
    /// Whether a local replica is mounted in this profile.
    pub local: bool,
    /// Device-local discovery visibility.
    pub visibility: AccountSpaceVisibility,
    /// Configured content remote when known.
    pub remote_url: Option<String>,
    /// Exact tree last accepted by that content remote.
    pub confirmed_revision: Option<String>,
    /// Whether explicit download has unambiguous saved access.
    pub pullable: bool,
    /// Last durable enrollment phase recorded on this browser profile.
    #[serde(default)]
    pub enrollment: AccountSpaceEnrollment,
    /// Last enrollment error, present only for `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment_error: Option<String>,
}

/// Result of explicitly downloading one account space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpaceDownloadResponse {
    /// Subject mounted locally.
    pub subject: String,
    /// Whether this profile now has a local replica.
    pub local: bool,
}

/// Result of writing an account-owned archive marker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpaceArchiveResponse {
    /// Subject archived in the canonical account repository.
    pub subject: String,
    /// Whether this request committed a new marker.
    pub newly_archived: bool,
    /// Best-effort provider-projection warning, if canonical commit succeeded.
    pub warning: Option<String>,
}

/// One device registered under the attached provider account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDevice {
    /// Exact attachment generation.
    pub attachment_id: String,
    /// The device's DID.
    pub did: String,
    /// Display name registered at link time.
    pub name: String,
    /// Registry status: `active` or `revoked`.
    pub status: String,
    /// Registration time, seconds since the epoch.
    pub created_at: u64,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Public path bytes needed to witness a revocation. Absent for devices
    /// registered before providers retained this evidence.
    pub delegation_hex: Option<String>,
    /// Whether this row is the profile making the request.
    pub this_device: bool,
}

/// Revoke one device under the attached provider account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// Exact attachment generation selected by the user.
    pub attachment_id: String,
    /// DID of the device to revoke.
    pub did: String,
    /// Hex-encoded signed revocation artifact.
    pub revocation: String,
}

/// Whether the account service's device-list projection caught up with a
/// published revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RevocationProjection {
    /// The mutable device row now reflects the revocation.
    Updated,
    /// The immutable revocation was published, but the device row is stale.
    Stale,
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
    /// Best-effort state of the account-service device-list projection.
    pub projection: RevocationProjection,
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
            descriptor_hex: "bb".into(),
            initialize_name: true,
            access_remote: Some("https://sync.example/ucan/".into()),
            revocation_relay: Some("https://relay.example/revocations/".into()),
        })
        .unwrap();
        assert_eq!(link["credentialId"], "cred");
        assert_eq!(link["delegationHex"], "aa");
        assert_eq!(link["descriptorHex"], "bb");
        assert_eq!(link["initializeName"], true);
        assert_eq!(link["accessRemote"], "https://sync.example/ucan/");
        assert_eq!(
            link["revocationRelay"],
            "https://relay.example/revocations/"
        );

        let legacy: AccountLinkRequest = serde_json::from_value(serde_json::json!({
            "provider": "https://accounts.example",
            "rootDid": "did:key:root",
            "credentialId": "cred",
            "delegationHex": "aa",
            "descriptorHex": "bb",
            "initializeName": false
        }))
        .unwrap();
        assert_eq!(legacy.access_remote, None);
        assert_eq!(legacy.revocation_relay, None);
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
            convergence: AccountConvergenceReport {
                profile_changed: true,
                changed_keys: vec!["one".into()],
                failed_keys: vec!["two".into()],
            },
        })
        .unwrap();
        assert_eq!(value["name"], "Alice");
        assert_eq!(value["convergence"]["profileChanged"], true);
        assert_eq!(value["convergence"]["changedKeys"][0], "one");
        assert_eq!(value["convergence"]["failedKeys"][0], "two");
    }

    #[dialog_common::test]
    fn it_serializes_account_space_lifecycle_without_collapsing_visibility() {
        let value = serde_json::to_value(AccountSpaceRow {
            version: 1,
            subject: "did:key:space".into(),
            name: Some("garden".into()),
            membership: AccountSpaceMembership::Active,
            local: false,
            visibility: AccountSpaceVisibility::HiddenOnThisDevice,
            remote_url: Some("https://sync.example/ucan/".into()),
            confirmed_revision: Some("#tree".into()),
            pullable: true,
            enrollment: AccountSpaceEnrollment::PendingPush,
            enrollment_error: None,
        })
        .unwrap();
        assert_eq!(value["membership"], "active");
        assert_eq!(value["visibility"], "hiddenOnThisDevice");
        assert_eq!(value["confirmedRevision"], "#tree");
        assert_eq!(value["enrollment"], "pendingPush");
        assert!(value.get("confirmed_revision").is_none());

        let legacy: AccountSpaceRow = serde_json::from_value(serde_json::json!({
            "version": 1,
            "subject": "did:key:legacy",
            "name": null,
            "membership": "active",
            "local": false,
            "visibility": "visible",
            "remoteUrl": null,
            "confirmedRevision": null,
            "pullable": false
        }))
        .unwrap();
        assert_eq!(legacy.enrollment, AccountSpaceEnrollment::LocalOnly);
        assert_eq!(legacy.enrollment_error, None);
    }

    #[dialog_common::test]
    fn it_serializes_account_devices_in_camel_case() {
        let json = serde_json::to_value(AccountDevice {
            attachment_id: "generation".into(),
            did: "did:key:device".into(),
            name: "laptop".into(),
            status: "active".into(),
            created_at: 1_753_300_000,
            delegation_cid: "bafycid".into(),
            delegation_hex: Some("beef".into()),
            this_device: true,
        })
        .unwrap();
        assert_eq!(json["attachmentId"], "generation");
        assert_eq!(json["createdAt"], 1_753_300_000u64);
        assert_eq!(json["thisDevice"], true);
        assert_eq!(json["delegationCid"], "bafycid");
        assert_eq!(json["delegationHex"], "beef");
    }

    #[dialog_common::test]
    fn it_represents_legacy_device_path_evidence_as_absent() {
        let json = serde_json::to_value(AccountDevice {
            attachment_id: "legacy-generation".into(),
            did: "did:key:legacy".into(),
            name: "old laptop".into(),
            status: "active".into(),
            created_at: 1_753_300_000,
            delegation_cid: "bafycid".into(),
            delegation_hex: None,
            this_device: false,
        })
        .unwrap();
        assert!(json["delegationHex"].is_null());
    }

    #[dialog_common::test]
    fn it_serializes_a_canonical_revocation_acknowledgement() {
        let json = serde_json::to_value(RevokeDeviceAcknowledgement {
            target_did: "did:key:device".into(),
            target_cid: "bafycid".into(),
            published: true,
            projection: RevocationProjection::Stale,
        })
        .unwrap();
        assert_eq!(json["targetDid"], "did:key:device");
        assert_eq!(json["targetCid"], "bafycid");
        assert_eq!(json["published"], true);
        assert_eq!(json["projection"], "stale");
    }
}

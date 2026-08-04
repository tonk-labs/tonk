//! Local-root and optional account-provider wire DTOs.

use serde::{Deserialize, Serialize};

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
}

/// Persist the service-selected descriptor for a legacy account link.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRepositoryEstablishRequest {
    /// Exact descriptor winner returned by the account service.
    pub descriptor_hex: String,
    /// Whether this ceremony created the service-side descriptor winner.
    pub created: bool,
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
    fn it_serializes_repository_setup_requests_in_camel_case() {
        let link = serde_json::to_value(AccountLinkRequest {
            provider: "https://accounts.example".into(),
            root_did: "did:key:root".into(),
            credential_id: "cred".into(),
            delegation_hex: "aa".into(),
            descriptor_hex: "bb".into(),
            initialize_name: true,
        })
        .unwrap();
        assert_eq!(link["credentialId"], "cred");
        assert_eq!(link["delegationHex"], "aa");
        assert_eq!(link["descriptorHex"], "bb");
        assert_eq!(link["initializeName"], true);

        let establish = serde_json::to_value(AccountRepositoryEstablishRequest {
            descriptor_hex: "cc".into(),
            created: false,
        })
        .unwrap();
        assert_eq!(establish["descriptorHex"], "cc");
        assert_eq!(establish["created"], false);
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

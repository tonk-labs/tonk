//! Local-root and optional account-provider wire DTOs.

use serde::{Deserialize, Serialize};

/// Attach provider services to an already persisted local root.
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
    },
}

/// One device registered under the attached provider account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDevice {
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
    /// Public path bytes needed to witness a revocation.
    pub delegation_hex: String,
    /// Whether this row is the profile making the request.
    pub this_device: bool,
}

/// Revoke one device under the attached provider account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// DID of the device to revoke.
    pub did: String,
    /// Hex-encoded signed revocation artifact.
    pub revocation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serializes_account_status_in_camel_case() {
        let json = serde_json::to_value(AccountStatus::Registered {
            root_did: "did:key:root".into(),
            device_did: "did:key:device".into(),
            provider: "https://accounts.example".into(),
        })
        .unwrap();
        assert_eq!(json["status"], "registered");
        assert_eq!(json["rootDid"], "did:key:root");
        assert_eq!(json["deviceDid"], "did:key:device");
    }

    #[dialog_common::test]
    fn it_serializes_account_devices_in_camel_case() {
        let json = serde_json::to_value(AccountDevice {
            did: "did:key:device".into(),
            name: "laptop".into(),
            status: "active".into(),
            created_at: 1_753_300_000,
            delegation_cid: "bafycid".into(),
            delegation_hex: "beef".into(),
            this_device: true,
        })
        .unwrap();
        assert_eq!(json["createdAt"], 1_753_300_000u64);
        assert_eq!(json["thisDevice"], true);
        assert_eq!(json["delegationCid"], "bafycid");
        assert_eq!(json["delegationHex"], "beef");
    }
}

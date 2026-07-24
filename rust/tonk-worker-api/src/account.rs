//! Account-link wire DTOs.

use serde::{Deserialize, Serialize};

/// Persist a verified `root → current profile` delegation locally.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLinkRequest {
    /// Root DID claimed by the account ceremony.
    pub root_did: String,
    /// Hex-encoded UCAN delegation chain from the root to this profile.
    pub delegation_hex: String,
}

/// Local account-link state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AccountStatus {
    /// This profile has not been linked to an account root.
    Unlinked {
        /// Current local profile DID.
        device_did: String,
    },
    /// This profile holds a verified delegation from an account root.
    Linked {
        /// Account root DID.
        root_did: String,
        /// Current local profile DID.
        device_did: String,
    },
}

/// Result of signing out on this device.
///
/// Sign-out is two acts: telling the registry this device is out, and
/// rotating the local key. The rotation always happens; the registry
/// call is best-effort, and this response is where its failure
/// surfaces instead of vanishing into a console log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutResponse {
    /// DID of the replacement profile this browser now signs as.
    pub device_did: String,
    /// Whether the registry recorded this device's self-revocation.
    /// `false` when there was nothing to record — never linked, or no
    /// account service for this deployment — as well as on failure.
    pub revoked: bool,
    /// Why the registry was not told, when it should have been. The
    /// device is signed out locally either way; a caller showing this
    /// should tell the user to revoke from another device.
    pub warning: Option<String>,
}

/// One device registered under the linked account, as returned by the
/// worker's device-list proxy.
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
    /// CID of the `root → device` delegation, which a revocation must
    /// name. Carried to the browser so it can sign one.
    pub delegation_cid: String,
    /// Whether this row is the profile making the request.
    pub this_device: bool,
}

/// Revoke one device under the linked account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// DID of the device to revoke.
    pub did: String,
    /// Hex-encoded root-signed revocation of that device's delegation.
    ///
    /// Required: cutting off a device other than the one in your hand
    /// takes the account root, and the root lives behind the passkey, so
    /// the browser must run the ceremony before calling this.
    pub revocation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_serializes_account_status_in_camel_case() {
        let json = serde_json::to_value(AccountStatus::Linked {
            root_did: "did:key:root".into(),
            device_did: "did:key:device".into(),
        })
        .unwrap();
        assert_eq!(json["status"], "linked");
        assert_eq!(json["rootDid"], "did:key:root");
        assert_eq!(json["deviceDid"], "did:key:device");
        assert!(json.get("root_did").is_none());
    }

    #[dialog_common::test]
    fn it_serializes_account_devices_in_camel_case() {
        let json = serde_json::to_value(AccountDevice {
            did: "did:key:device".into(),
            name: "laptop".into(),
            status: "active".into(),
            created_at: 1_753_300_000,
            delegation_cid: "bafycid".into(),
            this_device: true,
        })
        .unwrap();
        assert_eq!(json["did"], "did:key:device");
        assert_eq!(json["createdAt"], 1_753_300_000);
        assert_eq!(json["thisDevice"], true);
        assert_eq!(json["delegationCid"], "bafycid");
        assert!(json.get("created_at").is_none());
        assert!(json.get("delegation_cid").is_none());

        let request: RevokeDeviceRequest = serde_json::from_value(
            serde_json::json!({ "did": "did:key:device", "revocation": "beef" }),
        )
        .unwrap();
        assert_eq!(request.did, "did:key:device");
        assert_eq!(request.revocation, "beef");
    }

    #[dialog_common::test]
    fn it_serializes_the_sign_out_warning_in_camel_case() {
        let json = serde_json::to_value(SignOutResponse {
            device_did: "did:key:fresh".into(),
            revoked: false,
            warning: Some("service unreachable".into()),
        })
        .unwrap();
        assert_eq!(json["deviceDid"], "did:key:fresh");
        assert_eq!(json["revoked"], false);
        assert_eq!(json["warning"], "service unreachable");
    }
}

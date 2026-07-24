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
    /// Hex-encoded succession delegation authorizing replacement of an
    /// already-linked root with `root_did`, when this link is a rotation.
    #[serde(default)]
    pub succession_hex: Option<String>,
}

/// Recover an account onto a fresh root under a surviving device's
/// authority, replacing the account's current root registration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecoverRequest {
    /// The root DID the account recovers onto.
    pub new_root_did: String,
    /// The credential id backing the new root's passkey.
    pub new_credential_id: String,
    /// Hex-encoded new-root-signed confirmation container, proving control
    /// of `new_root_did`.
    pub confirmation_hex: String,
    /// Hex-encoded `newRoot → device` delegation the service installs on
    /// this device once recovery succeeds.
    pub device_delegation_hex: String,
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
    /// Whether this row is the profile making the request.
    pub this_device: bool,
}

/// Revoke one device under the linked account.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceRequest {
    /// DID of the device to revoke.
    pub did: String,
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
            this_device: true,
        })
        .unwrap();
        assert_eq!(json["did"], "did:key:device");
        assert_eq!(json["createdAt"], 1_753_300_000);
        assert_eq!(json["thisDevice"], true);
        assert!(json.get("created_at").is_none());

        let request: RevokeDeviceRequest =
            serde_json::from_value(serde_json::json!({ "did": "did:key:device" })).unwrap();
        assert_eq!(request.did, "did:key:device");
    }

    #[dialog_common::test]
    fn it_serializes_account_recover_request_in_camel_case() {
        let json = serde_json::to_value(AccountRecoverRequest {
            new_root_did: "did:key:new-root".into(),
            new_credential_id: "cred-new".into(),
            confirmation_hex: "deadbeef".into(),
            device_delegation_hex: "beefdead".into(),
        })
        .unwrap();
        assert_eq!(json["newRootDid"], "did:key:new-root");
        assert_eq!(json["newCredentialId"], "cred-new");
        assert_eq!(json["confirmationHex"], "deadbeef");
        assert_eq!(json["deviceDelegationHex"], "beefdead");

        let request: AccountRecoverRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.new_root_did, "did:key:new-root");
        assert_eq!(request.device_delegation_hex, "beefdead");
    }
}

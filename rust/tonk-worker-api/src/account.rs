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
}

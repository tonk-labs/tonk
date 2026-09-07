//! Local account-provider attachment record.
//!
//! One device's answer to "which provider is attached, and which account
//! repository does this root own". Both the browser worker and the CLI store
//! this at [`ACCOUNT_PROVIDER_CREDENTIAL_SITE`] in their own credential store.
//!
//! The type exists so the descriptor cannot be stored unbound. A descriptor is
//! self-signed, which proves it is well-formed and names *some* account — not
//! that it names *this* one. Every constructor therefore takes the local root
//! DID and rejects a descriptor whose account subject is anyone else's, so no
//! call site can persist a descriptor that would silently repoint this device's
//! account state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DescriptorError;

const RECORD_VERSION: u8 = 1;

/// Wire form. Kept JSON and field-compatible with records written when the
/// account service and the sync remote were separate addresses: `provider`
/// carried the service, `remote` the `/ucan/` endpoint. The service is
/// decommissioned, so both slots now carry the one address the account
/// syncs with — written to both so a record survives a rollback to a
/// reader that still requires `provider`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Wire {
    version: u8,
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attached_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
}

/// A provider attachment: the one address this account syncs with, and
/// when it attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountProviderRecord {
    address: String,
    attached_at: Option<u64>,
}

impl AccountProviderRecord {
    /// Attach the provider serving this account at `address` — the access
    /// service's `/ucan/` endpoint the account repository syncs through.
    pub fn attach(address: &str, attached_at: u64) -> Result<Self, AccountProviderError> {
        let address = address.trim();
        if address.is_empty() {
            return Err(AccountProviderError::EmptyProvider);
        }
        Ok(Self {
            address: address.to_owned(),
            attached_at: Some(attached_at),
        })
    }

    /// Decode a stored credential value. Empty bytes are the detach
    /// tombstone: the credential store has no delete, so unlinking writes
    /// an empty value.
    ///
    /// A record from the two-address era resolves to its `remote` — where
    /// the account actually syncs — and only a record that never named one
    /// falls back to the retired service address it carried.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, AccountProviderError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let wire: Wire = serde_json::from_slice(bytes)
            .map_err(|error| AccountProviderError::Encoding(error.to_string()))?;
        if wire.version != RECORD_VERSION {
            return Err(AccountProviderError::UnsupportedVersion(wire.version));
        }
        let address = wire
            .remote
            .map(|remote| remote.trim().to_owned())
            .filter(|remote| !remote.is_empty())
            .unwrap_or(wire.provider);
        Ok(Some(Self {
            address,
            attached_at: wire.attached_at,
        }))
    }

    /// Serialize for the credential store.
    pub fn encode(&self) -> Result<Vec<u8>, AccountProviderError> {
        serde_json::to_vec(&Wire {
            version: RECORD_VERSION,
            provider: self.address.clone(),
            attached_at: self.attached_at,
            remote: Some(self.address.clone()),
        })
        .map_err(|error| AccountProviderError::Encoding(error.to_string()))
    }

    /// The address this account syncs with.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// When the provider was attached, for records that recorded it.
    pub fn attached_at(&self) -> Option<u64> {
        self.attached_at
    }
}

/// Local provider-attachment validation failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccountProviderError {
    /// JSON encoding or decoding failed.
    #[error("invalid account-provider encoding: {0}")]
    Encoding(String),
    /// The record names an unsupported version.
    #[error("unsupported account-provider version {0}")]
    UnsupportedVersion(u8),
    /// The provider URL is blank.
    #[error("provider must not be empty")]
    EmptyProvider,
    /// Descriptor validation failed.
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
    /// The descriptor names an account other than this device's root.
    #[error("account repository descriptor names another account root")]
    DescriptorSubject,
    /// A descriptor is already established and may not be replaced.
    #[error("account repository is already configured")]
    DescriptorEstablished,
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;

    const REMOTE: &str = "https://accounts.example/ucan/";

    #[dialog_common::test]
    fn it_round_trips_an_attachment() {
        let record = AccountProviderRecord::attach(REMOTE, 42).unwrap();
        let decoded = AccountProviderRecord::decode(&record.encode().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(decoded.address(), REMOTE);
        assert_eq!(decoded.attached_at(), Some(42));
    }

    #[dialog_common::test]
    fn it_rejects_a_blank_address() {
        assert_eq!(
            AccountProviderRecord::attach("   ", 1).unwrap_err(),
            AccountProviderError::EmptyProvider
        );
    }

    /// A two-address-era record resolves to its remote — where the account
    /// actually syncs — and extra fields it carried are ignored.
    #[dialog_common::test]
    fn it_reads_a_two_address_record_as_its_remote() {
        let legacy = br#"{"version":1,"provider":"https://accounts.example","attached_at":9,"remote":"https://accounts.example/ucan/","descriptor":[1,2,3]}"#;
        let decoded = AccountProviderRecord::decode(legacy).unwrap().unwrap();
        assert_eq!(decoded.address(), REMOTE);
        assert_eq!(decoded.attached_at(), Some(9));
    }

    /// A record that never named a remote falls back to the service
    /// address it carried.
    #[dialog_common::test]
    fn it_falls_back_to_the_service_address_when_no_remote_was_named() {
        let legacy = br#"{"version":1,"provider":"https://accounts.example","attached_at":9}"#;
        let decoded = AccountProviderRecord::decode(legacy).unwrap().unwrap();
        assert_eq!(decoded.address(), "https://accounts.example");
    }

    #[dialog_common::test]
    fn it_treats_empty_bytes_as_the_detach_tombstone() {
        assert!(AccountProviderRecord::decode(&[]).unwrap().is_none());
    }
}

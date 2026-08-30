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

/// Wire form. Kept JSON and field-compatible with records written before the
/// descriptor existed: those decode with `descriptor: None`, which is exactly
/// the "attached but unconfigured" state a legacy account is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Wire {
    version: u8,
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attached_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
}

/// A provider attachment, with the account repository descriptor it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountProviderRecord {
    provider: String,
    attached_at: Option<u64>,
    remote: Option<String>,
}

impl AccountProviderRecord {
    /// Attach `provider` to this account, syncing at `remote`.
    ///
    /// The remote is where the account's data lives — the access
    /// service's `/ucan/` address — and is separate from the provider,
    /// which is the account service. An empty one is absent: the
    /// address is derivable from the origin, so recording nothing is a
    /// fallback rather than a failure.
    pub fn attach(
        provider: &str,
        remote: &str,
        attached_at: u64,
    ) -> Result<Self, AccountProviderError> {
        Ok(Self {
            provider: canonical_provider(provider)?,
            attached_at: Some(attached_at),
            remote: Some(remote.trim().to_owned()).filter(|value| !value.is_empty()),
        })
    }

    /// Decode a stored credential value against `root_did`. Empty bytes are
    /// the detach tombstone: the credential store has no delete, so unlinking
    /// writes an empty value.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, AccountProviderError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let wire: Wire = serde_json::from_slice(bytes)
            .map_err(|error| AccountProviderError::Encoding(error.to_string()))?;
        if wire.version != RECORD_VERSION {
            return Err(AccountProviderError::UnsupportedVersion(wire.version));
        }
        Ok(Some(Self {
            provider: wire.provider,
            attached_at: wire.attached_at,
            remote: wire.remote,
        }))
    }

    /// Serialize for the credential store.
    pub fn encode(&self) -> Result<Vec<u8>, AccountProviderError> {
        serde_json::to_vec(&Wire {
            version: RECORD_VERSION,
            provider: self.provider.clone(),
            attached_at: self.attached_at,
            remote: self.remote.clone(),
        })
        .map_err(|error| AccountProviderError::Encoding(error.to_string()))
    }

    /// Attached provider base URL, without a trailing slash.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Where the account syncs, when the link named it.
    pub fn remote(&self) -> Option<&str> {
        self.remote.as_deref()
    }

    /// When the provider was attached, for records that recorded it.
    pub fn attached_at(&self) -> Option<u64> {
        self.attached_at
    }
}

fn canonical_provider(provider: &str) -> Result<String, AccountProviderError> {
    let provider = provider.trim().trim_end_matches('/');
    if provider.is_empty() {
        return Err(AccountProviderError::EmptyProvider);
    }
    Ok(provider.to_owned())
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

    const PROVIDER: &str = "https://accounts.example";
    const REMOTE: &str = "https://accounts.example/ucan/";

    /// The record is provider plus when it was attached; the address an
    /// account syncs with is resolved, not stored here.
    #[dialog_common::test]
    fn it_round_trips_an_attachment() {
        let record = AccountProviderRecord::attach(PROVIDER, REMOTE, 42).unwrap();
        let decoded = AccountProviderRecord::decode(&record.encode().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(decoded.provider(), PROVIDER);
        assert_eq!(decoded.remote(), Some(REMOTE));
        assert_eq!(decoded.attached_at(), Some(42));
    }

    #[dialog_common::test]
    fn it_trims_a_trailing_slash_and_rejects_a_blank_provider() {
        let record = AccountProviderRecord::attach("https://accounts.example/", REMOTE, 1).unwrap();
        assert_eq!(record.provider(), PROVIDER);
        assert_eq!(
            AccountProviderRecord::attach("   ", REMOTE, 1).unwrap_err(),
            AccountProviderError::EmptyProvider
        );
    }

    /// A record written before the descriptor was dropped still decodes:
    /// the field it carried is ignored rather than refused.
    #[dialog_common::test]
    fn it_reads_a_record_that_still_carries_a_descriptor() {
        let legacy = br#"{"version":1,"provider":"https://accounts.example","attached_at":9,"descriptor":[1,2,3]}"#;
        let decoded = AccountProviderRecord::decode(legacy).unwrap().unwrap();
        assert_eq!(decoded.provider(), PROVIDER);
        assert_eq!(decoded.attached_at(), Some(9));
    }

    #[dialog_common::test]
    fn it_treats_empty_bytes_as_the_detach_tombstone() {
        assert!(AccountProviderRecord::decode(&[]).unwrap().is_none());
    }
}

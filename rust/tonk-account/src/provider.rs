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

use crate::{AccountRepositoryDescriptorV1, DescriptorError};

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
    descriptor: Option<Vec<u8>>,
}

/// A provider attachment, with the account repository descriptor it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountProviderRecord {
    provider: String,
    attached_at: Option<u64>,
    descriptor: Option<AccountRepositoryDescriptorV1>,
}

impl AccountProviderRecord {
    /// Attach `provider`, binding `descriptor_bytes` to `root_did`.
    pub async fn attach(
        provider: &str,
        descriptor_bytes: &[u8],
        root_did: &dialog_varsig::Did,
        attached_at: u64,
    ) -> Result<Self, AccountProviderError> {
        let provider = canonical_provider(provider)?;
        Ok(Self {
            provider,
            attached_at: Some(attached_at),
            descriptor: Some(checked_descriptor(descriptor_bytes, root_did).await?),
        })
    }

    /// Attach `provider` for an account whose repository is not established
    /// yet. Only a legacy account reaches this state; new accounts always
    /// carry a descriptor from their creation ceremony.
    pub fn attach_unconfigured(
        provider: &str,
        attached_at: u64,
    ) -> Result<Self, AccountProviderError> {
        Ok(Self {
            provider: canonical_provider(provider)?,
            attached_at: Some(attached_at),
            descriptor: None,
        })
    }

    /// Establish the descriptor on an attachment that has none.
    ///
    /// The descriptor is immutable in version 1: one account subject, one
    /// remote. Replacing an established one would repoint this device's
    /// account history, so it is refused rather than overwritten.
    pub async fn establish(
        &self,
        descriptor_bytes: &[u8],
        root_did: &dialog_varsig::Did,
    ) -> Result<Self, AccountProviderError> {
        if self.descriptor.is_some() {
            return Err(AccountProviderError::DescriptorEstablished);
        }
        Ok(Self {
            provider: self.provider.clone(),
            attached_at: self.attached_at,
            descriptor: Some(checked_descriptor(descriptor_bytes, root_did).await?),
        })
    }

    /// Decode a stored credential value against `root_did`. Empty bytes are
    /// the detach tombstone: the credential store has no delete, so unlinking
    /// writes an empty value.
    pub async fn decode(
        bytes: &[u8],
        root_did: &dialog_varsig::Did,
    ) -> Result<Option<Self>, AccountProviderError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let wire: Wire = serde_json::from_slice(bytes)
            .map_err(|error| AccountProviderError::Encoding(error.to_string()))?;
        if wire.version != RECORD_VERSION {
            return Err(AccountProviderError::UnsupportedVersion(wire.version));
        }
        let descriptor = match wire.descriptor.as_deref() {
            Some(bytes) => Some(checked_descriptor(bytes, root_did).await?),
            None => None,
        };
        Ok(Some(Self {
            provider: wire.provider,
            attached_at: wire.attached_at,
            descriptor,
        }))
    }

    /// Serialize for the credential store.
    pub fn encode(&self) -> Result<Vec<u8>, AccountProviderError> {
        serde_json::to_vec(&Wire {
            version: RECORD_VERSION,
            provider: self.provider.clone(),
            attached_at: self.attached_at,
            descriptor: self.descriptor.as_ref().map(|d| d.bytes().to_vec()),
        })
        .map_err(|error| AccountProviderError::Encoding(error.to_string()))
    }

    /// Attached provider base URL, without a trailing slash.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The account repository descriptor, absent until established.
    pub fn descriptor(&self) -> Option<&AccountRepositoryDescriptorV1> {
        self.descriptor.as_ref()
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

async fn checked_descriptor(
    bytes: &[u8],
    root_did: &dialog_varsig::Did,
) -> Result<AccountRepositoryDescriptorV1, AccountProviderError> {
    let descriptor = AccountRepositoryDescriptorV1::validate(bytes).await?;
    if descriptor.account_subject() != root_did {
        return Err(AccountProviderError::DescriptorSubject);
    }
    Ok(descriptor)
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
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;

    const PROVIDER: &str = "https://accounts.example";
    const REMOTE: &str = "https://accounts.example/ucan/";

    async fn root(seed: u8) -> (Ed25519Signer, dialog_varsig::Did) {
        let signer = Ed25519Signer::import(&[seed; 32]).await.unwrap();
        let did = signer.did();
        (signer, did)
    }

    #[dialog_common::test]
    async fn it_round_trips_an_attachment_with_its_descriptor() {
        let (signer, did) = root(7).await;
        let descriptor = AccountRepositoryDescriptorV1::sign(&signer, REMOTE)
            .await
            .unwrap();
        let record = AccountProviderRecord::attach(PROVIDER, descriptor.bytes(), &did, 42)
            .await
            .unwrap();
        let decoded = AccountProviderRecord::decode(&record.encode().unwrap(), &did)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(decoded.provider(), PROVIDER);
        assert_eq!(decoded.attached_at(), Some(42));
        assert_eq!(decoded.descriptor().unwrap().bytes(), descriptor.bytes());
    }

    #[dialog_common::test]
    async fn it_trims_a_trailing_slash_and_rejects_a_blank_provider() {
        let (signer, did) = root(7).await;
        let descriptor = AccountRepositoryDescriptorV1::sign(&signer, REMOTE)
            .await
            .unwrap();
        let record =
            AccountProviderRecord::attach("https://accounts.example/", descriptor.bytes(), &did, 1)
                .await
                .unwrap();
        assert_eq!(record.provider(), PROVIDER);
        assert_eq!(
            AccountProviderRecord::attach_unconfigured("   ", 1).unwrap_err(),
            AccountProviderError::EmptyProvider
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_descriptor_belonging_to_another_root() {
        let (_, did) = root(7).await;
        let (other, _) = root(9).await;
        let descriptor = AccountRepositoryDescriptorV1::sign(&other, REMOTE)
            .await
            .unwrap();
        assert_eq!(
            AccountProviderRecord::attach(PROVIDER, descriptor.bytes(), &did, 1)
                .await
                .unwrap_err(),
            AccountProviderError::DescriptorSubject
        );

        // ...and refuses to read one back, so a record tampered with in place
        // cannot repoint account state either.
        let mine = AccountProviderRecord::attach_unconfigured(PROVIDER, 1).unwrap();
        let smuggled = serde_json::to_vec(&Wire {
            version: RECORD_VERSION,
            provider: mine.provider().to_owned(),
            attached_at: None,
            descriptor: Some(descriptor.bytes().to_vec()),
        })
        .unwrap();
        assert_eq!(
            AccountProviderRecord::decode(&smuggled, &did)
                .await
                .unwrap_err(),
            AccountProviderError::DescriptorSubject
        );
    }

    #[dialog_common::test]
    async fn it_reads_a_pre_descriptor_record_as_attached_but_unconfigured() {
        let (_, did) = root(7).await;
        // Exactly what a browser linked before descriptors existed still holds.
        let legacy = br#"{"version":1,"provider":"https://accounts.example","attached_at":9}"#;
        let decoded = AccountProviderRecord::decode(legacy, &did)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decoded.provider(), PROVIDER);
        assert!(decoded.descriptor().is_none());
    }

    #[dialog_common::test]
    async fn it_treats_empty_bytes_as_the_detach_tombstone() {
        let (_, did) = root(7).await;
        assert!(
            AccountProviderRecord::decode(&[], &did)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[dialog_common::test]
    async fn it_establishes_a_descriptor_once_and_refuses_to_replace_it() {
        let (signer, did) = root(7).await;
        let descriptor = AccountRepositoryDescriptorV1::sign(&signer, REMOTE)
            .await
            .unwrap();
        let unconfigured = AccountProviderRecord::attach_unconfigured(PROVIDER, 1).unwrap();
        let established = unconfigured
            .establish(descriptor.bytes(), &did)
            .await
            .unwrap();
        assert_eq!(
            established.descriptor().unwrap().bytes(),
            descriptor.bytes()
        );

        let other = AccountRepositoryDescriptorV1::sign(&signer, "https://elsewhere.example/ucan/")
            .await
            .unwrap();
        assert_eq!(
            established
                .establish(other.bytes(), &did)
                .await
                .unwrap_err(),
            AccountProviderError::DescriptorEstablished
        );
    }
}

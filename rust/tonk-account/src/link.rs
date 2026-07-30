//! Versioned local account-link credential record.

use dialog_ucan_core::DelegationChain;
use serde_bytes::ByteBuf;
use thiserror::Error;

use crate::{AccountRepositoryDescriptorV1, DescriptorError};

const RECORD_VERSION: u64 = 2;
type RecordEnvelope = (u64, ByteBuf, Option<ByteBuf>);

/// A verified root-to-device delegation and its optional repository descriptor.
#[derive(Debug, Clone)]
pub struct AccountLinkRecord {
    delegation: DelegationChain,
    delegation_bytes: Vec<u8>,
    descriptor: Option<AccountRepositoryDescriptorV1>,
    bytes: Vec<u8>,
}

impl AccountLinkRecord {
    /// Build canonical V2 record bytes from service-returned delegation and descriptor bytes.
    pub async fn create(
        delegation_bytes: &[u8],
        descriptor_bytes: &[u8],
        expected_audience: &dialog_varsig::Did,
    ) -> Result<Self, AccountLinkError> {
        let record: RecordEnvelope = (
            RECORD_VERSION,
            ByteBuf::from(delegation_bytes.to_vec()),
            Some(ByteBuf::from(descriptor_bytes.to_vec())),
        );
        let bytes = serde_ipld_dagcbor::to_vec(&record)
            .map_err(|error| AccountLinkError::Encoding(error.to_string()))?;
        Self::decode_record(&bytes, expected_audience).await
    }

    /// Decode a credential value. Empty bytes are the unlinked tombstone.
    /// Legacy raw delegation bytes remain linked with no descriptor.
    pub async fn decode(
        bytes: &[u8],
        expected_audience: &dialog_varsig::Did,
    ) -> Result<Option<Self>, AccountLinkError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        if let Ok(record) = serde_ipld_dagcbor::from_slice::<RecordEnvelope>(bytes)
            && record.0 == RECORD_VERSION
        {
            return Self::decode_record(bytes, expected_audience)
                .await
                .map(Some);
        }

        let (delegation, delegation_bytes) = validate_delegation(bytes, expected_audience).await?;
        Ok(Some(Self {
            delegation,
            delegation_bytes,
            descriptor: None,
            bytes: bytes.to_vec(),
        }))
    }

    async fn decode_record(
        bytes: &[u8],
        expected_audience: &dialog_varsig::Did,
    ) -> Result<Self, AccountLinkError> {
        let record: RecordEnvelope = serde_ipld_dagcbor::from_slice(bytes)
            .map_err(|error| AccountLinkError::Encoding(error.to_string()))?;
        if record.0 != RECORD_VERSION {
            return Err(AccountLinkError::UnsupportedVersion(record.0));
        }
        let canonical = serde_ipld_dagcbor::to_vec(&record)
            .map_err(|error| AccountLinkError::Encoding(error.to_string()))?;
        if canonical != bytes {
            return Err(AccountLinkError::NonCanonical);
        }
        let delegation_bytes = record.1.into_vec();
        let (delegation, delegation_bytes) =
            validate_delegation(&delegation_bytes, expected_audience).await?;
        let descriptor = match record.2 {
            Some(bytes) => {
                let descriptor = AccountRepositoryDescriptorV1::validate(bytes.as_ref()).await?;
                if descriptor.account_subject() != delegation.issuer() {
                    return Err(AccountLinkError::DescriptorSubject);
                }
                Some(descriptor)
            }
            None => None,
        };
        Ok(Self {
            delegation,
            delegation_bytes,
            descriptor,
            bytes: canonical,
        })
    }

    /// Verified root-to-device delegation.
    pub fn delegation(&self) -> &DelegationChain {
        &self.delegation
    }

    /// Exact service-returned delegation bytes.
    pub fn delegation_bytes(&self) -> &[u8] {
        &self.delegation_bytes
    }

    /// Verified descriptor, absent for legacy or explicitly unconfigured records.
    pub fn descriptor(&self) -> Option<&AccountRepositoryDescriptorV1> {
        self.descriptor.as_ref()
    }

    /// Exact canonical V2 bytes, or exact legacy bytes when decoded from legacy storage.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

async fn validate_delegation(
    bytes: &[u8],
    expected_audience: &dialog_varsig::Did,
) -> Result<(DelegationChain, Vec<u8>), AccountLinkError> {
    let chain = DelegationChain::try_from(bytes).map_err(|_| AccountLinkError::Delegation)?;
    if chain.proof_cids().len() != 1 || chain.subject().is_some() {
        return Err(AccountLinkError::DelegationShape);
    }
    if chain.audience() != expected_audience {
        return Err(AccountLinkError::DelegationAudience);
    }
    let proof = chain
        .proofs()
        .next()
        .ok_or(AccountLinkError::DelegationShape)?;
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .map_err(|_| AccountLinkError::DelegationSignature)?;
    Ok((chain, bytes.to_vec()))
}

/// Local account-link record validation failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccountLinkError {
    /// DAG-CBOR encoding or decoding failed.
    #[error("invalid account-link encoding: {0}")]
    Encoding(String),
    /// The record names an unsupported version.
    #[error("unsupported account-link version {0}")]
    UnsupportedVersion(u64),
    /// The record is not the unique canonical DAG-CBOR representation.
    #[error("account-link record is not canonically encoded")]
    NonCanonical,
    /// Delegation bytes are malformed.
    #[error("account delegation is invalid")]
    Delegation,
    /// The delegation is not the one-hop, subject-open account grant.
    #[error("account delegation has an invalid shape")]
    DelegationShape,
    /// The delegation does not target the current profile.
    #[error("account delegation targets another profile")]
    DelegationAudience,
    /// The delegation signature is invalid.
    #[error("account delegation signature is invalid")]
    DelegationSignature,
    /// Descriptor validation failed.
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
    /// Descriptor subject and delegation issuer differ.
    #[error("account descriptor subject differs from delegation issuer")]
    DescriptorSubject,
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

    async fn fixture(
        root_seed: u8,
        device_seed: u8,
    ) -> (Ed25519Signer, dialog_varsig::Did, Vec<u8>) {
        let root = Ed25519Signer::import(&[root_seed; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[device_seed; 32]).await.unwrap();
        let delegation = tonk_identity_delegation(root.clone(), &device.did()).await;
        (root, device.did(), delegation)
    }

    async fn tonk_identity_delegation(root: Ed25519Signer, device: &dialog_varsig::Did) -> Vec<u8> {
        use dialog_ucan_core::subject::Subject;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain};

        let delegation = DelegationBuilder::new()
            .issuer(root)
            .audience(device)
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation).to_bytes().unwrap()
    }

    #[dialog_common::test]
    async fn it_round_trips_one_canonical_v2_record() {
        let (root, device, delegation) = fixture(7, 8).await;
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let record = AccountLinkRecord::create(&delegation, descriptor.bytes(), &device)
            .await
            .unwrap();
        let decoded = AccountLinkRecord::decode(record.bytes(), &device)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(decoded.delegation_bytes(), delegation);
        assert_eq!(decoded.delegation().issuer(), &root.did());
        assert_eq!(decoded.descriptor().unwrap().bytes(), descriptor.bytes());
        assert_eq!(decoded.bytes(), record.bytes());
    }

    #[dialog_common::test]
    async fn it_keeps_legacy_delegations_linked_but_unconfigured() {
        let (root, device, delegation) = fixture(7, 8).await;
        let decoded = AccountLinkRecord::decode(&delegation, &device)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decoded.delegation().issuer(), &root.did());
        assert!(decoded.descriptor().is_none());
        assert_eq!(decoded.bytes(), delegation);
        assert!(
            AccountLinkRecord::decode(&[], &device)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[dialog_common::test]
    async fn it_rejects_descriptor_and_delegation_identity_mismatches() {
        let (_, device, delegation) = fixture(7, 8).await;
        let other_root = Ed25519Signer::import(&[9; 32]).await.unwrap();
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&other_root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        assert_eq!(
            AccountLinkRecord::create(&delegation, descriptor.bytes(), &device)
                .await
                .unwrap_err(),
            AccountLinkError::DescriptorSubject
        );

        let other_device = Ed25519Signer::import(&[10; 32]).await.unwrap();
        assert_eq!(
            AccountLinkRecord::create(&delegation, descriptor.bytes(), &other_device.did())
                .await
                .unwrap_err(),
            AccountLinkError::DelegationAudience
        );
    }
}

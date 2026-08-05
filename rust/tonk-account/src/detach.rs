//! Canonical, device-signed account attachment detach intents.

use dialog_credentials::{Ed25519Verifier, SignerCredential};
use dialog_varsig::eddsa::Ed25519Signature;
use dialog_varsig::{Did, Principal as _, Signer as _, Verifier as _};
use thiserror::Error;

const VERSION: u8 = 1;
const MAX_PAYLOAD_BYTES: usize = 4096;
const SIGNATURE_DOMAIN: &[u8] = b"tonk/account-device-detach/v1\0";

/// The exact attachment generation a device asks the account service to hide.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DetachPayloadV1 {
    /// Detach contract version.
    pub version: u8,
    /// Account root that owns the attachment.
    pub account_root: String,
    /// DID of the persistent device signing this payload.
    pub device_did: String,
    /// Random service-issued attachment generation identifier.
    pub attachment_id: String,
    /// CID of the root-to-device delegation for this generation.
    pub delegation_cid: String,
    /// Unix timestamp supplied by the client.
    pub issued_at: u64,
}

/// Canonical detach payload bytes and the device's Ed25519 signature.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignedDetachIntent {
    /// Canonical DAG-CBOR [`DetachPayloadV1`] bytes.
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    /// Raw Ed25519 signature over the domain-separated payload.
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
}

impl SignedDetachIntent {
    /// Sign an exact attachment-generation detach request with the persistent
    /// device profile key.
    pub async fn sign(
        signer: &SignerCredential,
        account_root: &Did,
        attachment_id: &str,
        delegation_cid: &str,
        issued_at: u64,
    ) -> Result<Self, DetachIntentError> {
        validate_attachment_id(attachment_id)?;
        validate_delegation_cid(delegation_cid)?;
        let payload = DetachPayloadV1 {
            version: VERSION,
            account_root: account_root.to_string(),
            device_did: signer.did().to_string(),
            attachment_id: attachment_id.to_string(),
            delegation_cid: delegation_cid.to_string(),
            issued_at,
        };
        let payload = encode(&payload)?;
        let signature = signer
            .sign(&signing_message(&payload))
            .await
            .map_err(|_| DetachIntentError::Signature)?
            .to_bytes()
            .to_vec();
        Ok(Self { payload, signature })
    }

    /// Decode, canonicalize, and verify this exact device-signed intent.
    pub async fn validate(&self) -> Result<DetachPayloadV1, DetachIntentError> {
        if self.payload.len() > MAX_PAYLOAD_BYTES {
            return Err(DetachIntentError::TooLarge);
        }
        let payload: DetachPayloadV1 = decode(&self.payload)?;
        if encode(&payload)? != self.payload {
            return Err(DetachIntentError::NonCanonical);
        }
        if payload.version != VERSION {
            return Err(DetachIntentError::UnsupportedVersion(payload.version));
        }
        let account_root: Did = payload
            .account_root
            .parse()
            .map_err(|_| DetachIntentError::AccountRoot)?;
        if account_root.to_string() != payload.account_root {
            return Err(DetachIntentError::AccountRoot);
        }
        let verifier: Ed25519Verifier = payload
            .device_did
            .parse()
            .map_err(|_| DetachIntentError::DeviceDid)?;
        if verifier.to_string() != payload.device_did {
            return Err(DetachIntentError::DeviceDid);
        }
        validate_attachment_id(&payload.attachment_id)?;
        validate_delegation_cid(&payload.delegation_cid)?;
        let signature_bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| DetachIntentError::Signature)?;
        verifier
            .verify(
                &signing_message(&self.payload),
                &Ed25519Signature::from_bytes(signature_bytes),
            )
            .await
            .map_err(|_| DetachIntentError::Signature)?;
        Ok(payload)
    }
}

fn signing_message(payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGNATURE_DOMAIN.len() + payload.len());
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(payload);
    message
}

fn validate_attachment_id(value: &str) -> Result<(), DetachIntentError> {
    if value.is_empty()
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DetachIntentError::AttachmentId);
    }
    Ok(())
}

fn validate_delegation_cid(value: &str) -> Result<(), DetachIntentError> {
    if value.is_empty()
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DetachIntentError::DelegationCid);
    }
    Ok(())
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, DetachIntentError> {
    serde_ipld_dagcbor::to_vec(value)
        .map_err(|error| DetachIntentError::Encoding(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DetachIntentError> {
    serde_ipld_dagcbor::from_slice(bytes)
        .map_err(|error| DetachIntentError::Encoding(error.to_string()))
}

/// Detach-intent encoding, identifier, or signature failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DetachIntentError {
    /// The canonical payload exceeds the protocol bound.
    #[error("detach payload exceeds 4096 bytes")]
    TooLarge,
    /// DAG-CBOR encoding or decoding failed.
    #[error("invalid detach payload encoding: {0}")]
    Encoding(String),
    /// The bytes are not the unique canonical DAG-CBOR representation.
    #[error("detach payload is not canonically encoded")]
    NonCanonical,
    /// The payload version is not supported.
    #[error("unsupported detach payload version {0}")]
    UnsupportedVersion(u8),
    /// The account root is not a canonical DID.
    #[error("detach account root is not a canonical DID")]
    AccountRoot,
    /// The signer is not a canonical Ed25519 did:key.
    #[error("detach device DID is not a canonical Ed25519 did:key")]
    DeviceDid,
    /// The attachment generation is empty or not canonically spelled.
    #[error("detach attachment ID is invalid")]
    AttachmentId,
    /// The delegation CID is empty or not canonically spelled.
    #[error("detach delegation CID is invalid")]
    DelegationCid,
    /// The signature is absent, malformed, or invalid.
    #[error("detach signature is invalid")]
    Signature,
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn signer(seed: u8) -> dialog_credentials::Ed25519Signer {
        dialog_credentials::Ed25519Signer::import(&[seed; 32])
            .await
            .unwrap()
    }

    #[dialog_common::test]
    async fn it_round_trips_a_canonical_device_signed_detach_intent() {
        let device = signer(7).await;
        let account = signer(8).await.did();
        let credential = SignerCredential::from(device.clone());
        let attachment_id = "ab".repeat(32);

        let intent =
            SignedDetachIntent::sign(&credential, &account, &attachment_id, "bafyreicid", 42)
                .await
                .unwrap();
        let payload = intent.validate().await.unwrap();

        assert_eq!(payload.version, 1);
        assert_eq!(payload.account_root, account.to_string());
        assert_eq!(payload.device_did, device.did().to_string());
        assert_eq!(payload.attachment_id, attachment_id);
        assert_eq!(payload.delegation_cid, "bafyreicid");
        assert_eq!(payload.issued_at, 42);
    }

    #[dialog_common::test]
    async fn it_rejects_tampering_invalid_ids_and_unsupported_versions() {
        let device = signer(7).await;
        let credential = SignerCredential::from(device);
        let account = signer(8).await.did();
        let attachment = "cd".repeat(32);
        let intent = SignedDetachIntent::sign(&credential, &account, &attachment, "bafyreicid", 42)
            .await
            .unwrap();

        let mut bad_signature = intent.clone();
        bad_signature.signature[0] ^= 1;
        assert_eq!(
            bad_signature.validate().await.unwrap_err(),
            DetachIntentError::Signature
        );

        let mut bad_payload = intent;
        let last = bad_payload.payload.len() - 1;
        bad_payload.payload[last] ^= 1;
        assert!(bad_payload.validate().await.is_err());

        assert_eq!(
            SignedDetachIntent::sign(&credential, &account, "", "cid", 1)
                .await
                .unwrap_err(),
            DetachIntentError::AttachmentId
        );
        assert_eq!(
            SignedDetachIntent::sign(&credential, &account, &attachment, "", 1)
                .await
                .unwrap_err(),
            DetachIntentError::DelegationCid
        );

        let payload = DetachPayloadV1 {
            version: 2,
            account_root: account.to_string(),
            device_did: credential.did().to_string(),
            attachment_id: attachment,
            delegation_cid: "cid".to_string(),
            issued_at: 1,
        };
        let payload = encode(&payload).unwrap();
        let signature = credential
            .sign(&signing_message(&payload))
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        assert_eq!(
            SignedDetachIntent { payload, signature }
                .validate()
                .await
                .unwrap_err(),
            DetachIntentError::UnsupportedVersion(2)
        );
    }
}

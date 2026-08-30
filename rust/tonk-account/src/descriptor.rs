//! Immutable root-signed account repository descriptor.

use dialog_credentials::{Ed25519Signer, Ed25519Verifier};
use dialog_varsig::eddsa::Ed25519Signature;
use dialog_varsig::{Principal as _, Signer as _, Verifier as _};
use serde_bytes::ByteBuf;
use thiserror::Error;
use url::{Host, Url};

const VERSION: u64 = 1;
const MAX_DESCRIPTOR_BYTES: usize = 4096;
const MAX_REMOTE_BYTES: usize = 2048;
const SIGNATURE_DOMAIN: &[u8] = b"tonk/account-repository-descriptor/v1\0";

type Payload = (u64, String, String);
type Envelope = (ByteBuf, ByteBuf);

/// A canonical, signature-verified account repository descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRepositoryDescriptorV1 {
    account_subject: dialog_varsig::Did,
    remote: Url,
    bytes: Vec<u8>,
    hash: [u8; 32],
}

impl AccountRepositoryDescriptorV1 {
    /// Sign a V1 descriptor with the account's passkey-derived root signer.
    ///
    /// The concrete signer, not dialog's algorithm-agnostic one:
    /// [`Self::validate`] parses the subject as an `Ed25519Verifier` and
    /// the signature as an `Ed25519Signature`, so this format admits no
    /// other algorithm. Widening here would mint descriptors that never
    /// validate.
    pub async fn sign(root: &Ed25519Signer, remote: &str) -> Result<Self, DescriptorError> {
        let account_subject = root.did().to_string();
        let remote = canonical_remote(remote)?;
        let payload = (VERSION, account_subject, remote.to_string());
        let payload_bytes = encode(&payload)?;
        let signature = root
            .sign(&signing_message(&payload_bytes))
            .await
            .map_err(|_| DescriptorError::Signature)?;
        let bytes = encode(&(
            ByteBuf::from(payload_bytes),
            ByteBuf::from(signature.to_bytes().to_vec()),
        ))?;
        Self::validate(&bytes).await
    }

    /// Parse and verify exact canonical descriptor bytes.
    pub async fn validate(bytes: &[u8]) -> Result<Self, DescriptorError> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(DescriptorError::TooLarge);
        }
        let envelope: Envelope = decode(bytes)?;
        if encode(&envelope)? != bytes {
            return Err(DescriptorError::NonCanonical);
        }
        let payload_bytes = envelope.0.into_vec();
        let payload: Payload = decode(&payload_bytes)?;
        if encode(&payload)? != payload_bytes {
            return Err(DescriptorError::NonCanonical);
        }
        let (version, account_subject, remote) = payload;
        if version != VERSION {
            return Err(DescriptorError::UnsupportedVersion(version));
        }
        let verifier: Ed25519Verifier = account_subject
            .parse()
            .map_err(|_| DescriptorError::AccountSubject)?;
        if verifier.to_string() != account_subject {
            return Err(DescriptorError::AccountSubject);
        }
        let signature_bytes: [u8; 64] = envelope
            .1
            .as_ref()
            .try_into()
            .map_err(|_| DescriptorError::Signature)?;
        let signature = Ed25519Signature::from_bytes(signature_bytes);
        verifier
            .verify(&signing_message(&payload_bytes), &signature)
            .await
            .map_err(|_| DescriptorError::Signature)?;
        let remote = canonical_remote(&remote)?;
        let hash = *blake3::hash(bytes).as_bytes();
        Ok(Self {
            account_subject: verifier.did(),
            remote,
            bytes: bytes.to_vec(),
            hash,
        })
    }

    /// Immutable account subject named by this descriptor.
    pub fn account_subject(&self) -> &dialog_varsig::Did {
        &self.account_subject
    }

    /// Canonical V1 repository remote.
    pub fn remote(&self) -> &Url {
        &self.remote
    }

    /// Exact canonical signed envelope bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// BLAKE3 hash of the exact canonical envelope bytes.
    pub fn content_hash(&self) -> [u8; 32] {
        self.hash
    }
}

/// Domain-separated bytes the root signs and every verifier re-derives.
fn signing_message(payload_bytes: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGNATURE_DOMAIN.len() + payload_bytes.len());
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(payload_bytes);
    message
}

fn canonical_remote(input: &str) -> Result<Url, DescriptorError> {
    if input.len() > MAX_REMOTE_BYTES {
        return Err(DescriptorError::RemoteTooLarge);
    }
    let remote = Url::parse(input).map_err(|_| DescriptorError::Remote)?;
    if remote.username() != "" || remote.password().is_some() {
        return Err(DescriptorError::RemoteCredentials);
    }
    if remote.query().is_some() || remote.fragment().is_some() {
        return Err(DescriptorError::RemoteSuffix);
    }
    match remote.scheme() {
        "https" => {}
        "http" if is_loopback(&remote) => {}
        _ => return Err(DescriptorError::RemoteScheme),
    }
    if !remote.path().ends_with('/') {
        return Err(DescriptorError::RemoteTrailingSlash);
    }
    if remote.as_str() != input {
        return Err(DescriptorError::RemoteNonCanonical);
    }
    Ok(remote)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, DescriptorError> {
    serde_ipld_dagcbor::to_vec(value).map_err(|error| DescriptorError::Encoding(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DescriptorError> {
    serde_ipld_dagcbor::from_slice(bytes)
        .map_err(|error| DescriptorError::Encoding(error.to_string()))
}

/// Descriptor encoding, authentication, or URL validation failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DescriptorError {
    /// The descriptor envelope exceeds the V1 bound.
    #[error("descriptor exceeds 4096 bytes")]
    TooLarge,
    /// The remote exceeds the V1 bound.
    #[error("descriptor remote exceeds 2048 bytes")]
    RemoteTooLarge,
    /// DAG-CBOR encoding or decoding failed.
    #[error("invalid descriptor encoding: {0}")]
    Encoding(String),
    /// Decoded bytes were not the unique canonical representation.
    #[error("descriptor is not canonically encoded")]
    NonCanonical,
    /// The payload names an unsupported descriptor version.
    #[error("unsupported descriptor version {0}")]
    UnsupportedVersion(u64),
    /// The subject is not a canonical Ed25519 `did:key`.
    #[error("descriptor account subject is not an Ed25519 did:key")]
    AccountSubject,
    /// The root signature is absent, malformed, or invalid.
    #[error("descriptor signature is invalid")]
    Signature,
    /// The remote is not an absolute URL.
    #[error("descriptor remote is invalid")]
    Remote,
    /// The remote embeds username or password material.
    #[error("descriptor remote must not contain credentials")]
    RemoteCredentials,
    /// The remote contains a query or fragment.
    #[error("descriptor remote must not contain a query or fragment")]
    RemoteSuffix,
    /// The remote is not HTTPS or loopback HTTP.
    #[error("descriptor remote must use HTTPS, except on loopback")]
    RemoteScheme,
    /// The remote does not end in `/`.
    #[error("descriptor remote must end in a slash")]
    RemoteTrailingSlash,
    /// The supplied remote spelling differs from the URL canonical form.
    #[error("descriptor remote is not canonical")]
    RemoteNonCanonical,
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use super::*;

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    #[dialog_common::test]
    async fn it_produces_deterministic_canonical_bytes_and_hash() {
        let root = signer(7).await;
        let first = AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
            .await
            .unwrap();
        let second = AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
            .await
            .unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(
            first.content_hash(),
            *blake3::hash(first.bytes()).as_bytes()
        );
        assert_eq!(first.account_subject(), &root.did());
    }

    #[dialog_common::test]
    async fn it_rejects_a_subject_not_matching_the_signature() {
        let root = signer(7).await;
        let other = signer(8).await;
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let envelope: Envelope = decode(descriptor.bytes()).unwrap();
        let (_, _, remote): Payload = decode(envelope.0.as_ref()).unwrap();
        let payload = encode(&(VERSION, other.did().to_string(), remote)).unwrap();
        let tampered = encode(&(ByteBuf::from(payload), envelope.1)).unwrap();

        assert_eq!(
            AccountRepositoryDescriptorV1::validate(&tampered)
                .await
                .unwrap_err(),
            DescriptorError::Signature
        );
    }

    #[dialog_common::test]
    async fn it_rejects_wrong_versions_signatures_and_alternate_encoding() {
        let root = signer(7).await;
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let envelope: Envelope = decode(descriptor.bytes()).unwrap();
        let (_, subject, remote): Payload = decode(envelope.0.as_ref()).unwrap();
        let payload = encode(&(2_u64, subject, remote)).unwrap();
        let version = encode(&(ByteBuf::from(payload), envelope.1.clone())).unwrap();
        assert!(matches!(
            AccountRepositoryDescriptorV1::validate(&version).await,
            Err(DescriptorError::UnsupportedVersion(2))
        ));

        let mut signature = envelope.1.into_vec();
        signature[0] ^= 1;
        let invalid_signature = encode(&(
            ByteBuf::from(descriptor_payload(descriptor.bytes())),
            ByteBuf::from(signature),
        ))
        .unwrap();
        assert_eq!(
            AccountRepositoryDescriptorV1::validate(&invalid_signature)
                .await
                .unwrap_err(),
            DescriptorError::Signature
        );

        let mut alternate_payload = descriptor_payload(descriptor.bytes());
        assert_eq!(&alternate_payload[..2], &[0x83, 0x01]);
        alternate_payload.splice(1..2, [0x18, 0x01]);
        let signature = root
            .sign(&signing_message(&alternate_payload))
            .await
            .unwrap();
        let alternate = encode(&(
            ByteBuf::from(alternate_payload),
            ByteBuf::from(signature.to_bytes().to_vec()),
        ))
        .unwrap();
        assert_eq!(
            AccountRepositoryDescriptorV1::validate(&alternate)
                .await
                .unwrap_err(),
            DescriptorError::NonCanonical
        );
    }

    fn descriptor_payload(bytes: &[u8]) -> Vec<u8> {
        decode::<Envelope>(bytes).unwrap().0.into_vec()
    }

    #[dialog_common::test]
    async fn it_enforces_descriptor_and_remote_bounds() {
        assert_eq!(
            AccountRepositoryDescriptorV1::validate(&vec![0; MAX_DESCRIPTOR_BYTES + 1])
                .await
                .unwrap_err(),
            DescriptorError::TooLarge
        );
        let root = signer(7).await;
        let remote = format!("https://example.test/{}/", "x".repeat(MAX_REMOTE_BYTES));
        assert_eq!(
            AccountRepositoryDescriptorV1::sign(&root, &remote)
                .await
                .unwrap_err(),
            DescriptorError::RemoteTooLarge
        );
    }

    #[dialog_common::test]
    async fn it_rejects_unsafe_or_noncanonical_remotes() {
        let root = signer(7).await;
        for remote in [
            "https://user@example.test/ucan/",
            "https://example.test/ucan/?query=1",
            "https://example.test/ucan/#fragment",
            "ftp://example.test/ucan/",
            "http://example.test/ucan/",
            "https://EXAMPLE.test/ucan/",
            "https://example.test/ucan",
        ] {
            assert!(
                AccountRepositoryDescriptorV1::sign(&root, remote)
                    .await
                    .is_err(),
                "{remote} must be rejected"
            );
        }
        for remote in [
            "http://localhost:8080/ucan/",
            "http://127.0.0.1:8080/ucan/",
            "http://[::1]:8080/ucan/",
            "https://example.test/ucan/",
        ] {
            assert!(
                AccountRepositoryDescriptorV1::sign(&root, remote)
                    .await
                    .is_ok(),
                "{remote} must be accepted"
            );
        }
    }

    #[dialog_common::test]
    async fn it_contains_only_the_fixed_non_expiring_payload_tuple() {
        let root = signer(7).await;
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let payload: Payload = decode(&descriptor_payload(descriptor.bytes())).unwrap();
        assert_eq!(payload.0, 1);
        assert_eq!(payload.1, root.did().to_string());
        assert_eq!(payload.2, "https://accounts.example/ucan/");
    }
}

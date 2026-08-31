//! Canonical root-signed account-setup recovery manifest.

use dialog_credentials::{Ed25519Signer, Ed25519Verifier};
use dialog_varsig::eddsa::Ed25519Signature;
use dialog_varsig::{Principal as _, Signer as _, Verifier as _};
use serde_bytes::ByteBuf;
use thiserror::Error;
use url::{Host, Url};

use crate::creation::{AccountCreationFingerprint, AccountCreationPasskey};

const VERSION: u64 = 1;
const SIGNATURE_DOMAIN: &[u8] = b"tonk/account-setup-recovery-manifest/v1\0";
const HASH_DOMAIN: &[u8] = b"tonk/account-setup-recovery-artifact/v1";
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_OPERATION_BYTES: usize = 128;
const MAX_URL_BYTES: usize = 2048;
const MAX_DID_BYTES: usize = 512;
const MAX_CREDENTIAL_ID_BYTES: usize = 4096;
const MAX_PASSKEY_LABEL_CHARS: usize = 120;
const MAX_DELEGATION_BYTES: usize = 64 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 4096;
const MAX_INVOCATION_BYTES: usize = 128 * 1024;
const MAX_DEPOSIT_BYTES: usize = 64 * 1024;
const MAX_DEPOSITS: usize = 8;
const MAX_CONSENT_BYTES: usize = 64 * 1024;
const MAX_SEALED_BYTES: usize = 4096;
const MAX_TOTAL_ARTIFACT_BYTES: usize = 512 * 1024;

type Deployment = (String, String, Option<String>);
type PasskeyFacts = (u64, String);
type Identity = (
    String,
    String,
    String,
    ByteBuf,
    Option<PasskeyFacts>,
    Option<String>,
    String,
);
type ArtifactBindings = (
    ByteBuf,
    ByteBuf,
    ByteBuf,
    u64,
    ByteBuf,
    ByteBuf,
    ByteBuf,
    ByteBuf,
);
type Payload = (u64, String, u64, Deployment, Identity, ArtifactBindings);
type Envelope = (ByteBuf, ByteBuf);

/// Exact setup facts and artifact bytes bound by a recovery manifest.
///
/// Artifact bytes are hashed into the signed payload. The manifest is an
/// anti-mix proof only; consumers must still decode and authorize each
/// artifact independently before using it for an effect.
#[derive(Clone, Copy)]
pub struct AccountSetupRecoveryManifestInput<'a> {
    /// Worker-minted setup operation identifier.
    pub operation_id: &'a str,
    /// Immutable Unix-seconds reference captured by the ceremony after
    /// credential creation and before signing bounded invocations.
    pub ceremony_created_at: u64,
    /// Canonical deployment-selected account-service base URL.
    pub provider: &'a str,
    /// Canonical deployment-selected account repository remote.
    pub remote: &'a str,
    /// Canonical deployment service DID, when configured.
    pub service_did: Option<&'a str>,
    /// Expected root signer and manifest subject.
    pub root_did: &'a str,
    /// Expected first device and manifest audience.
    pub device_did: &'a str,
    /// Exact WebAuthn credential identifier.
    pub credential_id: &'a str,
    /// Canonical provider creation fingerprint.
    pub create_fingerprint: AccountCreationFingerprint,
    /// Normalized passkey creation facts, when present.
    pub passkey: Option<AccountCreationPasskey<'a>>,
    /// Account X25519 recipient fact, when present.
    pub encryption_recipient: Option<&'a str>,
    /// Custody signer DID named by consent and deferred publish.
    pub custody_did: &'a str,
    /// Exact stable root-to-device delegation container bytes.
    pub delegation: &'a [u8],
    /// Exact canonical repository descriptor bytes.
    pub descriptor: &'a [u8],
    /// Exact original account-create invocation container bytes.
    pub create_invocation: &'a [u8],
    /// Exact account-signed deposit containers in their durable order.
    pub deposits: &'a [Vec<u8>],
    /// Exact custody-consent container bytes.
    pub custody_consent: &'a [u8],
    /// Exact passkey-sealed account-secret envelope bytes.
    pub sealed_envelope: &'a [u8],
    /// Exact deferred custody-publish invocation container bytes.
    pub publish_invocation: &'a [u8],
}

/// Canonical, root-signed proof that protected setup records came from one
/// ceremony operation.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountSetupRecoveryManifestV1 {
    bytes: Vec<u8>,
    hash: [u8; 32],
}

impl AccountSetupRecoveryManifestV1 {
    /// Sign a canonical V1 manifest with the account root.
    pub async fn sign(
        root: &Ed25519Signer,
        input: AccountSetupRecoveryManifestInput<'_>,
    ) -> Result<Self, RecoveryManifestError> {
        if root.did().to_string() != input.root_did {
            return Err(RecoveryManifestError::Subject);
        }
        let payload = build_payload(input)?;
        let payload_bytes = encode(&payload)?;
        let signature = root
            .sign(&signing_message(&payload_bytes))
            .await
            .map_err(|_| RecoveryManifestError::Signature)?;
        let bytes = encode(&(
            ByteBuf::from(payload_bytes),
            ByteBuf::from(signature.to_bytes().to_vec()),
        ))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(RecoveryManifestError::TooLarge);
        }
        Self::validate(&bytes, input).await
    }

    /// Verify canonical encoding, the exact root signature, and every
    /// expected deployment/identity/artifact binding.
    pub async fn validate(
        bytes: &[u8],
        expected: AccountSetupRecoveryManifestInput<'_>,
    ) -> Result<Self, RecoveryManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(RecoveryManifestError::TooLarge);
        }
        // Bound the expected values before decoding or hashing attacker-held
        // artifacts. This also constructs the one canonical expected payload.
        let expected_payload = build_payload(expected)?;
        let envelope: Envelope = decode(bytes)?;
        if encode(&envelope)? != bytes {
            return Err(RecoveryManifestError::NonCanonical);
        }
        let payload_bytes = envelope.0.into_vec();
        let payload: Payload = decode(&payload_bytes)?;
        if encode(&payload)? != payload_bytes {
            return Err(RecoveryManifestError::NonCanonical);
        }
        if payload.0 != VERSION {
            return Err(RecoveryManifestError::UnsupportedVersion(payload.0));
        }

        let subject = &payload.4.0;
        let verifier: Ed25519Verifier = subject
            .parse()
            .map_err(|_| RecoveryManifestError::Subject)?;
        if verifier.to_string() != *subject {
            return Err(RecoveryManifestError::Subject);
        }
        let signature_bytes: [u8; 64] = envelope
            .1
            .as_ref()
            .try_into()
            .map_err(|_| RecoveryManifestError::Signature)?;
        verifier
            .verify(
                &signing_message(&payload_bytes),
                &Ed25519Signature::from_bytes(signature_bytes),
            )
            .await
            .map_err(|_| RecoveryManifestError::Signature)?;
        if payload != expected_payload {
            return Err(RecoveryManifestError::Binding);
        }

        Ok(Self {
            bytes: bytes.to_vec(),
            hash: *blake3::hash(bytes).as_bytes(),
        })
    }

    /// Exact canonical signed envelope bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// BLAKE3 hash of the exact canonical signed envelope bytes.
    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.hash
    }
}

fn build_payload(
    input: AccountSetupRecoveryManifestInput<'_>,
) -> Result<Payload, RecoveryManifestError> {
    validate_input(&input)?;
    let passkey = input
        .passkey
        .map(|passkey| (passkey.created_at, passkey.created_on.to_string()));
    let deposits_hash = list_hash(b"deposits", input.deposits);
    Ok((
        VERSION,
        input.operation_id.to_string(),
        input.ceremony_created_at,
        (
            input.provider.to_string(),
            input.remote.to_string(),
            input.service_did.map(str::to_string),
        ),
        (
            input.root_did.to_string(),
            input.device_did.to_string(),
            input.credential_id.to_string(),
            ByteBuf::from(input.create_fingerprint.as_bytes().to_vec()),
            passkey,
            input.encryption_recipient.map(str::to_string),
            input.custody_did.to_string(),
        ),
        (
            digest(b"delegation", input.delegation),
            digest(b"descriptor", input.descriptor),
            digest(b"create-invocation", input.create_invocation),
            input.deposits.len() as u64,
            ByteBuf::from(deposits_hash.to_vec()),
            digest(b"custody-consent", input.custody_consent),
            digest(b"sealed-envelope", input.sealed_envelope),
            digest(b"publish-invocation", input.publish_invocation),
        ),
    ))
}

fn validate_input(
    input: &AccountSetupRecoveryManifestInput<'_>,
) -> Result<(), RecoveryManifestError> {
    if !valid_text(input.operation_id, MAX_OPERATION_BYTES) {
        return Err(RecoveryManifestError::Input("operation_id"));
    }
    if input.ceremony_created_at == 0 || input.ceremony_created_at > 0x001F_FFFF_FFFF_FFFF {
        return Err(RecoveryManifestError::Input("ceremony_created_at"));
    }
    canonical_http_base(input.provider)?;
    canonical_http_base(input.remote)?;
    if input.service_did.is_some_and(|did| !canonical_did(did)) {
        return Err(RecoveryManifestError::Input("service_did"));
    }
    let root: Result<Ed25519Verifier, _> = input.root_did.parse();
    if root.is_err() || root.is_ok_and(|root| root.to_string() != input.root_did) {
        return Err(RecoveryManifestError::Subject);
    }
    if !canonical_did(input.device_did) {
        return Err(RecoveryManifestError::Input("device_did"));
    }
    if !canonical_lower_hex(input.credential_id, MAX_CREDENTIAL_ID_BYTES) {
        return Err(RecoveryManifestError::Input("credential_id"));
    }
    if let Some(passkey) = input.passkey
        && (passkey.created_at == 0
            || passkey.created_on.trim() != passkey.created_on
            || passkey.created_on.is_empty()
            || passkey.created_on.chars().count() > MAX_PASSKEY_LABEL_CHARS
            || passkey.created_on.chars().any(char::is_control))
    {
        return Err(RecoveryManifestError::Input("passkey"));
    }
    if input
        .encryption_recipient
        .is_some_and(|did| !valid_text(did, MAX_DID_BYTES))
    {
        return Err(RecoveryManifestError::Input("encryption_recipient"));
    }
    if !canonical_did(input.custody_did) {
        return Err(RecoveryManifestError::Input("custody_did"));
    }
    if input.delegation.len() > MAX_DELEGATION_BYTES {
        return Err(RecoveryManifestError::Input("delegation"));
    }
    if input.descriptor.len() > MAX_DESCRIPTOR_BYTES {
        return Err(RecoveryManifestError::Input("descriptor"));
    }
    if input.create_invocation.len() > MAX_INVOCATION_BYTES {
        return Err(RecoveryManifestError::Input("create_invocation"));
    }
    if input.deposits.len() > MAX_DEPOSITS
        || input
            .deposits
            .iter()
            .any(|deposit| deposit.len() > MAX_DEPOSIT_BYTES)
    {
        return Err(RecoveryManifestError::Input("deposits"));
    }
    if input.custody_consent.len() > MAX_CONSENT_BYTES {
        return Err(RecoveryManifestError::Input("custody_consent"));
    }
    if input.sealed_envelope.len() > MAX_SEALED_BYTES {
        return Err(RecoveryManifestError::Input("sealed_envelope"));
    }
    if input.publish_invocation.len() > MAX_INVOCATION_BYTES {
        return Err(RecoveryManifestError::Input("publish_invocation"));
    }
    let total = [
        input.delegation.len(),
        input.descriptor.len(),
        input.create_invocation.len(),
        input.custody_consent.len(),
        input.sealed_envelope.len(),
        input.publish_invocation.len(),
    ]
    .into_iter()
    .chain(input.deposits.iter().map(Vec::len))
    .try_fold(0usize, usize::checked_add)
    .ok_or(RecoveryManifestError::Input("artifact_total"))?;
    if total > MAX_TOTAL_ARTIFACT_BYTES {
        return Err(RecoveryManifestError::Input("artifact_total"));
    }
    Ok(())
}

fn digest(label: &[u8], bytes: &[u8]) -> ByteBuf {
    ByteBuf::from(hash_fields(label, [bytes]).to_vec())
}

fn list_hash(label: &[u8], values: &[Vec<u8>]) -> [u8; 32] {
    hash_fields(label, values.iter().map(Vec::as_slice))
}

fn hash_fields<'a>(label: &[u8], values: impl IntoIterator<Item = &'a [u8]>) -> [u8; 32] {
    fn field(hasher: &mut blake3::Hasher, value: &[u8]) {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let mut hasher = blake3::Hasher::new();
    field(&mut hasher, HASH_DOMAIN);
    field(&mut hasher, label);
    for value in values {
        field(&mut hasher, value);
    }
    *hasher.finalize().as_bytes()
}

fn signing_message(payload_bytes: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGNATURE_DOMAIN.len() + payload_bytes.len());
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(payload_bytes);
    message
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn canonical_lower_hex(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_did(value: &str) -> bool {
    if !valid_text(value, MAX_DID_BYTES) {
        return false;
    }
    value
        .parse::<dialog_varsig::Did>()
        .is_ok_and(|did| did.to_string() == value)
}

fn canonical_http_base(value: &str) -> Result<Url, RecoveryManifestError> {
    if !valid_text(value, MAX_URL_BYTES) {
        return Err(RecoveryManifestError::Input("url"));
    }
    let url = Url::parse(value).map_err(|_| RecoveryManifestError::Input("url"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
        || url.as_str() != value
    {
        return Err(RecoveryManifestError::Input("url"));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(&url) => Ok(url),
        _ => Err(RecoveryManifestError::Input("url")),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, RecoveryManifestError> {
    serde_ipld_dagcbor::to_vec(value)
        .map_err(|error| RecoveryManifestError::Encoding(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, RecoveryManifestError> {
    serde_ipld_dagcbor::from_slice(bytes)
        .map_err(|error| RecoveryManifestError::Encoding(error.to_string()))
}

/// Recovery-manifest encoding, binding, or signature failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RecoveryManifestError {
    /// The signed manifest envelope exceeds its fixed V1 bound.
    #[error("recovery manifest exceeds 16384 bytes")]
    TooLarge,
    /// A named expected fact or artifact exceeds bounds or is non-canonical.
    #[error("invalid recovery manifest input: {0}")]
    Input(&'static str),
    /// DAG-CBOR encoding or decoding failed.
    #[error("invalid recovery manifest encoding: {0}")]
    Encoding(String),
    /// Decoded bytes were not the unique canonical representation.
    #[error("recovery manifest is not canonically encoded")]
    NonCanonical,
    /// The signed payload names an unsupported manifest version.
    #[error("unsupported recovery manifest version {0}")]
    UnsupportedVersion(u64),
    /// The expected/signed root is not the canonical Ed25519 subject.
    #[error("recovery manifest subject is invalid")]
    Subject,
    /// The root signature is absent, malformed, or invalid for this domain.
    #[error("recovery manifest signature is invalid")]
    Signature,
    /// A signed operation, deployment, identity, or artifact binding differs.
    #[error("recovery manifest does not match the staged setup bundle")]
    Binding,
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::{Principal as _, Signer as _};
    use serde_bytes::ByteBuf;

    use super::{
        AccountSetupRecoveryManifestInput, AccountSetupRecoveryManifestV1, Envelope, Payload,
        RecoveryManifestError, decode, encode, signing_message,
    };
    use crate::creation::{AccountCreationFingerprint, AccountCreationPasskey};

    #[derive(Clone)]
    struct Fixture {
        operation_id: String,
        ceremony_created_at: u64,
        provider: String,
        remote: String,
        service_did: Option<String>,
        root_did: String,
        device_did: String,
        credential_id: String,
        create_fingerprint: AccountCreationFingerprint,
        passkey_created_at: u64,
        passkey_created_on: String,
        encryption_recipient: Option<String>,
        custody_did: String,
        delegation: Vec<u8>,
        descriptor: Vec<u8>,
        create_invocation: Vec<u8>,
        deposits: Vec<Vec<u8>>,
        custody_consent: Vec<u8>,
        sealed_envelope: Vec<u8>,
        publish_invocation: Vec<u8>,
    }

    impl Fixture {
        fn input(&self) -> AccountSetupRecoveryManifestInput<'_> {
            AccountSetupRecoveryManifestInput {
                operation_id: &self.operation_id,
                ceremony_created_at: self.ceremony_created_at,
                provider: &self.provider,
                remote: &self.remote,
                service_did: self.service_did.as_deref(),
                root_did: &self.root_did,
                device_did: &self.device_did,
                credential_id: &self.credential_id,
                create_fingerprint: self.create_fingerprint,
                passkey: Some(AccountCreationPasskey {
                    created_at: self.passkey_created_at,
                    created_on: &self.passkey_created_on,
                }),
                encryption_recipient: self.encryption_recipient.as_deref(),
                custody_did: &self.custody_did,
                delegation: &self.delegation,
                descriptor: &self.descriptor,
                create_invocation: &self.create_invocation,
                deposits: &self.deposits,
                custody_consent: &self.custody_consent,
                sealed_envelope: &self.sealed_envelope,
                publish_invocation: &self.publish_invocation,
            }
        }
    }

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    async fn fixture() -> Fixture {
        let root = signer(7).await;
        Fixture {
            operation_id: "setup-01".to_string(),
            ceremony_created_at: 1_754_380_800,
            provider: "https://accounts.example/".to_string(),
            remote: "https://app.example/ucan/".to_string(),
            service_did: Some(signer(8).await.did().to_string()),
            root_did: root.did().to_string(),
            device_did: signer(9).await.did().to_string(),
            credential_id: "aabbccdd".to_string(),
            create_fingerprint: AccountCreationFingerprint::from_hex(&"11".repeat(32)).unwrap(),
            passkey_created_at: 1_754_380_800,
            passkey_created_on: "Chrome on macOS".to_string(),
            encryption_recipient: Some(
                "did:key:z6LSgZMqF1tKjEBAB9Kx4w3VmpQ8BzYVUmHgscBpHN3yP7Qq".to_string(),
            ),
            custody_did: signer(10).await.did().to_string(),
            delegation: vec![1, 2, 3],
            descriptor: vec![4, 5, 6],
            create_invocation: vec![7, 8, 9],
            deposits: vec![vec![10, 11], vec![12, 13]],
            custody_consent: vec![14, 15],
            sealed_envelope: vec![16, 17, 18],
            publish_invocation: vec![19, 20],
        }
    }

    #[dialog_common::test]
    async fn it_signs_deterministic_canonical_bytes_and_validates_every_binding() {
        let root = signer(7).await;
        let fixture = fixture().await;
        let first = AccountSetupRecoveryManifestV1::sign(&root, fixture.input())
            .await
            .unwrap();
        let second = AccountSetupRecoveryManifestV1::sign(&root, fixture.input())
            .await
            .unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(
            first.content_hash(),
            *blake3::hash(first.bytes()).as_bytes()
        );
        assert!(
            AccountSetupRecoveryManifestV1::validate(first.bytes(), fixture.input())
                .await
                .unwrap()
                == first
        );
    }

    #[dialog_common::test]
    async fn it_rejects_cross_bundle_mixing_and_every_mutable_binding() {
        let root = signer(7).await;
        let fixture = fixture().await;
        let manifest = AccountSetupRecoveryManifestV1::sign(&root, fixture.input())
            .await
            .unwrap();

        let mutations: &[fn(&mut Fixture)] = &[
            |value| value.operation_id.push('x'),
            |value| value.ceremony_created_at += 1,
            |value| value.provider = "https://other-accounts.example/".to_string(),
            |value| value.remote = "https://other-app.example/ucan/".to_string(),
            |value| value.service_did = None,
            |value| value.root_did = value.device_did.clone(),
            |value| value.device_did = value.custody_did.clone(),
            |value| value.credential_id.push_str("00"),
            |value| {
                value.create_fingerprint =
                    AccountCreationFingerprint::from_hex(&"22".repeat(32)).unwrap()
            },
            |value| value.passkey_created_at += 1,
            |value| value.passkey_created_on.push('x'),
            |value| value.encryption_recipient = None,
            |value| value.custody_did = value.device_did.clone(),
            |value| value.delegation.push(21),
            |value| value.descriptor.push(21),
            |value| value.create_invocation.push(21),
            |value| value.deposits.swap(0, 1),
            |value| value.custody_consent.push(21),
            |value| value.sealed_envelope.push(21),
            |value| value.publish_invocation.push(21),
        ];
        for mutate in mutations {
            let mut crossed = fixture.clone();
            mutate(&mut crossed);
            assert!(
                AccountSetupRecoveryManifestV1::validate(manifest.bytes(), crossed.input())
                    .await
                    .is_err(),
                "a changed manifest binding was accepted"
            );
        }
    }

    #[dialog_common::test]
    async fn it_rejects_oversize_inputs_before_hashing_or_decoding() {
        let root = signer(7).await;
        let fixture = fixture().await;

        let mut long_operation = fixture.clone();
        long_operation.operation_id = "x".repeat(129);
        assert!(
            AccountSetupRecoveryManifestV1::sign(&root, long_operation.input())
                .await
                .is_err()
        );

        let mut oversized_artifact = fixture.clone();
        oversized_artifact.create_invocation = vec![0; 128 * 1024 + 1];
        assert!(
            AccountSetupRecoveryManifestV1::sign(&root, oversized_artifact.input())
                .await
                .is_err()
        );

        let manifest = AccountSetupRecoveryManifestV1::sign(&root, fixture.input())
            .await
            .unwrap();
        let mut oversized_manifest = manifest.bytes().to_vec();
        oversized_manifest.resize(16 * 1024 + 1, 0);
        assert!(
            AccountSetupRecoveryManifestV1::validate(&oversized_manifest, fixture.input())
                .await
                .is_err()
        );
    }

    #[dialog_common::test]
    async fn it_rejects_future_versions_wrong_domains_signatures_and_noncanonical_bytes() {
        let root = signer(7).await;
        let fixture = fixture().await;
        let manifest = AccountSetupRecoveryManifestV1::sign(&root, fixture.input())
            .await
            .unwrap();
        let envelope: Envelope = decode(manifest.bytes()).unwrap();
        let mut payload: Payload = decode(envelope.0.as_ref()).unwrap();

        payload.0 = 99;
        let future_payload = encode(&payload).unwrap();
        let future_signature = root.sign(&signing_message(&future_payload)).await.unwrap();
        let future = encode(&(
            ByteBuf::from(future_payload),
            ByteBuf::from(future_signature.to_bytes().to_vec()),
        ))
        .unwrap();
        assert!(matches!(
            AccountSetupRecoveryManifestV1::validate(&future, fixture.input()).await,
            Err(RecoveryManifestError::UnsupportedVersion(99))
        ));

        payload.0 = 1;
        let payload = encode(&payload).unwrap();
        let mut wrong_domain_message = b"tonk/wrong-domain/v1\0".to_vec();
        wrong_domain_message.extend_from_slice(&payload);
        let wrong_domain_signature = root.sign(&wrong_domain_message).await.unwrap();
        let wrong_domain = encode(&(
            ByteBuf::from(payload.clone()),
            ByteBuf::from(wrong_domain_signature.to_bytes().to_vec()),
        ))
        .unwrap();
        assert!(matches!(
            AccountSetupRecoveryManifestV1::validate(&wrong_domain, fixture.input()).await,
            Err(RecoveryManifestError::Signature)
        ));

        let mut invalid_signature: Envelope = decode(manifest.bytes()).unwrap();
        invalid_signature.1[0] ^= 1;
        let invalid_signature = encode(&invalid_signature).unwrap();
        assert!(matches!(
            AccountSetupRecoveryManifestV1::validate(&invalid_signature, fixture.input()).await,
            Err(RecoveryManifestError::Signature)
        ));

        let mut trailing = manifest.bytes().to_vec();
        trailing.push(0);
        assert!(
            AccountSetupRecoveryManifestV1::validate(&trailing, fixture.input())
                .await
                .is_err()
        );
    }
}

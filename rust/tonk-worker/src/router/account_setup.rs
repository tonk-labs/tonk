//! Durable, exclusively owned browser account-setup saga.

use std::sync::Arc;

use dialog_common::Checksum;
use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::verification::Environment;
use dialog_ucan_core::{
    Delegation, DelegationChain, InvocationChain, UnverifiedRevocations, VerificationContext,
};
use dialog_varsig::{AnySignature, Did};
use serde::{Deserialize, Serialize};
use tonk_account::creation::{
    AccountCreationFingerprint, AccountCreationFingerprintInput, AccountCreationPasskey,
};
use tonk_account::{
    AccountRepositoryDescriptorV1, AccountSetupRecoveryManifestInput,
    AccountSetupRecoveryManifestV1,
};
use tonk_identity::clearance::Recovery;
use tonk_identity::custody::DEFERRED_PUBLISH_TTL_SECONDS;
use tonk_identity::envelope::{
    CUSTODY_SECRET_CELL, CUSTODY_SPACE, Envelope as RecoveryEnvelopeV2, KekMethod,
};
use tonk_worker_api::PasskeyMetadata;
use url::Url;

const ACCOUNT_SETUP_CHECKPOINT_SITE: &str = "tonk-account-setup-v2";
const ACCOUNT_SETUP_RECOVERY_SITE: &str = "tonk-account-setup-recovery-v1";
const CHECKPOINT_VERSION: u16 = 2;
const RECOVERY_VERSION: u16 = 1;
const MAX_CHECKPOINT_BYTES: usize = 32 * 1024;
const MAX_RECOVERY_RECORD_BYTES: usize = 1024 * 1024;
const MAX_OPERATION_BYTES: usize = 128;
const MAX_EMAIL_CHARS: usize = 320;
const MAX_URL_BYTES: usize = 2048;
const MAX_DID_BYTES: usize = 512;
const MAX_DEVICE_NAME_CHARS: usize = 120;
const MAX_CREDENTIAL_HEX_BYTES: usize = 4096;
const MAX_CID_BYTES: usize = 512;
const MAX_DELEGATION_BYTES: usize = 64 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 4096;
const MAX_INVOCATION_BYTES: usize = 128 * 1024;
const MAX_DEPOSIT_BYTES: usize = 64 * 1024;
const MAX_DEPOSITS: usize = 8;
const MAX_CONSENT_BYTES: usize = 64 * 1024;
const MAX_SEALED_BYTES: usize = 4096;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_DECODED_RECOVERY_BYTES: usize = 512 * 1024;
const STAGE_CLOCK_SKEW_SECONDS: u64 = 60;
const MAX_STAGE_DELAY_SECONDS: u64 = 60 * 60;
const CREATE_EXPIRY_MIN_OFFSET: u64 = 4 * 60;
const CREATE_EXPIRY_MAX_OFFSET: u64 = 6 * 60;
const PUBLISH_EXPIRY_MIN_OFFSET: u64 = DEFERRED_PUBLISH_TTL_SECONDS - 60;
const PUBLISH_EXPIRY_MAX_OFFSET: u64 = DEFERRED_PUBLISH_TTL_SECONDS + 6 * 60;
const SEALED_ACCOUNT_SECRET_BYTES: usize = 68;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StoredSafePhaseV2 {
    RecoveryStaged,
    RootSaved,
    ProviderAccepted,
    Attached,
    CustomerEnrolled,
    CustodyQueued,
}

impl StoredSafePhaseV2 {
    fn has_provider_acceptance(self) -> bool {
        matches!(
            self,
            Self::ProviderAccepted | Self::Attached | Self::CustomerEnrolled | Self::CustodyQueued
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum StoredConflictCodeV2 {
    RecoveryMismatch,
    LocalRootMismatch,
    ProviderMismatch,
    AttachmentMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredPhaseV2 {
    Leased,
    Armed,
    RecoveryStaged,
    RootSaved,
    ProviderAccepted,
    Attached,
    CustomerEnrolled,
    CustodyQueued,
    Complete,
    Cancelled,
    InterruptedBeforeRecovery,
    Conflict {
        last_safe_phase: StoredSafePhaseV2,
        code: StoredConflictCodeV2,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCheckpointV2 {
    version: u16,
    operation_id: String,
    revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bound_client_id: Option<String>,
    provider_hash: String,
    phase: StoredPhaseV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    armed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    staged_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    create_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted_descriptor_hash: Option<String>,
    last_transition_at: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ValidatedCheckpoint(StoredCheckpointV2);

impl ValidatedCheckpoint {
    fn new(checkpoint: StoredCheckpointV2) -> Result<Self, StoredSetupError> {
        validate_checkpoint(&checkpoint)?;
        Ok(Self(checkpoint))
    }

    fn as_stored(&self) -> &StoredCheckpointV2 {
        &self.0
    }

    fn into_stored(self) -> StoredCheckpointV2 {
        self.0
    }
}

/// Private durable recovery encoding. Wire DTO changes do not alter this
/// schema; Stage performs an explicit conversion before validation/storage.
/// This type deliberately has no `Debug` implementation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredRecoveryBundleV1 {
    version: u16,
    operation_id: String,
    ceremony_created_at: u64,
    staged_at: u64,
    normalized_email: String,
    provider: String,
    root_did: String,
    device_did: String,
    device_name: String,
    credential_id: String,
    delegation_cid: String,
    delegation_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passkey: Option<PasskeyMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encryption_key: Option<String>,
    descriptor_hex: String,
    create_fingerprint: String,
    invocation_hex: String,
    #[serde(default)]
    deposits_hex: Vec<String>,
    custody_did: String,
    consent_hex: String,
    sealed_hex: String,
    publish_invocation_hex: String,
    recovery_manifest_hex: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryTombstoneV1 {
    version: u16,
    operation_id: String,
    completed_at: u64,
    recovery_hash: String,
}

/// Protected recovery-site record. This deliberately has no `Debug`
/// implementation because the bundle contains PII and bounded authorizations.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "record",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredRecoveryRecord {
    Bundle(StoredRecoveryBundleV1),
    Tombstone(RecoveryTombstoneV1),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoredSetupError {
    TooLargeCheckpoint,
    MalformedCheckpoint,
    UnsupportedCheckpointVersion(u16),
    UnsupportedCheckpointPhase,
    InvalidCheckpoint,
    TooLargeRecovery,
    MalformedRecovery,
    UnsupportedRecoveryVersion(u16),
    UnsupportedRecoveryRecord,
    InvalidRecovery,
    Serialization,
}

#[derive(Deserialize)]
struct CheckpointEnvelope {
    version: u16,
    phase: PhaseEnvelope,
}

#[derive(Deserialize)]
struct PhaseEnvelope {
    kind: String,
}

#[derive(Deserialize)]
struct RecoveryEnvelope {
    version: u16,
    record: String,
}

fn encode_checkpoint(checkpoint: &ValidatedCheckpoint) -> Result<Vec<u8>, StoredSetupError> {
    let bytes =
        serde_json::to_vec(checkpoint.as_stored()).map_err(|_| StoredSetupError::Serialization)?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(StoredSetupError::TooLargeCheckpoint);
    }
    Ok(bytes)
}

fn decode_checkpoint(bytes: &[u8]) -> Result<ValidatedCheckpoint, StoredSetupError> {
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(StoredSetupError::TooLargeCheckpoint);
    }
    let envelope: CheckpointEnvelope =
        serde_json::from_slice(bytes).map_err(|_| StoredSetupError::MalformedCheckpoint)?;
    if envelope.version != CHECKPOINT_VERSION {
        return Err(StoredSetupError::UnsupportedCheckpointVersion(
            envelope.version,
        ));
    }
    if !matches!(
        envelope.phase.kind.as_str(),
        "leased"
            | "armed"
            | "recoveryStaged"
            | "rootSaved"
            | "providerAccepted"
            | "attached"
            | "customerEnrolled"
            | "custodyQueued"
            | "complete"
            | "cancelled"
            | "interruptedBeforeRecovery"
            | "conflict"
    ) {
        return Err(StoredSetupError::UnsupportedCheckpointPhase);
    }
    let checkpoint: StoredCheckpointV2 =
        serde_json::from_slice(bytes).map_err(|_| StoredSetupError::MalformedCheckpoint)?;
    ValidatedCheckpoint::new(checkpoint)
}

fn encode_recovery(record: &StoredRecoveryRecord) -> Result<Vec<u8>, StoredSetupError> {
    let bytes = serde_json::to_vec(record).map_err(|_| StoredSetupError::Serialization)?;
    if bytes.len() > MAX_RECOVERY_RECORD_BYTES {
        return Err(StoredSetupError::TooLargeRecovery);
    }
    Ok(bytes)
}

fn decode_recovery(bytes: &[u8]) -> Result<StoredRecoveryRecord, StoredSetupError> {
    if bytes.len() > MAX_RECOVERY_RECORD_BYTES {
        return Err(StoredSetupError::TooLargeRecovery);
    }
    let envelope: RecoveryEnvelope =
        serde_json::from_slice(bytes).map_err(|_| StoredSetupError::MalformedRecovery)?;
    if envelope.version != RECOVERY_VERSION {
        return Err(StoredSetupError::UnsupportedRecoveryVersion(
            envelope.version,
        ));
    }
    if !matches!(envelope.record.as_str(), "bundle" | "tombstone") {
        return Err(StoredSetupError::UnsupportedRecoveryRecord);
    }
    let record: StoredRecoveryRecord =
        serde_json::from_slice(bytes).map_err(|_| StoredSetupError::MalformedRecovery)?;
    if let StoredRecoveryRecord::Tombstone(tombstone) = &record
        && (!valid_identifier(&tombstone.operation_id, 128)
            || tombstone.completed_at == 0
            || !valid_hash(&tombstone.recovery_hash))
    {
        return Err(StoredSetupError::InvalidRecovery);
    }
    Ok(record)
}

fn validate_checkpoint(checkpoint: &StoredCheckpointV2) -> Result<(), StoredSetupError> {
    let owner_pair = checkpoint.owner_hash.is_some() == checkpoint.bound_client_id.is_some();
    let owner_valid = checkpoint.owner_hash.as_deref().is_none_or(valid_hash)
        && checkpoint
            .bound_client_id
            .as_deref()
            .is_none_or(|client| valid_identifier(client, 512));
    if checkpoint.version != CHECKPOINT_VERSION
        || !valid_identifier(&checkpoint.operation_id, 128)
        || checkpoint.revision == 0
        || !owner_pair
        || !owner_valid
        || !valid_hash(&checkpoint.provider_hash)
        || checkpoint.last_transition_at == 0
    {
        return Err(StoredSetupError::InvalidCheckpoint);
    }

    let has_owner = checkpoint.owner_hash.is_some();
    let armed_at_valid = checkpoint
        .armed_at
        .is_none_or(|armed_at| armed_at > 0 && armed_at <= checkpoint.last_transition_at);
    let staged_at_valid = checkpoint.staged_at.is_none_or(|staged_at| {
        checkpoint
            .armed_at
            .is_some_and(|armed_at| staged_at >= armed_at)
            && staged_at <= checkpoint.last_transition_at
    });
    if !armed_at_valid || !staged_at_valid {
        return Err(StoredSetupError::InvalidCheckpoint);
    }
    let has_armed_at = checkpoint.armed_at.is_some();
    let has_staged_at = checkpoint.staged_at.is_some();
    let attempt_valid = checkpoint.attempt_hash.as_deref().is_some_and(valid_hash);
    let staged = checkpoint
        .root_did
        .as_deref()
        .is_some_and(|root| valid_identifier(root, 512))
        && checkpoint
            .create_fingerprint
            .as_deref()
            .is_some_and(valid_hash)
        && checkpoint.recovery_hash.as_deref().is_some_and(valid_hash);
    let has_any_staged = checkpoint.root_did.is_some()
        || checkpoint.create_fingerprint.is_some()
        || checkpoint.recovery_hash.is_some();
    let accepted = checkpoint
        .accepted_descriptor_hash
        .as_deref()
        .is_some_and(valid_hash);

    let legal = match checkpoint.phase {
        StoredPhaseV2::Leased => {
            !has_armed_at
                && !has_staged_at
                && checkpoint.attempt_hash.is_none()
                && !has_any_staged
                && checkpoint.accepted_descriptor_hash.is_none()
        }
        StoredPhaseV2::Armed => {
            has_armed_at
                && !has_staged_at
                && has_owner
                && attempt_valid
                && !has_any_staged
                && checkpoint.accepted_descriptor_hash.is_none()
        }
        StoredPhaseV2::RecoveryStaged | StoredPhaseV2::RootSaved => {
            has_armed_at
                && has_staged_at
                && checkpoint.attempt_hash.is_none()
                && staged
                && checkpoint.accepted_descriptor_hash.is_none()
        }
        StoredPhaseV2::ProviderAccepted
        | StoredPhaseV2::Attached
        | StoredPhaseV2::CustomerEnrolled
        | StoredPhaseV2::CustodyQueued => {
            has_armed_at && has_staged_at && checkpoint.attempt_hash.is_none() && staged && accepted
        }
        StoredPhaseV2::Complete => {
            has_armed_at
                && has_staged_at
                && !has_owner
                && checkpoint.attempt_hash.is_none()
                && staged
                && accepted
        }
        StoredPhaseV2::Cancelled => {
            !has_armed_at
                && !has_staged_at
                && !has_owner
                && checkpoint.attempt_hash.is_none()
                && !has_any_staged
                && checkpoint.accepted_descriptor_hash.is_none()
        }
        StoredPhaseV2::InterruptedBeforeRecovery => {
            has_armed_at
                && !has_staged_at
                && !has_owner
                && checkpoint.attempt_hash.is_none()
                && !has_any_staged
                && checkpoint.accepted_descriptor_hash.is_none()
        }
        StoredPhaseV2::Conflict {
            last_safe_phase, ..
        } => {
            has_armed_at
                && has_staged_at
                && !has_owner
                && checkpoint.attempt_hash.is_none()
                && staged
                && (accepted == last_safe_phase.has_provider_acceptance())
        }
    };
    legal
        .then_some(())
        .ok_or(StoredSetupError::InvalidCheckpoint)
}

fn valid_identifier(value: &str, max_chars: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max_chars && !value.chars().any(char::is_control)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Deployment and immutable clock facts selected by the worker, never the
/// requesting page.
#[derive(Clone, PartialEq, Eq)]
struct RecoveryTrustContext {
    operation_id: String,
    provider: Url,
    remote: Url,
    service_did: Option<Did>,
    device_did: Did,
    armed_at: u64,
    now: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryFreshness {
    Usable,
    NeedsRefresh,
}

/// The only recovery value post-stage effects may consume. It deliberately
/// has no `Debug` implementation because it owns PII and bounded authority.
struct ValidatedRecoveryBundle {
    stored: StoredRecoveryBundleV1,
    root_did: Did,
    delegation: DelegationChain,
    descriptor: AccountRepositoryDescriptorV1,
    create_invocation: InvocationChain<AnySignature>,
    deposits: Vec<Delegation<AnySignature>>,
    consent: DelegationChain,
    sealed: RecoveryEnvelopeV2<Recovery>,
    publish_invocation: InvocationChain<AnySignature>,
    manifest: AccountSetupRecoveryManifestV1,
    recovery_hash: String,
    create_expires_at: u64,
    publish_expires_at: u64,
    create_freshness: RecoveryFreshness,
    publish_freshness: RecoveryFreshness,
}

impl ValidatedRecoveryBundle {
    async fn new(
        stored: StoredRecoveryBundleV1,
        trust: &RecoveryTrustContext,
    ) -> Result<Self, RecoveryValidationError> {
        let decoded = decode_bounded_recovery(&stored)?;
        validate_trusted_recovery(&stored, trust)?;
        validate_stage_timestamps(
            trust.armed_at,
            stored.ceremony_created_at,
            stored.staged_at,
            trust.now,
        )?;

        let root_did: Did = stored
            .root_did
            .parse()
            .map_err(|_| RecoveryValidationError::Invalid("root_did"))?;
        if root_did.to_string() != stored.root_did {
            return Err(RecoveryValidationError::Invalid("root_did"));
        }
        let delegation = validate_stable_grant(
            &decoded.delegation,
            &stored.delegation_cid,
            &root_did,
            &trust.device_did,
        )
        .await?;
        let descriptor = AccountRepositoryDescriptorV1::validate(&decoded.descriptor)
            .await
            .map_err(|_| RecoveryValidationError::Artifact("descriptor"))?;
        if descriptor.account_subject() != &root_did || descriptor.remote() != &trust.remote {
            return Err(RecoveryValidationError::Trusted("descriptor"));
        }

        let passkey = stored
            .passkey
            .as_ref()
            .ok_or(RecoveryValidationError::Invalid("passkey"))?;
        let create_invocation =
            validate_create_invocation(&decoded.create_invocation, &stored, &root_did, passkey)
                .await?;
        let create_expires_at = create_invocation
            .invocation
            .expiration()
            .ok_or(RecoveryValidationError::Artifact("create_expiration"))?
            .to_unix();
        validate_original_expiration(
            stored.ceremony_created_at,
            create_expires_at,
            CREATE_EXPIRY_MIN_OFFSET,
            CREATE_EXPIRY_MAX_OFFSET,
        )?;

        let fingerprint = AccountCreationFingerprint::from_hex(&stored.create_fingerprint)
            .map_err(|_| RecoveryValidationError::Invalid("create_fingerprint"))?;
        let recomputed = AccountCreationFingerprintInput {
            email: &stored.normalized_email,
            root_did: &stored.root_did,
            credential_id: &stored.credential_id,
            passkey: Some(AccountCreationPasskey {
                created_at: passkey.created_at,
                created_on: &passkey.created_on,
            }),
            descriptor: &decoded.descriptor,
            device_did: &stored.device_did,
            device_name: &stored.device_name,
            delegation_cid: &stored.delegation_cid,
            delegation: &decoded.delegation,
        }
        .fingerprint();
        if fingerprint != recomputed {
            return Err(RecoveryValidationError::Artifact("create_fingerprint"));
        }

        if let Some(encryption_key) = &stored.encryption_key {
            let did: Did = encryption_key
                .parse()
                .map_err(|_| RecoveryValidationError::Invalid("encryption_key"))?;
            if did.to_string() != *encryption_key
                || tonk_identity::sealed::RecipientKey::from_did(&did).is_err()
            {
                return Err(RecoveryValidationError::Invalid("encryption_key"));
            }
        }

        let deposits =
            validate_deposits(&decoded.deposits, &root_did, trust.service_did.as_ref()).await?;
        let custody_did: Did = stored
            .custody_did
            .parse()
            .map_err(|_| RecoveryValidationError::Invalid("custody_did"))?;
        if custody_did.to_string() != stored.custody_did {
            return Err(RecoveryValidationError::Invalid("custody_did"));
        }
        let consent = validate_custody_consent(&decoded.consent, &custody_did, &root_did).await?;
        let sealed = validate_sealed_envelope(&decoded.sealed)?;
        let publish_invocation = validate_publish_invocation(
            &decoded.publish_invocation,
            &custody_did,
            &decoded.sealed,
            stored.ceremony_created_at,
        )
        .await?;
        let publish_expires_at = publish_invocation
            .invocation
            .expiration()
            .ok_or(RecoveryValidationError::Artifact("publish_expiration"))?
            .to_unix();
        validate_original_expiration(
            stored.ceremony_created_at,
            publish_expires_at,
            PUBLISH_EXPIRY_MIN_OFFSET,
            PUBLISH_EXPIRY_MAX_OFFSET,
        )?;

        let service_did = trust.service_did.as_ref().map(ToString::to_string);
        let manifest = AccountSetupRecoveryManifestV1::validate(
            &decoded.manifest,
            AccountSetupRecoveryManifestInput {
                operation_id: &stored.operation_id,
                ceremony_created_at: stored.ceremony_created_at,
                provider: trust.provider.as_str(),
                remote: trust.remote.as_str(),
                service_did: service_did.as_deref(),
                root_did: &stored.root_did,
                device_did: &stored.device_did,
                credential_id: &stored.credential_id,
                create_fingerprint: fingerprint,
                passkey: Some(AccountCreationPasskey {
                    created_at: passkey.created_at,
                    created_on: &passkey.created_on,
                }),
                encryption_recipient: stored.encryption_key.as_deref(),
                custody_did: &stored.custody_did,
                delegation: &decoded.delegation,
                descriptor: &decoded.descriptor,
                create_invocation: &decoded.create_invocation,
                deposits: &decoded.deposits,
                custody_consent: &decoded.consent,
                sealed_envelope: &decoded.sealed,
                publish_invocation: &decoded.publish_invocation,
            },
        )
        .await
        .map_err(|_| RecoveryValidationError::Artifact("recovery_manifest"))?;

        let record_bytes = encode_recovery(&StoredRecoveryRecord::Bundle(stored.clone()))
            .map_err(|_| RecoveryValidationError::Invalid("recovery_record"))?;
        let recovery_hash = blake3::hash(&record_bytes).to_hex().to_string();
        Ok(Self {
            stored,
            root_did,
            delegation,
            descriptor,
            create_invocation,
            deposits,
            consent,
            sealed,
            publish_invocation,
            manifest,
            recovery_hash,
            create_expires_at,
            publish_expires_at,
            create_freshness: freshness(create_expires_at, trust.now),
            publish_freshness: freshness(publish_expires_at, trust.now),
        })
    }

    fn root_did(&self) -> &Did {
        &self.root_did
    }

    fn create_expires_at(&self) -> u64 {
        self.create_expires_at
    }

    fn create_freshness(&self) -> RecoveryFreshness {
        self.create_freshness
    }

    fn publish_freshness(&self) -> RecoveryFreshness {
        self.publish_freshness
    }

    fn evidence(&self) -> RecoveryEvidence {
        RecoveryEvidence {
            staged_at: self.stored.staged_at,
            root_did: self.stored.root_did.clone(),
            create_fingerprint: self.stored.create_fingerprint.clone(),
            recovery_hash: self.recovery_hash.clone(),
        }
    }
}

struct DecodedRecovery {
    delegation: Vec<u8>,
    descriptor: Vec<u8>,
    create_invocation: Vec<u8>,
    deposits: Vec<Vec<u8>>,
    consent: Vec<u8>,
    sealed: Vec<u8>,
    publish_invocation: Vec<u8>,
    manifest: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
enum RecoveryValidationError {
    #[error("recovery field exceeds its fixed bound: {0}")]
    TooLarge(&'static str),
    #[error("recovery field is invalid or non-canonical: {0}")]
    Invalid(&'static str),
    #[error("recovery field differs from trusted deployment state: {0}")]
    Trusted(&'static str),
    #[error("recovery artifact failed semantic validation: {0}")]
    Artifact(&'static str),
    #[error("recovery timestamps are outside the setup window")]
    Timestamp,
}

fn decode_bounded_recovery(
    stored: &StoredRecoveryBundleV1,
) -> Result<DecodedRecovery, RecoveryValidationError> {
    if stored.version != RECOVERY_VERSION
        || !bounded_text(&stored.operation_id, MAX_OPERATION_BYTES)
        || !bounded_text(&stored.normalized_email, MAX_EMAIL_CHARS)
        || stored.normalized_email.to_lowercase() != stored.normalized_email
        || !bounded_text(&stored.provider, MAX_URL_BYTES)
        || !bounded_text(&stored.root_did, MAX_DID_BYTES)
        || !bounded_text(&stored.device_did, MAX_DID_BYTES)
        || !bounded_text(&stored.device_name, MAX_DEVICE_NAME_CHARS)
        || !bounded_text(&stored.delegation_cid, MAX_CID_BYTES)
        || !bounded_text(&stored.custody_did, MAX_DID_BYTES)
    {
        return Err(RecoveryValidationError::Invalid("text"));
    }
    if !canonical_hex_text(&stored.credential_id, MAX_CREDENTIAL_HEX_BYTES) {
        return Err(RecoveryValidationError::Invalid("credential_id"));
    }
    if stored.ceremony_created_at == 0 || stored.staged_at == 0 {
        return Err(RecoveryValidationError::Invalid("timestamp"));
    }
    let passkey = stored
        .passkey
        .as_ref()
        .ok_or(RecoveryValidationError::Invalid("passkey"))?;
    if passkey.created_at != stored.ceremony_created_at
        || passkey.created_on.trim() != passkey.created_on
        || !bounded_text(&passkey.created_on, 120)
    {
        return Err(RecoveryValidationError::Invalid("passkey"));
    }
    if stored
        .encryption_key
        .as_deref()
        .is_some_and(|value| !bounded_text(value, MAX_DID_BYTES))
    {
        return Err(RecoveryValidationError::Invalid("encryption_key"));
    }
    if !valid_hash(&stored.create_fingerprint) {
        return Err(RecoveryValidationError::Invalid("create_fingerprint"));
    }
    if stored.deposits_hex.len() > MAX_DEPOSITS {
        return Err(RecoveryValidationError::TooLarge("deposits"));
    }

    let delegation = decode_hex_field("delegation", &stored.delegation_hex, MAX_DELEGATION_BYTES)?;
    let descriptor = decode_hex_field("descriptor", &stored.descriptor_hex, MAX_DESCRIPTOR_BYTES)?;
    let create_invocation =
        decode_hex_field("invocation", &stored.invocation_hex, MAX_INVOCATION_BYTES)?;
    let deposits = stored
        .deposits_hex
        .iter()
        .map(|value| decode_hex_field("deposit", value, MAX_DEPOSIT_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let consent = decode_hex_field("consent", &stored.consent_hex, MAX_CONSENT_BYTES)?;
    let sealed = decode_hex_field("sealed", &stored.sealed_hex, MAX_SEALED_BYTES)?;
    let publish_invocation = decode_hex_field(
        "publish_invocation",
        &stored.publish_invocation_hex,
        MAX_INVOCATION_BYTES,
    )?;
    let manifest = decode_hex_field(
        "recovery_manifest",
        &stored.recovery_manifest_hex,
        MAX_MANIFEST_BYTES,
    )?;
    let total = [
        delegation.len(),
        descriptor.len(),
        create_invocation.len(),
        consent.len(),
        sealed.len(),
        publish_invocation.len(),
        manifest.len(),
    ]
    .into_iter()
    .chain(deposits.iter().map(Vec::len))
    .try_fold(0usize, usize::checked_add)
    .ok_or(RecoveryValidationError::TooLarge("decoded_total"))?;
    if total > MAX_DECODED_RECOVERY_BYTES {
        return Err(RecoveryValidationError::TooLarge("decoded_total"));
    }
    let encoded = serde_json::to_vec(&StoredRecoveryRecord::Bundle(stored.clone()))
        .map_err(|_| RecoveryValidationError::Invalid("recovery_record"))?;
    if encoded.len() > MAX_RECOVERY_RECORD_BYTES {
        return Err(RecoveryValidationError::TooLarge("recovery_record"));
    }
    Ok(DecodedRecovery {
        delegation,
        descriptor,
        create_invocation,
        deposits,
        consent,
        sealed,
        publish_invocation,
        manifest,
    })
}

fn decode_hex_field(
    name: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, RecoveryValidationError> {
    if value.len() > max_bytes.saturating_mul(2) {
        return Err(RecoveryValidationError::TooLarge(name));
    }
    if !canonical_hex_text(value, max_bytes.saturating_mul(2)) {
        return Err(RecoveryValidationError::Invalid(name));
    }
    hex::decode(value).map_err(|_| RecoveryValidationError::Invalid(name))
}

fn canonical_hex_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_chars
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max_chars && !value.chars().any(char::is_control)
}

fn validate_trusted_recovery(
    stored: &StoredRecoveryBundleV1,
    trust: &RecoveryTrustContext,
) -> Result<(), RecoveryValidationError> {
    if stored.operation_id != trust.operation_id
        || stored.device_did != trust.device_did.to_string()
    {
        return Err(RecoveryValidationError::Trusted("operation_or_device"));
    }
    let provider: Url = stored
        .provider
        .parse()
        .map_err(|_| RecoveryValidationError::Invalid("provider"))?;
    if provider.as_str() != stored.provider || provider != trust.provider {
        return Err(RecoveryValidationError::Trusted("provider"));
    }
    Ok(())
}

fn validate_stage_timestamps(
    armed_at: u64,
    ceremony_created_at: u64,
    staged_at: u64,
    now: u64,
) -> Result<(), RecoveryValidationError> {
    let earliest = armed_at
        .checked_sub(STAGE_CLOCK_SKEW_SECONDS)
        .ok_or(RecoveryValidationError::Timestamp)?;
    let latest = staged_at
        .checked_add(STAGE_CLOCK_SKEW_SECONDS)
        .ok_or(RecoveryValidationError::Timestamp)?;
    let latest_stage = ceremony_created_at
        .checked_add(MAX_STAGE_DELAY_SECONDS)
        .ok_or(RecoveryValidationError::Timestamp)?;
    if staged_at < armed_at
        || now < staged_at
        || ceremony_created_at < earliest
        || ceremony_created_at > latest
        || staged_at > latest_stage
    {
        return Err(RecoveryValidationError::Timestamp);
    }
    Ok(())
}

fn validate_original_expiration(
    created_at: u64,
    expires_at: u64,
    min_offset: u64,
    max_offset: u64,
) -> Result<(), RecoveryValidationError> {
    let earliest = created_at
        .checked_add(min_offset)
        .ok_or(RecoveryValidationError::Timestamp)?;
    let latest = created_at
        .checked_add(max_offset)
        .ok_or(RecoveryValidationError::Timestamp)?;
    if expires_at < earliest || expires_at > latest {
        return Err(RecoveryValidationError::Artifact("expiration_window"));
    }
    Ok(())
}

fn freshness(expires_at: u64, now: u64) -> RecoveryFreshness {
    if expires_at < now {
        RecoveryFreshness::NeedsRefresh
    } else {
        RecoveryFreshness::Usable
    }
}

async fn validate_stable_grant(
    bytes: &[u8],
    expected_cid: &str,
    root: &Did,
    device: &Did,
) -> Result<DelegationChain, RecoveryValidationError> {
    let chain = DelegationChain::try_from(bytes)
        .map_err(|_| RecoveryValidationError::Artifact("delegation"))?;
    if chain
        .to_bytes()
        .map_err(|_| RecoveryValidationError::Artifact("delegation"))?
        != bytes
        || chain.proof_cids().len() != 1
        || chain.proof_cids()[0].to_string() != expected_cid
    {
        return Err(RecoveryValidationError::Artifact("delegation"));
    }
    let proof = chain
        .proofs()
        .next()
        .ok_or(RecoveryValidationError::Artifact("delegation"))?;
    if proof.issuer() != root
        || proof.audience() != device
        || proof.subject() != &Subject::Any
        || !proof.command().segments().is_empty()
        || !proof.policy().is_empty()
        || proof.expiration().is_some()
        || proof.not_before().is_some()
        || !proof.meta().is_empty()
        || nonce_len(proof.nonce()) != 16
    {
        return Err(RecoveryValidationError::Artifact("delegation"));
    }
    proof
        .verify_signature(&DidKeyResolver)
        .await
        .map_err(|_| RecoveryValidationError::Artifact("delegation"))?;
    Ok(chain)
}

async fn validate_create_invocation(
    bytes: &[u8],
    stored: &StoredRecoveryBundleV1,
    root: &Did,
    passkey: &PasskeyMetadata,
) -> Result<InvocationChain<AnySignature>, RecoveryValidationError> {
    let chain = parse_canonical_self_invocation(bytes, root, &["account", "create"], "create")?;
    let arguments = chain.arguments();
    if arguments.len() != 8
        || promised_string(arguments.get("email")) != Some(stored.normalized_email.as_str())
        || promised_string(arguments.get("credentialId")) != Some(stored.credential_id.as_str())
        || promised_string(arguments.get("deviceDid")) != Some(stored.device_did.as_str())
        || promised_string(arguments.get("deviceName")) != Some(stored.device_name.as_str())
        || promised_string(arguments.get("delegation")) != Some(stored.delegation_hex.as_str())
        || promised_string(arguments.get("repositoryDescriptor"))
            != Some(stored.descriptor_hex.as_str())
        || arguments.get("passkeyCreatedAt")
            != Some(&Promised::Integer(i128::from(passkey.created_at)))
        || promised_string(arguments.get("passkeyCreatedOn")) != Some(passkey.created_on.as_str())
    {
        return Err(RecoveryValidationError::Artifact("create_arguments"));
    }
    verify_invocation_at(&chain, stored.ceremony_created_at, "create").await?;
    Ok(chain)
}

async fn validate_deposits(
    bytes: &[Vec<u8>],
    root: &Did,
    service: Option<&Did>,
) -> Result<Vec<Delegation<AnySignature>>, RecoveryValidationError> {
    let Some(service) = service else {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(RecoveryValidationError::Trusted("deposits"));
    };
    let expected = tonk_account::customer::deposit_scopes(root, service);
    if bytes.len() != expected.len() {
        return Err(RecoveryValidationError::Artifact("deposits"));
    }

    let mut deposits = Vec::with_capacity(bytes.len());
    for (raw, scope) in bytes.iter().zip(expected) {
        let deposit_chain = DelegationChain::from_delegation_bytes(vec![raw.clone()])
            .map_err(|_| RecoveryValidationError::Artifact("deposit"))?;
        let deposit = deposit_chain
            .proofs()
            .next()
            .ok_or(RecoveryValidationError::Artifact("deposit"))?
            .clone();
        if deposit.encoded() != raw
            || deposit.issuer() != root
            || deposit.audience() != service
            || deposit.subject() != &scope.subject
            || deposit.command().segments() != scope.command.segments()
            || deposit.policy() != &scope.policy()
            || deposit.expiration().is_some()
            || deposit.not_before().is_some()
            || !deposit.meta().is_empty()
            || nonce_len(deposit.nonce()) != 16
        {
            return Err(RecoveryValidationError::Artifact("deposit"));
        }
        deposit
            .verify_signature(&DidKeyResolver)
            .await
            .map_err(|_| RecoveryValidationError::Artifact("deposit"))?;
        deposits.push(deposit);
    }
    Ok(deposits)
}

async fn validate_custody_consent(
    bytes: &[u8],
    custody: &Did,
    root: &Did,
) -> Result<DelegationChain, RecoveryValidationError> {
    let chain = DelegationChain::try_from(bytes)
        .map_err(|_| RecoveryValidationError::Artifact("custody_consent"))?;
    if chain
        .to_bytes()
        .map_err(|_| RecoveryValidationError::Artifact("custody_consent"))?
        != bytes
        || chain.proof_cids().len() != 1
    {
        return Err(RecoveryValidationError::Artifact("custody_consent"));
    }
    let proof = chain
        .proofs()
        .next()
        .ok_or(RecoveryValidationError::Artifact("custody_consent"))?;
    if proof.issuer() != custody
        || proof.audience() != root
        || proof.subject() != &Subject::Specific(custody.clone())
        || !proof.command().segments().is_empty()
        || !proof.policy().is_empty()
        || proof.expiration().is_some()
        || proof.not_before().is_some()
        || !proof.meta().is_empty()
        || nonce_len(proof.nonce()) != 16
    {
        return Err(RecoveryValidationError::Artifact("custody_consent"));
    }
    proof
        .verify_signature(&DidKeyResolver)
        .await
        .map_err(|_| RecoveryValidationError::Artifact("custody_consent"))?;
    Ok(chain)
}

fn validate_sealed_envelope(
    bytes: &[u8],
) -> Result<RecoveryEnvelopeV2<Recovery>, RecoveryValidationError> {
    if bytes.len() != SEALED_ACCOUNT_SECRET_BYTES {
        return Err(RecoveryValidationError::Artifact("sealed"));
    }
    let envelope = RecoveryEnvelopeV2::<Recovery>::decode(bytes)
        .map_err(|_| RecoveryValidationError::Artifact("sealed"))?;
    if envelope.encode() != bytes
        || envelope.generation != 0
        || envelope.method != KekMethod::Passkey
        || envelope.ciphertext().len() != 48
    {
        return Err(RecoveryValidationError::Artifact("sealed"));
    }
    Ok(envelope)
}

async fn validate_publish_invocation(
    bytes: &[u8],
    custody: &Did,
    sealed: &[u8],
    ceremony_created_at: u64,
) -> Result<InvocationChain<AnySignature>, RecoveryValidationError> {
    let chain = parse_canonical_self_invocation(bytes, custody, &["memory", "publish"], "publish")?;
    let arguments = chain.arguments();
    let checksum = match arguments.get("checksum") {
        Some(Promised::Bytes(bytes)) => Checksum::try_from(bytes.clone())
            .map_err(|_| RecoveryValidationError::Artifact("publish_checksum"))?,
        _ => return Err(RecoveryValidationError::Artifact("publish_arguments")),
    };
    if arguments.len() != 3
        || promised_string(arguments.get("space")) != Some(CUSTODY_SPACE)
        || promised_string(arguments.get("cell")) != Some(CUSTODY_SECRET_CELL)
        || checksum != Checksum::sha256(sealed)
        || arguments.contains_key("when")
    {
        return Err(RecoveryValidationError::Artifact("publish_arguments"));
    }
    verify_invocation_at(&chain, ceremony_created_at, "publish").await?;
    Ok(chain)
}

fn parse_canonical_self_invocation(
    bytes: &[u8],
    principal: &Did,
    command: &[&str],
    name: &'static str,
) -> Result<InvocationChain<AnySignature>, RecoveryValidationError> {
    let chain =
        InvocationChain::try_from(bytes).map_err(|_| RecoveryValidationError::Artifact(name))?;
    if chain
        .to_bytes()
        .map_err(|_| RecoveryValidationError::Artifact(name))?
        != bytes
        || chain.issuer() != principal
        || chain.subject() != principal
        || chain.invocation.audience() != principal
        || chain
            .command()
            .segments()
            .iter()
            .map(String::as_str)
            .ne(command.iter().copied())
        || !chain.proofs().is_empty()
        || !proof_store_is_empty(&chain)
        || chain.invocation.cause().is_some()
        || !chain.invocation.meta().is_empty()
        || nonce_len(chain.invocation.nonce()) != 16
        || chain.invocation.expiration().is_none()
    {
        return Err(RecoveryValidationError::Artifact(name));
    }
    Ok(chain)
}

async fn verify_invocation_at(
    chain: &InvocationChain<AnySignature>,
    ceremony_created_at: u64,
    name: &'static str,
) -> Result<(), RecoveryValidationError> {
    // Check at the end of the accepted ceremony-skew window. This verifies
    // signatures, `iat`, and `exp` against the immutable signed ceremony
    // reference without requiring a currently unexpired invocation.
    let verify_at = ceremony_created_at
        .checked_add(STAGE_CLOCK_SKEW_SECONDS)
        .ok_or(RecoveryValidationError::Timestamp)?;
    let time = UNIX_EPOCH
        .checked_add(Duration::from_secs(verify_at))
        .ok_or(RecoveryValidationError::Timestamp)?;
    let timestamp = Timestamp::new(time).map_err(|_| RecoveryValidationError::Timestamp)?;
    let environment: Environment<_, _, _, Arc<Delegation<AnySignature>>> =
        Environment::new(chain.proof_store(), DidKeyResolver, UnverifiedRevocations);
    chain
        .verify(&VerificationContext::at(&environment, Some(timestamp)))
        .await
        .map_err(|_| RecoveryValidationError::Artifact(name))?;
    Ok(())
}

fn proof_store_is_empty(chain: &InvocationChain<AnySignature>) -> bool {
    chain
        .proof_store()
        .lock()
        .is_ok_and(|proofs| proofs.is_empty())
}

fn nonce_len(nonce: &dialog_ucan_core::crypto::nonce::Nonce) -> usize {
    Vec::<u8>::from(nonce.clone()).len()
}

fn promised_string(value: Option<&Promised>) -> Option<&str> {
    match value {
        Some(Promised::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct MutationContext {
    operation_id: String,
    owner_hash: String,
    client_id: String,
    expected_revision: u64,
    now: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct RecoveryEvidence {
    staged_at: u64,
    root_did: String,
    create_fingerprint: String,
    recovery_hash: String,
}

impl RecoveryEvidence {
    fn is_valid(&self) -> bool {
        self.staged_at > 0
            && valid_identifier(&self.root_did, 512)
            && valid_hash(&self.create_fingerprint)
            && valid_hash(&self.recovery_hash)
    }
}

#[derive(Clone, PartialEq, Eq)]
enum RecoveryObservation {
    Absent,
    Staged(RecoveryEvidence),
}

#[derive(Clone, PartialEq, Eq)]
enum VerifiedEvidence {
    RecoveryStaged(RecoveryEvidence),
    LocalRootSaved,
    ProviderAccepted { descriptor_hash: String },
    AttachmentSaved,
    CustomerEnrolled,
    CustodyQueued,
    CompletionRecorded,
}

#[derive(Clone, PartialEq, Eq)]
enum ReducerCommand {
    Arm {
        mutation: MutationContext,
        attempt_hash: String,
    },
    Cancel {
        mutation: MutationContext,
    },
    Acquire {
        operation_id: String,
        new_owner_hash: String,
        new_client_id: String,
        expected_revision: u64,
        recovery: RecoveryObservation,
        now: u64,
    },
    OwnerAbsent {
        expected_revision: u64,
        observed_client_id: String,
        recovery: RecoveryObservation,
        now: u64,
    },
    Observe {
        mutation: MutationContext,
        evidence: VerifiedEvidence,
    },
    Conflict {
        mutation: MutationContext,
        code: StoredConflictCodeV2,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DurableAction {
    None,
    SaveCheckpoint,
    SaveCheckpointBeforeRecoveryTombstone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateNextAction {
    ApprovePasskey,
    AwaitPasskeyResult,
    PersistLocalRoot,
    QueryProviderStatus,
    PersistAttachment,
    EnrollCustomer,
    QueueCustody,
    RecordCompletion,
    TombstoneRecovery,
    Acquire,
    Wait,
    CancelTooLate,
    StartOver,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReductionError {
    WrongOperation,
    StaleRevision,
    Unauthorized,
    InvalidTimestamp,
    InvalidTransition,
    InvalidEvidence,
    InvalidResult,
}

#[derive(Clone, PartialEq, Eq)]
struct Reduction {
    checkpoint: ValidatedCheckpoint,
    durable_action: DurableAction,
    next_action: PrivateNextAction,
}

fn reduce(
    checkpoint: ValidatedCheckpoint,
    command: ReducerCommand,
) -> Result<Reduction, ReductionError> {
    match command {
        ReducerCommand::Arm {
            mutation,
            attempt_hash,
        } => {
            authenticate(&checkpoint, &mutation)?;
            if checkpoint.as_stored().phase != StoredPhaseV2::Leased || !valid_hash(&attempt_hash) {
                return Err(ReductionError::InvalidTransition);
            }
            transition(
                checkpoint,
                mutation.now,
                |next| {
                    next.phase = StoredPhaseV2::Armed;
                    next.armed_at = Some(mutation.now);
                    next.attempt_hash = Some(attempt_hash);
                },
                DurableAction::SaveCheckpoint,
                PrivateNextAction::AwaitPasskeyResult,
            )
        }
        ReducerCommand::Cancel { mutation } => {
            authenticate(&checkpoint, &mutation)?;
            match checkpoint.as_stored().phase {
                StoredPhaseV2::Leased => transition(
                    checkpoint,
                    mutation.now,
                    |next| {
                        next.phase = StoredPhaseV2::Cancelled;
                        clear_owner(next);
                    },
                    DurableAction::SaveCheckpoint,
                    PrivateNextAction::Done,
                ),
                StoredPhaseV2::Armed => {
                    reduction_without_write(checkpoint, PrivateNextAction::CancelTooLate)
                }
                _ => Err(ReductionError::InvalidTransition),
            }
        }
        ReducerCommand::Acquire {
            operation_id,
            new_owner_hash,
            new_client_id,
            expected_revision,
            recovery,
            now,
        } => {
            let stored = checkpoint.as_stored();
            if stored.operation_id != operation_id {
                return Err(ReductionError::WrongOperation);
            }
            if stored.revision != expected_revision {
                return Err(ReductionError::StaleRevision);
            }
            if stored.owner_hash.is_some()
                || !valid_hash(&new_owner_hash)
                || !valid_identifier(&new_client_id, 512)
            {
                return Err(ReductionError::Unauthorized);
            }
            if !matches!(
                stored.phase,
                StoredPhaseV2::Leased
                    | StoredPhaseV2::RecoveryStaged
                    | StoredPhaseV2::RootSaved
                    | StoredPhaseV2::ProviderAccepted
                    | StoredPhaseV2::Attached
                    | StoredPhaseV2::CustomerEnrolled
                    | StoredPhaseV2::CustodyQueued
            ) {
                return Err(ReductionError::InvalidTransition);
            }
            require_recovery_observation(stored, &recovery)?;
            let next_action = next_action_for_phase(&stored.phase);
            transition(
                checkpoint,
                now,
                |next| {
                    next.owner_hash = Some(new_owner_hash);
                    next.bound_client_id = Some(new_client_id);
                },
                DurableAction::SaveCheckpoint,
                next_action,
            )
        }
        ReducerCommand::OwnerAbsent {
            expected_revision,
            observed_client_id,
            recovery,
            now,
        } => {
            let stored = checkpoint.as_stored();
            if stored.revision != expected_revision {
                return Err(ReductionError::StaleRevision);
            }
            if stored.bound_client_id.as_deref() != Some(observed_client_id.as_str()) {
                return Err(ReductionError::Unauthorized);
            }
            match (&stored.phase, &recovery) {
                (StoredPhaseV2::Armed, RecoveryObservation::Absent) => transition(
                    checkpoint,
                    now,
                    |next| {
                        next.phase = StoredPhaseV2::InterruptedBeforeRecovery;
                        next.attempt_hash = None;
                        clear_owner(next);
                    },
                    DurableAction::SaveCheckpoint,
                    PrivateNextAction::StartOver,
                ),
                (StoredPhaseV2::Armed, RecoveryObservation::Staged(evidence))
                    if evidence.is_valid() =>
                {
                    let evidence = evidence.clone();
                    transition(
                        checkpoint,
                        now,
                        |next| {
                            apply_staged_recovery(next, evidence);
                            clear_owner(next);
                        },
                        DurableAction::SaveCheckpoint,
                        PrivateNextAction::Acquire,
                    )
                }
                (StoredPhaseV2::Leased, RecoveryObservation::Absent) => transition(
                    checkpoint,
                    now,
                    clear_owner,
                    DurableAction::SaveCheckpoint,
                    PrivateNextAction::Acquire,
                ),
                (
                    StoredPhaseV2::RecoveryStaged
                    | StoredPhaseV2::RootSaved
                    | StoredPhaseV2::ProviderAccepted
                    | StoredPhaseV2::Attached
                    | StoredPhaseV2::CustomerEnrolled
                    | StoredPhaseV2::CustodyQueued,
                    RecoveryObservation::Staged(_),
                ) => {
                    require_recovery_observation(stored, &recovery)?;
                    transition(
                        checkpoint,
                        now,
                        clear_owner,
                        DurableAction::SaveCheckpoint,
                        PrivateNextAction::Acquire,
                    )
                }
                _ => Err(ReductionError::InvalidEvidence),
            }
        }
        ReducerCommand::Observe { mutation, evidence } => {
            authenticate(&checkpoint, &mutation)?;
            reduce_evidence(checkpoint, mutation.now, evidence)
        }
        ReducerCommand::Conflict { mutation, code } => {
            authenticate(&checkpoint, &mutation)?;
            let last_safe_phase = safe_phase(&checkpoint.as_stored().phase)
                .ok_or(ReductionError::InvalidTransition)?;
            transition(
                checkpoint,
                mutation.now,
                |next| {
                    next.phase = StoredPhaseV2::Conflict {
                        last_safe_phase,
                        code,
                    };
                    next.attempt_hash = None;
                    clear_owner(next);
                },
                DurableAction::SaveCheckpoint,
                PrivateNextAction::Done,
            )
        }
    }
}

fn authenticate(
    checkpoint: &ValidatedCheckpoint,
    mutation: &MutationContext,
) -> Result<(), ReductionError> {
    let stored = checkpoint.as_stored();
    if stored.operation_id != mutation.operation_id {
        return Err(ReductionError::WrongOperation);
    }
    if stored.revision != mutation.expected_revision {
        return Err(ReductionError::StaleRevision);
    }
    if stored.owner_hash.as_deref() != Some(mutation.owner_hash.as_str())
        || stored.bound_client_id.as_deref() != Some(mutation.client_id.as_str())
    {
        return Err(ReductionError::Unauthorized);
    }
    if mutation.now < stored.last_transition_at {
        return Err(ReductionError::InvalidTimestamp);
    }
    Ok(())
}

fn reduce_evidence(
    checkpoint: ValidatedCheckpoint,
    now: u64,
    evidence: VerifiedEvidence,
) -> Result<Reduction, ReductionError> {
    match (checkpoint.as_stored().phase.clone(), evidence) {
        (StoredPhaseV2::Armed, VerifiedEvidence::RecoveryStaged(evidence))
            if evidence.is_valid() =>
        {
            transition(
                checkpoint,
                now,
                |next| {
                    apply_staged_recovery(next, evidence);
                },
                DurableAction::SaveCheckpoint,
                PrivateNextAction::PersistLocalRoot,
            )
        }
        (StoredPhaseV2::RecoveryStaged, VerifiedEvidence::LocalRootSaved) => transition(
            checkpoint,
            now,
            |next| next.phase = StoredPhaseV2::RootSaved,
            DurableAction::SaveCheckpoint,
            PrivateNextAction::QueryProviderStatus,
        ),
        (StoredPhaseV2::RootSaved, VerifiedEvidence::ProviderAccepted { descriptor_hash })
            if valid_hash(&descriptor_hash) =>
        {
            transition(
                checkpoint,
                now,
                |next| {
                    next.phase = StoredPhaseV2::ProviderAccepted;
                    next.accepted_descriptor_hash = Some(descriptor_hash);
                },
                DurableAction::SaveCheckpoint,
                PrivateNextAction::PersistAttachment,
            )
        }
        (StoredPhaseV2::ProviderAccepted, VerifiedEvidence::AttachmentSaved) => transition(
            checkpoint,
            now,
            |next| next.phase = StoredPhaseV2::Attached,
            DurableAction::SaveCheckpoint,
            PrivateNextAction::EnrollCustomer,
        ),
        (StoredPhaseV2::Attached, VerifiedEvidence::CustomerEnrolled) => transition(
            checkpoint,
            now,
            |next| next.phase = StoredPhaseV2::CustomerEnrolled,
            DurableAction::SaveCheckpoint,
            PrivateNextAction::QueueCustody,
        ),
        (StoredPhaseV2::CustomerEnrolled, VerifiedEvidence::CustodyQueued) => transition(
            checkpoint,
            now,
            |next| next.phase = StoredPhaseV2::CustodyQueued,
            DurableAction::SaveCheckpoint,
            PrivateNextAction::RecordCompletion,
        ),
        (StoredPhaseV2::CustodyQueued, VerifiedEvidence::CompletionRecorded) => transition(
            checkpoint,
            now,
            |next| {
                next.phase = StoredPhaseV2::Complete;
                clear_owner(next);
            },
            DurableAction::SaveCheckpointBeforeRecoveryTombstone,
            PrivateNextAction::TombstoneRecovery,
        ),
        _ => Err(ReductionError::InvalidTransition),
    }
}

fn safe_phase(phase: &StoredPhaseV2) -> Option<StoredSafePhaseV2> {
    match phase {
        StoredPhaseV2::RecoveryStaged => Some(StoredSafePhaseV2::RecoveryStaged),
        StoredPhaseV2::RootSaved => Some(StoredSafePhaseV2::RootSaved),
        StoredPhaseV2::ProviderAccepted => Some(StoredSafePhaseV2::ProviderAccepted),
        StoredPhaseV2::Attached => Some(StoredSafePhaseV2::Attached),
        StoredPhaseV2::CustomerEnrolled => Some(StoredSafePhaseV2::CustomerEnrolled),
        StoredPhaseV2::CustodyQueued => Some(StoredSafePhaseV2::CustodyQueued),
        _ => None,
    }
}

fn transition(
    checkpoint: ValidatedCheckpoint,
    now: u64,
    update: impl FnOnce(&mut StoredCheckpointV2),
    durable_action: DurableAction,
    next_action: PrivateNextAction,
) -> Result<Reduction, ReductionError> {
    let mut next = checkpoint.into_stored();
    if now < next.last_transition_at {
        return Err(ReductionError::InvalidTimestamp);
    }
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or(ReductionError::InvalidResult)?;
    next.last_transition_at = now;
    update(&mut next);
    let checkpoint = ValidatedCheckpoint::new(next).map_err(|_| ReductionError::InvalidResult)?;
    Ok(Reduction {
        checkpoint,
        durable_action,
        next_action,
    })
}

fn reduction_without_write(
    checkpoint: ValidatedCheckpoint,
    next_action: PrivateNextAction,
) -> Result<Reduction, ReductionError> {
    ValidatedCheckpoint::new(checkpoint.as_stored().clone())
        .map_err(|_| ReductionError::InvalidResult)?;
    Ok(Reduction {
        checkpoint,
        durable_action: DurableAction::None,
        next_action,
    })
}

fn clear_owner(checkpoint: &mut StoredCheckpointV2) {
    checkpoint.owner_hash = None;
    checkpoint.bound_client_id = None;
}

fn apply_staged_recovery(checkpoint: &mut StoredCheckpointV2, evidence: RecoveryEvidence) {
    checkpoint.phase = StoredPhaseV2::RecoveryStaged;
    checkpoint.attempt_hash = None;
    checkpoint.staged_at = Some(evidence.staged_at);
    checkpoint.root_did = Some(evidence.root_did);
    checkpoint.create_fingerprint = Some(evidence.create_fingerprint);
    checkpoint.recovery_hash = Some(evidence.recovery_hash);
}

fn require_recovery_observation(
    checkpoint: &StoredCheckpointV2,
    observation: &RecoveryObservation,
) -> Result<(), ReductionError> {
    match (&checkpoint.phase, observation) {
        (StoredPhaseV2::Leased, RecoveryObservation::Absent) => Ok(()),
        (
            StoredPhaseV2::RecoveryStaged
            | StoredPhaseV2::RootSaved
            | StoredPhaseV2::ProviderAccepted
            | StoredPhaseV2::Attached
            | StoredPhaseV2::CustomerEnrolled
            | StoredPhaseV2::CustodyQueued,
            RecoveryObservation::Staged(evidence),
        ) if evidence.is_valid()
            && checkpoint.staged_at == Some(evidence.staged_at)
            && checkpoint.root_did.as_deref() == Some(evidence.root_did.as_str())
            && checkpoint.create_fingerprint.as_deref()
                == Some(evidence.create_fingerprint.as_str())
            && checkpoint.recovery_hash.as_deref() == Some(evidence.recovery_hash.as_str()) =>
        {
            Ok(())
        }
        _ => Err(ReductionError::InvalidEvidence),
    }
}

fn next_action_for_phase(phase: &StoredPhaseV2) -> PrivateNextAction {
    match phase {
        StoredPhaseV2::Leased => PrivateNextAction::ApprovePasskey,
        StoredPhaseV2::RecoveryStaged => PrivateNextAction::PersistLocalRoot,
        StoredPhaseV2::RootSaved => PrivateNextAction::QueryProviderStatus,
        StoredPhaseV2::ProviderAccepted => PrivateNextAction::PersistAttachment,
        StoredPhaseV2::Attached => PrivateNextAction::EnrollCustomer,
        StoredPhaseV2::CustomerEnrolled => PrivateNextAction::QueueCustody,
        StoredPhaseV2::CustodyQueued => PrivateNextAction::RecordCompletion,
        StoredPhaseV2::Armed => PrivateNextAction::AwaitPasskeyResult,
        StoredPhaseV2::Complete => PrivateNextAction::TombstoneRecovery,
        StoredPhaseV2::Cancelled
        | StoredPhaseV2::InterruptedBeforeRecovery
        | StoredPhaseV2::Conflict { .. } => PrivateNextAction::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACCOUNT_SETUP_CHECKPOINT_SITE, ACCOUNT_SETUP_RECOVERY_SITE, CREATE_EXPIRY_MAX_OFFSET,
        CREATE_EXPIRY_MIN_OFFSET, DurableAction, MAX_RECOVERY_RECORD_BYTES, MutationContext,
        PUBLISH_EXPIRY_MAX_OFFSET, PUBLISH_EXPIRY_MIN_OFFSET, PrivateNextAction, RecoveryEvidence,
        RecoveryFreshness, RecoveryObservation, RecoveryTrustContext, RecoveryValidationError,
        ReducerCommand, ReductionError, StoredCheckpointV2, StoredConflictCodeV2, StoredPhaseV2,
        StoredRecoveryBundleV1, StoredRecoveryRecord, StoredSafePhaseV2, StoredSetupError,
        ValidatedCheckpoint, ValidatedRecoveryBundle, VerifiedEvidence, decode_bounded_recovery,
        decode_checkpoint, decode_recovery, encode_checkpoint, encode_recovery, reduce,
        validate_original_expiration, validate_stage_timestamps,
    };
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
    use dialog_varsig::Principal as _;
    use tonk_account::creation::{AccountCreationFingerprintInput, AccountCreationPasskey};
    use tonk_account::{AccountSetupRecoveryManifestInput, AccountSetupRecoveryManifestV1};
    use tonk_identity::ceremony::{PasskeyCreationMetadata, mint_service_deposits};
    use tonk_identity::custody::{
        DEFERRED_PUBLISH_TTL_SECONDS, build_publish_invocation, mint_custody_consent,
    };
    use tonk_identity::envelope::{AccountSecret, KekMethod, custody_kek, custody_signer};
    use tonk_worker_api::PasskeyMetadata;

    fn hash(byte: &str) -> String {
        byte.repeat(32)
    }

    fn leased() -> ValidatedCheckpoint {
        ValidatedCheckpoint::new(StoredCheckpointV2 {
            version: 2,
            operation_id: "setup-1".to_string(),
            revision: 1,
            owner_hash: Some(hash("11")),
            bound_client_id: Some("client-1".to_string()),
            provider_hash: hash("22"),
            phase: StoredPhaseV2::Leased,
            armed_at: None,
            staged_at: None,
            attempt_hash: None,
            root_did: None,
            create_fingerprint: None,
            recovery_hash: None,
            accepted_descriptor_hash: None,
            last_transition_at: 1_754_380_800,
        })
        .unwrap()
    }

    fn mutation(checkpoint: &ValidatedCheckpoint, now: u64) -> MutationContext {
        MutationContext {
            operation_id: "setup-1".to_string(),
            owner_hash: hash("11"),
            client_id: "client-1".to_string(),
            expected_revision: checkpoint.as_stored().revision,
            now,
        }
    }

    fn bundle() -> StoredRecoveryBundleV1 {
        StoredRecoveryBundleV1 {
            version: 1,
            operation_id: "setup-1".to_string(),
            ceremony_created_at: 1_754_380_800,
            staged_at: 1_754_380_802,
            normalized_email: "person@example.com".to_string(),
            provider: "https://accounts.example/".to_string(),
            root_did: "did:key:root".to_string(),
            device_did: "did:key:device".to_string(),
            device_name: "Jack's laptop".to_string(),
            credential_id: "aabb".to_string(),
            delegation_cid: "bafydelegation".to_string(),
            delegation_hex: "ccdd".to_string(),
            passkey: Some(PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".to_string(),
            }),
            encryption_key: Some("did:key:z6LSrecipient".to_string()),
            descriptor_hex: "eeff".to_string(),
            create_fingerprint: hash("33"),
            invocation_hex: "2233".to_string(),
            deposits_hex: vec!["4455".to_string()],
            custody_did: "did:key:custody".to_string(),
            consent_hex: "6677".to_string(),
            sealed_hex: "8899".to_string(),
            publish_invocation_hex: "aabb".to_string(),
            recovery_manifest_hex: "bbcc".to_string(),
        }
    }

    fn recovery_evidence() -> RecoveryEvidence {
        RecoveryEvidence {
            staged_at: 1_754_380_802,
            root_did: "did:key:root".to_string(),
            create_fingerprint: hash("44"),
            recovery_hash: hash("55"),
        }
    }

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    async fn valid_recovery(seed: u8) -> (StoredRecoveryBundleV1, RecoveryTrustContext) {
        let secret = AccountSecret::generate().unwrap();
        let root = secret.signer().await.unwrap();
        let device = signer(seed).await;
        let service = signer(seed.wrapping_add(1)).await;
        let entry = [seed.wrapping_add(2); 32];
        let custody = custody_signer(&entry).await.unwrap();
        let root_did = root.did().to_string();
        let device_did = device.did().to_string();
        let service_did = service.did().to_string();
        let custody_did = custody.did().to_string();
        let ceremony_created_at = Timestamp::now().to_unix();
        let armed_at = ceremony_created_at - 1;
        let staged_at = ceremony_created_at + 1;
        let provider: url::Url = "https://accounts.example/".parse().unwrap();
        let remote: url::Url = "https://app.example/ucan/".parse().unwrap();
        let operation_id = format!("setup-{seed}");
        let credential_id = format!("{seed:02x}").repeat(16);
        let device_name = format!("Test device {seed}");
        let email = format!("person-{seed}@example.com");
        let passkey = PasskeyMetadata {
            created_at: ceremony_created_at,
            created_on: "Chrome on macOS".to_string(),
        };

        let delegation =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
                .await
                .unwrap();
        let delegation_cid = delegation.proof_cids()[0].to_string();
        let delegation_bytes = delegation.to_bytes().unwrap();
        let delegation_hex = hex::encode(&delegation_bytes);
        let account = tonk_identity::ceremony::create_account(
            root.clone(),
            email.clone(),
            credential_id.clone(),
            device.did(),
            device_name.clone(),
            delegation_hex.clone(),
            remote.to_string(),
            Some(PasskeyCreationMetadata {
                created_at: passkey.created_at,
                created_on: passkey.created_on.clone(),
            }),
        )
        .await
        .unwrap();
        let descriptor_hex = account.descriptor_hex.unwrap();
        let descriptor = hex::decode(&descriptor_hex).unwrap();
        let create_invocation = hex::decode(&account.invocation_hex).unwrap();
        let create_fingerprint = AccountCreationFingerprintInput {
            email: &email,
            root_did: &root_did,
            credential_id: &credential_id,
            passkey: Some(AccountCreationPasskey {
                created_at: passkey.created_at,
                created_on: &passkey.created_on,
            }),
            descriptor: &descriptor,
            device_did: &device_did,
            device_name: &device_name,
            delegation_cid: &delegation_cid,
            delegation: &delegation_bytes,
        }
        .fingerprint();
        let deposits_hex = mint_service_deposits(&root, &service.did()).await.unwrap();
        let deposits = deposits_hex
            .iter()
            .map(|value| hex::decode(value).unwrap())
            .collect::<Vec<_>>();
        let consent = mint_custody_consent(custody.clone(), &root.did())
            .await
            .unwrap();
        let consent_bytes = consent.to_bytes().unwrap();
        let sealed = custody_kek(&entry)
            .seal(&secret, KekMethod::Passkey)
            .unwrap()
            .encode();
        let publish_expiration = Timestamp::new(
            UNIX_EPOCH + Duration::from_secs(ceremony_created_at + DEFERRED_PUBLISH_TTL_SECONDS),
        )
        .unwrap();
        let publish_invocation =
            build_publish_invocation(custody.clone(), &sealed, None, publish_expiration)
                .await
                .unwrap();
        let encryption_key = secret.encryption_key().recipient().did().to_string();
        let manifest = AccountSetupRecoveryManifestV1::sign(
            &root,
            AccountSetupRecoveryManifestInput {
                operation_id: &operation_id,
                ceremony_created_at,
                provider: provider.as_str(),
                remote: remote.as_str(),
                service_did: Some(&service_did),
                root_did: &root_did,
                device_did: &device_did,
                credential_id: &credential_id,
                create_fingerprint,
                passkey: Some(AccountCreationPasskey {
                    created_at: passkey.created_at,
                    created_on: &passkey.created_on,
                }),
                encryption_recipient: Some(&encryption_key),
                custody_did: &custody_did,
                delegation: &delegation_bytes,
                descriptor: &descriptor,
                create_invocation: &create_invocation,
                deposits: &deposits,
                custody_consent: &consent_bytes,
                sealed_envelope: &sealed,
                publish_invocation: &publish_invocation,
            },
        )
        .await
        .unwrap();

        let bundle = StoredRecoveryBundleV1 {
            version: 1,
            operation_id: operation_id.clone(),
            ceremony_created_at,
            staged_at,
            normalized_email: email,
            provider: provider.to_string(),
            root_did,
            device_did,
            device_name,
            credential_id,
            delegation_cid,
            delegation_hex,
            passkey: Some(passkey),
            encryption_key: Some(encryption_key),
            descriptor_hex,
            create_fingerprint: create_fingerprint.to_hex(),
            invocation_hex: account.invocation_hex,
            deposits_hex,
            custody_did,
            consent_hex: hex::encode(consent_bytes),
            sealed_hex: hex::encode(sealed),
            publish_invocation_hex: hex::encode(publish_invocation),
            recovery_manifest_hex: hex::encode(manifest.bytes()),
        };
        let trust = RecoveryTrustContext {
            operation_id,
            provider,
            remote,
            service_did: Some(service.did()),
            device_did: device.did(),
            armed_at,
            now: staged_at + 1,
        };
        (bundle, trust)
    }

    #[dialog_common::test]
    fn it_decodes_private_v2_records_and_classifies_future_shapes_as_unsupported() {
        assert_eq!(ACCOUNT_SETUP_CHECKPOINT_SITE, "tonk-account-setup-v2");
        assert_eq!(
            ACCOUNT_SETUP_RECOVERY_SITE,
            "tonk-account-setup-recovery-v1"
        );

        let checkpoint = leased();
        let bytes = encode_checkpoint(&checkpoint).unwrap();
        assert!(decode_checkpoint(&bytes).unwrap() == checkpoint);
        let mut invalid_armed_timestamp = checkpoint.as_stored().clone();
        invalid_armed_timestamp.armed_at = Some(0);
        assert!(matches!(
            ValidatedCheckpoint::new(invalid_armed_timestamp),
            Err(StoredSetupError::InvalidCheckpoint)
        ));

        let armed = reduce(
            leased(),
            ReducerCommand::Arm {
                mutation: mutation(&leased(), 1_754_380_801),
                attempt_hash: hash("33"),
            },
        )
        .unwrap()
        .checkpoint;
        let staged = reduce(
            armed.clone(),
            ReducerCommand::Observe {
                mutation: mutation(&armed, 1_754_380_802),
                evidence: VerifiedEvidence::RecoveryStaged(recovery_evidence()),
            },
        )
        .unwrap()
        .checkpoint;
        let mut staged_before_arm = staged.as_stored().clone();
        staged_before_arm.staged_at = staged_before_arm.armed_at.map(|armed_at| armed_at - 1);
        assert!(matches!(
            ValidatedCheckpoint::new(staged_before_arm),
            Err(StoredSetupError::InvalidCheckpoint)
        ));

        let future = br#"{
            "version":99,
            "phase":{"kind":"quantumRecovery","newField":true},
            "futureRequiredField":{"nested":[1,2,3]}
        }"#;
        assert!(matches!(
            decode_checkpoint(future),
            Err(StoredSetupError::UnsupportedCheckpointVersion(99))
        ));
        assert!(matches!(
            decode_checkpoint(br#"{"version":2,"phase":{"kind":"future"}}"#),
            Err(StoredSetupError::UnsupportedCheckpointPhase)
        ));

        let recovery = StoredRecoveryRecord::Bundle(bundle());
        let bytes = encode_recovery(&recovery).unwrap();
        assert!(decode_recovery(&bytes).unwrap() == recovery);
        let future = br#"{
            "record":"futureBundle",
            "version":99,
            "futureProtectedShape":{"ciphertext":"opaque"}
        }"#;
        assert!(matches!(
            decode_recovery(future),
            Err(StoredSetupError::UnsupportedRecoveryVersion(99))
        ));
        assert!(matches!(
            decode_recovery(br#"{"record":"futureBundle","version":1,"opaque":true}"#),
            Err(StoredSetupError::UnsupportedRecoveryRecord)
        ));
    }

    #[dialog_common::test]
    fn it_reduces_complete_records_and_owns_revision_timestamp_and_field_clearing() {
        let leased_checkpoint = leased();
        let armed = reduce(
            leased_checkpoint.clone(),
            ReducerCommand::Arm {
                mutation: mutation(&leased_checkpoint, 1_754_380_801),
                attempt_hash: hash("33"),
            },
        )
        .unwrap();
        let stored = armed.checkpoint.as_stored();
        assert_eq!(stored.phase, StoredPhaseV2::Armed);
        assert_eq!(stored.revision, 2);
        assert_eq!(stored.last_transition_at, 1_754_380_801);
        assert_eq!(stored.armed_at, Some(1_754_380_801));
        assert_eq!(stored.attempt_hash.as_deref(), Some(hash("33").as_str()));
        assert_eq!(armed.durable_action, DurableAction::SaveCheckpoint);
        assert_eq!(armed.next_action, PrivateNextAction::AwaitPasskeyResult);

        let cancel = reduce(
            armed.checkpoint.clone(),
            ReducerCommand::Cancel {
                mutation: mutation(&armed.checkpoint, 1_754_380_802),
            },
        )
        .unwrap();
        assert!(cancel.checkpoint == armed.checkpoint);
        assert_eq!(cancel.durable_action, DurableAction::None);
        assert_eq!(cancel.next_action, PrivateNextAction::CancelTooLate);

        let interrupted = reduce(
            armed.checkpoint,
            ReducerCommand::OwnerAbsent {
                expected_revision: 2,
                observed_client_id: "client-1".to_string(),
                recovery: RecoveryObservation::Absent,
                now: 1_754_380_803,
            },
        )
        .unwrap();
        let stored = interrupted.checkpoint.as_stored();
        assert_eq!(stored.phase, StoredPhaseV2::InterruptedBeforeRecovery);
        assert_eq!(stored.revision, 3);
        assert!(stored.owner_hash.is_none());
        assert!(stored.bound_client_id.is_none());
        assert!(stored.attempt_hash.is_none());
        assert_eq!(stored.armed_at, Some(1_754_380_801));
        assert_eq!(interrupted.next_action, PrivateNextAction::StartOver);
    }

    #[dialog_common::test]
    fn it_accepts_only_the_next_typed_evidence_and_returns_the_next_durable_action() {
        let armed = reduce(
            leased(),
            ReducerCommand::Arm {
                mutation: mutation(&leased(), 1_754_380_801),
                attempt_hash: hash("33"),
            },
        )
        .unwrap()
        .checkpoint;

        let staged = reduce(
            armed.clone(),
            ReducerCommand::Observe {
                mutation: mutation(&armed, 1_754_380_802),
                evidence: VerifiedEvidence::RecoveryStaged(recovery_evidence()),
            },
        )
        .unwrap();
        assert_eq!(
            staged.checkpoint.as_stored().phase,
            StoredPhaseV2::RecoveryStaged
        );
        assert!(staged.checkpoint.as_stored().attempt_hash.is_none());
        assert_eq!(staged.next_action, PrivateNextAction::PersistLocalRoot);

        assert!(matches!(
            reduce(
                staged.checkpoint.clone(),
                ReducerCommand::Observe {
                    mutation: mutation(&staged.checkpoint, 1_754_380_803),
                    evidence: VerifiedEvidence::ProviderAccepted {
                        descriptor_hash: hash("66"),
                    },
                },
            ),
            Err(ReductionError::InvalidTransition)
        ));

        let mut checkpoint = staged.checkpoint;
        let evidence = [
            VerifiedEvidence::LocalRootSaved,
            VerifiedEvidence::ProviderAccepted {
                descriptor_hash: hash("66"),
            },
            VerifiedEvidence::AttachmentSaved,
            VerifiedEvidence::CustomerEnrolled,
            VerifiedEvidence::CustodyQueued,
            VerifiedEvidence::CompletionRecorded,
        ];
        let phases = [
            StoredPhaseV2::RootSaved,
            StoredPhaseV2::ProviderAccepted,
            StoredPhaseV2::Attached,
            StoredPhaseV2::CustomerEnrolled,
            StoredPhaseV2::CustodyQueued,
            StoredPhaseV2::Complete,
        ];
        let next_actions = [
            PrivateNextAction::QueryProviderStatus,
            PrivateNextAction::PersistAttachment,
            PrivateNextAction::EnrollCustomer,
            PrivateNextAction::QueueCustody,
            PrivateNextAction::RecordCompletion,
            PrivateNextAction::TombstoneRecovery,
        ];
        for ((evidence, phase), next) in evidence.into_iter().zip(phases).zip(next_actions) {
            let now = checkpoint.as_stored().last_transition_at + 1;
            let reduction = reduce(
                checkpoint.clone(),
                ReducerCommand::Observe {
                    mutation: mutation(&checkpoint, now),
                    evidence,
                },
            )
            .unwrap();
            assert_eq!(reduction.checkpoint.as_stored().phase, phase);
            assert_eq!(reduction.next_action, next);
            checkpoint = reduction.checkpoint;
        }
        assert!(checkpoint.as_stored().owner_hash.is_none());
        assert!(checkpoint.as_stored().bound_client_id.is_none());
    }

    #[dialog_common::test]
    fn it_records_only_staged_conflicts_with_provenance_and_a_stable_code() {
        let partial = StoredCheckpointV2 {
            version: 2,
            operation_id: "setup-1".to_string(),
            revision: 4,
            owner_hash: None,
            bound_client_id: None,
            provider_hash: hash("22"),
            phase: StoredPhaseV2::Conflict {
                last_safe_phase: StoredSafePhaseV2::RootSaved,
                code: StoredConflictCodeV2::ProviderMismatch,
            },
            armed_at: Some(1_754_380_801),
            staged_at: Some(1_754_380_802),
            attempt_hash: None,
            root_did: None,
            create_fingerprint: None,
            recovery_hash: None,
            accepted_descriptor_hash: None,
            last_transition_at: 1_754_380_900,
        };
        assert!(ValidatedCheckpoint::new(partial).is_err());

        let armed = reduce(
            leased(),
            ReducerCommand::Arm {
                mutation: mutation(&leased(), 1_754_380_801),
                attempt_hash: hash("33"),
            },
        )
        .unwrap()
        .checkpoint;
        assert!(matches!(
            reduce(
                armed.clone(),
                ReducerCommand::Conflict {
                    mutation: mutation(&armed, 1_754_380_802),
                    code: StoredConflictCodeV2::ProviderMismatch,
                },
            ),
            Err(ReductionError::InvalidTransition)
        ));
    }

    #[dialog_common::test]
    fn it_repairs_a_staged_bundle_behind_armed_and_never_rebinds_an_unstaged_attempt() {
        let armed = reduce(
            leased(),
            ReducerCommand::Arm {
                mutation: mutation(&leased(), 1_754_380_801),
                attempt_hash: hash("33"),
            },
        )
        .unwrap()
        .checkpoint;

        let repaired = reduce(
            armed,
            ReducerCommand::OwnerAbsent {
                expected_revision: 2,
                observed_client_id: "client-1".to_string(),
                recovery: RecoveryObservation::Staged(recovery_evidence()),
                now: 1_754_380_802,
            },
        )
        .unwrap();
        let stored = repaired.checkpoint.as_stored();
        assert_eq!(stored.phase, StoredPhaseV2::RecoveryStaged);
        assert!(stored.owner_hash.is_none());
        assert!(stored.bound_client_id.is_none());
        assert!(stored.attempt_hash.is_none());
        assert_eq!(stored.root_did.as_deref(), Some("did:key:root"));
        assert_eq!(stored.staged_at, Some(1_754_380_802));
        assert_eq!(repaired.next_action, PrivateNextAction::Acquire);
    }

    #[dialog_common::test]
    fn it_requires_exact_recovery_presence_for_release_and_takeover() {
        let leased = leased();
        let released = reduce(
            leased,
            ReducerCommand::OwnerAbsent {
                expected_revision: 1,
                observed_client_id: "client-1".to_string(),
                recovery: RecoveryObservation::Absent,
                now: 1_754_380_801,
            },
        )
        .unwrap()
        .checkpoint;
        assert!(released.as_stored().owner_hash.is_none());

        assert!(matches!(
            reduce(
                released.clone(),
                ReducerCommand::Acquire {
                    operation_id: "setup-1".to_string(),
                    new_owner_hash: hash("77"),
                    new_client_id: "client-2".to_string(),
                    expected_revision: 2,
                    recovery: RecoveryObservation::Staged(recovery_evidence()),
                    now: 1_754_380_802,
                },
            ),
            Err(ReductionError::InvalidEvidence)
        ));

        let acquired = reduce(
            released,
            ReducerCommand::Acquire {
                operation_id: "setup-1".to_string(),
                new_owner_hash: hash("77"),
                new_client_id: "client-2".to_string(),
                expected_revision: 2,
                recovery: RecoveryObservation::Absent,
                now: 1_754_380_802,
            },
        )
        .unwrap();
        assert_eq!(acquired.next_action, PrivateNextAction::ApprovePasskey);
        assert_eq!(acquired.checkpoint.as_stored().revision, 3);
        assert_eq!(
            acquired.checkpoint.as_stored().bound_client_id.as_deref(),
            Some("client-2")
        );
    }

    #[dialog_common::test]
    async fn it_constructs_one_semantically_validated_bundle_from_exact_signed_artifacts() {
        let (bundle, trust) = valid_recovery(41).await;
        let validated = ValidatedRecoveryBundle::new(bundle.clone(), &trust)
            .await
            .unwrap();
        assert_eq!(validated.create_freshness(), RecoveryFreshness::Usable);
        assert_eq!(validated.publish_freshness(), RecoveryFreshness::Usable);
        assert_eq!(validated.root_did().to_string(), bundle.root_did);
        assert_eq!(validated.evidence().staged_at, bundle.staged_at);
    }

    #[dialog_common::test]
    async fn it_rejects_mutated_cross_record_and_untrusted_deployment_bundles() {
        let (bundle, trust) = valid_recovery(42).await;
        let mut mutations = Vec::new();

        let mut changed = bundle.clone();
        changed.operation_id.push('x');
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.normalized_email.push('x');
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.credential_id.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.delegation_cid.push('x');
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.delegation_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.descriptor_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.invocation_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.deposits_hex.swap(0, 1);
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.consent_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.sealed_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.publish_invocation_hex.push_str("00");
        mutations.push(changed);
        let mut changed = bundle.clone();
        changed.recovery_manifest_hex.push_str("00");
        mutations.push(changed);

        for changed in mutations {
            assert!(
                ValidatedRecoveryBundle::new(changed, &trust).await.is_err(),
                "mutated protected record was accepted"
            );
        }

        let (other, _) = valid_recovery(43).await;
        let mut crossed = bundle.clone();
        crossed.custody_did = other.custody_did;
        crossed.consent_hex = other.consent_hex;
        crossed.sealed_hex = other.sealed_hex;
        crossed.publish_invocation_hex = other.publish_invocation_hex;
        crossed.recovery_manifest_hex = other.recovery_manifest_hex;
        assert!(ValidatedRecoveryBundle::new(crossed, &trust).await.is_err());

        let mut wrong_provider = trust.clone();
        wrong_provider.provider = "https://other-accounts.example/".parse().unwrap();
        assert!(
            ValidatedRecoveryBundle::new(bundle.clone(), &wrong_provider)
                .await
                .is_err()
        );
        let mut wrong_remote = trust.clone();
        wrong_remote.remote = "https://other-app.example/ucan/".parse().unwrap();
        assert!(
            ValidatedRecoveryBundle::new(bundle.clone(), &wrong_remote)
                .await
                .is_err()
        );
        let mut wrong_device = trust.clone();
        wrong_device.device_did = signer(99).await.did();
        assert!(
            ValidatedRecoveryBundle::new(bundle, &wrong_device)
                .await
                .is_err()
        );
        let (bundle, mut wrong_service) = valid_recovery(42).await;
        wrong_service.service_did = Some(signer(98).await.did());
        assert!(
            ValidatedRecoveryBundle::new(bundle, &wrong_service)
                .await
                .is_err()
        );
    }

    #[dialog_common::test]
    async fn it_bounds_every_input_before_parsing_and_classifies_current_expiry() {
        let (bundle, trust) = valid_recovery(44).await;
        let validated = ValidatedRecoveryBundle::new(bundle.clone(), &trust)
            .await
            .unwrap();

        let mut expired_create = trust.clone();
        expired_create.now = validated.create_expires_at() + 1;
        let expired = ValidatedRecoveryBundle::new(bundle.clone(), &expired_create)
            .await
            .unwrap();
        assert_eq!(expired.create_freshness(), RecoveryFreshness::NeedsRefresh);
        assert_eq!(expired.publish_freshness(), RecoveryFreshness::Usable);

        macro_rules! too_large {
            ($field:ident, $value:expr, $name:literal) => {{
                let mut changed = bundle.clone();
                changed.$field = $value;
                assert!(matches!(
                    decode_bounded_recovery(&changed),
                    Err(RecoveryValidationError::TooLarge($name))
                ));
            }};
        }
        macro_rules! invalid_text {
            ($field:ident, $value:expr) => {{
                let mut changed = bundle.clone();
                changed.$field = $value;
                assert!(matches!(
                    decode_bounded_recovery(&changed),
                    Err(RecoveryValidationError::Invalid("text"))
                ));
            }};
        }

        invalid_text!(operation_id, "x".repeat(129));
        invalid_text!(normalized_email, "x".repeat(321));
        invalid_text!(
            provider,
            format!("https://example.test/{}/", "x".repeat(2048))
        );
        invalid_text!(root_did, "x".repeat(513));
        invalid_text!(device_did, "x".repeat(513));
        invalid_text!(device_name, "x".repeat(121));
        invalid_text!(delegation_cid, "x".repeat(513));
        invalid_text!(custody_did, "x".repeat(513));

        let mut credential = bundle.clone();
        credential.credential_id = "aa".repeat(2049);
        assert!(matches!(
            decode_bounded_recovery(&credential),
            Err(RecoveryValidationError::Invalid("credential_id"))
        ));
        let mut passkey_label = bundle.clone();
        passkey_label.passkey.as_mut().unwrap().created_on = "x".repeat(121);
        assert!(matches!(
            decode_bounded_recovery(&passkey_label),
            Err(RecoveryValidationError::Invalid("passkey"))
        ));
        let mut encryption_key = bundle.clone();
        encryption_key.encryption_key = Some("x".repeat(513));
        assert!(matches!(
            decode_bounded_recovery(&encryption_key),
            Err(RecoveryValidationError::Invalid("encryption_key"))
        ));

        too_large!(delegation_hex, "aa".repeat(64 * 1024 + 1), "delegation");
        too_large!(descriptor_hex, "aa".repeat(4096 + 1), "descriptor");
        too_large!(invocation_hex, "aa".repeat(128 * 1024 + 1), "invocation");
        too_large!(consent_hex, "aa".repeat(64 * 1024 + 1), "consent");
        too_large!(sealed_hex, "aa".repeat(4096 + 1), "sealed");
        too_large!(
            publish_invocation_hex,
            "aa".repeat(128 * 1024 + 1),
            "publish_invocation"
        );
        too_large!(
            recovery_manifest_hex,
            "aa".repeat(16 * 1024 + 1),
            "recovery_manifest"
        );
        let mut oversized_deposit = bundle.clone();
        oversized_deposit.deposits_hex = vec!["aa".repeat(64 * 1024 + 1)];
        assert!(matches!(
            decode_bounded_recovery(&oversized_deposit),
            Err(RecoveryValidationError::TooLarge("deposit"))
        ));

        let mut oversized = bundle.clone();
        oversized.invocation_hex = "aa".repeat(128 * 1024 + 1);
        assert!(matches!(
            ValidatedRecoveryBundle::new(oversized, &trust).await,
            Err(RecoveryValidationError::TooLarge("invocation"))
        ));
        let mut too_many_deposits = bundle.clone();
        too_many_deposits.deposits_hex = vec!["00".to_string(); 9];
        assert!(matches!(
            ValidatedRecoveryBundle::new(too_many_deposits, &trust).await,
            Err(RecoveryValidationError::TooLarge("deposits"))
        ));

        let mut decoded_total = bundle.clone();
        decoded_total.delegation_hex = "aa".repeat(64 * 1024);
        decoded_total.descriptor_hex = "aa".repeat(4096);
        decoded_total.invocation_hex = "aa".repeat(128 * 1024);
        decoded_total.deposits_hex = vec!["aa".repeat(64 * 1024); 2];
        decoded_total.consent_hex = "aa".repeat(64 * 1024);
        decoded_total.sealed_hex = "aa".repeat(4096);
        decoded_total.publish_invocation_hex = "aa".repeat(128 * 1024);
        decoded_total.recovery_manifest_hex = "aa".repeat(16 * 1024);
        assert!(matches!(
            decode_bounded_recovery(&decoded_total),
            Err(RecoveryValidationError::TooLarge("decoded_total"))
        ));

        let mut oversized_record = decoded_total;
        oversized_record.publish_invocation_hex = "aa".repeat(106_300);
        assert!(matches!(
            decode_bounded_recovery(&oversized_record),
            Err(RecoveryValidationError::TooLarge("recovery_record"))
        ));
        assert!(matches!(
            decode_recovery(&vec![0; MAX_RECOVERY_RECORD_BYTES + 1]),
            Err(StoredSetupError::TooLargeRecovery)
        ));

        let mut noncanonical = bundle;
        noncanonical.sealed_hex = "AA".to_string();
        assert!(
            ValidatedRecoveryBundle::new(noncanonical, &trust)
                .await
                .is_err()
        );
    }

    #[dialog_common::test]
    fn it_checks_exact_stage_and_original_expiry_boundaries_without_clock_saturation() {
        let armed_at = 1_000;
        for (created_at, staged_at) in [(940, 1_000), (1_060, 1_000), (1_000, 4_600)] {
            assert!(validate_stage_timestamps(armed_at, created_at, staged_at, staged_at).is_ok());
        }
        for (created_at, staged_at, now) in [
            (940, 999, 1_000),
            (939, 1_000, 1_000),
            (1_061, 1_000, 1_000),
            (1_000, 4_601, 4_601),
            (1_000, 1_001, 1_000),
            (u64::MAX, u64::MAX, u64::MAX),
        ] {
            assert!(validate_stage_timestamps(armed_at, created_at, staged_at, now).is_err());
        }

        assert!(
            validate_original_expiration(
                10_000,
                10_000 + CREATE_EXPIRY_MIN_OFFSET,
                CREATE_EXPIRY_MIN_OFFSET,
                CREATE_EXPIRY_MAX_OFFSET,
            )
            .is_ok()
        );
        assert!(
            validate_original_expiration(
                10_000,
                10_000 + CREATE_EXPIRY_MAX_OFFSET,
                CREATE_EXPIRY_MIN_OFFSET,
                CREATE_EXPIRY_MAX_OFFSET,
            )
            .is_ok()
        );
        assert!(
            validate_original_expiration(
                10_000,
                10_000 + CREATE_EXPIRY_MIN_OFFSET - 1,
                CREATE_EXPIRY_MIN_OFFSET,
                CREATE_EXPIRY_MAX_OFFSET,
            )
            .is_err()
        );
        assert!(
            validate_original_expiration(
                10_000,
                10_000 + PUBLISH_EXPIRY_MIN_OFFSET,
                PUBLISH_EXPIRY_MIN_OFFSET,
                PUBLISH_EXPIRY_MAX_OFFSET,
            )
            .is_ok()
        );
        assert!(
            validate_original_expiration(
                10_000,
                10_000 + PUBLISH_EXPIRY_MAX_OFFSET + 1,
                PUBLISH_EXPIRY_MIN_OFFSET,
                PUBLISH_EXPIRY_MAX_OFFSET,
            )
            .is_err()
        );
        assert!(
            validate_original_expiration(
                u64::MAX,
                u64::MAX,
                CREATE_EXPIRY_MIN_OFFSET,
                CREATE_EXPIRY_MAX_OFFSET,
            )
            .is_err()
        );
    }
}

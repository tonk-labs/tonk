//! Versioned wire contract for durable browser account setup.

use serde::{Deserialize, Serialize};

/// Account-setup protocol implemented by a compatible service worker.
pub const ACCOUNT_SETUP_PROTOCOL_VERSION: u16 = 2;

/// Provider contract required for proof-bound setup status and exact replay.
pub const ACCOUNT_SETUP_PROVIDER_RECOVERY_VERSION: u16 = 1;

/// Versions confirmed before the page may request passkey creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupCapabilities {
    /// Account-setup protocol implemented by the controlling worker.
    pub worker_protocol_version: u16,
    /// Recovery contract advertised by the selected account provider.
    pub provider_recovery_version: u16,
}

/// Canonical deployment facts selected by the worker for one setup lease.
///
/// The page signs these exact facts into the recovery manifest. It never
/// chooses a provider or turns the worker into a fetch proxy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupCeremonyContext {
    /// Canonical account-service base URL whose capability the worker checked.
    pub provider: String,
    /// Canonical repository remote bound by the root-signed descriptor.
    pub remote: String,
    /// Configured access-service DID whose exact deposit scopes are required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_did: Option<String>,
}

/// Ask the worker to prove both sides of the recovery contract before the
/// browser is allowed to request passkey creation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupHandshake {
    /// Protocol version required by the page.
    pub protocol_version: u16,
}

/// Establish a new pre-WebAuthn lease after a successful capability check.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupBegin {
    /// Protocol version required by the page.
    pub protocol_version: u16,
    /// High-entropy document owner token. Only a domain-separated hash is
    /// persisted.
    pub owner_token: String,
}

/// Acquire a recoverable operation after its previous document is absent.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupAcquire {
    /// Operation being acquired.
    pub operation_id: String,
    /// New high-entropy owner token. Only its hash is persisted.
    pub owner_token: String,
    /// Compare-and-swap revision read from the redacted view.
    pub expected_revision: u64,
}

/// Common proof of ownership and revision for a setup mutation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupMutation {
    /// Operation being changed.
    pub operation_id: String,
    /// Raw owner token held only by the page.
    pub owner_token: String,
    /// Compare-and-swap revision read from the preceding response.
    pub expected_revision: u64,
}

/// Arm exactly one passkey-creation attempt.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupArm {
    /// Ownership and revision proof.
    pub mutation: AccountSetupMutation,
    /// Document-memory attempt token. Only a domain-separated hash is stored.
    pub attempt_token: String,
}

/// Credential-store-protected recovery material produced by account creation.
///
/// This is not secret-free: it contains PII, a passkey-sealed envelope, and
/// bounded signed authorizations. It intentionally does not implement
/// [`Debug`] and must never be included in general status or logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupRecoveryBundle {
    /// Recovery bundle schema version.
    pub version: u16,
    /// Root-signed immutable Unix-seconds reference captured after passkey
    /// creation and before the bounded invocations were minted.
    pub ceremony_created_at: u64,
    /// Normalized provider account email.
    pub normalized_email: String,
    /// Account-provider base URL.
    pub provider: String,
    /// Passkey-derived account root DID.
    pub root_did: String,
    /// Current profile/device DID.
    pub device_did: String,
    /// Device label bound into the account-creation fingerprint.
    pub device_name: String,
    /// Opaque WebAuthn credential identifier.
    pub credential_id: String,
    /// CID of the stable root-to-device delegation.
    pub delegation_cid: String,
    /// Exact hex-encoded root-to-device delegation.
    pub delegation_hex: String,
    /// Informational passkey creation metadata, when captured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<crate::PasskeyMetadata>,
    /// Account X25519 recipient published to the account repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<String>,
    /// Canonical signed account-repository descriptor.
    pub descriptor_hex: String,
    /// Canonical version-1 provider creation fingerprint.
    pub create_fingerprint: String,
    /// Original root-signed account-creation invocation.
    pub invocation_hex: String,
    /// Account-signed customer enrollment deposits.
    #[serde(default)]
    pub deposits_hex: Vec<String>,
    /// Custody space DID.
    pub custody_did: String,
    /// Signed custody provisioning consent.
    pub consent_hex: String,
    /// Passkey-sealed account-secret envelope.
    pub sealed_hex: String,
    /// Bounded custody-cell publish invocation.
    pub publish_invocation_hex: String,
    /// Canonical root-signed anti-mix manifest binding this operation and all
    /// preceding protected artifacts.
    pub recovery_manifest_hex: String,
}

/// Save recovery material for the one armed attempt.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupStage {
    /// Ownership and revision proof.
    pub mutation: AccountSetupMutation,
    /// Raw document-memory attempt token matching the armed hash.
    pub attempt_token: String,
    /// Protected bundle to durably validate and save before any provider call.
    pub recovery: AccountSetupRecoveryBundle,
}

/// Replace an expired create invocation after asserting the same credential.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupInvocation {
    /// Ownership and revision proof.
    pub mutation: AccountSetupMutation,
    /// Fresh invocation whose decoded facts must reproduce the stored
    /// canonical fingerprint.
    pub invocation_hex: String,
}

/// Inspect the current operation without granting access to protected state.
///
/// A matching owner token lets the worker return a separate protected response
/// only when a same-passkey resume gesture is required. The public view remains
/// redacted either way.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupInspect {
    /// Existing document owner token, when this session has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_token: Option<String>,
}

/// One command sent to the worker's account-setup coordinator.
///
/// This type intentionally does not implement [`Debug`]. Later commands carry
/// credential-store-protected recovery material and bounded authorizations;
/// keeping the outer request non-debuggable prevents accidental whole-request
/// logging as the protocol grows.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "command",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AccountSetupRequest {
    /// Verify the worker protocol and provider recovery capability.
    Handshake(AccountSetupHandshake),
    /// Inspect redacted state and authenticate an optional protected resume.
    Inspect(AccountSetupInspect),
    /// Begin a new capability-checked lease.
    Begin(AccountSetupBegin),
    /// Take over a recoverable operation whose previous client is absent.
    Acquire(AccountSetupAcquire),
    /// Fence one passkey creation attempt after rechecking provider support.
    Arm(AccountSetupArm),
    /// Durably stage the recoverable ceremony result.
    Stage(Box<AccountSetupStage>),
    /// Reconcile and advance only verified post-stage effects.
    Continue(AccountSetupMutation),
    /// Replace an expired create invocation after a same-credential assertion.
    ReplaceInvocation(AccountSetupInvocation),
    /// Cancel a lease before it is armed.
    Cancel(AccountSetupMutation),
}

/// Coarse progress label for presentation only.
///
/// Durable phases are private worker data. The UI must follow
/// [`AccountSetupNextAction`] rather than derive behavior from this label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSetupProgress {
    /// Capability and ownership checks are being prepared.
    Preparing,
    /// The current document may be waiting on the authenticator prompt.
    PasskeyApproval,
    /// A staged bundle is being reconciled locally or with the provider.
    Recovering,
    /// Provider acceptance and local attachment are being reconciled.
    ConnectingServices,
    /// Customer and custody work is being made durable.
    Finishing,
    /// Account setup is durably complete.
    Complete,
}

/// Closed status category rendered by the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSetupDisposition {
    /// No operation exists.
    Missing,
    /// The named next action is safe now.
    Ready,
    /// Another live document owns the operation.
    InProgressElsewhere,
    /// No state was lost, but a transient dependency should be retried later.
    RetryLater,
    /// A pre-WebAuthn operation was cancelled.
    Cancelled,
    /// Cancellation lost the atomic race with arming.
    CancelTooLate,
    /// Passkey approval may have completed before recovery was staged.
    InterruptedBeforeRecovery,
    /// Durable local or provider facts do not match this operation.
    Conflict,
    /// Setup and its completion checkpoint are durable.
    Complete,
    /// Stored data is malformed and must not be overwritten.
    Corrupt,
    /// Stored data belongs to a future unsupported schema.
    Unsupported,
    /// Worker or provider recovery capability is incompatible.
    UpdateRequired,
    /// A same-credential assertion is required to replace an expired request.
    NeedsPasskey,
}

/// Closed action the UI may offer for a disposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSetupNextAction {
    /// Offer no action.
    None,
    /// Begin a new capability-checked operation.
    Begin,
    /// Ask for passkey approval after arming.
    ApprovePasskey,
    /// Continue deterministic worker reconciliation.
    Continue,
    /// Wait for the current owner or authenticator prompt.
    Wait,
    /// Acquire a staged operation whose previous client is absent.
    Acquire,
    /// Retry a transiently unavailable dependency.
    Retry,
    /// Ask for an assertion of the exact staged credential.
    ResumeWithPasskey,
    /// Explicitly abandon a terminal interrupted attempt and start over.
    StartOver,
    /// Reload only when it is safe to adopt compatible code.
    Reload,
}

/// Stable non-sensitive reason for a terminal conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountSetupConflictCode {
    /// Protected recovery data does not match the checkpoint.
    RecoveryMismatch,
    /// A different local root is already durable.
    LocalRootMismatch,
    /// Provider status returned a semantic mismatch.
    ProviderMismatch,
    /// A different local provider link is already durable.
    AttachmentMismatch,
}

/// Redacted account-setup status returned by general inspection.
///
/// It deliberately contains no email, token or token hash, credential ID,
/// ciphertext, delegation, invocation, or provider artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupView {
    /// Worker protocol that produced this view.
    pub worker_protocol_version: u16,
    /// Opaque operation identifier, absent when no valid operation is decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// Compare-and-swap revision, absent when no valid operation is decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Presentation-only progress, never an instruction to the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AccountSetupProgress>,
    /// Closed status category.
    pub disposition: AccountSetupDisposition,
    /// Only action the UI may offer for this response.
    pub next_action: AccountSetupNextAction,
    /// Stable conflict reason when [`AccountSetupDisposition::Conflict`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_code: Option<AccountSetupConflictCode>,
    /// Future stored schema version when safely decoded from its envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_version: Option<u16>,
}

/// Owner-bound lease plus the worker-selected facts for the passkey ceremony.
///
/// `Begin` returns this only after capability/configuration checks. `Arm`
/// rechecks the checkpoint's configuration hash before passkey creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupLease {
    /// Redacted leased-operation state and compare-and-swap revision.
    pub view: AccountSetupView,
    /// Exact deployment facts the recovery manifest must bind.
    pub ceremony: AccountSetupCeremonyContext,
}

/// Minimum protected input for asserting the exact staged credential and
/// rebuilding only the original semantic account creation.
///
/// This type is owner-authenticated and intentionally does not implement
/// [`Debug`]. It must never be returned by unauthenticated inspection.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSetupResumeInput {
    /// Operation whose expired invocation is being replaced.
    pub operation_id: String,
    /// Exact staged WebAuthn credential identifier.
    pub credential_id: String,
    /// Root DID the reopened envelope must reproduce.
    pub expected_root_did: String,
    /// Normalized provider account email bound into the fingerprint.
    pub normalized_email: String,
    /// Exact device DID receiving the stable delegation.
    pub device_did: String,
    /// Exact device label bound into the fingerprint.
    pub device_name: String,
    /// Stable root-to-device delegation bytes.
    pub delegation_hex: String,
    /// Original canonical signed descriptor bytes.
    pub descriptor_hex: String,
    /// Passkey creation metadata bound into the fingerprint, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<crate::PasskeyMetadata>,
    /// Passkey-sealed account-secret envelope.
    pub sealed_hex: String,
}

/// Owner-authenticated response carrying protected recovery material.
///
/// It intentionally does not implement [`Debug`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "protectedOutcome",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AccountSetupProtectedResponse {
    /// The original invocation expired and the same credential must be asserted.
    NeedsPasskey {
        /// Redacted status used for presentation.
        view: AccountSetupView,
        /// Minimum protected ceremony input.
        resume: AccountSetupResumeInput,
    },
}

/// Worker response to one account-setup command.
///
/// The outer response intentionally does not implement [`Debug`]; an
/// owner-authenticated response will later carry protected recovery input.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AccountSetupResponse {
    /// Both worker and provider support the required recovery protocols.
    Capabilities(AccountSetupCapabilities),
    /// New owner-bound operation and its worker-selected ceremony context.
    Lease(AccountSetupLease),
    /// Redacted current status.
    View(AccountSetupView),
    /// Owner-authenticated protected response, never returned by public status.
    Protected(AccountSetupProtectedResponse),
}

#[cfg(test)]
mod tests {
    use super::{
        ACCOUNT_SETUP_PROTOCOL_VERSION, ACCOUNT_SETUP_PROVIDER_RECOVERY_VERSION,
        AccountSetupAcquire, AccountSetupArm, AccountSetupBegin, AccountSetupCapabilities,
        AccountSetupCeremonyContext, AccountSetupDisposition, AccountSetupHandshake,
        AccountSetupInspect, AccountSetupInvocation, AccountSetupLease, AccountSetupMutation,
        AccountSetupNextAction, AccountSetupProgress, AccountSetupProtectedResponse,
        AccountSetupRecoveryBundle, AccountSetupRequest, AccountSetupResponse,
        AccountSetupResumeInput, AccountSetupStage, AccountSetupView,
    };

    #[dialog_common::test]
    fn it_serializes_the_versioned_handshake_and_a_redacted_view() {
        let request = AccountSetupRequest::Handshake(AccountSetupHandshake {
            protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
        });
        assert_eq!(
            serde_json::to_value(request).expect("serialize handshake"),
            serde_json::json!({
                "command": "handshake",
                "protocolVersion": 2,
            })
        );

        let view = AccountSetupView {
            worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
            operation_id: Some("setup-1".to_string()),
            revision: Some(7),
            progress: Some(AccountSetupProgress::Recovering),
            disposition: AccountSetupDisposition::Ready,
            next_action: AccountSetupNextAction::Acquire,
            conflict_code: None,
            stored_version: None,
        };
        let value = serde_json::to_value(view).expect("serialize redacted view");
        assert_eq!(value["workerProtocolVersion"], 2);
        assert_eq!(value["operationId"], "setup-1");
        assert_eq!(value["revision"], 7);
        assert_eq!(value["progress"], "recovering");
        assert_eq!(value["disposition"], "ready");
        assert_eq!(value["nextAction"], "acquire");

        let encoded = serde_json::to_string(&value).expect("encode redacted view");
        for forbidden in [
            "email",
            "owner",
            "attempt",
            "credential",
            "delegation",
            "invocation",
            "sealed",
            "descriptor",
        ] {
            assert!(
                !encoded.to_ascii_lowercase().contains(forbidden),
                "general setup status leaked {forbidden}: {encoded}"
            );
        }
    }

    #[dialog_common::test]
    fn it_round_trips_every_command_and_rejects_unknown_wire_fields() {
        let mutation = AccountSetupMutation {
            operation_id: "setup-1".to_string(),
            owner_token: "owner-token".to_string(),
            expected_revision: 7,
        };
        let recovery = AccountSetupRecoveryBundle {
            version: 1,
            ceremony_created_at: 1_754_380_800,
            normalized_email: "person@example.com".to_string(),
            provider: "https://accounts.example".to_string(),
            root_did: "did:key:root".to_string(),
            device_did: "did:key:device".to_string(),
            device_name: "Jack's laptop".to_string(),
            credential_id: "aabb".to_string(),
            delegation_cid: "bafydelegation".to_string(),
            delegation_hex: "ccdd".to_string(),
            passkey: Some(crate::PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".to_string(),
            }),
            encryption_key: Some("did:key:z6LSrecipient".to_string()),
            descriptor_hex: "eeff".to_string(),
            create_fingerprint: "11".repeat(32),
            invocation_hex: "2233".to_string(),
            deposits_hex: vec!["4455".to_string()],
            custody_did: "did:key:custody".to_string(),
            consent_hex: "6677".to_string(),
            sealed_hex: "8899".to_string(),
            publish_invocation_hex: "aabb".to_string(),
            recovery_manifest_hex: "bbcc".to_string(),
        };
        let requests = [
            AccountSetupRequest::Inspect(AccountSetupInspect { owner_token: None }),
            AccountSetupRequest::Begin(AccountSetupBegin {
                protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
                owner_token: "owner-token".to_string(),
            }),
            AccountSetupRequest::Acquire(AccountSetupAcquire {
                operation_id: "setup-1".to_string(),
                owner_token: "new-owner-token".to_string(),
                expected_revision: 7,
            }),
            AccountSetupRequest::Arm(AccountSetupArm {
                mutation: mutation.clone(),
                attempt_token: "attempt-token".to_string(),
            }),
            AccountSetupRequest::Stage(Box::new(AccountSetupStage {
                mutation: mutation.clone(),
                attempt_token: "attempt-token".to_string(),
                recovery,
            })),
            AccountSetupRequest::Continue(mutation.clone()),
            AccountSetupRequest::ReplaceInvocation(AccountSetupInvocation {
                mutation: mutation.clone(),
                invocation_hex: "deadbeef".to_string(),
            }),
            AccountSetupRequest::Cancel(mutation),
        ];
        let expected_commands = [
            "inspect",
            "begin",
            "acquire",
            "arm",
            "stage",
            "continue",
            "replaceInvocation",
            "cancel",
        ];

        for (request, command) in requests.into_iter().zip(expected_commands) {
            let value = serde_json::to_value(&request).expect("serialize command");
            assert_eq!(value["command"], command);
            let decoded: AccountSetupRequest =
                serde_json::from_value(value).expect("deserialize command");
            assert!(decoded == request, "{command} did not round trip");
        }

        assert!(
            serde_json::from_value::<AccountSetupRequest>(serde_json::json!({
                "command": "begin",
                "protocolVersion": 2,
                "ownerToken": "owner-token",
                "provider": "https://evil.example",
            }))
            .is_err(),
            "provider selection belongs to deployment config, never caller input"
        );
        assert!(
            serde_json::from_value::<AccountSetupRequest>(serde_json::json!({
                "command": "advance",
                "phase": "complete",
            }))
            .is_err(),
            "callers must not be able to declare a later phase"
        );
    }

    #[dialog_common::test]
    fn it_names_every_phase_and_capability_without_exposing_protected_state() {
        let progress = [
            (AccountSetupProgress::Preparing, "preparing"),
            (AccountSetupProgress::PasskeyApproval, "passkeyApproval"),
            (AccountSetupProgress::Recovering, "recovering"),
            (
                AccountSetupProgress::ConnectingServices,
                "connectingServices",
            ),
            (AccountSetupProgress::Finishing, "finishing"),
            (AccountSetupProgress::Complete, "complete"),
        ];
        for (progress, expected) in progress {
            assert_eq!(serde_json::to_value(progress).unwrap(), expected);
        }

        for (disposition, expected) in [
            (AccountSetupDisposition::Missing, "missing"),
            (AccountSetupDisposition::Ready, "ready"),
            (
                AccountSetupDisposition::InProgressElsewhere,
                "inProgressElsewhere",
            ),
            (AccountSetupDisposition::RetryLater, "retryLater"),
            (AccountSetupDisposition::Cancelled, "cancelled"),
            (AccountSetupDisposition::CancelTooLate, "cancelTooLate"),
            (
                AccountSetupDisposition::InterruptedBeforeRecovery,
                "interruptedBeforeRecovery",
            ),
            (AccountSetupDisposition::Conflict, "conflict"),
            (AccountSetupDisposition::Complete, "complete"),
            (AccountSetupDisposition::Corrupt, "corrupt"),
            (AccountSetupDisposition::Unsupported, "unsupported"),
            (AccountSetupDisposition::UpdateRequired, "updateRequired"),
            (AccountSetupDisposition::NeedsPasskey, "needsPasskey"),
        ] {
            assert_eq!(serde_json::to_value(disposition).unwrap(), expected);
        }

        let response = AccountSetupResponse::Capabilities(AccountSetupCapabilities {
            worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
            provider_recovery_version: ACCOUNT_SETUP_PROVIDER_RECOVERY_VERSION,
        });
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "outcome": "capabilities",
                "workerProtocolVersion": 2,
                "providerRecoveryVersion": 1,
            })
        );

        let response = AccountSetupResponse::Lease(AccountSetupLease {
            view: AccountSetupView {
                worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
                operation_id: Some("setup-1".to_string()),
                revision: Some(1),
                progress: Some(AccountSetupProgress::Preparing),
                disposition: AccountSetupDisposition::Ready,
                next_action: AccountSetupNextAction::ApprovePasskey,
                conflict_code: None,
                stored_version: None,
            },
            ceremony: AccountSetupCeremonyContext {
                provider: "https://accounts.example/".to_string(),
                remote: "https://app.example/ucan/".to_string(),
                service_did: Some("did:key:service".to_string()),
            },
        });
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "outcome": "lease",
                "view": {
                    "workerProtocolVersion": 2,
                    "operationId": "setup-1",
                    "revision": 1,
                    "progress": "preparing",
                    "disposition": "ready",
                    "nextAction": "approvePasskey",
                },
                "ceremony": {
                    "provider": "https://accounts.example/",
                    "remote": "https://app.example/ucan/",
                    "serviceDid": "did:key:service",
                },
            })
        );

        for view in [
            AccountSetupView {
                worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
                operation_id: None,
                revision: None,
                progress: None,
                disposition: AccountSetupDisposition::Missing,
                next_action: AccountSetupNextAction::Begin,
                conflict_code: None,
                stored_version: None,
            },
            AccountSetupView {
                worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
                operation_id: None,
                revision: None,
                progress: None,
                disposition: AccountSetupDisposition::Corrupt,
                next_action: AccountSetupNextAction::None,
                conflict_code: None,
                stored_version: None,
            },
            AccountSetupView {
                worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
                operation_id: None,
                revision: None,
                progress: None,
                disposition: AccountSetupDisposition::Unsupported,
                next_action: AccountSetupNextAction::Reload,
                conflict_code: None,
                stored_version: Some(99),
            },
        ] {
            let encoded = serde_json::to_string(&view).unwrap();
            assert!(!encoded.contains("email"));
            assert!(!encoded.contains("owner"));
            assert!(!encoded.contains("invocation"));
        }
    }

    #[dialog_common::test]
    fn it_keeps_same_passkey_resume_material_behind_a_protected_response() {
        let view = AccountSetupView {
            worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
            operation_id: Some("setup-1".to_string()),
            revision: Some(9),
            progress: Some(AccountSetupProgress::Recovering),
            disposition: AccountSetupDisposition::NeedsPasskey,
            next_action: AccountSetupNextAction::ResumeWithPasskey,
            conflict_code: None,
            stored_version: None,
        };
        let protected = AccountSetupProtectedResponse::NeedsPasskey {
            view,
            resume: AccountSetupResumeInput {
                operation_id: "setup-1".to_string(),
                credential_id: "aabb".to_string(),
                expected_root_did: "did:key:root".to_string(),
                normalized_email: "person@example.com".to_string(),
                device_did: "did:key:device".to_string(),
                device_name: "Jack's laptop".to_string(),
                delegation_hex: "ccdd".to_string(),
                descriptor_hex: "eeff".to_string(),
                passkey: None,
                sealed_hex: "8899".to_string(),
            },
        };
        let value = serde_json::to_value(AccountSetupResponse::Protected(protected)).unwrap();
        assert_eq!(value["outcome"], "protected");
        assert_eq!(value["protectedOutcome"], "needsPasskey");
        assert_eq!(value["resume"]["credentialId"], "aabb");
        assert_eq!(value["resume"]["sealedHex"], "8899");

        let public = serde_json::to_string(&AccountSetupResponse::View(AccountSetupView {
            worker_protocol_version: ACCOUNT_SETUP_PROTOCOL_VERSION,
            operation_id: Some("setup-1".to_string()),
            revision: Some(9),
            progress: Some(AccountSetupProgress::Recovering),
            disposition: AccountSetupDisposition::RetryLater,
            next_action: AccountSetupNextAction::Retry,
            conflict_code: None,
            stored_version: None,
        }))
        .unwrap();
        for forbidden in ["email", "credential", "delegation", "descriptor", "sealed"] {
            assert!(!public.contains(forbidden));
        }
    }
}

//! Browser account-space discovery, explicit download, and archive routes.

use axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_varsig::Did;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{
    AccountSpaceArchiveResponse, AccountSpaceDownloadResponse, AccountSpaceEnrollment,
    AccountSpaceMembership, AccountSpaceRow, AccountSpaceVisibility,
};

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

const ENROLLMENT_PREFIX: &str = "tonk-account-enrollment-v1/";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollmentRecord {
    version: u8,
    subject: String,
    phase: AccountSpaceEnrollment,
    #[serde(default)]
    connected: bool,
    error: Option<String>,
}

fn decode_enrollment(bytes: &[u8], subject: &str) -> (AccountSpaceEnrollment, Option<String>) {
    match serde_json::from_slice::<EnrollmentRecord>(bytes) {
        Ok(record)
            if record.version == 1
                && record.subject == subject
                && record.connected == (record.phase == AccountSpaceEnrollment::Connected) =>
        {
            (record.phase, record.error)
        }
        Ok(_) => (
            AccountSpaceEnrollment::Error,
            Some("enrollment state has an unsupported version or subject".to_string()),
        ),
        Err(error) => (AccountSpaceEnrollment::Error, Some(error.to_string())),
    }
}

async fn enrollment(tonk: &TonkState, subject: &str) -> (AccountSpaceEnrollment, Option<String>) {
    let bytes = match tonk
        .profile
        .credential()
        .site(format!("{ENROLLMENT_PREFIX}{subject}"))
        .load::<Vec<u8>>()
        .perform(&tonk.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => {
            return (AccountSpaceEnrollment::LocalOnly, None);
        }
        Err(error) => {
            return (AccountSpaceEnrollment::Error, Some(error.to_string()));
        }
    };
    decode_enrollment(&bytes, subject)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod enrollment_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_decodes_durable_pending_and_error_enrollment_states() {
        let pending = br#"{
            "version": 1,
            "subject": "did:key:space",
            "phase": "pendingPush",
            "connected": false,
            "error": null
        }"#;
        assert_eq!(
            decode_enrollment(pending, "did:key:space"),
            (AccountSpaceEnrollment::PendingPush, None)
        );

        let failed = br#"{
            "version": 1,
            "subject": "did:key:space",
            "phase": "error",
            "connected": false,
            "error": "push failed"
        }"#;
        assert_eq!(
            decode_enrollment(failed, "did:key:space"),
            (
                AccountSpaceEnrollment::Error,
                Some("push failed".to_string())
            )
        );
        assert_eq!(
            decode_enrollment(pending, "did:key:other").0,
            AccountSpaceEnrollment::Error
        );
        assert_eq!(
            decode_enrollment(b"not json", "did:key:space").0,
            AccountSpaceEnrollment::Error
        );
    }

    #[dialog_common::test]
    async fn it_keeps_failures_pending_without_false_recovery() {
        for (step, message) in [
            ("provision", "create remote failed"),
            ("push", "content push failed"),
            ("account-push", "account push failed"),
            ("project", "provider projection failed"),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "subject": "did:key:space",
                "phase": "error",
                "connected": false,
                "error": format!("{step}: {message}"),
            }))
            .unwrap();
            let (phase, error) = decode_enrollment(&bytes, "did:key:space");
            assert_eq!(phase, AccountSpaceEnrollment::Error);
            assert!(error.as_deref().is_some_and(|error| error.contains(step)));
        }

        let false_recovery = br#"{
            "version": 1,
            "subject": "did:key:space",
            "phase": "connected",
            "connected": false,
            "error": null
        }"#;
        assert_eq!(
            decode_enrollment(false_recovery, "did:key:space").0,
            AccountSpaceEnrollment::Error,
            "a phase label alone must not claim recovery"
        );
    }
}

async fn canonical_rows(
    tonk: &TonkState,
) -> Result<
    (
        dialog_varsig::Did,
        Vec<tonk_schema::account::AccountSpaceRecord>,
    ),
    TonkWorkerError,
> {
    let ready = super::account_state::require_ready_account_state(tonk).await?;
    let branch = tonk
        .reactor
        .repository(&ready.key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open account-space branch: {error}"))
        })?;
    let rows = tonk_schema::account::list_account_spaces(branch.handle(), &tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("read account spaces: {error}")))?;
    Ok((ready.subject, rows))
}

async fn inventory(tonk: &TonkState) -> Result<Vec<AccountSpaceRow>, TonkWorkerError> {
    let (_account, canonical) = canonical_rows(tonk).await?;
    let suppressions = tonk
        .registry
        .read_space_suppressions(&tonk.storage, &tonk.profile_name)
        .await?;
    let mut rows = Vec::new();

    for record in canonical {
        let subject_text = record.subject.to_string();
        let local = super::join::find_replica_for_subject(tonk, &record.subject).await?;
        let hidden = suppressions.contains(&subject_text);
        let authority = super::repository::space_root_prefix(tonk, &record.subject)
            .await
            .is_ok();
        let mountable = super::adopt::directory_configuration(tonk, &record.subject)
            .await
            .is_some();
        let pullable = !record.archived && !local && authority && mountable;
        let (enrollment, enrollment_error) = enrollment(tonk, &subject_text).await;
        rows.push(AccountSpaceRow {
            version: 1,
            subject: subject_text,
            name: record.name,
            membership: if record.archived {
                AccountSpaceMembership::Archived
            } else {
                AccountSpaceMembership::Active
            },
            local,
            visibility: if hidden {
                AccountSpaceVisibility::HiddenOnThisDevice
            } else {
                AccountSpaceVisibility::Visible
            },
            remote_url: record.remote_url,
            confirmed_revision: record.confirmed_revision,
            pullable,
            enrollment,
            enrollment_error,
        });
    }

    rows.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(rows)
}

/// List canonical account spaces without mounting account-only rows.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AccountSpaceRow>>, TonkWorkerError> {
    let tonk = state.read().await;
    Ok(Json(inventory(&tonk).await?))
}

/// Explicitly download one active account space.
#[wasm_compat]
pub async fn download(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> Result<Json<AccountSpaceDownloadResponse>, TonkWorkerError> {
    let subject_did: Did = subject
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid space subject: {error:?}")))?;
    let tonk = state.read().await;
    let active = canonical_rows(&tonk)
        .await?
        .1
        .into_iter()
        .any(|record| record.subject == subject_did && !record.archived);
    if !active {
        return Err(TonkWorkerError::Conflict(format!(
            "account space '{subject}' is unknown or archived"
        )));
    }
    if !super::adopt::ensure_space_mounted(&tonk, subject_did.as_ref()).await? {
        return Err(TonkWorkerError::NotFound(format!(
            "account space '{subject}' has no mountable directory record"
        )));
    }
    tonk.registry
        .unsuppress_space(&tonk.storage, &tonk.profile_name, subject_did.as_ref())
        .await?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    super::account_reconcile::reconcile(&tonk).await;
    Ok(Json(AccountSpaceDownloadResponse {
        subject,
        local: true,
    }))
}

/// Commit a monotonic canonical archive, then project it best-effort.
#[wasm_compat]
pub async fn archive(
    State(state): State<AppState>,
    Path(subject): Path<String>,
) -> Result<Json<AccountSpaceArchiveResponse>, TonkWorkerError> {
    let subject_did: Did = subject
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid space subject: {error:?}")))?;
    let tonk = state.read().await;
    let (account, _) = canonical_rows(&tonk).await?;
    let ready = super::account_state::require_ready_account_state(&tonk).await?;
    let branch = tonk
        .reactor
        .repository(&ready.key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("open account-space branch: {error}"))
        })?;
    let newly_archived = tonk_schema::account::archive_account_space(
        branch.handle(),
        &account,
        &subject_did,
        &tonk.operator,
    )
    .await
    .map_err(|error| TonkWorkerError::Conflict(error.to_string()))?;
    tonk.reactor
        .repository(&ready.key)
        .branch(tonk_account::MAIN_BRANCH)
        .push()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("push account archive: {error}")))?;

    Ok(Json(AccountSpaceArchiveResponse {
        subject,
        newly_archived,
        warning: None,
    }))
}

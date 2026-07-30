//! Attach optional provider services to the provider-neutral local root, and
//! name the account repository that root owns.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_effects::credential::CredentialError;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::{AccountRepositoryDescriptorV1, AccountStateStatus};
use tonk_common::log;
use tonk_worker_api::{
    AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest,
    AccountRepositoryEstablishRequest, AccountStatus,
};

use super::AppState;
use crate::TonkWorkerError;

pub(crate) const ACCOUNT_PROVIDER_SITE: &str = "tonk-account-provider-v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ProviderRecord {
    version: u8,
    provider: String,
    attached_at: u64,
    /// Exact root-signed account repository descriptor. Absent for an account
    /// created before descriptors existed and not yet established; that case
    /// is what [`establish_repository`] fills in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    descriptor: Option<Vec<u8>>,
}

/// Decode a descriptor and pin it to the local root.
///
/// [`AccountRepositoryDescriptorV1::validate`] proves the bytes are canonical
/// and self-signed, which says nothing about *whose* account they name. The
/// subject check is what stops a valid descriptor for someone else's account
/// from redirecting this device's account state.
async fn checked_descriptor(
    descriptor_hex: &str,
    root_did: &dialog_varsig::Did,
) -> Result<AccountRepositoryDescriptorV1, TonkWorkerError> {
    let bytes = hex::decode(descriptor_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid descriptor hex: {error}")))?;
    let descriptor = AccountRepositoryDescriptorV1::validate(&bytes)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid account repository descriptor: {error}"))
        })?;
    if descriptor.account_subject() != root_did {
        return Err(TonkWorkerError::Forbidden(
            "account repository descriptor names another account root".to_string(),
        ));
    }
    Ok(descriptor)
}

async fn load_provider(
    state: &crate::worker::TonkState,
) -> Result<Option<ProviderRecord>, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => return Ok(None),
        Ok(bytes) => bytes,
        Err(CredentialError::NotFound(_)) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load account provider: {error}"
            )));
        }
    };
    let record: ProviderRecord = serde_json::from_slice(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored account provider is malformed: {error}"))
    })?;
    if record.version != 1 {
        return Err(TonkWorkerError::Internal(format!(
            "unsupported account provider version {}",
            record.version
        )));
    }
    Ok(Some(record))
}

async fn save_provider(
    state: &crate::worker::TonkState,
    record: &ProviderRecord,
) -> Result<(), TonkWorkerError> {
    let bytes = serde_json::to_vec(record).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize account provider: {error}"))
    })?;
    state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save account provider: {error}"))
        })
}

/// Attached provider base URL, if any.
pub(crate) async fn provider(state: &crate::worker::TonkState) -> Option<String> {
    load_provider(state)
        .await
        .ok()
        .flatten()
        .map(|record| record.provider)
}

/// The root-signed descriptor naming this account's repository and remote.
///
/// Fail-safe: an unreadable or invalid descriptor resolves to `None`, so the
/// device behaves as one whose account state is merely unconfigured and keeps
/// working, rather than failing every account-state read.
pub(crate) async fn descriptor(
    state: &crate::worker::TonkState,
) -> Option<AccountRepositoryDescriptorV1> {
    let bytes = load_provider(state).await.ok().flatten()?.descriptor?;
    match AccountRepositoryDescriptorV1::validate(&bytes).await {
        Ok(descriptor) => Some(descriptor),
        Err(error) => {
            log!("stored account repository descriptor is unusable: {error}");
            None
        }
    }
}

/// The stable local root grant, available to provider operations only when attached.
pub(crate) async fn account_link(
    state: &crate::worker::TonkState,
) -> Option<dialog_ucan_core::DelegationChain> {
    provider(state).await?;
    super::identity::local_root(state)
        .await
        .ok()
        .map(|root| root.delegation)
}

/// The local root DID used for every durable membership operation.
pub(crate) async fn member_did(
    state: &crate::worker::TonkState,
) -> Result<dialog_varsig::Did, TonkWorkerError> {
    super::identity::root_did(state).await
}

async fn status(state: &crate::worker::TonkState) -> Result<AccountStatus, TonkWorkerError> {
    let device_did = state.profile.did().to_string();
    let root = match super::identity::local_root(state).await {
        Ok(root) => root,
        Err(TonkWorkerError::RootRequired) => {
            return Ok(AccountStatus::RootMissing { device_did });
        }
        Err(error) => return Err(error),
    };
    match load_provider(state).await? {
        None => Ok(AccountStatus::Unregistered {
            root_did: root.root_did.to_string(),
            device_did,
        }),
        Some(record) => {
            let account_state = if record.descriptor.is_some() {
                super::account_state::status(state).await
            } else {
                AccountStateStatus::Unconfigured
            };
            Ok(AccountStatus::Registered {
                root_did: root.root_did.to_string(),
                device_did,
                provider: record.provider,
                account_state,
            })
        }
    }
}

/// Return local-root and provider attachment state.
#[wasm_compat]
pub async fn get(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(status(&state).await?))
}

/// Commit a display name through the linked-account authority when present.
#[wasm_compat]
pub async fn set_display_name(
    State(state): State<AppState>,
    Json(request): Json<AccountDisplayNameRequest>,
) -> Result<Json<AccountDisplayNameResponse>, TonkWorkerError> {
    let tonk = state.read().await;
    match super::account_state::rename_display_name(&tonk, &request.name).await? {
        Some(response) => Ok(Json(response)),
        None => Ok(Json(AccountDisplayNameResponse {
            name: crate::router::profile_name::resolve_display_name(&tonk).await,
            convergence: Default::default(),
        })),
    }
}

/// Persist the service-selected descriptor winner for a legacy account.
///
/// An account created before repository descriptors existed is attached to a
/// provider but names no repository. The service picks one winner among the
/// devices racing to establish it; this stores exactly the bytes it returned,
/// never the caller's own losing candidate.
#[wasm_compat]
pub async fn establish_repository(
    State(state): State<AppState>,
    Json(request): Json<AccountRepositoryEstablishRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let tonk = state.read().await;
    let root = super::identity::local_root(&tonk).await?;
    let mut record = load_provider(&tonk)
        .await?
        .ok_or_else(|| TonkWorkerError::Conflict("no account provider is attached".to_string()))?;
    if record.descriptor.is_some() {
        return Err(TonkWorkerError::Conflict(
            "account repository is already configured".to_string(),
        ));
    }

    let descriptor = checked_descriptor(&request.descriptor_hex, &root.root_did).await?;
    record.descriptor = Some(descriptor.bytes().to_vec());
    save_provider(&tonk, &record).await?;
    // This profile now has an account routing key to hide where a moment ago
    // it had none.
    tonk.account_keys.invalidate();

    if request.created {
        if let Err(error) = super::account_state::initialize_display_name(&tonk).await {
            log!("initial account display-name seed did not complete: {error}");
        }
    } else {
        super::account_state::ensure_account_state(&tonk).await;
    }
    Ok(Json(status(&tonk).await?))
}

/// Validate that provider ceremony metadata exactly matches the local root,
/// then store provider metadata and the account repository descriptor without
/// changing authority.
pub(crate) async fn persist_link(
    state: &crate::worker::TonkState,
    request: &AccountLinkRequest,
) -> Result<(), TonkWorkerError> {
    if request.provider.trim().is_empty() {
        return Err(TonkWorkerError::Router(
            "provider must not be empty".to_string(),
        ));
    }
    let root = super::identity::local_root(state).await?;
    if request.root_did != root.root_did.to_string()
        || request.credential_id != root.credential_id
        || request.delegation_hex != hex::encode(&root.bytes)
    {
        return Err(TonkWorkerError::Forbidden(
            "provider ceremony does not match the persisted local root".to_string(),
        ));
    }
    let descriptor = checked_descriptor(&request.descriptor_hex, &root.root_did).await?;
    if let Some(existing) = load_provider(state).await? {
        if existing.provider != request.provider {
            return Err(TonkWorkerError::Conflict(
                "another account provider is already attached".to_string(),
            ));
        }
        // The descriptor is immutable: one account subject, one remote. A
        // second, different one would silently repoint this device's account
        // history, so it is a conflict rather than an update.
        if let Some(stored) = existing.descriptor.as_deref()
            && stored != descriptor.bytes()
        {
            return Err(TonkWorkerError::Conflict(
                "another account repository is already established".to_string(),
            ));
        }
    }
    let record = ProviderRecord {
        version: 1,
        provider: request.provider.trim_end_matches('/').to_string(),
        attached_at: web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        descriptor: Some(descriptor.bytes().to_vec()),
    };
    save_provider(state, &record).await?;
    // This profile now has an account repository to keep hidden.
    state.account_keys.invalidate();
    Ok(())
}

/// Attach provider services and start best-effort backup/restore.
#[wasm_compat]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let app_state = state.clone();
    let state = state.read().await;
    persist_link(&state, &request).await?;
    if request.initialize_name
        && let Err(error) = super::account_state::initialize_display_name(&state).await
    {
        log!("new-account display-name seed did not complete: {error}");
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_futures::spawn_local(async move {
        let state = app_state.read().await;
        // Mount/hydrate the hidden account repository before touching user
        // spaces. Remote failure leaves it retryable and unready.
        super::account_state::ensure_account_state(&state).await;
        crate::router::account_backup::back_up_existing_spaces(&state).await;
        crate::router::restore::restore_spaces(&state).await;
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        super::account_state::ensure_account_state(&state).await;
        crate::router::restore::restore_spaces(&state).await;
    }

    Ok(Json(status(&state).await?))
}

/// Disconnect provider services while preserving the local root and spaces.
#[wasm_compat]
pub async fn unlink(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .save(Vec::<u8>::new())
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to clear account provider: {error}"))
        })?;
    // The account repository is no longer this profile's to hide.
    state.account_keys.invalidate();
    Ok(Json(status(&state).await?))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn tests_matching_request(
    state: &crate::worker::TonkState,
) -> tonk_worker_api::AccountLinkRequest {
    // `test_state` persists a root derived from this exact seed, so a test can
    // re-derive it to sign a descriptor the local root will accept.
    let signer = dialog_credentials::Ed25519Signer::import(&[42u8; 32])
        .await
        .expect("the test root seed imports");
    let descriptor = AccountRepositoryDescriptorV1::sign(&signer, "https://accounts.example/ucan/")
        .await
        .expect("the test descriptor signs");
    let root = super::identity::local_root(state).await.unwrap();
    tonk_worker_api::AccountLinkRequest {
        provider: "https://accounts.tonk.xyz".into(),
        root_did: root.root_did.to_string(),
        credential_id: root.credential_id,
        delegation_hex: hex::encode(root.bytes),
        descriptor_hex: hex::encode(descriptor.bytes()),
        initialize_name: false,
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::tests::{test_state, test_state_without_root};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn matching_request(state: &crate::worker::TonkState) -> AccountLinkRequest {
        super::tests_matching_request(state).await
    }

    #[dialog_common::test]
    async fn it_reports_an_unregistered_local_root_without_an_account() {
        let state = Arc::new(RwLock::new(test_state().await));
        let Json(status) = get(State(state)).await.unwrap();
        assert!(matches!(status, AccountStatus::Unregistered { .. }));
    }

    #[dialog_common::test]
    async fn it_reports_a_missing_root_separately() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let Json(status) = get(State(state)).await.unwrap();
        assert!(matches!(status, AccountStatus::RootMissing { .. }));
    }

    #[dialog_common::test]
    async fn it_attaches_a_provider_without_replacing_the_root_grant() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = {
            let state = state.read().await;
            super::super::identity::local_root(&state)
                .await
                .unwrap()
                .bytes
        };
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        let after = {
            let state = state.read().await;
            super::super::identity::local_root(&state)
                .await
                .unwrap()
                .bytes
        };
        assert_eq!(before, after);
    }

    #[dialog_common::test]
    async fn it_stores_the_account_repository_descriptor_with_the_provider() {
        let state = Arc::new(RwLock::new(test_state().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let expected = request.descriptor_hex.clone();
        let _ = link(State(state.clone()), Json(request)).await.unwrap();

        let state = state.read().await;
        let stored = super::descriptor(&state)
            .await
            .expect("a descriptor is stored");
        assert_eq!(hex::encode(stored.bytes()), expected);
        assert_eq!(
            stored.account_subject(),
            &super::super::identity::local_root(&state)
                .await
                .unwrap()
                .root_did
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_descriptor_for_another_account_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let stranger = dialog_credentials::Ed25519Signer::import(&[9u8; 32])
            .await
            .unwrap();
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&stranger, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let request = {
            let state = state.read().await;
            AccountLinkRequest {
                descriptor_hex: hex::encode(descriptor.bytes()),
                ..matching_request(&state).await
            }
        };
        let error = link(State(state.clone()), Json(request)).await.unwrap_err();
        assert!(matches!(error, TonkWorkerError::Forbidden(_)));
    }

    #[dialog_common::test]
    async fn it_detaches_a_provider_without_revoking_or_rotating_the_device() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = state.read().await.profile.did();
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        let Json(status) = unlink(State(state.clone())).await.unwrap();
        assert!(matches!(status, AccountStatus::Unregistered { .. }));
        assert_eq!(state.read().await.profile.did(), before);
    }
}

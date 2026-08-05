//! Attach optional provider services to the provider-neutral local root, and
//! name the account repository that root owns.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::{AccountProviderRecord, AccountRepositoryDescriptorV1, AccountStateStatus};
use tonk_common::log;
use tonk_worker_api::{
    AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest,
    AccountRepositoryEstablishRequest, AccountStatus,
};

use super::AppState;
use crate::TonkWorkerError;

const ACCOUNT_PROVIDER_SITE: &str = tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE;

/// Map an attachment failure onto the router's error taxonomy. A rejected
/// descriptor is the caller presenting the wrong account's bytes, not a local
/// fault, so it answers 403 rather than 500.
fn provider_error(error: tonk_account::AccountProviderError) -> TonkWorkerError {
    use tonk_account::AccountProviderError as E;
    match error {
        E::EmptyProvider => TonkWorkerError::Router(error.to_string()),
        E::DescriptorEstablished => TonkWorkerError::Conflict(error.to_string()),
        E::Descriptor(_) | E::DescriptorSubject => TonkWorkerError::Forbidden(error.to_string()),
        E::Encoding(_) | E::UnsupportedVersion(_) => TonkWorkerError::Internal(error.to_string()),
    }
}

async fn load_provider(
    state: &crate::worker::TonkState,
    root_did: &dialog_varsig::Did,
) -> Result<Option<AccountProviderRecord>, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load account provider: {error}"
            )));
        }
    };
    AccountProviderRecord::decode(&bytes, root_did)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("stored account provider is unusable: {error}"))
        })
}

async fn save_provider(
    state: &crate::worker::TonkState,
    record: &AccountProviderRecord,
) -> Result<(), TonkWorkerError> {
    let bytes = record.encode().map_err(|error| {
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

/// The stored attachment, resolved against this device's local root.
///
/// Fail-safe: no root, an unreadable record, or a descriptor bound to another
/// account all resolve to `None`, so the device behaves as an unattached one
/// and keeps working rather than failing every account read.
async fn attachment(state: &crate::worker::TonkState) -> Option<AccountProviderRecord> {
    let root = super::identity::local_root(state).await.ok()?;
    match load_provider(state, &root.root_did).await {
        Ok(record) => record,
        Err(error) => {
            log!("account provider attachment unusable: {error}");
            None
        }
    }
}

/// Attached provider base URL, if any.
pub(crate) async fn provider(state: &crate::worker::TonkState) -> Option<String> {
    attachment(state)
        .await
        .map(|record| record.provider().to_owned())
}

/// The root-signed descriptor naming this account's repository and remote.
pub(crate) async fn descriptor(
    state: &crate::worker::TonkState,
) -> Option<AccountRepositoryDescriptorV1> {
    attachment(state).await?.descriptor().cloned()
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

/// Refuse unless this device holds an account.
///
/// The precondition every durable operation shares. Durable authority is only
/// ever issued to an account: a spot created without one is local-only and
/// un-backed-up, and a membership claimed without one has nothing revocable
/// behind it. `Unhydrated` and `Unconfigured` accounts pass — those are
/// synchronization states of an account that exists, and refusing on them
/// would invent a way to be stuck with no way out.
///
/// [`attachment`] resolves the record against the local root and is fail-safe,
/// so a missing root, an unreadable record and another account's descriptor
/// all land here as "no account" rather than as an error the caller has to
/// distinguish.
pub(crate) async fn require_account(
    state: &crate::worker::TonkState,
) -> Result<(), TonkWorkerError> {
    match attachment(state).await {
        Some(_) => Ok(()),
        None => Err(TonkWorkerError::AccountRequired),
    }
}

/// Attach a descriptor-less provider record to the test profile's root.
///
/// The cheapest thing that satisfies [`require_account`]: an account exists,
/// its repository is not established yet. Signing a descriptor here would fix
/// one, and a test that wants a specific one signs its own.
///
/// The provider URL matches [`tests_matching_request`] deliberately. A test
/// that links on top of this fixture is then an upgrade — the same account
/// gaining its descriptor — rather than a second account arriving, which
/// `persist_link` refuses.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn attach_test_account(
    state: &crate::worker::TonkState,
) -> Result<(), TonkWorkerError> {
    let record = AccountProviderRecord::attach_unconfigured(TEST_ACCOUNT_PROVIDER, 0)
        .map_err(provider_error)?;
    save_provider(state, &record).await
}

/// The provider both test fixtures name. See [`attach_test_account`].
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) const TEST_ACCOUNT_PROVIDER: &str = "https://accounts.tonk.xyz";

/// Detach the test account, leaving the profile's root and spaces alone.
///
/// The state a device reaches by signing out (`unlink`): local authority and
/// every replica intact, no account behind them. The local profile-name path
/// belongs to it, so the tests that cover that path end up here rather than in
/// a state the account gate no longer allows — a profile that created spaces
/// without ever having an account.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn detach_test_account(
    state: &crate::worker::TonkState,
) -> Result<(), TonkWorkerError> {
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
    state.account_keys.invalidate();
    Ok(())
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
    match load_provider(state, &root.root_did).await? {
        None => Ok(AccountStatus::Unregistered {
            root_did: root.root_did.to_string(),
            device_did,
        }),
        Some(record) => {
            let account_state = if record.descriptor().is_some() {
                super::account_state::status(state).await
            } else {
                AccountStateStatus::Unconfigured
            };
            Ok(AccountStatus::Registered {
                root_did: root.root_did.to_string(),
                device_did,
                provider: record.provider().to_owned(),
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
    let attached = load_provider(&tonk, &root.root_did)
        .await?
        .ok_or_else(|| TonkWorkerError::Conflict("no account provider is attached".to_string()))?;
    let descriptor = hex::decode(&request.descriptor_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid descriptor hex: {error}")))?;
    let record = attached
        .establish(&descriptor, &root.root_did)
        .await
        .map_err(provider_error)?;
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
    let root = super::identity::local_root(state).await?;
    if request.root_did != root.root_did.to_string()
        || request.credential_id != root.credential_id
        || request.delegation_hex != hex::encode(&root.bytes)
    {
        return Err(TonkWorkerError::Forbidden(
            "provider ceremony does not match the persisted local root".to_string(),
        ));
    }
    let descriptor = hex::decode(&request.descriptor_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid descriptor hex: {error}")))?;
    let now = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let record = AccountProviderRecord::attach(&request.provider, &descriptor, &root.root_did, now)
        .await
        .map_err(provider_error)?;

    if let Some(existing) = load_provider(state, &root.root_did).await? {
        if existing.provider() != record.provider() {
            return Err(TonkWorkerError::Conflict(
                "another account provider is already attached".to_string(),
            ));
        }
        // The descriptor is immutable: one account subject, one remote. A
        // second, different one would silently repoint this device's account
        // history, so it is a conflict rather than an update.
        if let Some(stored) = existing.descriptor()
            && stored.bytes() != descriptor
        {
            return Err(TonkWorkerError::Conflict(
                "another account repository is already established".to_string(),
            ));
        }
    }
    save_provider(state, &record).await?;
    // This profile now has an account repository to keep hidden.
    state.account_keys.invalidate();
    Ok(())
}

/// Attach provider services and finish bounded backup/restore before reporting
/// the account as linked.
#[wasm_compat]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    persist_link(&state, &request).await?;
    if request.initialize_name
        && let Err(error) = super::account_state::initialize_display_name(&state).await
    {
        log!("new-account display-name seed did not complete: {error}");
    }

    // Mount/hydrate the hidden account repository before touching user
    // spaces. Each account-service request is bounded by the shared HTTP
    // timeout, and awaiting the sequence keeps it inside the fetch lifetime.
    super::account_state::ensure_account_state(&state).await;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    crate::router::account_backup::back_up_existing_spaces(&state).await;
    crate::router::restore::restore_spaces(&state).await;

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
    // `test_state` persists a root derived from this profile's seed, so a test
    // can re-derive it to sign a descriptor the local root will accept.
    let signer = dialog_credentials::Ed25519Signer::import(&crate::router::tests::test_root_seed(
        &state.profile_name,
    ))
    .await
    .expect("the test root seed imports");
    let descriptor = AccountRepositoryDescriptorV1::sign(&signer, "https://accounts.example/ucan/")
        .await
        .expect("the test descriptor signs");
    let root = super::identity::local_root(state).await.unwrap();
    tonk_worker_api::AccountLinkRequest {
        provider: TEST_ACCOUNT_PROVIDER.into(),
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

    use crate::router::tests::{test_state_without_account, test_state_without_root};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn matching_request(state: &crate::worker::TonkState) -> AccountLinkRequest {
        super::tests_matching_request(state).await
    }

    #[dialog_common::test]
    async fn it_reports_an_unregistered_local_root_without_an_account() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
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
        let state = Arc::new(RwLock::new(test_state_without_account().await));
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
        let state = Arc::new(RwLock::new(test_state_without_account().await));
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
        let state = Arc::new(RwLock::new(test_state_without_account().await));
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
        let state = Arc::new(RwLock::new(test_state_without_account().await));
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

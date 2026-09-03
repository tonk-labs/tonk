//! Attach optional provider services to the provider-neutral local root, and
//! name the account repository that root owns.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::AccountProviderRecord;
use tonk_common::log;
use tonk_worker_api::{
    AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest, AccountStatus,
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
    _root_did: &dialog_varsig::Did,
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
    AccountProviderRecord::decode(&bytes).map_err(|error| {
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
pub(crate) async fn attachment(state: &crate::worker::TonkState) -> Option<AccountProviderRecord> {
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
        .map(|record| record.address().to_owned())
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

/// The DID membership rows are keyed on: the account this device acts
/// for, so a founder/member row converges across every device on the
/// same account. See [`current_account`].
pub(crate) async fn member_did(
    state: &crate::worker::TonkState,
) -> Result<dialog_varsig::Did, TonkWorkerError> {
    current_account(state).await.map(|(did, _)| did)
}

/// The account this device acts for, and its grant to the device: the
/// passkey root and its `root -> device` delegation when one is linked,
/// the onboarding account and its powerline otherwise, minted on first
/// use. Every device has one of the two, so a membership or a space
/// always terminates at an account and never at the device key.
pub(crate) async fn current_account(
    state: &crate::worker::TonkState,
) -> Result<(dialog_varsig::Did, dialog_ucan_core::DelegationChain), TonkWorkerError> {
    match super::identity::local_root(state).await {
        Ok(root) => Ok((root.root_did, root.delegation)),
        Err(TonkWorkerError::RootRequired) => {
            let grant = crate::onboarding::grant_device(state).await?;
            Ok((grant.issuer().clone(), grant))
        }
        Err(error) => Err(error),
    }
}

/// Whether this profile is linked, read from the account replica the
/// profile repository indexes rather than a stored flag (plan/Account
/// model.md §5). Two deliberate deviations from the pure signal:
///
/// - A legacy account attached before repository descriptors existed has
///   nothing mounted to read; its descriptor-less record stands in until
///   `establish_repository` upgrades it.
/// - A transient index read failure falls back to the stored attachment
///   instead of signing the profile out on a flaky read.
async fn linked(state: &crate::worker::TonkState) -> bool {
    match super::account_state::linked_account(state).await {
        Ok(Some(_)) => true,
        // An attachment with no account replica is a link that did not
        // finish — the record was written and the mount never landed —
        // so the record alone does not read as linked.
        Ok(None) => false,
        Err(error) => {
            log!("linked-state read failed, falling back to the stored attachment: {error}");
            attachment(state).await.is_some()
        }
    }
}

/// Attach a descriptor-less provider record to the test profile's root.
///
/// The cheapest thing that reads as linked: an account exists, its
/// repository is not established yet. Signing a descriptor here would fix
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
    let record = AccountProviderRecord::attach(TEST_ACCOUNT_REMOTE, 0).map_err(provider_error)?;
    save_provider(state, &record).await
}

/// Whether this profile carries ANY account-attachment history: a
/// stored provider record (configured or not) or the sign-out
/// tombstone. Only a profile with no history at all — a creation
/// ceremony whose registration never completed — may have its root
/// replaced by a retry; a signed-out profile keeps refusing a
/// different root, because its spaces still hang off the stored one.
pub(crate) async fn has_attachment_history(state: &crate::worker::TonkState) -> bool {
    match state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(_) => true,
        Err(error) if crate::credential::is_missing(&error) => false,
        // An unreadable record still counts as history: refusing a
        // replacement is recoverable, silently rebinding is not.
        Err(_) => true,
    }
}

/// The provider both test fixtures name. See [`attach_test_account`].
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) const TEST_ACCOUNT_PROVIDER: &str = "https://accounts.tonk.xyz";
/// Where a test account syncs.
#[allow(dead_code)]
pub(crate) const TEST_ACCOUNT_REMOTE: &str = "https://accounts.tonk.xyz/ucan/";

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
        // The record is provider metadata, not the linked flag: an
        // attachment whose account was never mounted is a link that
        // did not complete. That state is ordinary mid-enrollment — a
        // ceremony writes the record and the replica mount lands as
        // its own commit — so a status read that lands between the two
        // must not report the signed-out answer a page acts on.
        // Mounting is idempotent and serialized, so run it here and
        // answer from the outcome: healed reads as the registered
        // account it is, and only a profile the mount cannot configure
        // (no address anywhere) stays unregistered.
        Some(_) if !linked(state).await => {
            let _ = super::account_state::ensure_account_state(state).await;
            match load_provider(state, &root.root_did).await? {
                Some(record) if linked(state).await => {
                    let account_state = super::account_state::status(state).await;
                    Ok(AccountStatus::Registered {
                        root_did: root.root_did.to_string(),
                        device_did,
                        provider: record.address().to_owned(),
                        account_state,
                    })
                }
                _ => Ok(AccountStatus::Unregistered {
                    root_did: root.root_did.to_string(),
                    device_did,
                }),
            }
        }
        Some(record) => {
            let account_state = super::account_state::status(state).await;
            Ok(AccountStatus::Registered {
                root_did: root.root_did.to_string(),
                device_did,
                provider: record.address().to_owned(),
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
        })),
    }
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
    let now = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // The page names where the account syncs in `remote`; `provider`
    // stands in only for a request from before the two collapsed.
    let address = Some(request.remote.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(request.provider.trim());
    let record = AccountProviderRecord::attach(address, now).map_err(provider_error)?;

    if let Some(existing) = load_provider(state, &root.root_did).await?
        && existing.address() != record.address()
    {
        return Err(TonkWorkerError::Conflict(
            "another account provider is already attached".to_string(),
        ));
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
    // Everything created or joined before this account existed hangs off
    // the onboarding account; re-issue it to the root from the custodied
    // seeds ahead of the backup sweep, so what gets backed up is the
    // account-rooted authority, and retire the onboarding account.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    super::rotation::rotate_from_onboarding(&state).await;

    // Roster upkeep: this profile just became an account row. The email
    // comes best-effort from the provider; a failed fetch leaves it
    // blank until a later refresh.
    let email = super::account_devices::account_summary(&state)
        .await
        .ok()
        .and_then(|summary| summary.email);
    super::profiles::upsert_active_entry(&state, email).await;

    Ok(Json(status(&state).await?))
}

/// Disconnect provider services while preserving the local root and spaces.
#[wasm_compat]
pub async fn unlink(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    // The replica retraction is the unlink: it clears the linked-state
    // signal sync and status read. It goes first so a failure leaves the
    // device consistently linked rather than half signed out.
    super::account_state::retract_account_replicas(&state).await?;
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
    // Roster upkeep: with no provider attached the entry's account
    // fields clear, so the switcher renders this row as a local
    // workspace. The persisted root stays, so signing back in with the
    // same passkey still short-circuits in place.
    super::profiles::upsert_active_entry(&state, None).await;
    Ok(Json(status(&state).await?))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn tests_matching_request(
    state: &crate::worker::TonkState,
) -> tonk_worker_api::AccountLinkRequest {
    let root = super::identity::local_root(state).await.unwrap();
    tonk_worker_api::AccountLinkRequest {
        provider: TEST_ACCOUNT_PROVIDER.into(),
        root_did: root.root_did.to_string(),
        credential_id: root.credential_id,
        delegation_hex: hex::encode(root.bytes),
        remote: TEST_ACCOUNT_REMOTE.to_string(),
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
    /// The roster keeps its entry across link and unlink: it records which
    /// profiles this device can open, which does not change when one signs
    /// out.
    ///
    /// It carries no account state to assert on any more. Whether a profile
    /// is signed in is the `account -> profile` delegation, not a roster
    /// stamp that could disagree with it, so this pins only that the entry
    /// survives and still names the handle to open.
    #[dialog_common::test]
    async fn it_keeps_the_roster_entry_across_link_and_unlink() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        {
            let tonk = state.read().await;
            let roster = tonk
                .registry
                .read_roster(&tonk.storage, &tonk.operator)
                .await
                .unwrap();
            roster
                .iter()
                .find(|entry| entry.profile_name == tonk.profile_name)
                .expect("link writes the profile's roster entry");
        }

        let _ = unlink(State(state.clone())).await.unwrap();

        let tonk = state.read().await;
        let roster = tonk
            .registry
            .read_roster(&tonk.storage, &tonk.operator)
            .await
            .unwrap();
        roster
            .iter()
            .find(|entry| entry.profile_name == tonk.profile_name)
            .expect("unlink keeps the roster entry: the profile is still openable");
    }

    #[dialog_common::test]
    async fn it_reads_linked_state_from_the_replica_signal() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();

        let tonk = state.read().await;
        let root = super::super::identity::local_root(&tonk).await.unwrap();
        let linked = super::super::account_state::linked_account(&tonk)
            .await
            .unwrap()
            .expect("link records the account replica");
        assert_eq!(linked, root.root_did);
    }

    #[dialog_common::test]
    async fn it_finishes_an_unmounted_attachment_at_status_time() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        // Persist the attachment record directly, without the mount that
        // link performs: the state a crash mid-link leaves behind, and
        // also the ordinary mid-enrollment window between a ceremony's
        // record write and its replica-mount commit. The record alone
        // must not READ as a linked account — but a status read that
        // finds it runs the idempotent mount and answers from the
        // outcome, so an interrupted link with a usable address heals
        // into the registered account it was becoming rather than
        // reporting the signed-out answer.
        {
            let tonk = state.read().await;
            let request = matching_request(&tonk).await;
            let record = AccountProviderRecord::attach(&request.remote, 1).unwrap();
            save_provider(&tonk, &record).await.unwrap();

            assert!(!linked(&tonk).await);
        }
        let Json(status) = get(State(state.clone())).await.unwrap();
        assert!(matches!(status, AccountStatus::Registered { .. }));
        let tonk = state.read().await;
        assert!(
            linked(&tonk).await,
            "the status read completes the mount, not merely reports it"
        );
    }

    #[dialog_common::test]
    async fn it_retracts_the_replica_signal_on_unlink() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        let _ = unlink(State(state.clone())).await.unwrap();

        let tonk = state.read().await;
        assert!(
            super::super::account_state::linked_account(&tonk)
                .await
                .unwrap()
                .is_none(),
            "unlink retracts the account replica"
        );
        assert!(!linked(&tonk).await);
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

//! Attach optional provider services to the provider-neutral local root, and
//! name the account repository that root owns.

use dialog_operator::Profile;
use tonk_account::AccountProviderRecord;
use tonk_common::log;
use tonk_worker_api::{AccountLinkRequest, AccountStatus};

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
use super::AppState;
use crate::TonkWorkerError;
use crate::worker::DefaultOperator;

const ACCOUNT_PROVIDER_SITE: &str = tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE;

// Only the browser's account ceremonies reach this now; the CLI links
// through its own path.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
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
    load_provider_from(&state.profile, &state.operator, &state.active_branch).await
}

async fn load_provider_from(
    profile: &Profile,
    operator: &DefaultOperator,
    branch: &str,
) -> Result<Option<AccountProviderRecord>, TonkWorkerError> {
    let bytes = match profile
        .credential()
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, branch).as_str())
        .load::<Vec<u8>>()
        .perform(operator)
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
    if bytes.is_empty() {
        return Ok(None);
    }
    AccountProviderRecord::decode(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored account provider is unusable: {error}"))
    })
}

// Only the browser's account ceremonies reach this now; the CLI links
// through its own path.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
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
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, &state.active_branch).as_str())
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
#[cfg(all(
    any(test, feature = "helpers"),
    target_arch = "wasm32",
    target_os = "unknown"
))]
pub(crate) async fn attach_test_account(
    state: &crate::worker::TonkState,
) -> Result<(), TonkWorkerError> {
    let record = AccountProviderRecord::attach(TEST_ACCOUNT_REMOTE, 0).map_err(provider_error)?;
    save_provider(state, &record).await
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
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, &state.active_branch).as_str())
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

// Only the browser's account ceremonies reach this now; the CLI links
// through its own path.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
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
    publish_link(state).await;
    Ok(())
}

/// Publish whether this device holds an account link on the active
/// branch, as the `state:account-link` overlay row. The facts on a branch
/// describe the account and stay after a sign-out there, so a view can
/// only tell a linked branch by this row: asserted while a link exists,
/// retracted otherwise, and re-published whenever a state boots. The
/// local-root rows ride along, since a link and a root change together.
pub(crate) async fn publish_link(state: &crate::worker::TonkState) {
    use tonk_schema::{AccountLink, prelude::DidExt as _};

    let Ok(this) = AccountLink::ENTITY.parse::<dialog_artifacts::Entity>() else {
        return;
    };
    let linked = account_link(state).await.map(|chain| chain.issuer().this());
    let main = match state
        .reactor
        .profile_repository()
        .branch(&state.active_branch)
        .acquire(&state.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("account link row: open the active branch: {error}");
            return;
        }
    };
    match linked {
        Some(account) => main.state.assert_overlay(AccountLink::new(this, account)),
        None => {
            main.state
                .retain_overlay_entities(|overlaid| overlaid != &this);
        }
    }
    state
        .reactor
        .schedule_poll(std::sync::Arc::clone(&main.state));
    state.reactor.run_scheduled_polls(&state.operator).await;
    super::identity::publish_local_root(state).await;
}

// Only the browser's account ceremonies reach this now; the CLI links
// through its own path.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
/// Finish the bounded local setup for a provider attachment already persisted.
///
/// The HTTP attachment route and the worker-owned passkey-login ceremony meet
/// here. It runs the first bounded hydration/convergence attempt and refreshes
/// the roster before returning; with a healthy account remote that projects
/// the authoritative display name into the local profile synchronously instead
/// of leaving it to a later background sweep. Callers must not report login
/// complete after only writing the provider record.
pub(crate) async fn finish_link(
    state: &crate::worker::TonkState,
) -> Result<AccountStatus, TonkWorkerError> {
    // Mount/hydrate the hidden account repository before touching user
    // spaces. Each account-service request is bounded by the shared HTTP
    // timeout, and awaiting the sequence keeps it inside the fetch lifetime.
    super::account_state::ensure_account_state(state).await;
    // Everything created or joined before this account existed hangs off
    // the onboarding account; re-issue it to the root from the custodied
    // seeds ahead of the backup sweep, so what gets backed up is the
    // account-rooted authority, and retire the onboarding account.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    super::rotation::rotate_from_onboarding(state).await;

    // Roster upkeep: this profile just became an account row. The email
    // comes best-effort from the provider; a failed fetch leaves it
    // blank until a later refresh.
    let email = super::account_devices::account_summary(state)
        .await
        .ok()
        .and_then(|summary| summary.email);
    super::profiles::upsert_active_entry(state, email).await;

    status(state).await
}

/// Attach `request`'s provider and finish the bounded local setup, the
/// way the passkey ceremonies do — the fixture tests link an account with.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn link_for_test(
    state: &AppState,
    request: &AccountLinkRequest,
) -> Result<AccountStatus, TonkWorkerError> {
    let state = state.read().await;
    persist_link(&state, request).await?;
    finish_link(&state).await
}

/// Disconnect provider services while preserving the local root and spaces.
pub(crate) async fn disconnect(
    state: &crate::worker::TonkState,
) -> Result<AccountStatus, TonkWorkerError> {
    // The account branch keeps its replica rows: they are that branch's
    // bookkeeping, and signing back in returns to it. What clears the
    // linked-state signal is leaving the branch, which the caller does
    // after this. Here only the provider goes, so nothing routes to the
    // account meanwhile.
    state
        .profile
        .credential()
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, &state.active_branch).as_str())
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
    super::profiles::upsert_active_entry(state, None).await;
    publish_link(state).await;
    status(state).await
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

/// The link row the hub bar reads: on while this device holds the
/// account's authority on the active branch, gone when it does not.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod link_row_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::helpers::state::test_state;
    use crate::router::{ClientId, api_router_with_state};

    wasm_bindgen_test_configure!(run_in_browser);

    async fn link_rows(app: &axum::Router, branch: &str) -> usize {
        let query = r#"{"predicate":{"with":{"account":{"the":"xyz.tonk.link/account","as":"Entity","cardinality":"one"}}},"terms":{"this":{"?":{"name":"this"}},"account":{"?":{"name":"account"}}}}"#;
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/profile/branch/{branch}/query"))
            .header("content-type", "application/json")
            .body(Body::from(query))
            .unwrap();
        request.extensions_mut().insert(ClientId("bar".to_owned()));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|rows| rows.as_array().map(Vec::len))
            .unwrap_or(0)
    }

    #[dialog_common::test]
    async fn it_publishes_the_link_row_while_the_device_holds_the_account() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let branch = state.read().await.active_branch.clone();
        {
            let tonk = state.read().await;
            super::publish_link(&tonk).await;
        }
        assert_eq!(
            link_rows(&app, &branch).await,
            1,
            "a linked device publishes one link row on its branch",
        );

        {
            let tonk = state.read().await;
            super::disconnect(&tonk).await.unwrap();
        }
        assert_eq!(
            link_rows(&app, &branch).await,
            0,
            "disconnecting retracts it, whatever account facts the branch keeps",
        );

        let request = {
            let tonk = state.read().await;
            super::tests_matching_request(&tonk).await
        };
        {
            let tonk = state.read().await;
            super::persist_link(&tonk, &request).await.unwrap();
        }
        assert_eq!(
            link_rows(&app, &branch).await,
            1,
            "linking again publishes it again",
        );
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::tests::{
        put_repo, test_state, test_state_without_account, test_state_without_root,
    };
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn matching_request(state: &crate::worker::TonkState) -> AccountLinkRequest {
        super::tests_matching_request(state).await
    }

    #[dialog_common::test]
    async fn it_reports_an_unregistered_local_root_without_an_account() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let status = super::status(&*state.read().await).await.unwrap();
        assert!(matches!(status, AccountStatus::Unregistered { .. }));
    }

    #[dialog_common::test]
    async fn it_reports_a_missing_root_separately() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let status = super::status(&*state.read().await).await.unwrap();
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
        let _ = crate::router::account::link_for_test(&state, &request)
            .await
            .unwrap();
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
        let _ = crate::router::account::link_for_test(&state, &request)
            .await
            .unwrap();
        let signed_out_profile = state.read().await.profile_name.clone();
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

        let _ = crate::router::profiles::sign_out(&state, None)
            .await
            .unwrap();

        let tonk = state.read().await;
        let roster = tonk
            .registry
            .read_roster(&tonk.storage, &tonk.operator)
            .await
            .unwrap();
        roster
            .iter()
            .find(|entry| entry.profile_name == signed_out_profile)
            .expect("unlink keeps the roster entry: the profile is still openable");
    }

    #[dialog_common::test]
    async fn it_reads_linked_state_from_the_replica_signal() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = crate::router::account::link_for_test(&state, &request)
            .await
            .unwrap();

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
        let status = super::status(&*state.read().await).await.unwrap();
        assert!(matches!(status, AccountStatus::Registered { .. }));
        let tonk = state.read().await;
        assert!(
            linked(&tonk).await,
            "the status read completes the mount, not merely reports it"
        );
    }

    /// Unlink drops the provider and leaves the branch; the branch keeps
    /// its replica rows for signing back in, but with no provider nothing
    /// reads them as a link.
    #[dialog_common::test]
    async fn it_keeps_the_account_branch_rows_but_drops_the_provider_on_unlink() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = crate::router::account::link_for_test(&state, &request)
            .await
            .unwrap();
        let account_branch = state.read().await.active_branch.clone();
        let _ = crate::router::profiles::sign_out(&state, None)
            .await
            .unwrap();
        let _ = super::super::profiles::activate_named(&state, account_branch, None)
            .await
            .unwrap();

        let tonk = state.read().await;
        assert!(
            super::super::account_state::linked_account(&tonk)
                .await
                .unwrap()
                .is_some(),
            "the account branch keeps its replica rows"
        );
        assert!(provider(&tonk).await.is_none(), "unlink drops the provider");
    }

    /// Signing out leaves the account's branch and forgets the grant. The
    /// branch keeps its spaces, out of view until it is returned to.
    #[dialog_common::test]
    async fn it_signs_out_by_leaving_the_branch_and_forgetting_the_root() {
        use tonk_schema::prelude::DidExt as _;

        let state: crate::router::AppState =
            std::sync::Arc::new(tokio::sync::RwLock::new(test_state().await));
        let key = put_repo(&state, "retained-local-space").await;
        let (account_branch, profile_did, root_key) = {
            let tonk = state.read().await;
            let root_key = super::super::identity::local_root(&tonk)
                .await
                .unwrap()
                .root_did
                .repo_key()
                .to_string();
            assert!(
                super::super::account_state::is_account_key(&tonk, &root_key).await,
                "the linked account key is hidden from generic repository routing"
            );
            (tonk.active_branch.clone(), tonk.profile.did(), root_key)
        };

        let status = crate::router::profiles::sign_out(&state, None)
            .await
            .unwrap();
        assert!(
            matches!(status, AccountStatus::RootMissing { .. }),
            "the grant is forgotten, so there is no root to report"
        );

        let tonk = state.read().await;
        assert_eq!(tonk.profile.did(), profile_did, "the device keeps its key");
        assert_ne!(
            tonk.active_branch, account_branch,
            "sign-out leaves the branch"
        );
        assert!(
            super::super::identity::load_record(&tonk)
                .await
                .unwrap()
                .is_none(),
            "the grant is forgotten"
        );
        assert!(provider(&tonk).await.is_none());
        assert!(
            !super::super::account_state::is_account_key(&tonk, &root_key).await,
            "sign-out releases hidden account-key routing"
        );
        drop(tonk);
        let signed_out = super::super::profile::space_keys(&state).await;
        assert!(
            !signed_out.contains(&key),
            "the post-sign-out hub must not render the signed-out account's spaces"
        );

        let _ = super::super::profiles::activate_named(&state, account_branch.clone(), None)
            .await
            .unwrap();
        let tonk = state.read().await;
        assert_eq!(tonk.active_branch, account_branch);
        assert!(
            provider(&tonk).await.is_none(),
            "returning to the branch is not signing in"
        );
        assert!(
            super::super::identity::load_record(&tonk)
                .await
                .unwrap()
                .is_none(),
            "returning does not restore the grant"
        );
        drop(tonk);

        let spaces = super::super::profile::space_keys(&state).await;
        assert!(spaces.contains(&key), "the branch kept its spaces");
        let repository = super::super::repository::load_repository_info(&state, &key)
            .await
            .expect("the retained space remains loadable");
        assert!(repository.remote.is_empty());
    }

    /// Unlink withdraws the grant without rotating the device: the key
    /// stays, the authority goes.
    #[dialog_common::test]
    async fn it_withdraws_the_grant_without_rotating_the_device() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let before = state.read().await.profile.did();
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = crate::router::account::link_for_test(&state, &request)
            .await
            .unwrap();
        let account_branch = state.read().await.active_branch.clone();
        assert!(account_link(&*state.read().await).await.is_some());

        let status = crate::router::profiles::sign_out(&state, None)
            .await
            .unwrap();
        assert!(
            matches!(status, AccountStatus::RootMissing { .. }),
            "the grant is forgotten, so there is no root to report"
        );

        let _ = super::super::profiles::activate_named(&state, account_branch, None)
            .await
            .unwrap();
        let tonk = state.read().await;
        assert_eq!(tonk.profile.did(), before, "the device keeps its key");
        assert!(
            account_link(&tonk).await.is_none(),
            "the grant is withdrawn, so nothing links the device to the account"
        );
        assert!(
            super::super::identity::load_record(&tonk)
                .await
                .unwrap()
                .is_none()
        );
    }
}

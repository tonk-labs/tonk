//! Attach optional provider services to the provider-neutral local root, and
//! name the account repository that root owns.

use crate::worker::DefaultProfile;
#[cfg(feature = "persona")]
use axum::{Extension, Json, extract::State};
use tonk_account::AccountProviderRecord;
use tonk_common::log;
#[cfg(feature = "persona")]
use tonk_macros::handler;
#[cfg(feature = "persona")]
use tonk_worker_api::{
    AccountDisplayNameRequest, AccountDisplayNameResponse, AccountLinkRequest, AccountStatus,
};

#[cfg(feature = "persona")]
use super::AppState;
use crate::TonkWorkerError;

const ACCOUNT_PROVIDER_SITE: &str = tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE;

/// Map an attachment failure onto the router's error taxonomy. A rejected
/// descriptor is the caller presenting the wrong account's bytes, not a local
/// fault, so it answers 403 rather than 500.
#[cfg(feature = "persona")]
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
    load_provider_from(&state.profile, &state.active_branch).await
}

async fn load_provider_from(
    profile: &DefaultProfile,
    branch: &str,
) -> Result<Option<AccountProviderRecord>, TonkWorkerError> {
    let bytes = match profile
        .secrets()
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, branch).as_str())
        .load::<Vec<u8>>()
        .perform(profile)
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

#[cfg(feature = "persona")]
async fn save_provider(
    state: &crate::worker::TonkState,
    record: &AccountProviderRecord,
) -> Result<(), TonkWorkerError> {
    let bytes = record.encode().map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize account provider: {error}"))
    })?;
    state
        .profile
        .secrets()
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, &state.active_branch).as_str())
        .save(bytes)
        .perform(&state.profile)
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
    if let Some(account) = acts_for(state).await? {
        return Ok(account);
    }
    // A kernel acts under its own authority until the person signs in.
    #[cfg(target_arch = "wasm32")]
    if let Err(TonkWorkerError::RootRequired) = super::identity::local_root(state).await {
        return Ok(state.profile.did());
    }
    current_account(state).await.map(|(did, _)| did)
}

/// The profile secret a space's own worker keeps the account it acts for
/// under.
const ACTS_FOR_SITE: &str = "tonk-acts-for-account";

/// The account this worker acts for on someone else's say-so, when it is a
/// space's own worker.
///
/// Such a worker runs under a profile of its own, generated on its origin,
/// which belongs to no account. The person whose profile handed it the
/// space tells it which account that is ([`act_for`]), and it is that
/// account, not one this worker would mint for itself, that the space's
/// memberships name. `None` in the person's own worker, which answers from
/// its own account ([`current_account`]).
pub(crate) async fn acts_for(
    state: &crate::worker::TonkState,
) -> Result<Option<dialog_varsig::Did>, TonkWorkerError> {
    let bytes = match state
        .profile
        .secrets()
        .site(ACTS_FOR_SITE)
        .load::<Vec<u8>>()
        .perform(&state.profile)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load the account this worker acts for: {error}"
            )));
        }
    };
    String::from_utf8(bytes)
        .ok()
        .and_then(|did| did.parse().ok())
        .map(Some)
        .ok_or_else(|| {
            TonkWorkerError::Internal("the account this worker acts for is not a DID".to_string())
        })
}

/// Record the account this worker, a space's own, acts for.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn act_for(
    state: &crate::worker::TonkState,
    account: &dialog_varsig::Did,
) -> Result<(), TonkWorkerError> {
    state
        .profile
        .secrets()
        .site(ACTS_FOR_SITE)
        .save(account.to_string().into_bytes())
        .perform(&state.profile)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to keep the account this worker acts for: {error}"
            ))
        })
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
            state.registry.account()?;
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
#[cfg(feature = "persona")]
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

/// Where a test account syncs.
#[allow(dead_code)]
pub(crate) const TEST_ACCOUNT_REMOTE: &str = "https://accounts.tonk.xyz/ucan/";

#[cfg(feature = "persona")]
pub(super) async fn status(
    state: &crate::worker::TonkState,
) -> Result<AccountStatus, TonkWorkerError> {
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
#[cfg(feature = "persona")]
#[handler]
pub async fn get(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(status(&state).await?))
}

/// Commit a display name through the linked-account authority when present.
#[cfg(feature = "persona")]
#[handler]
pub async fn set_display_name(
    State(state): State<AppState>,
    Json(request): Json<AccountDisplayNameRequest>,
) -> Result<Json<AccountDisplayNameResponse>, TonkWorkerError> {
    let tonk = state.read().await.clone();
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
#[cfg(feature = "persona")]
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
/// retracted otherwise, and re-published whenever a state boots.
pub(crate) async fn publish_link(state: &crate::worker::TonkState) {
    let Some(branch) = write_link_row(state).await else {
        return;
    };
    state.reactor.schedule_poll(branch);
    state.reactor.run_scheduled_polls(&state.operator).await;
}

/// Assert or retract the link row, answering the branch it went on.
///
/// Writing the row bumps the branch's overlay epoch, so the poll that
/// delivers it re-evaluates every subscription on the branch, and some
/// await the network. [`publish_link`] runs that poll for the views open
/// now; a caller about to replace the state leaves it.
async fn write_link_row(
    state: &crate::worker::TonkState,
) -> Option<std::sync::Arc<dialog_reactor::BranchState>> {
    use tonk_schema::{AccountLink, prelude::DidExt as _};

    let this = AccountLink::ENTITY
        .parse::<dialog_artifacts::Entity>()
        .ok()?;
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
            return None;
        }
    };
    match linked {
        Some(account) => main.state.assert_overlay(AccountLink::new(this, account)),
        None => {
            main.state
                .retain_overlay_entities(|overlaid| overlaid != &this);
        }
    }
    Some(main.state)
}

/// Attach provider services and finish bounded backup/restore before reporting
/// the account as linked.
#[cfg(feature = "persona")]
#[handler]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let app = state.clone();
    let state = state.read().await.clone();
    persist_link(&state, &request).await?;
    if request.initialize_name
        && let Err(error) = super::account_state::initialize_display_name(&state).await
    {
        log!("new-account display-name seed did not complete: {error}");
    }

    let status = finish_link(&state).await?;
    // The answer is ready; the push goes behind it rather than ahead of
    // it (see `Publish`), once this read guard is released.
    drop(state);
    super::account_state::push_after_answer(app).await;
    Ok(Json(status))
}

/// Finish the bounded local setup for a provider attachment already persisted.
///
/// The HTTP attachment route and the worker-owned passkey-login ceremony meet
/// here. It runs the first bounded hydration/convergence attempt and refreshes
/// the roster before returning; with a healthy account remote that projects
/// the authoritative display name into the local profile synchronously instead
/// of leaving it to a later background sweep. Callers must not report login
/// complete after only writing the provider record.
#[cfg(feature = "persona")]
pub(crate) async fn finish_link(
    state: &crate::worker::TonkState,
) -> Result<AccountStatus, TonkWorkerError> {
    let mut timing = super::link_timing::LinkTiming::new("attachment", "hydrate-account");
    // Mount/hydrate the hidden account repository before touching user
    // spaces. Each account-service request is bounded by the shared HTTP
    // timeout, and awaiting the sequence keeps it inside the fetch lifetime.
    super::account_state::ensure_account_state(state).await;
    timing.next("public-powerline");
    // Public spaces are read through the account's powerline from the
    // public principal. Best-effort: opening one mints it if this did not.
    if let Ok(root) = super::identity::local_root(state).await
        && let Err(error) = super::public_access::ensure_powerline(state, &root.root_did).await
    {
        log!("public powerline was not minted at link: {error}");
    }
    timing.next("rotate-onboarding");
    // Everything created or joined before this account existed hangs off
    // the onboarding account; re-issue it to the root from the custodied
    // seeds ahead of the backup sweep, so what gets backed up is the
    // account-rooted authority, and retire the onboarding account.

    // Roster upkeep: this profile just became an account row. The email
    // comes best-effort from the provider; a failed fetch leaves it
    // blank until a later refresh.
    timing.next("account-summary");
    let email = super::account_devices::account_summary(state)
        .await
        .ok()
        .and_then(|summary| summary.email);
    timing.next("update-roster");
    super::profiles::upsert_active_entry(state, email).await;

    timing.next("read-status");
    status(state).await
}

/// Disconnect provider services while preserving the local root and spaces.
#[cfg(feature = "persona")]
pub(crate) async fn disconnect(
    state: &crate::worker::TonkState,
) -> Result<AccountStatus, TonkWorkerError> {
    // The account branch keeps its replica rows: they are that branch's
    // bookkeeping, and signing back in returns to it. What clears the
    // linked-state signal is leaving the branch, which the caller does
    // after this. Here only the provider goes, so nothing routes to the
    // account meanwhile.
    let mut timing = super::link_timing::LinkTiming::new("disconnect", "clear-provider");
    state
        .profile
        .secrets()
        .site(crate::credential::branch_site(ACCOUNT_PROVIDER_SITE, &state.active_branch).as_str())
        .save(Vec::<u8>::new())
        .perform(&state.profile)
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
    timing.next("update-roster");
    super::profiles::upsert_active_entry(state, None).await;
    // The row is retracted without the poll that would deliver it: the
    // caller swaps the state next, which drops this branch's subscribers
    // and reloads the page, and the state that boots publishes its own
    // row. On a real account that poll re-evaluated every subscription on
    // the branch, and held a sign-out for half a minute.
    timing.next("retract-link-row");
    let _ = write_link_row(state).await;
    timing.next("read-status");
    status(state).await
}

/// Disconnect this account, then leave its retained spaces behind by
/// promoting another signed-in profile or a rootless local workspace.
#[cfg(feature = "persona")]
#[handler]
pub async fn unlink(
    State(state): State<AppState>,
    source: Option<Extension<super::ClientId>>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let source = source.as_ref().map(|source| &source.0);
    super::profiles::sign_out(&state, source).await.map(Json)
}

//! Profile roster and switching — one profile per account, swapped in
//! place.
//!
//! Everything that should follow the active account is already scoped to
//! the worker profile (the replica index, the local root, the provider
//! attachment, the hidden account repository, the display name, the
//! certificate store), so switching accounts is switching profiles: build
//! a replacement [`TonkState`] for the target profile, repoint the
//! registry's active-profile pointer, and swap the value inside the
//! shared state handle. A page reload does not restart the service
//! worker, so the in-place swap is what a switch IS; the pointer write
//! only covers a genuine SW restart.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::ops::Deref;
use std::sync::{Arc, atomic::Ordering};

use axum::{Extension, Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::Entity;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_varsig::Did;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};

use super::AppState;
use crate::TonkWorkerError;
use crate::device::RosterEntry;
use crate::worker::TonkState;

/// How account routing selected the profile pinned by an
/// [`AccountProfileGuard`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) enum AccountProfileDisposition {
    /// The active profile was already the correct target.
    Current,
    /// A matching profile already present in the browser roster was activated.
    Existing,
    /// No existing profile owned the account, so a fresh one was created.
    Created,
}

/// A read lock that pins the account ceremony to the selected profile.
/// Profile changes queue behind this guard until all local account writes have
/// completed.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct AccountProfileGuard {
    tonk: tokio::sync::OwnedRwLockReadGuard<TonkState>,
    disposition: AccountProfileDisposition,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl AccountProfileGuard {
    pub(crate) fn disposition(&self) -> AccountProfileDisposition {
        self.disposition
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Deref for AccountProfileGuard {
    type Target = TonkState;

    fn deref(&self) -> &Self::Target {
        &self.tonk
    }
}

/// Refresh the active profile's roster entry from live state.
///
/// Best-effort: every caller is a moment that already succeeded (boot,
/// link, unlink, establish, rename), and a roster miss must not turn it
/// into a failure. A stale entry costs a stale switcher row, healed by
/// the next refresh.
pub(crate) async fn upsert_active_entry(tonk: &TonkState, email: Option<String>) {
    if let Err(error) = try_upsert_active_entry(tonk, email).await {
        log!("profile roster upsert skipped: {error}");
    }
}

async fn try_upsert_active_entry(
    tonk: &TonkState,
    _email: Option<String>,
) -> Result<(), TonkWorkerError> {
    tonk.registry
        .upsert_roster(
            &tonk.storage,
            &tonk.operator,
            &tonk.profile.did(),
            &tonk.profile_name,
        )
        .await
}

fn response_from(active: &str, roster: Vec<RosterEntry>) -> ProfilesResponse {
    ProfilesResponse {
        active: active.to_string(),
        profiles: roster
            .into_iter()
            .map(|entry| ProfileRosterEntry {
                active: entry.profile_name == active,
                profile_name: entry.profile_name,
                root_did: entry.root_did,
                provider: entry.provider,
                email: entry.email,
                display_name: entry.display_name,
            })
            .collect(),
    }
}

/// Every branch of this profile as a switcher row: the account it follows
/// (if any), that account's name read off the branch, and where the
/// account is served from. Republished to the active branch's overlay so
/// a sealed guest, which can read only that branch, sees them all.
async fn refreshed_roster(tonk: &TonkState) -> Result<Vec<RosterEntry>, TonkWorkerError> {
    let mut roster = Vec::new();
    let mut rows = Vec::new();
    for (name, entity) in super::profile::local_branches(tonk).await {
        let account = super::profile::account_of_branch(tonk, &entity).await;
        let display_name = match &account {
            Some(account) => {
                super::account_devices::account_display_name_on(tonk, &name, account).await
            }
            None => None,
        };
        let provider = match &account {
            Some(_) => super::profile::provider_of_branch(tonk, &entity).await,
            None => None,
        };
        roster.push(RosterEntry {
            profile_name: name.clone(),
            root_did: account.as_ref().map(ToString::to_string),
            provider: provider.clone(),
            email: None,
            display_name: display_name.clone(),
        });
        rows.push(SwitcherRow {
            branch: entity,
            name,
            label: display_name,
            provider,
        });
    }
    publish_roster_overlay(tonk, &rows).await;
    Ok(roster)
}

/// One switcher row, keyed on the branch it is for.
struct SwitcherRow {
    branch: Entity,
    name: String,
    label: Option<String>,
    provider: Option<String>,
}

/// Republish the switcher rows as overlay facts on the active branch, so
/// a sealed guest can render the switcher from a query.
///
/// The guest can only read the ACTIVE branch; every other branch's name
/// and provider live where it cannot reach. The worker has just read
/// them all, so it stamps what it found where the guest is looking.
///
/// Overlay rather than a durable write, which is the whole point: a
/// committed copy would be a second home for a name owned elsewhere, free
/// to disagree after a rename on another device. These rows are rebuilt
/// from source on every roster read and vanish with the session.
///
/// Best-effort. The switcher is a convenience; `GET /api/profiles` remains
/// the authority and is unaffected if this write fails.
async fn publish_roster_overlay(tonk: &TonkState, rows: &[SwitcherRow]) {
    use tonk_schema::ProfileRow;

    // Cardinality-one fields supersede in place, so re-asserting a row
    // updates it rather than stacking a second copy.
    let mut overlay = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .overlay();
    for row in rows {
        overlay = overlay.assert(ProfileRow::new(
            row.branch.clone(),
            row.name.clone(),
            row.label.as_deref(),
            row.provider.as_deref(),
            row.name == tonk.active_branch,
        ));
    }
    if let Err(error) = overlay.write().perform(&tonk.operator).await {
        log!("switcher rows not published: {error}");
    }
}

/// Publish the switcher rows for the branch the profile booted onto.
///
/// The rows are session overlay facts, so a fresh worker holds none
/// until a roster read republishes them. The hub renders the account
/// list from them and nothing on the page reads the roster on its own,
/// so a worker restart would otherwise show an empty list until the
/// next `GET /api/profiles`. Best-effort, like the link row beside it.
pub(crate) async fn publish_roster(tonk: &TonkState) {
    if let Err(error) = refreshed_roster(tonk).await {
        log!("switcher rows not published at boot: {error}");
    }
}

/// The roster with every branch's live label and account state.
async fn refreshed_response(tonk: &TonkState) -> Result<ProfilesResponse, TonkWorkerError> {
    try_upsert_active_entry(tonk, None).await?;
    let roster = refreshed_roster(tonk).await?;
    Ok(response_from(&tonk.active_branch, roster))
}

/// `GET /api/profiles`.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
    let tonk = state.read().await;
    Ok(Json(refreshed_response(&tonk).await?))
}

/// `POST /api/profiles/activate`.
#[wasm_compat]
pub async fn activate(
    State(state): State<AppState>,
    source: Option<Extension<super::ClientId>>,
    Json(request): Json<ActivateProfileRequest>,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
    let source = source.as_ref().map(|source| &source.0);
    activate_named(&state, request.profile, source)
        .await
        .map(Json)
}

/// Switch to branch `name`.
///
/// Switching away from an account signs out of it: the profile holds at
/// most one grant, and the branch being left keeps its data for when it
/// is returned to. The target is validated before anything is withdrawn,
/// so a mistyped name costs nothing.
async fn activate_named(
    state: &AppState,
    name: String,
    source: Option<&super::ClientId>,
) -> Result<ProfilesResponse, TonkWorkerError> {
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    let (storage, profile_name, profile, registry, profile_library) = {
        let tonk = state.read().await;
        if name == tonk.active_branch {
            return refreshed_response(&tonk).await;
        }
        if !super::profile::local_branches(&tonk)
            .await
            .iter()
            .any(|(known, _)| *known == name)
        {
            return Err(TonkWorkerError::NotFound(format!(
                "no branch '{name}' on this profile"
            )));
        }
        // The branch keeps its link: what this device holds about an
        // account lives with the branch that follows it, so switching
        // neither revokes nor forgets it. Only signing out does.
        super::profile::set_active_branch(&tonk, &name).await?;
        (
            tonk.storage.clone(),
            tonk.profile_name.clone(),
            tonk.profile.clone(),
            tonk.registry.clone(),
            tonk.profile_library.clone(),
        )
    };

    // Build the replacement state WITHOUT holding the state write lock —
    // booting onto the branch awaits storage IO, and in-flight requests
    // must keep being served meanwhile.
    let new_state = crate::worker::boot_state_with_profile_library(
        storage,
        profile_name,
        profile,
        registry,
        profile_library,
    )
    .await?;
    let response = promote(state, new_state, source).await?;
    // The tab that asked cannot await a command the way it awaited the
    // endpoint, so it no longer reloads itself. Its requests are fenced
    // from here on; reload it once the swap is published.
    super::navigate::notify_profile_changed_to(source);
    Ok(response)
}

/// Run the [`AddProfile`] command.
///
/// The declarative twin of `POST /api/profiles/add`. Rotating onto a
/// fresh profile is worker work and happens here; the ceremony that
/// follows is a top-page dialog with a passkey prompt, so the page is
/// asked to raise it.
///
/// The order matters: rotate first, notify second. A ceremony opened
/// before the rotation would run against the outgoing profile and write
/// its account onto the wrong one.
///
/// [`AddProfile`]: tonk_schema::command::AddProfile
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AddProfile> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::AddProfile) {
        if let Err(error) = add_profile(self.state(), self.client()).await {
            log!("AddProfile failed: {error}");
            return;
        }
        // The ceremony is NOT raised from here. It is a top-page passkey
        // dialog whose handler lives behind the portal bridge, reachable
        // only from a guest calling `window.tonk.register`; a worker
        // message routed through `tonk-host` reaches the top page, where
        // there is no `window.tonk` to forward to, and intercepting that
        // path stopped the real handler from ever seeing the request.
        //
        // So the rotation is all this command does, and the view raises
        // the ceremony from the guest as it always has.

        // Report the ask, so the hub can render "a signup is up" from a
        // fact rather than from element state. The element carried this
        // on the document body precisely because a re-render replaced it
        // mid-ceremony; an overlay row survives re-renders by not living
        // in the DOM at all.
        //
        // The worker does not drive this ceremony to completion — the
        // page's signup does — so the terminal states are written by
        // whatever finishes it, not here.
        let tonk = self.state().read().await;
        super::ceremony::report(
            &tonk,
            tonk_schema::ceremony::ADD_PROFILE,
            tonk_schema::ceremony_state::PENDING_CEREMONY,
            "",
        )
        .await;
    }
}

/// Run the [`SwitchProfile`] command.
///
/// The declarative twin of `POST /api/profiles/activate`: a switcher row
/// dispatches this instead of the element fetching. Both land in
/// `activate_named`, so the validation that refuses a handle the roster
/// does not name covers the command path too — a guest cannot switch to a
/// profile this device has no record of.
///
/// [`SwitchProfile`]: tonk_schema::command::SwitchProfile
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::SwitchProfile>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::SwitchProfile) {
        let handle = command.handle.0;
        if handle.is_empty() {
            log!("SwitchProfile: empty handle, skipping");
            return;
        }
        // No source client: the switch was asked for from inside a guest,
        // so every window reloads, including the one that asked. The HTTP
        // route passes its caller so that tab keeps its response instead.
        if let Err(error) = activate_named(self.state(), handle.clone(), None).await {
            log!("SwitchProfile to '{handle}' failed: {error}");
        }
    }
}

/// `POST /api/profiles/add`.
///
/// Promote a fresh profile as the landing pad for Add Account. The account
/// ceremony may keep it for a new account or route to another roster profile
/// after discovering an existing account root.
#[wasm_compat]
pub async fn add(
    State(state): State<AppState>,
    source: Option<Extension<super::ClientId>>,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
    let source = source.as_ref().map(|source| &source.0);
    add_profile(&state, source).await.map(Json)
}

/// Start an empty branch to sign a new account in on.
///
/// Leaving the current account is what makes room: the profile holds at
/// most one grant, so the account being added mints its own on a branch
/// that follows nothing yet. A branch that already follows nothing and
/// holds no root is that landing pad already, and is handed back rather
/// than abandoned for another.
async fn add_profile(
    state: &AppState,
    source: Option<&super::ClientId>,
) -> Result<ProfilesResponse, TonkWorkerError> {
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    let (storage, name, profile, registry, profile_library) = {
        let tonk = state.read().await;
        if super::identity::load_record(&tonk).await?.is_none()
            && super::profile::active_account(&tonk).await.is_none()
        {
            let fresh = true;
            #[cfg(target_arch = "wasm32")]
            let fresh = fresh && super::profile_name::real_space_keys(&tonk).await.is_empty();
            if fresh {
                return refreshed_response(&tonk).await;
            }
        }
        // The branch being left keeps its link (see `activate_named`).
        super::profile::leave_account(&tonk).await;
        (
            tonk.storage.clone(),
            tonk.profile_name.clone(),
            tonk.profile.clone(),
            tonk.registry.clone(),
            tonk.profile_library.clone(),
        )
    };
    let new_state = crate::worker::boot_state_with_profile_library(
        storage,
        name,
        profile,
        registry,
        profile_library,
    )
    .await?;
    let response = promote(state, new_state, source).await?;
    // The tab that asked cannot await a command the way it awaited the
    // endpoint, so it no longer reloads itself. Its requests are fenced
    // from here on; reload it once the swap is published.
    super::navigate::notify_profile_changed_to(source);
    Ok(response)
}

/// Sign out: withdraw this device's grant, forget the provider, and move
/// to a branch that follows nothing.
///
/// The grant goes first, while the operator still holds it and can push
/// the retraction. The account branch itself is kept, spaces and all:
/// signing back in returns to it.
pub(crate) async fn sign_out(
    state: &AppState,
    source: Option<&super::ClientId>,
) -> Result<tonk_worker_api::AccountStatus, TonkWorkerError> {
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    let (status, storage, name, profile, registry, profile_library) = {
        let tonk = state.read().await;
        super::account_devices::withdraw_own_authority(&tonk).await;
        let status = super::account::disconnect(&tonk).await?;
        super::profile::leave_account(&tonk).await;
        (
            status,
            tonk.storage.clone(),
            tonk.profile_name.clone(),
            tonk.profile.clone(),
            tonk.registry.clone(),
            tonk.profile_library.clone(),
        )
    };
    let new_state = crate::worker::boot_state_with_profile_library(
        storage,
        name,
        profile,
        registry,
        profile_library,
    )
    .await?;
    promote(state, new_state, source).await?;
    Ok(status)
}

/// Route an account ceremony to the branch for `root`.
///
/// Already on that account's branch: nothing moves. A branch already
/// following it: switch there, spaces and all. Otherwise the branch the
/// profile is on takes the upstream once the ceremony links — after
/// leaving whatever account it followed, so the new account starts from
/// an empty branch and the old one keeps its grant withdrawn.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn for_account(
    state: AppState,
    root: &Did,
    source: Option<&super::ClientId>,
) -> Result<AccountProfileGuard, TonkWorkerError> {
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    let current = state.clone().read_owned().await;
    let on = super::profile::active_account(&current).await;
    if on.as_ref() == Some(&root.this()) {
        return Ok(AccountProfileGuard {
            tonk: current,
            disposition: AccountProfileDisposition::Current,
        });
    }
    let existing = super::profile::branch_following(&current, root).await;
    if existing.is_none() && on.is_none() {
        // An empty branch: the link attaches the upstream here.
        return Ok(AccountProfileGuard {
            tonk: current,
            disposition: AccountProfileDisposition::Current,
        });
    }

    // The branch being left keeps its link (see `activate_named`).
    match &existing {
        Some(branch) => super::profile::set_active_branch(&current, branch).await?,
        None => super::profile::leave_account(&current).await,
    }
    let (storage, name, profile, registry, profile_library) = (
        current.storage.clone(),
        current.profile_name.clone(),
        current.profile.clone(),
        current.registry.clone(),
        current.profile_library.clone(),
    );
    drop(current);

    let new_state = crate::worker::boot_state_with_profile_library(
        storage,
        name,
        profile,
        registry,
        profile_library,
    )
    .await?;
    promote(&state, new_state, source).await?;
    let tonk = state.read_owned().await;
    let disposition = if existing.is_some() {
        AccountProfileDisposition::Existing
    } else {
        AccountProfileDisposition::Created
    };
    log!("account branch routing disposition: {disposition:?}");
    Ok(AccountProfileGuard { tonk, disposition })
}

/// Stamp the incoming profile's roster entry, swap the state in, and
/// kick off the same detached catch-up the boot path runs.
async fn promote(
    state: &AppState,
    mut new_state: TonkState,
    source: Option<&super::ClientId>,
) -> Result<ProfilesResponse, TonkWorkerError> {
    // The service-worker wrapper owns the same one-way retirement flag. A
    // profile swap changes account state, not worker generation, so preserve
    // that identity across the replacement instead of installing a fresh
    // false latch. A swap that finishes after retirement began also closes
    // its never-exposed reactor before publishing it.
    let (retiring, profile_transition, context_generation, clients) = {
        let current = state.read().await;
        (
            Arc::clone(&current.retiring),
            Arc::clone(&current.profile_transition),
            Arc::clone(&current.context_generation),
            Arc::clone(&current.clients),
        )
    };
    new_state.retiring = retiring;
    new_state.profile_transition = profile_transition;
    new_state.context_generation = context_generation;
    new_state.clients = clients;
    if new_state.is_retiring() {
        new_state.reactor.shutdown();
    }
    let name = new_state.profile_name.clone();
    let registry = new_state.registry.clone();
    registry
        .upsert_roster(
            &new_state.storage,
            &new_state.operator,
            &new_state.profile.did(),
            &name,
        )
        .await?;
    let roster = refreshed_roster(&new_state).await?;
    let response = response_from(&new_state.active_branch, roster);

    // The roster and candidate are durable before the pointer changes. From
    // here through the in-memory swap there are no fallible operations.
    registry.set_active(&new_state.storage, &name).await?;
    {
        let mut active = state.write().await;
        *active = new_state;
        active.context_generation.fetch_add(1, Ordering::AcqRel);
    }
    super::navigate::notify_profile_changed(source);

    // Catch up on whatever account the swapped-in profile is attached
    // to, exactly as a boot would. Fire-and-forget: account-service
    // latency must not delay the switch. Spaces themselves need no
    // catch-up pass — the Hub renders from the account directory and
    // the data-plane routes mount directory spaces on first use.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let state = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let tonk = state.read().await;
            super::account_state::ensure_account_state(&tonk).await;
        });
    }

    Ok(response)
}

/// Sign out, as the settings page asks for it.
///
/// The same path the `DELETE /api/account` route takes, so the two
/// cannot drift while the route lives. The originating tab is fenced by
/// the swap and cannot await a command, so it is reloaded from here once
/// the empty branch is active.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::SignOut> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::SignOut) {
        match sign_out(self.state(), self.client()).await {
            Ok(status) => log!("SignOut: {status:?}"),
            Err(error) => {
                log!("SignOut failed: {error}");
                return;
            }
        }
        super::navigate::notify_profile_changed_to(self.client());
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal as _;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::account::TEST_ACCOUNT_REMOTE;
    use crate::router::profile::{
        active_account, active_branch_name, branch_following, local_branches,
    };
    use crate::router::tests::{put_repo, test_state, test_state_without_root};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn space_keys(state: &AppState) -> Vec<String> {
        let axum::Json(info) = crate::router::profile::get_profile(State(state.clone()))
            .await
            .unwrap();
        info.space.into_iter().map(|entry| entry.key).collect()
    }

    async fn active(state: &AppState) -> String {
        state.read().await.active_branch.clone()
    }

    /// The account this profile's root record names.
    async fn own_root(state: &AppState) -> Did {
        let tonk = state.read().await;
        super::super::identity::local_root(&tonk)
            .await
            .unwrap()
            .root_did
    }

    /// An account nothing on this profile has met.
    async fn fresh_root() -> Did {
        Ed25519Signer::generate().await.unwrap().did()
    }

    /// Record the active branch as following `account`, the way a link
    /// does: the upstream, the served replica, and where it is served.
    async fn follow(state: &AppState, account: &Did) {
        let tonk = state.read().await;
        let address = dialog_repository::SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
            TEST_ACCOUNT_REMOTE,
        ));
        super::super::account_state::record_account_branch(&tonk, account, &address).await;
    }

    async fn activate_branch(state: &AppState, name: &str) -> ProfilesResponse {
        let Json(response) = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest {
                profile: name.to_owned(),
            }),
        )
        .await
        .unwrap();
        response
    }

    /// The add-account command starts an empty branch.
    ///
    /// Exercised through the Provider, not `add_profile`, so the command
    /// wiring is covered too: a command registered but never reaching its
    /// handler would pass a test that called the inner function.
    #[dialog_common::test]
    async fn it_starts_an_empty_branch_for_the_add_command() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = active(&state).await;

        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        <crate::router::CommandEnv as dialog_capability::Provider<
            tonk_schema::command::AddProfile,
        >>::execute(
            &env,
            tonk_schema::command::AddProfile {
                this: "cmd:add-one".parse().expect("entity"),
                time: tonk_schema::domain::command::current::add_profile::Time(1.0),
            },
        )
        .await;

        assert_ne!(
            before,
            active(&state).await,
            "adding an account must land on a different branch",
        );
        let tonk = state.read().await;
        assert!(
            active_account(&tonk).await.is_none(),
            "the branch an account is added on follows nothing yet",
        );
    }

    /// The sign-out command leaves the account branch behind.
    ///
    /// Through the Provider, as the settings page fires it, so the
    /// registration is covered along with the handler.
    #[dialog_common::test]
    async fn it_leaves_the_account_for_the_sign_out_command() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = active(&state).await;
        assert!(
            active_account(&*state.read().await).await.is_some(),
            "the fixture starts signed in",
        );

        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        <crate::router::CommandEnv as dialog_capability::Provider<
            tonk_schema::command::SignOut,
        >>::execute(
            &env,
            tonk_schema::command::SignOut {
                this: "cmd:sign-out".parse().expect("entity"),
                time: tonk_schema::domain::command::current::sign_out::Time(1.0),
            },
        )
        .await;

        assert_ne!(
            before,
            active(&state).await,
            "signing out moves onto a branch that follows no account",
        );
        let tonk = state.read().await;
        assert!(
            active_account(&tonk).await.is_none(),
            "the active branch follows nothing after sign-out",
        );
    }

    /// After a sign-out the top page asks meta which branch is active and
    /// what it is called, with exactly these two queries
    /// (`tonk_host::bridge::resolve_profile_branch`). A page that cannot
    /// read the answer falls back to `main` and boots onto the branch it
    /// just left.
    #[dialog_common::test]
    async fn it_answers_the_page_which_branch_is_active_after_sign_out() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt as _;

        let (app, state, _lsp) = crate::router::api_router_with_state(test_state().await);
        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        <crate::router::CommandEnv as dialog_capability::Provider<
            tonk_schema::command::SignOut,
        >>::execute(
            &env,
            tonk_schema::command::SignOut {
                this: "cmd:sign-out".parse().expect("entity"),
                time: tonk_schema::domain::command::current::sign_out::Time(1.0),
            },
        )
        .await;
        let expected = active(&state).await;
        assert_ne!(
            expected, "main",
            "the fixture signs out onto a fresh branch"
        );

        let ask = |body: String| {
            let app = app.clone();
            async move {
                let mut request = Request::builder()
                    .method("POST")
                    .uri("/api/profile/branch/meta/query")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap();
                request
                    .extensions_mut()
                    .insert(crate::router::ClientId("page".to_owned()));
                let response = app.oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
            }
        };
        let active_query = r#"{"predicate":{"with":{"branch":{"the":"tonk.dialog.replica/active-branch","as":"Entity","cardinality":"one"}}},"terms":{"this":{"?":{"name":"this"}},"branch":{"?":{"name":"branch"}}}}"#;
        let rows = ask(active_query.to_owned()).await;
        let entity = rows[0]["fields"]["branch"]
            .as_str()
            .unwrap_or_else(|| panic!("the active branch is a string the page can read: {rows}"))
            .to_owned();
        let name_query = format!(
            r#"{{"predicate":{{"with":{{"name":{{"the":"xyz.tonk.branch/name","as":"Text","cardinality":"one"}}}}}},"terms":{{"this":{entity:?},"name":{{"?":{{"name":"name"}}}}}}}}"#
        );
        let rows = ask(name_query).await;
        assert_eq!(
            rows[0]["fields"]["name"].as_str(),
            Some(expected.as_str()),
            "the page must be told the branch it should boot onto: {rows}",
        );
    }

    /// Reading the roster publishes it where a sealed guest can query it.
    ///
    /// The guest reads the ACTIVE branch and nothing else, so the switcher
    /// could never be a view while every other branch's name lived only
    /// on that branch. The worker can read them all, so it republishes
    /// what it found as overlay facts.
    ///
    /// Asserted through a QUERY rather than by inspecting the response: the
    /// point is that the guest's own read path finds them.
    #[dialog_common::test]
    async fn it_publishes_the_roster_where_a_guest_can_query_it() {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::ProfileRow;

        let state = Arc::new(RwLock::new(test_state().await));
        let Json(response) = list(State(state.clone())).await.unwrap();

        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .expect("profile branch opens");
        let rows: Vec<ProfileRow> = session
            .handle()
            .query()
            .select(Query::<ProfileRow> {
                this: Term::var("this"),
                name: Term::var("name"),
                label: Term::var("label"),
                provider: Term::var("provider"),
                active: Term::var("active"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("roster rows read back");

        assert_eq!(
            rows.len(),
            response.profiles.len(),
            "every branch the roster lists gets a queryable row",
        );
        let current: Vec<_> = rows.iter().filter(|row| row.active.0).collect();
        assert_eq!(current.len(), 1, "exactly one row is active");
        assert_eq!(
            current[0].name.0, tonk.active_branch,
            "the active row names the branch the profile is on",
        );
    }

    /// A fresh worker holds no overlay, so the rows have to be published
    /// at boot: the hub reads them and nothing on the page asks for the
    /// roster, so a worker restart would otherwise empty the account list.
    #[dialog_common::test]
    async fn it_publishes_the_roster_when_the_worker_boots() {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::ProfileRow;

        let state = test_state().await;
        let tonk = crate::worker::boot_state_with_profile_library(
            state.storage.clone(),
            state.profile_name.clone(),
            state.profile.clone(),
            state.registry.clone(),
            state.profile_library.clone(),
        )
        .await
        .expect("the worker boots");
        drop(state);

        let session = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .expect("profile branch opens");
        let rows: Vec<ProfileRow> = session
            .handle()
            .query()
            .select(Query::<ProfileRow> {
                this: Term::var("this"),
                name: Term::var("name"),
                label: Term::var("label"),
                provider: Term::var("provider"),
                active: Term::var("active"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("roster rows read back");

        assert!(
            rows.iter().any(|row| row.active.0 && row.name.0 == tonk.active_branch),
            "the booted worker published the branch it is on: {rows:?}",
        );
    }

    /// A second read updates the rows rather than stacking duplicates.
    #[dialog_common::test]
    async fn it_republishes_the_roster_without_duplicating_rows() {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::ProfileRow;

        let state = Arc::new(RwLock::new(test_state().await));

        let count = || async {
            let tonk = state.read().await;
            let session = tonk
                .reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await
                .expect("profile branch opens");
            let rows: Vec<ProfileRow> = session
                .handle()
                .query()
                .select(Query::<ProfileRow> {
                    this: Term::var("this"),
                    name: Term::var("name"),
                    label: Term::var("label"),
                    provider: Term::var("provider"),
                    active: Term::var("active"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("roster rows read back");
            rows.len()
        };

        let _ = list(State(state.clone())).await.unwrap();
        let first = count().await;
        let _ = list(State(state.clone())).await.unwrap();
        let second = count().await;

        assert_eq!(
            first, second,
            "re-reading supersedes each row in place; it must not accumulate",
        );
    }

    #[dialog_common::test]
    async fn it_lists_the_active_branch_with_its_account_state() {
        let state = Arc::new(RwLock::new(test_state().await));

        let Json(response) = list(State(state.clone())).await.unwrap();

        let current = response
            .profiles
            .iter()
            .find(|entry| entry.active)
            .expect("the active branch lists itself");
        assert_eq!(current.profile_name, response.active);
        assert_eq!(current.profile_name, active(&state).await);
        assert_eq!(
            current.provider.as_deref(),
            Some(TEST_ACCOUNT_REMOTE),
            "where the account is served from is read off the peer's address",
        );
        assert!(
            current.root_did.is_some(),
            "a branch following an account names it"
        );
        // No name until the ACCOUNT carries one: a fresh branch has not
        // replicated an account name, and the roster does not invent a
        // petname to fill the gap.
        assert!(
            current.display_name.is_none(),
            "an unnamed account reports no name rather than a generated one"
        );
    }

    /// Signing out lands on a branch that follows nothing and keeps the
    /// account's branch for signing back in. The grant is gone: no root
    /// record, no provider.
    #[dialog_common::test]
    async fn it_signs_out_onto_an_empty_branch_and_keeps_the_account_branch() {
        let state = Arc::new(RwLock::new(test_state().await));
        let account_branch = active(&state).await;
        let root = own_root(&state).await;

        let status = sign_out(&state, None).await.unwrap();

        assert!(
            matches!(status, tonk_worker_api::AccountStatus::RootMissing { .. }),
            "the grant is forgotten, so there is no root to report",
        );
        let tonk = state.read().await;
        assert_ne!(tonk.active_branch, account_branch, "sign-out moves branch");
        assert!(
            active_account(&tonk).await.is_none(),
            "the new branch follows nothing"
        );
        assert!(super::super::account::provider(&tonk).await.is_none());
        assert!(
            super::super::identity::load_record(&tonk)
                .await
                .unwrap()
                .is_none(),
            "the grant is forgotten, so signing back in reopens the passkey",
        );
        assert_eq!(
            branch_following(&tonk, &root).await.as_deref(),
            Some(account_branch.as_str()),
            "the account's branch is kept, still following the account",
        );
    }

    /// Signing out reuses a branch that already follows nothing rather
    /// than minting another.
    #[dialog_common::test]
    async fn it_reuses_an_empty_branch_when_signing_out() {
        let state = Arc::new(RwLock::new(test_state().await));
        let account_branch = active(&state).await;
        let Json(added) = add(State(state.clone()), None).await.unwrap();
        let empty = added.active;
        let count = local_branches(&*state.read().await).await.len();
        activate_branch(&state, &account_branch).await;

        sign_out(&state, None).await.unwrap();

        let tonk = state.read().await;
        assert_eq!(
            tonk.active_branch, empty,
            "the existing empty branch is reused"
        );
        assert_eq!(
            local_branches(&tonk).await.len(),
            count,
            "no branch was created for a sign-out with one free",
        );
    }

    #[dialog_common::test]
    async fn it_refuses_to_activate_a_branch_meta_does_not_name() {
        let state = Arc::new(RwLock::new(test_state().await));

        let error = activate(
            State(state),
            None,
            Json(ActivateProfileRequest {
                profile: "no-such-branch".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(error, TonkWorkerError::NotFound(_)),
            "a name meta does not enumerate must never be opened (open-or-create \
             would mint an empty branch), got {error:?}"
        );
    }

    /// Add Account starts an empty branch on the SAME profile: the device
    /// keeps its key, and the branch being left stays in the roster.
    #[dialog_common::test]
    async fn it_starts_an_empty_branch_for_add_account() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (original, original_did) = {
            let tonk = state.read().await;
            (tonk.active_branch.clone(), tonk.profile.did())
        };

        let Json(response) = add(State(state.clone()), None).await.unwrap();

        let tonk = state.read().await;
        assert_ne!(tonk.active_branch, original);
        assert_eq!(
            tonk.profile.did(),
            original_did,
            "add-account keeps the device key; only the branch changes"
        );
        assert_eq!(response.active, tonk.active_branch);
        let fresh = response
            .profiles
            .iter()
            .find(|entry| entry.active)
            .expect("the fresh branch lists itself");
        assert!(
            fresh.provider.is_none() && fresh.root_did.is_none(),
            "the landing pad starts as a local workspace"
        );
        assert!(
            response
                .profiles
                .iter()
                .any(|entry| entry.profile_name == original),
            "the outgoing branch stays reachable from the roster"
        );
    }

    #[dialog_common::test]
    async fn it_reuses_an_empty_branch_instead_of_starting_another() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let before = active(&state).await;

        let Json(response) = add(State(state.clone()), None).await.unwrap();

        assert_eq!(
            response.active, before,
            "a branch following nothing, with no root and no spaces, is already a landing pad"
        );
        assert_eq!(active(&state).await, before);
    }

    #[dialog_common::test]
    async fn it_serves_a_branchs_spaces_after_activating_it() {
        let (app, state, _lsp) = crate::api_router_with_state(test_state().await);
        let original = active(&state).await;
        let key = put_repo(&app, "switching-space").await;
        assert!(space_keys(&state).await.contains(&key));

        let _ = add(State(state.clone()), None).await.unwrap();
        assert!(
            space_keys(&state).await.is_empty(),
            "an empty branch must not see the other account's spaces"
        );

        activate_branch(&state, &original).await;
        assert_eq!(active(&state).await, original);
        assert!(
            space_keys(&state).await.contains(&key),
            "switching back must restore the original space list"
        );
    }

    /// `meta` records the active branch only once the target booted, so a
    /// refused switch leaves both the state and the record where they were.
    #[dialog_common::test]
    async fn it_records_the_active_branch_only_after_the_target_boots() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = active(&state).await;

        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest {
                profile: "no-such-branch".to_string(),
            }),
        )
        .await
        .unwrap_err();
        {
            let tonk = state.read().await;
            assert_eq!(tonk.active_branch, before);
            assert_eq!(
                active_branch_name(&tonk.reactor, &tonk.operator)
                    .await
                    .as_deref(),
                Some(before.as_str()),
                "a refused switch leaves meta untouched",
            );
        }

        let Json(response) = add(State(state.clone()), None).await.unwrap();
        let tonk = state.read().await;
        assert_eq!(
            active_branch_name(&tonk.reactor, &tonk.operator).await,
            Some(response.active),
            "a switch that booted is what meta records",
        );
    }

    #[dialog_common::test]
    async fn it_keeps_a_rootless_local_workspace_for_its_first_account() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let before = active(&state).await;
        let root = fresh_root().await;

        let guard = for_account(state, &root, None).await.unwrap();

        assert_eq!(guard.active_branch, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Current);
    }

    /// Signing back in returns to the branch that followed the account,
    /// spaces and all, rather than attaching a second one.
    #[dialog_common::test]
    async fn it_routes_a_signed_out_login_back_to_the_matching_branch() {
        let state = Arc::new(RwLock::new(test_state().await));
        let account_branch = active(&state).await;
        let root = own_root(&state).await;
        sign_out(&state, None).await.unwrap();
        assert_ne!(active(&state).await, account_branch);

        let guard = for_account(state, &root, None).await.unwrap();

        assert_eq!(guard.active_branch, account_branch);
        assert_eq!(guard.disposition, AccountProfileDisposition::Existing);
    }

    #[dialog_common::test]
    async fn it_keeps_the_current_branch_for_the_same_account_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = active(&state).await;
        let root = own_root(&state).await;

        let guard = for_account(state, &root, None).await.unwrap();

        assert_eq!(guard.active_branch, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Current);
    }

    /// Which account a branch follows is read off `meta` without
    /// activating it, which is what lets the switcher name every branch.
    #[dialog_common::test]
    async fn it_names_the_account_a_branch_follows_without_activating_it() {
        let state = Arc::new(RwLock::new(test_state().await));
        let account_branch = active(&state).await;
        let root = own_root(&state).await;
        let _ = add(State(state.clone()), None).await.unwrap();
        assert_ne!(active(&state).await, account_branch);

        let tonk = state.read().await;
        assert_eq!(
            branch_following(&tonk, &root).await.as_deref(),
            Some(account_branch.as_str()),
        );
    }

    #[dialog_common::test]
    async fn it_returns_to_the_branch_following_the_discovered_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = active(&state).await;
        let Json(added) = add(State(state.clone()), None).await.unwrap();
        let second = added.active;
        let second_root = fresh_root().await;
        follow(&state, &second_root).await;
        activate_branch(&state, &first).await;

        let guard = for_account(state, &second_root, None).await.unwrap();

        assert_eq!(guard.active_branch, second);
        assert_eq!(guard.disposition, AccountProfileDisposition::Existing);
    }

    /// An unknown account while signed in to another: the account branch
    /// is left (its grant withdrawn) and an empty branch takes the login.
    #[dialog_common::test]
    async fn it_leaves_the_account_branch_for_an_unknown_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (before, device) = {
            let tonk = state.read().await;
            (tonk.active_branch.clone(), tonk.profile.did())
        };
        let root = fresh_root().await;

        let guard = for_account(state, &root, None).await.unwrap();

        assert_ne!(guard.active_branch, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Created);
        assert_eq!(guard.profile.did(), device, "the device keeps its key");
        assert!(
            active_account(&guard).await.is_none(),
            "the login lands on a branch following nothing",
        );
        assert!(
            super::super::identity::load_record(&guard)
                .await
                .unwrap()
                .is_none(),
            "leaving the other account forgot its grant",
        );
        let names: Vec<String> = local_branches(&guard)
            .await
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(names.contains(&before), "the left branch is kept");
        assert!(names.contains(&guard.active_branch));
    }

    #[dialog_common::test]
    async fn it_never_moves_spaces_when_routing_between_accounts() {
        let (app, state, _lsp) = crate::api_router_with_state(test_state().await);
        let first = active(&state).await;
        let retained = put_repo(&app, "retained-by-first-account").await;
        let _ = add(State(state.clone()), None).await.unwrap();
        let second_root = fresh_root().await;
        follow(&state, &second_root).await;
        activate_branch(&state, &first).await;

        let second = for_account(state.clone(), &second_root, None)
            .await
            .unwrap();
        assert!(
            !super::super::profile_name::real_space_keys(&second)
                .await
                .contains(&retained)
        );
        drop(second);

        activate_branch(&state, &first).await;
        assert!(space_keys(&state).await.contains(&retained));
    }

    #[dialog_common::test]
    async fn it_holds_the_selected_branch_stable_for_account_writes() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = active(&state).await;
        let root = own_root(&state).await;
        let second = add_profile(&state, None).await.unwrap().active;
        activate_named(&state, first.clone(), None).await.unwrap();

        let guard = for_account(state.clone(), &root, None).await.unwrap();
        assert!(
            state.try_write().is_err(),
            "a switch cannot acquire the state write lock while the account guard lives"
        );

        let switching = state.clone();
        let mut activation = Box::pin(activate_named(&switching, second, None));
        assert!(
            futures_util::FutureExt::now_or_never(activation.as_mut()).is_none(),
            "a concurrent switch must not finish while account writes are pinned"
        );
        assert_eq!(active(&state).await, first);

        drop(guard);
        activation
            .await
            .expect("the switch succeeds after the account guard drops");
    }

    #[dialog_common::test]
    async fn it_serializes_add_activate_and_automatic_account_routing() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = active(&state).await;
        let transition = state.read().await.profile_transition.clone();

        let held = transition.lock().await;
        let mut adding = Box::pin(add_profile(&state, None));
        assert!(
            futures_util::FutureExt::now_or_never(adding.as_mut()).is_none(),
            "Add Account must wait for the shared transition mutex"
        );
        drop(held);
        let second = adding.await.unwrap().active;
        let second_root = fresh_root().await;
        follow(&state, &second_root).await;

        let held = transition.lock().await;
        let mut activating = Box::pin(activate_named(&state, first.clone(), None));
        assert!(
            futures_util::FutureExt::now_or_never(activating.as_mut()).is_none(),
            "explicit activation must wait for the shared transition mutex"
        );
        drop(held);
        activating.await.unwrap();

        let held = transition.lock().await;
        let mut routing = Box::pin(for_account(state.clone(), &second_root, None));
        assert!(
            futures_util::FutureExt::now_or_never(routing.as_mut()).is_none(),
            "automatic account routing must wait for the shared transition mutex"
        );
        drop(held);
        let selected = routing.await.unwrap();
        assert_eq!(selected.active_branch, second);
        assert_eq!(selected.disposition, AccountProfileDisposition::Existing);
    }
}

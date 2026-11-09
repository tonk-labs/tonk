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

use std::ops::Deref;
use std::sync::{Arc, atomic::Ordering};

use axum::{Extension, Json, extract::State};
use dialog_artifacts::Entity;
use dialog_varsig::Did;
use tonk_common::log;
use tonk_macros::handler;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};

use super::AppState;
use crate::TonkWorkerError;
use crate::device::RosterEntry;
use crate::worker::TonkState;

/// How account routing selected the profile pinned by an
/// [`AccountProfileGuard`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub(crate) struct AccountProfileGuard {
    tonk: crate::state_lock::OwnedReadGuard<TonkState>,
    disposition: AccountProfileDisposition,
    signed_out: Option<String>,
}

impl AccountProfileGuard {
    pub(crate) fn disposition(&self) -> AccountProfileDisposition {
        self.disposition
    }

    /// The signed-out workspace this sign-in came back from, when it
    /// returned to a branch that already followed the account. What was
    /// made there was made before signing in, and belongs to the account
    /// once the link finishes ([`super::rotation::carry_from`]).
    pub(crate) fn signed_out(&self) -> Option<&str> {
        self.signed_out.as_deref()
    }
}

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
#[handler]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
    let tonk = state.read().await;
    Ok(Json(refreshed_response(&tonk).await?))
}

/// `POST /api/profiles/activate`.
#[handler]
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
#[cfg(feature = "persona")]
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
#[cfg(feature = "persona")]
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
#[handler]
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

/// Bring down everything the account's branch refers to, while this device
/// may still read it.
///
/// A pull takes the branch's head and leaves what it refers to where it
/// is, to be read when wanted. Signing out gives up the right to read it,
/// and the branch is kept and written to after: a write that reaches a
/// part never brought down then has nowhere to read it from, and fails.
/// So what the branch holds is made this device's own first. A device
/// that cannot reach the account now signs out with what it has.
async fn keep_what_the_account_branch_holds(tonk: &TonkState) {
    let kept = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .pull()
        .download()
        .perform(&tonk.operator)
        .await;
    if let Err(error) = kept {
        log!("sign-out: the account branch was not brought down whole: {error}");
    }
}

/// Sign out: withdraw this device's grant, forget the provider, and move
/// to a branch that follows nothing.
///
/// The grant is withdrawn around the provider being cleared (see
/// [`Withdrawal`](super::account_devices::Withdrawal)), and before the
/// branch is left. The account branch itself is kept, spaces and all:
/// signing back in returns to it.
pub(crate) async fn sign_out(
    state: &AppState,
    source: Option<&super::ClientId>,
) -> Result<tonk_worker_api::AccountStatus, TonkWorkerError> {
    let mut timing = super::link_timing::LinkTiming::new("sign-out", "transition-lock");
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    timing.next("state-lock");
    let (status, storage, name, profile, registry, profile_library) = {
        let tonk = state.read().await.clone();
        keep_what_the_account_branch_holds(&tonk).await;
        // Clearing the provider writes into the account branch, so it
        // lands between the two halves of the withdrawal, while the
        // chain still proves this device may read what it writes into.
        timing.next("begin-withdrawal");
        let withdrawal = super::account_devices::begin_withdrawal(&tonk).await;
        timing.next("disconnect");
        let disconnected = super::account::disconnect(&tonk).await;
        timing.next("finish-withdrawal");
        super::account_devices::finish_withdrawal(&tonk, withdrawal).await;
        disconnected?;
        // Read once the withdrawal has forgotten the root, so the status
        // reports the grant gone.
        timing.next("status");
        let status = super::account::status(&tonk).await?;
        timing.next("leave-account");
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
    timing.next("boot-state");
    let new_state = crate::worker::boot_state_with_profile_library(
        storage,
        name,
        profile,
        registry,
        profile_library,
    )
    .await?;
    timing.next("promote");
    promote(state, new_state, source).await?;
    Ok(status)
}

/// Route an account ceremony to the branch for `root`.
///
/// Already on that account's branch: nothing moves. A branch already
/// following it: switch there, spaces and all, and when the branch left
/// behind is a signed-out workspace, name it on the guard so its spaces
/// can follow once the link finishes. Otherwise the branch the profile is
/// on takes the upstream once the ceremony links — after leaving whatever
/// account it followed, so the new account starts from an empty branch
/// and the old one keeps its grant withdrawn.
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
            signed_out: None,
        });
    }
    let existing = super::profile::branch_following(&current, root).await;
    if existing.is_none() && on.is_none() {
        // An empty branch: the link attaches the upstream here.
        return Ok(AccountProfileGuard {
            tonk: current,
            disposition: AccountProfileDisposition::Current,
            signed_out: None,
        });
    }
    // Following no account, the branch being left is a signed-out
    // workspace; one following another account keeps what it holds.
    let signed_out = on.is_none().then(|| current.active_branch.clone());

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
    Ok(AccountProfileGuard {
        tonk,
        disposition,
        signed_out,
    })
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
    //
    // Whether sites have origins of their own is the deployment's, told to
    // the worker once when it starts. A profile that takes over without it
    // would go back to holding its spaces' content itself.
    let (retiring, profile_transition, context_generation, clients, site_origins, profile_swaps) = {
        let current = state.read().await;
        (
            Arc::clone(&current.retiring),
            Arc::clone(&current.profile_transition),
            Arc::clone(&current.context_generation),
            Arc::clone(&current.clients),
            Arc::clone(&current.site_origins),
            current.profile_swaps.clone(),
        )
    };
    new_state.shared_mut().retiring = retiring;
    new_state.shared_mut().profile_transition = profile_transition;
    new_state.shared_mut().context_generation = context_generation;
    new_state.shared_mut().clients = clients;
    new_state.shared_mut().profile_swaps = profile_swaps.clone();
    new_state.shared_mut().site_origins = site_origins;
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
        // The swap waits for every guard over the state. Syncs hold theirs
        // across remote round trips, so they are told to give way first.
        let _timing = super::link_timing::LinkTiming::new("promote", "state-write-lock");
        profile_swaps.begin();
        let mut active = state.write().await;
        *active = new_state;
        active.context_generation.fetch_add(1, Ordering::AcqRel);
    }
    // On a kernel, the embedder holds the persona the profile acts for.
    #[cfg(target_arch = "wasm32")]
    super::merge::stand(&*state.read().await).await;
    super::navigate::notify_profile_changed(source);
    // A space's own worker acts for the account its profile does, and was
    // told so by the profile this one replaces.
    super::space_reach::changed(None);

    // Catch up on whatever account the swapped-in profile is attached
    // to, exactly as a boot would. Fire-and-forget: account-service
    // latency must not delay the switch. Spaces themselves need no
    // catch-up pass — the Hub renders from the account directory and
    // the data-plane routes mount directory spaces on first use.

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
#[cfg(feature = "persona")]
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

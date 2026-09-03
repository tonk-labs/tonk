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
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_operator::{DeriveOperator as _, Profile};
use dialog_storage::provider::storage::Storage;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_varsig::Did;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_worker_api::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};

use super::AppState;
use crate::TonkWorkerError;
use crate::device::RosterEntry;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::worker::DefaultOperator;
use crate::worker::{DefaultSpace, TonkState};

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

/// The active profile's switcher row, built from live state.
///
/// Every field but the storage handle is read where it actually lives: the
/// provider attachment, the local root, the display name on the account
/// branch. Nothing is carried forward from the roster, which now stores only
/// the handle — a copy there could only go stale, since nothing invalidated
/// it when the source changed.
///
/// The account fields all derive from the provider attachment: an unattached
/// profile is a local workspace whatever root record it still holds, which is
/// how sign-out demotes a row without deleting anything.
async fn refreshed_entry(tonk: &TonkState, email: Option<String>) -> RosterEntry {
    let provider = super::account::provider(tonk).await;
    let root_did = if provider.is_some() {
        super::identity::local_root(tonk)
            .await
            .ok()
            .map(|root| root.root_did.to_string())
    } else {
        None
    };
    RosterEntry {
        profile_name: tonk.profile_name.clone(),
        root_did,
        provider: provider.clone(),
        email: provider.and(email),
        display_name: super::profile_name::resolve_display_name(tonk).await,
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
                display_name: Some(entry.display_name),
            })
            .collect(),
    }
}

/// The roster with the active profile's entry refreshed from live state,
/// written back so the stored roster converges as a side effect.
/// Inactive entries are served as-of their profile's last activation.
async fn refreshed_response(tonk: &TonkState) -> Result<ProfilesResponse, TonkWorkerError> {
    let mut roster = tonk
        .registry
        .read_roster(&tonk.storage, &tonk.operator)
        .await?;
    // The roster holds only handles, so the active profile's row is built
    // from live state and spliced over whatever the roster listed.
    let entry = refreshed_entry(tonk, None).await;
    try_upsert_active_entry(tonk, None).await?;
    match roster
        .iter_mut()
        .find(|slot| slot.profile_name == entry.profile_name)
    {
        Some(slot) => *slot = entry,
        None => roster.push(entry),
    }
    Ok(response_from(&tonk.profile_name, roster))
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

    let (registry, active) = {
        let tonk = state.read().await;
        // Validate before opening anything: `Profile::open` is
        // open-or-create, so an unvalidated name would silently mint a
        // garbage key. The initial profile is always a valid target even
        // when nothing ever wrote its roster entry.
        if name != tonk.registry.initial_profile()
            && !tonk
                .registry
                .read_roster(&tonk.storage, &tonk.operator)
                .await?
                .iter()
                .any(|entry| entry.profile_name == name)
        {
            return Err(TonkWorkerError::NotFound(format!(
                "no profile '{name}' on this browser"
            )));
        }
        // Keep the outgoing profile reachable: a switch must not proceed if
        // its local workspace cannot first be named in the roster.
        try_upsert_active_entry(&tonk, None).await?;
        (tonk.registry.clone(), tonk.profile_name.clone())
    };
    if name == active {
        let tonk = state.read().await;
        return refreshed_response(&tonk).await;
    }

    // Build the replacement state WITHOUT holding the state write lock —
    // opening a profile and bootstrapping its meta branch await storage
    // IO, and in-flight requests must keep being served meanwhile.
    let storage = Storage::<DefaultSpace>::default();
    let profile = registry.open_profile(&storage, &name).await?;
    let new_state =
        crate::worker::boot_state(storage, name.clone(), profile, registry.clone()).await?;
    // Only a target that opened and booted repoints the pointer, so a
    // failed activation never strands the next SW restart.
    promote(state, new_state, source).await
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

async fn add_profile(
    state: &AppState,
    source: Option<&super::ClientId>,
) -> Result<ProfilesResponse, TonkWorkerError> {
    let transition = {
        let tonk = state.read().await;
        Arc::clone(&tonk.profile_transition)
    };
    let _transition = transition.lock().await;

    let registry = {
        let tonk = state.read().await;
        // Abandoned-add reuse: a profile with no persisted root and no
        // user spaces is already a fresh landing pad — hand it back
        // rather than minting another orphan key.
        if super::identity::load_record(&tonk).await?.is_none() {
            let fresh = true;
            #[cfg(target_arch = "wasm32")]
            let fresh = fresh && super::profile_name::real_space_keys(&tonk).await.is_empty();
            if fresh {
                return refreshed_response(&tonk).await;
            }
        }
        try_upsert_active_entry(&tonk, None).await?;
        tonk.registry.clone()
    };

    let storage = Storage::<DefaultSpace>::default();
    let (name, profile) = registry.create_profile(&storage).await?;
    let new_state = crate::worker::boot_state(storage, name, profile, registry).await?;
    promote(state, new_state, source).await
}

/// Select and pin the profile that historically owns `root`.
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
    match super::identity::historical_root_did(&current.profile, &current.operator).await? {
        None => {
            return Ok(AccountProfileGuard {
                tonk: current,
                disposition: AccountProfileDisposition::Current,
            });
        }
        Some(historical) if historical == *root => {
            return Ok(AccountProfileGuard {
                tonk: current,
                disposition: AccountProfileDisposition::Current,
            });
        }
        Some(_) => {}
    }

    // Routing must not make the outgoing local workspace unreachable.
    try_upsert_active_entry(&current, None).await?;
    let registry = current.registry.clone();
    let storage = current.storage.clone();
    let roster = registry
        .read_roster(&current.storage, &current.operator)
        .await?;
    let active_name = current.profile_name.clone();
    drop(current);

    let mut matched: Option<(String, Profile)> = None;
    for entry in roster {
        if entry.profile_name == active_name {
            continue;
        }
        let profile = match registry.open_profile(&storage, &entry.profile_name).await {
            Ok(profile) => profile,
            Err(_) => {
                log!(
                    "profile routing skipped unreadable roster handle {}",
                    entry.profile_name
                );
                continue;
            }
        };
        let operator = match inspection_operator(&profile, &storage).await {
            Ok(operator) => operator,
            Err(_) => {
                log!(
                    "profile routing skipped unreadable roster handle {}",
                    entry.profile_name
                );
                continue;
            }
        };
        match super::identity::historical_root_did(&profile, &operator).await {
            Ok(Some(historical)) if historical == *root => {
                if matched.is_none() {
                    matched = Some((entry.profile_name, profile));
                } else {
                    log!("profile routing retained a duplicate matching profile handle");
                }
            }
            Ok(_) => {}
            Err(_) => {
                log!(
                    "profile routing skipped unreadable roster handle {}",
                    entry.profile_name
                );
            }
        }
    }

    let (new_state, disposition) = match matched {
        Some((name, profile)) => (
            crate::worker::boot_state(storage, name, profile, registry).await?,
            AccountProfileDisposition::Existing,
        ),
        None => {
            let (name, profile) = registry.create_profile(&storage).await?;
            (
                crate::worker::boot_state(storage, name, profile, registry).await?,
                AccountProfileDisposition::Created,
            )
        }
    };

    promote(&state, new_state, source).await?;
    let tonk = state.read_owned().await;
    log!("account profile routing disposition: {disposition:?}");
    Ok(AccountProfileGuard { tonk, disposition })
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn inspection_operator(
    profile: &Profile,
    storage: &Storage<DefaultSpace>,
) -> Result<DefaultOperator, TonkWorkerError> {
    let context: [u8; 16] = rand::random();
    profile
        .derive(context.to_vec())
        .build(storage.clone())
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to inspect a roster profile: {error}"))
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
    let mut roster = registry
        .read_roster(&new_state.storage, &new_state.operator)
        .await?;
    let entry = refreshed_entry(&new_state, None).await;
    {
        registry
            .upsert_roster(
                &new_state.storage,
                &new_state.operator,
                &new_state.profile.did(),
                &name,
            )
            .await?;
    }
    match roster
        .iter_mut()
        .find(|slot| slot.profile_name == entry.profile_name)
    {
        Some(slot) => *slot = entry,
        None => roster.push(entry),
    }
    let response = response_from(&name, roster);

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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::account::TEST_ACCOUNT_REMOTE;
    use crate::router::tests::{persist_test_root, put_repo, test_state, test_state_without_root};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn space_keys(state: &AppState) -> Vec<String> {
        let axum::Json(info) = crate::router::profile::get_profile(State(state.clone()))
            .await
            .unwrap();
        info.space.into_iter().map(|entry| entry.key).collect()
    }

    #[dialog_common::test]
    async fn it_lists_the_active_profile_with_its_account_state() {
        let state = Arc::new(RwLock::new(test_state().await));

        let Json(response) = list(State(state.clone())).await.unwrap();

        let active = response
            .profiles
            .iter()
            .find(|entry| entry.active)
            .expect("the active profile lists itself");
        assert_eq!(active.profile_name, response.active);
        assert_eq!(active.provider.as_deref(), Some(TEST_ACCOUNT_REMOTE));
        assert!(
            active.root_did.is_some(),
            "an attached profile names its account root"
        );
        assert!(active.display_name.is_some());
    }

    #[dialog_common::test]
    async fn it_refuses_to_activate_a_profile_the_roster_does_not_name() {
        let state = Arc::new(RwLock::new(test_state().await));

        let error = activate(
            State(state),
            None,
            Json(ActivateProfileRequest {
                profile: "no-such-profile".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(error, TonkWorkerError::NotFound(_)),
            "an unvalidated name must never be opened (open-or-create \
             would mint a garbage key), got {error:?}"
        );
    }

    #[dialog_common::test]
    async fn it_rotates_to_a_fresh_profile_for_add_account() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (original_name, original_did) = {
            let tonk = state.read().await;
            (tonk.profile_name.clone(), tonk.profile.did())
        };

        let Json(response) = add(State(state.clone()), None).await.unwrap();

        let tonk = state.read().await;
        assert_ne!(tonk.profile_name, original_name);
        assert_ne!(
            tonk.profile.did(),
            original_did,
            "add-account must land on a fresh key"
        );
        assert_eq!(response.active, tonk.profile_name);
        let fresh = response
            .profiles
            .iter()
            .find(|entry| entry.active)
            .expect("the fresh profile lists itself");
        assert!(
            fresh.provider.is_none() && fresh.root_did.is_none(),
            "the landing pad starts as a local workspace"
        );
        assert!(
            response
                .profiles
                .iter()
                .any(|entry| entry.profile_name == original_name),
            "the outgoing profile stays reachable from the roster"
        );
    }

    #[dialog_common::test]
    async fn it_reuses_an_unattached_empty_profile_instead_of_rotating_again() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let before = state.read().await.profile_name.clone();

        let Json(response) = add(State(state.clone()), None).await.unwrap();

        assert_eq!(
            response.active, before,
            "a rootless, space-less profile is already a fresh landing pad"
        );
        assert_eq!(state.read().await.profile_name, before);
    }

    #[dialog_common::test]
    async fn it_serves_the_other_profiles_spaces_after_activation() {
        let (app, state, _lsp) = crate::api_router_with_state(test_state().await);
        let original = state.read().await.profile_name.clone();
        let key = put_repo(&app, "switching-space").await;
        assert!(space_keys(&state).await.contains(&key));

        let _ = add(State(state.clone()), None).await.unwrap();
        assert!(
            space_keys(&state).await.is_empty(),
            "a fresh profile must not see the other account's spaces"
        );

        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest {
                profile: original.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(state.read().await.profile_name, original);
        assert!(
            space_keys(&state).await.contains(&key),
            "switching back must restore the original space list"
        );
    }

    #[dialog_common::test]
    async fn it_repoints_the_active_pointer_only_after_the_target_profile_opens() {
        let state = Arc::new(RwLock::new(test_state().await));
        let registry = state.read().await.registry.clone();
        let initial = registry.initial_profile().to_string();

        // A refused activation leaves the pointer untouched.
        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest {
                profile: "no-such-profile".to_string(),
            }),
        )
        .await
        .unwrap_err();
        let (name, _) = registry
            .open_active(&Storage::<DefaultSpace>::default())
            .await
            .unwrap();
        assert_eq!(name, initial);

        // A successful swap repoints it at the profile that booted.
        let Json(response) = add(State(state.clone()), None).await.unwrap();
        let (name, _) = registry
            .open_active(&Storage::<DefaultSpace>::default())
            .await
            .unwrap();
        assert_eq!(name, response.active);
    }

    #[dialog_common::test]
    async fn it_keeps_a_rootless_local_workspace_for_its_first_account() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;

        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let before = state.read().await.profile_name.clone();
        let root = Ed25519Signer::generate().await.unwrap().did();

        let guard = for_account(state, &root, None).await.unwrap();

        assert_eq!(guard.profile_name, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Current);
    }

    #[dialog_common::test]
    async fn it_keeps_the_current_profile_for_the_same_account_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (before, root) = {
            let tonk = state.read().await;
            (
                tonk.profile_name.clone(),
                super::super::identity::local_root(&tonk)
                    .await
                    .unwrap()
                    .root_did,
            )
        };

        let guard = for_account(state, &root, None).await.unwrap();

        assert_eq!(guard.profile_name, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Current);
    }

    #[dialog_common::test]
    async fn it_reads_an_inactive_profiles_historical_root_without_booting_it() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = state.read().await.profile_name.clone();
        let _ = add(State(state.clone()), None).await.unwrap();
        let (profile, storage, root) = {
            let tonk = state.read().await;
            (
                tonk.profile.clone(),
                tonk.storage.clone(),
                persist_test_root(&tonk).await,
            )
        };
        let _ = activate(
            State(state),
            None,
            Json(ActivateProfileRequest { profile: first }),
        )
        .await
        .unwrap();

        let operator = inspection_operator(&profile, &storage).await.unwrap();
        assert_eq!(
            super::super::identity::historical_root_did(&profile, &operator)
                .await
                .unwrap(),
            Some(root)
        );
    }

    #[dialog_common::test]
    async fn it_reuses_the_roster_profile_with_the_discovered_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = state.read().await.profile_name.clone();
        let _ = add(State(state.clone()), None).await.unwrap();
        let (second, second_root) = {
            let tonk = state.read().await;
            let second = tonk.profile_name.clone();
            let root = persist_test_root(&tonk).await;
            (second, root)
        };
        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest { profile: first }),
        )
        .await
        .unwrap();

        let guard = for_account(state, &second_root, None).await.unwrap();

        assert_eq!(guard.profile_name, second);
        assert_eq!(guard.disposition, AccountProfileDisposition::Existing);
    }

    #[dialog_common::test]
    async fn it_creates_a_fresh_profile_for_an_unknown_account_root() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;

        let state = Arc::new(RwLock::new(test_state().await));
        let before = state.read().await.profile_name.clone();
        let root = Ed25519Signer::generate().await.unwrap().did();

        let guard = for_account(state, &root, None).await.unwrap();

        assert_ne!(guard.profile_name, before);
        assert_eq!(guard.disposition, AccountProfileDisposition::Created);
        let roster = guard
            .registry
            .read_roster(&guard.storage, &guard.operator)
            .await
            .unwrap();
        assert!(roster.iter().any(|entry| entry.profile_name == before));
        assert!(
            roster
                .iter()
                .any(|entry| entry.profile_name == guard.profile_name)
        );
    }

    #[dialog_common::test]
    async fn it_never_moves_spaces_when_routing_between_accounts() {
        let (app, state, _lsp) = crate::api_router_with_state(test_state().await);
        let first = state.read().await.profile_name.clone();
        let retained = put_repo(&app, "retained-by-first-account").await;
        let _ = add(State(state.clone()), None).await.unwrap();
        let second_root = {
            let tonk = state.read().await;
            persist_test_root(&tonk).await
        };
        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest {
                profile: first.clone(),
            }),
        )
        .await
        .unwrap();

        let second = for_account(state.clone(), &second_root, None)
            .await
            .unwrap();
        assert!(
            !super::super::profile_name::real_space_keys(&second)
                .await
                .contains(&retained)
        );
        drop(second);

        let _ = activate(
            State(state.clone()),
            None,
            Json(ActivateProfileRequest { profile: first }),
        )
        .await
        .unwrap();
        assert!(space_keys(&state).await.contains(&retained));
    }

    #[dialog_common::test]
    async fn it_holds_the_selected_profile_stable_for_account_writes() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (first, root) = {
            let tonk = state.read().await;
            (
                tonk.profile_name.clone(),
                super::super::identity::local_root(&tonk)
                    .await
                    .unwrap()
                    .root_did,
            )
        };
        let second = add_profile(&state, None).await.unwrap().active;
        activate_named(&state, first.clone(), None).await.unwrap();

        let guard = for_account(state.clone(), &root, None).await.unwrap();
        assert!(
            state.try_write().is_err(),
            "profile activation cannot acquire the state write lock while the account guard lives"
        );

        let switching = state.clone();
        let mut activation = Box::pin(activate_named(&switching, second, None));
        assert!(
            futures_util::FutureExt::now_or_never(activation.as_mut()).is_none(),
            "a concurrent activation must not finish while account writes are pinned"
        );
        assert_eq!(state.read().await.profile_name, first);

        drop(guard);
        activation
            .await
            .expect("activation succeeds after the account guard drops");
    }

    #[dialog_common::test]
    async fn it_serializes_add_activate_and_automatic_account_routing() {
        let state = Arc::new(RwLock::new(test_state().await));
        let first = state.read().await.profile_name.clone();
        let transition = state.read().await.profile_transition.clone();

        let held = transition.lock().await;
        let mut adding = Box::pin(add_profile(&state, None));
        assert!(
            futures_util::FutureExt::now_or_never(adding.as_mut()).is_none(),
            "Add Account must wait for the shared transition mutex"
        );
        drop(held);
        let second = adding.await.unwrap().active;
        let second_root = {
            let tonk = state.read().await;
            persist_test_root(&tonk).await
        };

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
        assert_eq!(selected.profile_name, second);
        assert_eq!(selected.disposition, AccountProfileDisposition::Existing);
    }
}

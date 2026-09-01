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

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_storage::provider::storage::Storage;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_worker_api::{ActivateProfileRequest, ProfileRosterEntry, ProfilesResponse};

use super::AppState;
use crate::TonkWorkerError;
use crate::device::RosterEntry;
use crate::worker::{DefaultSpace, TonkState};

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
    Json(request): Json<ActivateProfileRequest>,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
    let name = request.profile;
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
        // Keep the outgoing profile reachable: its entry may never have
        // been written if every earlier best-effort upsert failed.
        upsert_active_entry(&tonk, None).await;
        (tonk.registry.clone(), tonk.profile_name.clone())
    };
    if name == active {
        let tonk = state.read().await;
        return Ok(Json(refreshed_response(&tonk).await?));
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
    registry.set_active(&new_state.storage, &name).await?;
    finish_swap(&state, new_state).await
}

/// `POST /api/profiles/add`.
///
/// Rotate to a fresh profile and swap onto it — the landing pad the
/// unchanged sign-in ceremony then runs on. `validate_grant` binds a
/// ceremony to the profile that ran it, so "add account" moves first;
/// the ceremony that follows can only ever persist here.
#[wasm_compat]
pub async fn add(State(state): State<AppState>) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
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
                return Ok(Json(refreshed_response(&tonk).await?));
            }
        }
        upsert_active_entry(&tonk, None).await;
        tonk.registry.clone()
    };

    let storage = Storage::<DefaultSpace>::default();
    let (name, profile) = registry.rotate(&storage).await?;
    let new_state = crate::worker::boot_state(storage, name, profile, registry).await?;
    finish_swap(&state, new_state).await
}

/// Stamp the incoming profile's roster entry, swap the state in, and
/// kick off the same detached catch-up the boot path runs.
async fn finish_swap(
    state: &AppState,
    new_state: TonkState,
) -> Result<Json<ProfilesResponse>, TonkWorkerError> {
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

    *state.write().await = new_state;

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

    Ok(Json(response))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::account::TEST_ACCOUNT_PROVIDER;
    use crate::router::tests::{put_repo, test_state, test_state_without_root};
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
        assert_eq!(active.provider.as_deref(), Some(TEST_ACCOUNT_PROVIDER));
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

        let Json(response) = add(State(state.clone())).await.unwrap();

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

        let Json(response) = add(State(state.clone())).await.unwrap();

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

        let _ = add(State(state.clone())).await.unwrap();
        assert!(
            space_keys(&state).await.is_empty(),
            "a fresh profile must not see the other account's spaces"
        );

        let _ = activate(
            State(state.clone()),
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
        let Json(response) = add(State(state.clone())).await.unwrap();
        let (name, _) = registry
            .open_active(&Storage::<DefaultSpace>::default())
            .await
            .unwrap();
        assert_eq!(name, response.active);
    }
}

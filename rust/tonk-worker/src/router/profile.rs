//! Profile route — reports the profile and the spaces (replicas)
//! it owns.

use std::collections::HashMap;

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{Replica, domain::replica::Profile as ProfileEntity, prelude::DidExt as _};

use super::{AppState, RepositoryInfo, repository::build_repository_info};
use crate::TonkWorkerError;

/// Name of the meta branch on the profile repository — mirrors
/// the constant in `super::repository`. Keeping a private copy
/// rather than exporting the one from `super::repository` avoids
/// a cross-module coupling for a one-character string.
const META_BRANCH: &str = "meta";

/// Response body for `GET /api/profile`.
///
/// `profile` describes the profile "as a repository" (see
/// [`bootstrap_profile_meta`]) so the UI can render it the same
/// way it renders any other space — populated by
/// [`build_repository_info`], which reads the profile's meta
/// branch and surfaces its branches and remotes. `space` is a
/// flat `{ name -> subject DID }` map of every replica this
/// profile owns — enough to populate the sidebar without per-repo
/// round-trips.
///
/// [`bootstrap_profile_meta`]: super::repository::bootstrap_profile_meta
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// [`RepositoryInfo`] for the profile itself — same shape as
    /// any other space, including the meta-branch entries for the
    /// profile's own branches and remotes.
    pub profile: RepositoryInfo,
    /// Every replica owned by this profile except the profile's
    /// own self-replica, keyed by local name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub space: HashMap<String, Did>,
}

/// Handler for `GET /api/profile`.
///
/// The profile itself goes through [`build_repository_info`] so
/// the UI can render the profile screen with the exact same view
/// it uses for a space. `space` is populated by a separate
/// `Query<Replica>` on the profile's meta branch, filtered to
/// exclude the self replica.
#[wasm_compat]
pub async fn get_profile(
    State(state): State<AppState>,
) -> Result<Json<ProfileInfo>, TonkWorkerError> {
    log!("GET /api/profile");

    let tonk = state.read().await;
    let profile_did = tonk.profile.did();

    // Read through the reactor's cached profile-repository handle so
    // reads see exactly what writes (which also go through the reactor)
    // committed — a separate `Repository::from(&tonk.profile)` handle
    // would resolve a different cached branch state and could disagree.
    let profile_repository = tonk
        .reactor
        .profile_repository()
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to acquire profile repository: {e}"))
        })?
        .repository();

    // Full info for the profile-as-repository. This handles the
    // branches/remotes surfacing — the profile's meta branch is a
    // real meta branch with the same schema as any other.
    let profile = build_repository_info(&tonk, &tonk.profile_name, &profile_repository).await;

    // Space list lives on the same meta branch but is specific
    // to the profile route (regular repositories don't have a
    // sidebar index to build). Run it through the reactor's cached
    // branch session for the same coherence reason.
    let session = tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open profile meta branch: {e}"))
        })?;

    let rows: Vec<Replica> = session
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            name: Term::var("name"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(profile_did.this())),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Replica query on profile meta failed: {:?}", e))
        })?;

    // Build the space map — every row except the self-replica,
    // which has `subject == profile`. An unparseable subject is
    // a single bad entry; log and skip it rather than failing
    // the whole response.
    let profile_entity = profile_did.this();
    let mut space = HashMap::with_capacity(rows.len());

    for replica in rows {
        if replica.subject.0 == profile_entity {
            continue;
        }
        let did = match replica.subject.0.to_string().parse::<Did>() {
            Ok(did) => did,
            Err(e) => {
                log!(
                    "Replica '{}' has an unparseable subject: {:?}",
                    replica.name.0,
                    e
                );
                continue;
            }
        };
        space.insert(replica.name.0, did);
    }

    Ok(Json(ProfileInfo { profile, space }))
}

//! Profile route — reports the profile and the spaces (replicas)
//! it owns.

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

use super::repository::PROFILE_BRANCH;

/// Give the profile repository the `meta` branch every space
/// repository has, recording `main` in the branch enumeration.
///
/// A space's meta branch carries the bookkeeping that must never
/// replicate — the local [`Replica`] record and which branches exist.
/// The profile repository was created before that split and has only
/// `main`, so nothing can enumerate its branches; a profile-per-branch
/// switcher needs exactly that enumeration.
///
/// On demand and idempotent: `branch(META_BRANCH).open()` creates the
/// branch if it is absent, and re-asserting a [`Replica`] or
/// [`MetaBranch`] is a no-op because both hash from their own fields —
/// the same record converges rather than duplicating. So an existing
/// profile gains its meta branch the next time it boots, and a profile
/// that already has one pays a no-op transaction.
///
/// Best-effort by design: a profile whose meta branch cannot be written
/// still works exactly as it does today, because nothing reads this
/// enumeration yet. Failing the boot over bookkeeping nobody consults
/// would trade a working hub for a tidy one.
pub(crate) async fn ensure_profile_meta_branch(tonk: &crate::worker::TonkState) {
    use tonk_schema::Branch as MetaBranch;

    // The profile repository IS the subject here: a replica of itself,
    // held by itself, which is what makes its branch entities derive
    // the same way a space's do.
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);
    // `.transaction()` on the branch reference opens it if absent —
    // the same on-demand creation a space's meta branch gets.
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(super::repository::META_BRANCH))
        .assert(MetaBranch::new(&replica, PROFILE_BRANCH));

    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("profile meta branch not recorded: {error}");
    }
}

/// One space the profile owns, as listed by `GET /api/profile`.
///
/// A repository's identity is its credential's `did:key` (`subject`);
/// the routing/storage key is the DID suffix (`key`). The membership
/// index carries no display name: the space's name lives in its own
/// `tonk/repository` concept on its content branch, so the UI resolves
/// the label from the space's own repo (per-space `<tonk-display
/// model=tonk:repository>`), not from this listing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpaceEntry {
    /// Routing/storage key — the `subject` DID suffix. The URL segment
    /// the UI links by.
    pub key: String,
    /// The space's identity DID.
    pub subject: Did,
}

/// Response body for `GET /api/profile`.
///
/// `profile` describes the profile "as a repository" (see
/// [`bootstrap_profile`]) so the UI can render it the same
/// way it renders any other space — populated by
/// [`build_repository_info`], which reads the profile's meta
/// branch and surfaces its branches and remotes. `space` lists every
/// replica this profile owns — enough to populate the sidebar without
/// per-repo round-trips.
///
/// [`bootstrap_profile`]: super::repository::bootstrap_profile
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// [`RepositoryInfo`] for the profile itself — same shape as
    /// any other space, including the meta-branch entries for the
    /// profile's own branches and remotes.
    pub profile: RepositoryInfo,
    /// Every replica owned by this profile except the profile's
    /// own self-replica.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub space: Vec<SpaceEntry>,
    /// The member's effective display name (override, else petname).
    /// Lets the shell read identity without going through a space branch.
    pub display_name: String,
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

    // A profile created before the content/meta split has no meta
    // branch. Give it one here — this route is the hub's first touch of
    // the profile repository — so the branch enumeration exists for
    // whatever reads it later.
    ensure_profile_meta_branch(&tonk).await;

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
        .branch(PROFILE_BRANCH)
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

    // Build the space list from explicitly real-space replicas only.
    // System replicas (profile and account) never enter user navigation.
    // Each entry carries the routing
    // key (the subject DID suffix, what the UI links by) and the
    // identity DID. The display name is not here: the Hub card resolves
    // it from the space's own `tonk/repository` concept. An unparseable
    // subject is a single bad entry; log and skip it rather than failing
    // the whole response.
    let mut space = Vec::with_capacity(rows.len());

    for replica in rows {
        if replica.kind != Replica::repository_kind() {
            continue;
        }
        let did = match replica.subject.0.to_string().parse::<Did>() {
            Ok(did) => did,
            Err(e) => {
                log!(
                    "Replica subject {:?} is unparseable: {:?}",
                    replica.subject.0,
                    e
                );
                continue;
            }
        };
        space.push(SpaceEntry {
            key: did.repo_key().to_owned(),
            subject: did,
        });
    }

    let display_name = crate::router::profile_name::resolve_display_name(&tonk).await;

    Ok(Json(ProfileInfo {
        profile,
        space,
        display_name,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    use crate::api_router;
    use crate::router::tests::test_state;

    /// The profile gains the branch enumeration a space has.
    ///
    /// Reading the enumeration back through a QUERY, not by checking
    /// the transaction succeeded: the point of the record is that
    /// something can later ask "which branches does this profile
    /// have?" and get `main`, which is what a per-profile-branch
    /// switcher will do.
    #[dialog_common::test]
    async fn it_gives_the_profile_a_meta_branch_naming_its_main() {
        use dialog_query::{Query, Term};
        use tonk_schema::Branch as MetaBranch;

        let state = test_state().await;
        let (app, state, _lsp) = crate::router::api_router_with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let tonk = state.read().await;
        let session = tonk
            .reactor
            .profile_repository()
            .branch(super::super::repository::META_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("meta branch opens");
        let branches: Vec<MetaBranch> = session
            .handle()
            .query()
            .select(Query::<MetaBranch> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::var("origin"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("branch enumeration reads");

        let names: Vec<String> = branches.iter().map(|b| b.name.0.clone()).collect();
        assert!(
            names.iter().any(|n| n == PROFILE_BRANCH),
            "the content branch must appear in the enumeration; got {names:?}",
        );
        assert!(
            names
                .iter()
                .any(|n| n == super::super::repository::META_BRANCH),
            "and the meta branch enumerates itself, as a space's does; got {names:?}",
        );
    }

    /// Booting twice converges rather than accumulating.
    ///
    /// The records hash from their own fields, so re-asserting is a
    /// no-op — but that is a property of the concepts, not of this
    /// call, so it is worth pinning: an existing profile boots through
    /// this path on every load.
    #[dialog_common::test]
    async fn it_records_the_same_branches_however_often_the_profile_boots() {
        use dialog_query::{Query, Term};
        use tonk_schema::Branch as MetaBranch;

        let state = test_state().await;
        let (_app, state, _lsp) = crate::router::api_router_with_state(state);

        let count = || async {
            let tonk = state.read().await;
            let session = tonk
                .reactor
                .profile_repository()
                .branch(super::super::repository::META_BRANCH)
                .acquire(&tonk.operator)
                .await
                .expect("meta branch opens");
            let branches: Vec<MetaBranch> = session
                .handle()
                .query()
                .select(Query::<MetaBranch> {
                    this: Term::var("this"),
                    name: Term::var("name"),
                    origin: Term::var("origin"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("branch enumeration reads");
            branches.len()
        };

        {
            let tonk = state.read().await;
            ensure_profile_meta_branch(&tonk).await;
        }
        let first = count().await;
        {
            let tonk = state.read().await;
            ensure_profile_meta_branch(&tonk).await;
        }
        let second = count().await;

        assert_eq!(
            first, second,
            "a second boot must converge on the same records, not add more",
        );
    }

    #[dialog_common::test]
    async fn it_reports_a_display_name_on_the_profile() {
        let state = test_state().await;
        let expected = tonk_schema::petname(&state.profile.did());
        let (app, _lsp) = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let name = json["display_name"]
            .as_str()
            .expect("display_name is a string");
        assert_eq!(name, expected);
    }

    #[dialog_common::test]
    async fn it_lists_only_real_spaces_when_an_account_replica_is_indexed() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;
        use tonk_schema::Replica;

        let state = test_state().await;
        let (app, state, _lsp) = crate::router::api_router_with_state(state);
        let real_key = crate::router::tests::put_repo(&app, "visible-space").await;
        let account = Ed25519Signer::import(&[73; 32]).await.unwrap().did();
        {
            let tonk = state.read().await;
            tonk.reactor
                .profile_repository()
                .branch(PROFILE_BRANCH)
                .transaction()
                .assert(Replica::account(tonk.profile.did(), account.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let profile: ProfileInfo = serde_json::from_slice(&body).unwrap();

        assert_eq!(profile.space.len(), 1);
        assert_eq!(profile.space[0].key, real_key);
        assert_ne!(profile.space[0].subject, account);
    }
}

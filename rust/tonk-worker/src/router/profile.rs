//! Profile route — reports the profile and the spaces (replicas)
//! it owns.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::Entity;
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
    use tonk_schema::{Branch as MetaBranch, ReplicaActiveBranch};

    // The profile repository IS the subject here: a replica of itself,
    // held by itself, which is what makes its branch entities derive
    // the same way a space's do.
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);
    // `.transaction()` on the branch reference opens it if absent —
    // the same on-demand creation a space's meta branch gets.
    // Which branch this replica is on. Asserted here rather than only
    // on a switch, so a profile that has never switched still answers
    // the question — an absent active branch would be indistinguishable
    // from a profile whose bookkeeping failed.
    //
    // `PROFILE_BRANCH` is the seed value because that is where an
    // unlinked profile starts: a branch with no upstream, which is what
    // signed out MEANS. Signing in points this at the account's branch.
    //
    // Re-asserting is a no-op: cardinality-one supersedes with the same
    // value, so a boot after a switch does not drag the replica back to
    // where it started.
    let content = MetaBranch::new(&replica, PROFILE_BRANCH);
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(super::repository::META_BRANCH))
        .assert(content.clone());

    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("profile meta branch not recorded: {error}");
        return;
    }

    // Seed the active branch only when none is recorded. Cardinality-one
    // means an unconditional assert would supersede on every boot,
    // dragging a profile that had switched back to its content branch.
    if active_branch(tonk).await.is_some() {
        return;
    }
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .assert(ReplicaActiveBranch::new(&replica, &content))
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("profile active branch not recorded: {error}");
    }
}

/// Which branch the profile replica is on, or `None` when nothing has
/// recorded one yet.
///
/// Reads `meta`, which never replicates: which branch this device is
/// looking at is nobody else's business.
pub(crate) async fn active_branch(tonk: &crate::worker::TonkState) -> Option<Entity> {
    use tonk_schema::ReplicaActiveBranch;

    let session = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    let rows: Vec<ReplicaActiveBranch> = session
        .handle()
        .query()
        .select(Query::<ReplicaActiveBranch> {
            this: Term::var("this"),
            active_branch: Term::var("active_branch"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.active_branch.0)
}

/// The account this profile is signed in as, or `None` when it is not.
///
/// Derived, never stored. The active branch's upstream is a branch on
/// a replica held by another peer, and that replica's subject is the
/// account:
///
/// ```text
/// active branch -> upstream -> branch/replica -> replica/subject
/// ```
///
/// Signed out is the active branch having no upstream — a branch that
/// follows nothing. No flag, no null value, no account-shaped absence.
///
/// This replaces asking which replicas have kind `tonk:account`, which
/// answered with every account the device had ever linked: nothing in
/// those rows said which was current.
pub(crate) async fn active_account(tonk: &crate::worker::TonkState) -> Option<Entity> {
    use tonk_schema::Branch as MetaBranch;
    use tonk_schema::{BranchUpstream, Replica as ReplicaConcept};

    let active = active_branch(tonk).await?;
    let session = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    let handle = session.handle();

    // What the active branch follows. Absent means signed out.
    let upstream: Vec<BranchUpstream> = handle
        .query()
        .select(Query::<BranchUpstream> {
            this: Term::from(active),
            upstream: Term::var("upstream"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    let upstream = upstream.into_iter().next()?.upstream.0;

    // The upstream branch's replica, and that replica's subject.
    let branches: Vec<MetaBranch> = handle
        .query()
        .select(Query::<MetaBranch> {
            this: Term::from(upstream),
            name: Term::var("name"),
            origin: Term::var("origin"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    let replica = branches.into_iter().next()?.origin.0;

    let replicas: Vec<ReplicaConcept> = handle
        .query()
        .select(Query::<ReplicaConcept> {
            this: Term::from(replica),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    replicas.into_iter().next().map(|row| row.subject.0)
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

    /// The account is the subject of the active branch's upstream.
    ///
    /// Built end to end rather than asserted piecemeal: a peer's
    /// replica of the account repository, a branch on it, and a local
    /// branch following that. If the traversal is right, the account
    /// falls out; if any hop is wrong, nothing does.
    #[dialog_common::test]
    async fn it_reads_the_account_off_the_active_branch() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;
        use tonk_schema::{Branch as MetaBranch, BranchUpstream, ReplicaActiveBranch};

        let state = test_state().await;
        let profile_did = state.profile.did();
        let replica = Replica::new(profile_did.clone(), profile_did.clone());

        // The account, and the peer that serves it.
        let account = Ed25519Signer::import(&[91; 32]).await.unwrap().did();
        let service = Ed25519Signer::import(&[92; 32]).await.unwrap().did();
        let served = Replica::new(service, account.clone());
        let upstream = MetaBranch::new(&served, "main");

        // The local branch that follows it.
        let local = MetaBranch::new(&replica, &format!("account/{}", account.repo_key()));

        state
            .reactor
            .profile_repository()
            .branch(super::super::repository::META_BRANCH)
            .transaction()
            .assert(served.clone())
            .assert(upstream.clone())
            .assert(local.clone())
            .assert(BranchUpstream::new(&local, &upstream))
            .assert(ReplicaActiveBranch::new(&replica, &local))
            .commit()
            .perform(&state.operator)
            .await
            .expect("the link commits");

        let resolved = active_account(&state).await.expect("an account");
        assert_eq!(
            resolved,
            account.this(),
            "the account is the subject of the upstream's replica",
        );
    }

    /// A branch that follows nothing means signed out.
    ///
    /// Not a flag and not a null: the absence of an upstream IS the
    /// state, the same way a local-only git branch has no remote.
    #[dialog_common::test]
    async fn it_reads_no_account_from_a_branch_with_no_upstream() {
        let state = test_state().await;
        ensure_profile_meta_branch(&state).await;

        assert!(
            active_account(&state).await.is_none(),
            "a fresh profile follows nothing, so it is signed out",
        );
    }

    /// A branch name may carry a DID, colons and all.
    ///
    /// Account branches are named `account/<did>`, following
    /// `repo_key`'s "one identifier, no suffix-stripping". Dialog does
    /// not validate branch names — they are cell paths — but that is
    /// worth pinning rather than assuming, since the whole naming
    /// scheme rests on it.
    #[dialog_common::test]
    async fn it_opens_a_branch_named_for_a_did() {
        use tonk_schema::Branch as MetaBranch;

        let state = test_state().await;
        let name = "account/did:key:z6MkTestAccountBranchName";
        state
            .reactor
            .profile_repository()
            .branch(name)
            .transaction()
            .assert(MetaBranch::new(
                &Replica::new(state.profile.did(), state.profile.did()),
                name,
            ))
            .commit()
            .perform(&state.operator)
            .await
            .expect("a branch named for a DID commits");
    }

    /// A fresh profile starts on a branch with no upstream.
    ///
    /// Which is what signed out MEANS: not a flag, but a branch that
    /// follows nothing. Seeding it at bootstrap is what makes the
    /// question answerable at all — an absent active branch would be
    /// indistinguishable from bookkeeping that failed.
    #[dialog_common::test]
    async fn it_starts_on_the_branch_with_no_upstream() {
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
        let active = active_branch(&tonk).await.expect("an active branch");
        let profile_did = tonk.profile.did();
        let replica = Replica::new(profile_did.clone(), profile_did);
        assert_eq!(
            active,
            MetaBranch::new(&replica, PROFILE_BRANCH).this,
            "a fresh profile is on its content branch",
        );
    }

    /// Booting again does not drag a switched profile back.
    ///
    /// The seed is cardinality-one, so an unconditional assert would
    /// supersede on every boot and undo the switch. This pins that the
    /// seed only runs when nothing is recorded.
    #[dialog_common::test]
    async fn it_keeps_the_active_branch_a_switch_chose() {
        use tonk_schema::{Branch as MetaBranch, ReplicaActiveBranch};

        let state = test_state().await;
        let profile_did = state.profile.did();
        let replica = Replica::new(profile_did.clone(), profile_did);
        // Stand in for a switch: point active at a branch the seed
        // would never choose.
        let elsewhere = MetaBranch::new(&replica, "account/somewhere");
        state
            .reactor
            .profile_repository()
            .branch(super::super::repository::META_BRANCH)
            .transaction()
            .assert(ReplicaActiveBranch::new(&replica, &elsewhere))
            .commit()
            .perform(&state.operator)
            .await
            .expect("the switch commits");

        ensure_profile_meta_branch(&state).await;

        let active = active_branch(&state).await.expect("an active branch");
        assert_eq!(active, elsewhere.this, "a boot must not undo a switch",);
    }

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

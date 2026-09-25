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
    active_branch_entity(&tonk.reactor, &tonk.operator).await
}

/// The active branch's entity, from `meta`.
///
/// Takes the reactor and an operator rather than a [`TonkState`] because
/// boot needs the answer before there is a state: the operator it builds
/// has to prove with the active branch's authority.
///
/// [`TonkState`]: crate::worker::TonkState
async fn active_branch_entity(
    reactor: &crate::Reactor,
    operator: &crate::worker::DefaultOperator,
) -> Option<Entity> {
    use tonk_schema::ReplicaActiveBranch;

    let session = reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(operator)
        .await
        .ok()?;
    let rows: Vec<ReplicaActiveBranch> = session
        .handle()
        .query()
        .select(Query::<ReplicaActiveBranch> {
            this: Term::var("this"),
            active_branch: Term::var("active_branch"),
        })
        .perform(operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.active_branch.0)
}

/// The active branch's NAME, from `meta` — what a session opens and a
/// location token names. `None` when nothing has recorded one, which a
/// caller reads as `main`.
pub(crate) async fn active_branch_name(
    reactor: &crate::Reactor,
    operator: &crate::worker::DefaultOperator,
) -> Option<String> {
    use tonk_schema::Branch as MetaBranch;

    let entity = active_branch_entity(reactor, operator).await?;
    let session = reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(operator)
        .await
        .ok()?;
    let rows: Vec<MetaBranch> = session
        .handle()
        .query()
        .select(Query::<MetaBranch> {
            this: Term::from(entity),
            name: Term::var("name"),
            replica: Term::var("replica"),
        })
        .perform(operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.name.0)
}

/// The account this profile is signed in as, or `None` when it is not.
///
/// Derived, never stored: the active branch's upstream is a branch on a
/// replica held by another peer, and that replica's subject is the
/// account. A branch that follows nothing is signed out, so absence is
/// the signal rather than a flag free to disagree with the branch.
pub(crate) async fn active_account(tonk: &crate::worker::TonkState) -> Option<Entity> {
    let active = active_branch(tonk).await?;
    account_followed_by(tonk, &active).await
}

/// The account `branch` follows, or `None` for a branch following nothing.
///
/// The traversal `meta.yaml` declares as the `account` rule, walked here
/// because the worker needs the answer on `meta`, where no rule runs.
async fn account_followed_by(tonk: &crate::worker::TonkState, branch: &Entity) -> Option<Entity> {
    upstream_replica(tonk, branch)
        .await
        .map(|served| served.subject.0)
}

/// The account a branch follows, as a DID.
pub(crate) async fn account_of_branch(
    tonk: &crate::worker::TonkState,
    branch: &Entity,
) -> Option<Did> {
    account_followed_by(tonk, branch)
        .await
        .and_then(|entity| entity.to_string().parse().ok())
}

/// Where the account a branch follows is served from: the address of the
/// peer holding the upstream's replica, as the endpoint a UCAN remote
/// dials. `None` for a branch following nothing, or one whose peer
/// recorded no address.
pub(crate) async fn provider_of_branch(
    tonk: &crate::worker::TonkState,
    branch: &Entity,
) -> Option<String> {
    use dialog_repository::SiteAddress;
    use tonk_schema::PeerAddress;

    let peer = upstream_replica(tonk, branch).await?.profile.0;
    let session = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    let addresses: Vec<PeerAddress> = session
        .handle()
        .query()
        .select(Query::<PeerAddress> {
            this: Term::from(peer),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    addresses
        .into_iter()
        .filter_map(|row| row.address.decode().ok())
        .find_map(|address| match address {
            SiteAddress::Ucan(ucan) => Some(ucan.endpoint().to_owned()),
            _ => None,
        })
}

/// The replica a branch's upstream lives on: `branch -> upstream ->
/// branch/replica`, read off `meta`. `None` for a branch following nothing.
async fn upstream_replica(
    tonk: &crate::worker::TonkState,
    branch: &Entity,
) -> Option<tonk_schema::Replica> {
    use tonk_schema::Branch as MetaBranch;
    use tonk_schema::{BranchUpstream, Replica as ReplicaConcept};

    let session = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    let handle = session.handle();

    let upstream: Vec<BranchUpstream> = handle
        .query()
        .select(Query::<BranchUpstream> {
            this: Term::from(branch.clone()),
            upstream: Term::var("upstream"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    let upstream = upstream.into_iter().next()?.upstream.0;

    let branches: Vec<MetaBranch> = handle
        .query()
        .select(Query::<MetaBranch> {
            this: Term::from(upstream),
            name: Term::var("name"),
            replica: Term::var("replica"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    let replica = branches.into_iter().next()?.replica.0;

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
    replicas.into_iter().next()
}

/// Every branch of this profile's replica but `meta`, as `(name, entity)`.
pub(crate) async fn local_branches(tonk: &crate::worker::TonkState) -> Vec<(String, Entity)> {
    use tonk_schema::Branch as MetaBranch;

    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);
    let Ok(session) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        return Vec::new();
    };
    let branches: Vec<MetaBranch> = session
        .handle()
        .query()
        .select(Query::<MetaBranch> {
            this: Term::var("this"),
            name: Term::var("name"),
            replica: Term::from(replica.this.clone()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    let mut branches: Vec<(String, Entity)> = branches
        .into_iter()
        .filter(|branch| branch.name.0 != super::repository::META_BRANCH)
        .map(|branch| (branch.name.0, branch.this))
        .collect();
    branches.sort();
    branches
}

/// The name of this profile's branch that follows `account`, if one does.
///
/// Signing back in returns to the branch that was signed in to that
/// account before, spaces and all, rather than attaching a second one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn branch_following(
    tonk: &crate::worker::TonkState,
    account: &Did,
) -> Option<String> {
    use tonk_schema::prelude::DidExt as _;

    let wanted = account.this();
    for (name, entity) in local_branches(tonk).await {
        if account_followed_by(tonk, &entity).await.as_ref() == Some(&wanted) {
            return Some(name);
        }
    }
    None
}

/// Make `name` the profile's active branch.
///
/// Only a branch `meta` enumerates: activating a name nothing recorded
/// would boot the next state onto a branch with no bookkeeping.
pub(crate) async fn set_active_branch(
    tonk: &crate::worker::TonkState,
    name: &str,
) -> Result<(), TonkWorkerError> {
    use tonk_schema::{Branch as MetaBranch, ReplicaActiveBranch};

    if !local_branches(tonk)
        .await
        .iter()
        .any(|(known, _)| known == name)
    {
        return Err(TonkWorkerError::NotFound(format!(
            "no branch '{name}' on this profile"
        )));
    }
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);
    let target = MetaBranch::new(&replica, name);
    tonk.reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .assert(ReplicaActiveBranch::new(&replica, &target))
        .commit()
        .perform(&tonk.operator)
        .await
        .map(|_| ())
        .map_err(|error| TonkWorkerError::Internal(format!("active branch not recorded: {error}")))
}

/// Leave the account: move to a branch that follows nothing.
///
/// Signing out is not a flag and not a return to `main`. It is a branch
/// with no upstream — and a FRESH one when every existing local branch
/// is taken, because sharing one would mix the workspaces of two
/// accounts that were never related.
///
/// That mirrors what sign-out does today by rotating profiles: promote
/// an existing rootless local workspace, or create one. Nothing is
/// emptied and no branch is deleted, so the account branch keeps its
/// spaces and signing back in returns to them.
pub(crate) async fn leave_account(tonk: &crate::worker::TonkState) {
    use tonk_schema::{Branch as MetaBranch, BranchUpstream, ReplicaActiveBranch};

    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);

    let Ok(session) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        log!("sign-out could not open meta; the active branch is unchanged");
        return;
    };
    let handle = session.handle();

    let branches: Vec<MetaBranch> = handle
        .query()
        .select(Query::<MetaBranch> {
            this: Term::var("this"),
            name: Term::var("name"),
            replica: Term::from(replica.this.clone()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    let followed: Vec<BranchUpstream> = handle
        .query()
        .select(Query::<BranchUpstream> {
            this: Term::var("this"),
            upstream: Term::var("upstream"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    // A local workspace is a branch of this replica that follows
    // nothing. `meta` itself is bookkeeping rather than a workspace, so
    // it is never a landing place.
    let landing = branches.iter().find(|branch| {
        branch.name.0 != super::repository::META_BRANCH
            && !followed.iter().any(|row| row.this == branch.this)
    });

    let target = match landing {
        Some(branch) => branch.clone(),
        // Every local branch is taken, so make another. The name only
        // has to be unique on this replica; what makes it a workspace
        // is that nothing is asserted about what it follows.
        None => {
            let mut next = 2usize;
            loop {
                let name = format!("{}-{next}", PROFILE_BRANCH);
                if !branches.iter().any(|branch| branch.name.0 == name) {
                    break MetaBranch::new(&replica, &name);
                }
                next += 1;
            }
        }
    };

    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .assert(target.clone())
        .assert(ReplicaActiveBranch::new(&replica, &target))
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("sign-out did not record the active branch: {error}");
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

/// Forget a branch: retract its record and what it follows from
/// `meta`, so it is no longer listed or offered. The branch's data
/// stays where dialog keeps it, since there is no branch deletion and
/// a retained branch keeps clocks safe; it just stops being one of
/// this profile's branches. Used when the account a branch followed
/// has been deleted and nothing on the branch is worth returning to.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn forget_branch(tonk: &crate::worker::TonkState, name: &str) {
    use tonk_schema::{Branch as MetaBranch, BranchUpstream};

    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);
    let branch = MetaBranch::new(&replica, name);

    let Ok(session) = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        log!("forget branch '{name}': meta could not be opened; it stays listed");
        return;
    };
    let followed: Vec<BranchUpstream> = session
        .handle()
        .query()
        .select(Query::<BranchUpstream> {
            this: Term::from(branch.this.clone()),
            upstream: Term::var("upstream"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(super::repository::META_BRANCH)
        .transaction()
        .retract(branch);
    for row in followed {
        transaction = transaction.retract(row);
    }
    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("forget branch '{name}': {error}");
    }
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
        .branch(&tonk.active_branch)
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
pub(crate) mod tests {
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
    /// A forgotten branch drops out of the listing while its data stays.
    #[dialog_common::test]
    async fn it_forgets_a_branch_by_retracting_its_record() {
        let state = test_state().await;
        leave_account(&state).await;
        let names = |branches: Vec<(String, Entity)>| {
            branches
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        assert!(
            names(local_branches(&state).await).contains(&"main".to_owned()),
            "the account branch is listed before it is forgotten",
        );

        forget_branch(&state, "main").await;

        let listed = names(local_branches(&state).await);
        assert!(
            !listed.contains(&"main".to_owned()),
            "the forgotten branch is no longer listed: {listed:?}",
        );
        assert!(
            !listed.is_empty(),
            "the branch the profile moved onto is still listed: {listed:?}",
        );
    }

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

    /// Signing out twice does not share one workspace.
    ///
    /// The point of creating a branch rather than returning to `main`:
    /// with a single shared local branch, leaving account A and later
    /// account B would land both workspaces in the same place, mixing
    /// spaces that were never related.
    #[dialog_common::test]
    async fn it_lands_each_sign_out_on_its_own_workspace() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;
        use tonk_schema::{Branch as MetaBranch, BranchUpstream, ReplicaActiveBranch};

        let state = crate::router::tests::test_state_without_account().await;
        ensure_profile_meta_branch(&state).await;
        let profile_did = state.profile.did();
        let replica = Replica::new(profile_did.clone(), profile_did.clone());

        // First sign-out takes the content branch, which follows nothing.
        leave_account(&state).await;
        let first = active_branch(&state).await.expect("a branch");
        assert_eq!(
            first,
            MetaBranch::new(&replica, PROFILE_BRANCH).this,
            "the first sign-out takes the workspace already there",
        );

        // Link an account, so the only upstream-less branch is taken.
        let account = Ed25519Signer::import(&[93; 32]).await.unwrap().did();
        let served = Replica::new(account.clone(), account.clone());
        let upstream = MetaBranch::new(&served, "main");
        let taken = MetaBranch::new(&replica, PROFILE_BRANCH);
        state
            .reactor
            .profile_repository()
            .branch(super::super::repository::META_BRANCH)
            .transaction()
            .assert(served)
            .assert(upstream.clone())
            // The branch that WAS the workspace now follows an account.
            .assert(BranchUpstream::new(&taken, &upstream))
            .assert(ReplicaActiveBranch::new(&replica, &taken))
            .commit()
            .perform(&state.operator)
            .await
            .expect("the link commits");

        leave_account(&state).await;
        let second = active_branch(&state).await.expect("a branch");
        assert_ne!(
            second, first,
            "a second sign-out must not land on a branch that now follows an account",
        );
        assert!(
            active_account(&state).await.is_none(),
            "and the branch it lands on follows nothing",
        );
    }

    /// A branch that follows nothing means signed out.
    ///
    /// Not a flag and not a null: the absence of an upstream IS the
    /// state, the same way a local-only git branch has no remote.
    #[dialog_common::test]
    async fn it_reads_no_account_from_a_branch_with_no_upstream() {
        let state = crate::router::tests::test_state_without_account().await;
        ensure_profile_meta_branch(&state).await;

        assert!(
            active_account(&state).await.is_none(),
            "a branch following nothing is signed out",
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
                replica: Term::var("replica"),
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
        let state: crate::router::AppState = std::sync::Arc::new(tokio::sync::RwLock::new(state));

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
                    replica: Term::var("replica"),
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
        let real_key = crate::router::tests::put_repo(&state, "visible-space").await;
        let account = Ed25519Signer::import(&[73; 32]).await.unwrap().did();
        {
            let tonk = state.read().await;
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
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

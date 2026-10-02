//! A space's own worker (proof of concept).
//!
//! Each space renders on an origin of its own, where a worker of its own keeps
//! the space's database in that origin's storage. That worker runs this same
//! crate against a profile of its own, generated on its first boot, and holds
//! nothing but a delegation for its one space, issued by the person's profile.
//!
//! The space worker asks for the delegation with its profile's DID; the host
//! worker issues it ([`delegate`]) for the space the asking port is bound to.
//! The space worker saves it and mounts the space as a replica ([`adopt`]),
//! and asks again before it expires.
//!
//! A freshly mounted replica is empty. The host sends the space's `main` as a
//! snapshot ([`snapshot`]): its revision, and every block and blob it reaches.
//! The space worker stores it and publishes that revision ([`seed`]), so the
//! replica holds the same history the host did, and syncs on from there.

use dialog_artifacts::Value;
use dialog_capability::Subject;
use dialog_common::Blake3Hash;
use dialog_effects::Use;
use dialog_repository::schema::Session;
use dialog_repository::{RepositoryExt as _, Revision, codec};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, time::Timestamp};
use dialog_varsig::Did;
use futures_util::stream;
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;

use super::account::{act_for, acts_for, member_did};
use super::claim::RawClaim;
use super::create_invite::{ConfiguredRemoteRequirement, resolve_configured_remote_url_with};
use super::join::mount_replica;
use super::repository::{CONTENT_BRANCH, record_initialized_replica_in_profile};
use super::rotation::migrate_membership_rows;
use crate::{TonkWorkerError, worker::TonkState};

/// How long a space worker's delegation lasts. It asks for a new one before
/// this runs out, so a lapsed grant costs at most one boot's request.
pub(crate) const DELEGATION_TTL_SECONDS: u64 = 12 * 60 * 60;

/// A delegation for a space worker: the chain from the space to its profile,
/// and where the space syncs, when it syncs anywhere.
pub(crate) struct Grant {
    /// The encoded chain, from the space down to the space worker's profile.
    pub(crate) chain: Vec<u8>,
    /// When the leaf lapses, in unix seconds.
    pub(crate) expires: u64,
    /// The space's upstream, `None` for a space that only exists here.
    pub(crate) remote: Option<String>,
    /// The account the person's profile acts for, and so the one the
    /// space's worker acts for in turn.
    pub(crate) account: Did,
}

/// Issue the worker whose profile is `audience` a delegation to use `space`,
/// signed by the person's profile. Scoped to that one space with `Use`, which
/// covers reading and writing its content, not delegating it further.
pub(crate) async fn delegate(
    tonk: &TonkState,
    space: &Did,
    audience: &Did,
) -> Result<Grant, TonkWorkerError> {
    let now = Timestamp::now().to_unix();
    let expires = now + DELEGATION_TTL_SECONDS;
    let until = Timestamp::try_from(expires as i128)
        .map_err(|e| TonkWorkerError::Internal(format!("delegation expiry: {e:?}")))?;
    let delegation: UcanDelegation = tonk
        .profile
        .access()
        .claim(Subject::from(space.clone()).attenuate(Use))
        .expires(until)
        .delegate(audience.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to delegate {space}: {e}")))?;
    let chain = delegation
        .into_chain()
        .to_bytes()
        .map_err(|e| TonkWorkerError::Internal(format!("failed to encode delegation: {e}")))?;

    let Terms { remote, account } = terms(tonk, space).await?;
    Ok(Grant {
        chain,
        expires,
        remote,
        account,
    })
}

/// What a space's worker is told beside its delegation, and has to take up
/// again when it changes: where the space syncs, and which account the
/// person's profile acts for.
pub(crate) struct Terms {
    /// The space's upstream, `None` for a space that only exists here.
    pub(crate) remote: Option<String>,
    /// The account the person's profile acts for.
    pub(crate) account: Did,
}

/// The [`Terms`] the person's profile holds for `space` now.
pub(crate) async fn terms(tonk: &TonkState, space: &Did) -> Result<Terms, TonkWorkerError> {
    let repository = tonk
        .profile
        .space(space.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("space {space}: {e}")))?;
    let remote = match resolve_configured_remote_url_with(&repository, &tonk.operator).await? {
        ConfiguredRemoteRequirement::Ready(remote) => Some(remote.access_url.to_string()),
        ConfiguredRemoteRequirement::Refused(_) => None,
    };
    let account = member_did(tonk).await?;
    Ok(Terms { remote, account })
}

/// Take up a delegation for `space`: save its chain to this worker's profile,
/// where every proof is looked up, and mount the space as a replica syncing
/// with `remote`. From then on this worker acts for `account`, the one the
/// person's profile acts for. Refuses a chain for another space or another
/// audience.
pub(crate) async fn adopt(
    tonk: &TonkState,
    space: &Did,
    chain: &[u8],
    remote: Option<&str>,
    account: &Did,
) -> Result<(), TonkWorkerError> {
    let chain = DelegationChain::try_from(chain)
        .map_err(|e| TonkWorkerError::Router(format!("malformed delegation: {e}")))?;
    if chain.subject() != Some(space) {
        return Err(TonkWorkerError::Forbidden(format!(
            "delegation is not for {space}"
        )));
    }
    let profile = tonk.profile.did();
    if *chain.audience() != profile {
        return Err(TonkWorkerError::Forbidden(format!(
            "delegation is not for this worker's profile {profile}"
        )));
    }
    tonk.profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to save delegation: {e}")))?;
    mount_replica(tonk, space, remote, None).await?;
    // Listed in this worker's profile, as a joined space is in the host's.
    // What asks whether a space is mounted reads that listing, and a space
    // missing from it is never brought up to the library this worker ships.
    record_initialized_replica_in_profile(tonk, space)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to list {space}: {e}")))?;
    // The roster names the account a member acts for. A profile that has
    // since signed in acts for another, and the entry this worker made under
    // the last one moves to it.
    let previous = acts_for(tonk).await?;
    act_for(tonk, account).await?;
    if let Some(previous) = previous.filter(|previous| previous != account)
        && let Err(error) = migrate_membership_rows(tonk, space, &previous, account).await
    {
        log!("space worker: the roster of {space} still names {previous}: {error}");
    }
    resume(tonk, space).await?;
    log!("space worker: adopted {space} for {account} (remote: {remote:?})");
    Ok(())
}

/// The fact saying which account a session acts for, on dialog's session
/// entity beside the profile and operator dialog records there.
const SESSION_ACCOUNT: &str = "xyz.tonk.session/account";

/// Put back what a restart of this worker lost about `space`: the account
/// this session acts for, in the session overlay, where a view reads who is
/// looking at it. An overlay lives only as long as the worker, so this runs
/// each time one starts. Nothing to do before the worker has been told an
/// account.
pub(crate) async fn resume(tonk: &TonkState, space: &Did) -> Result<(), TonkWorkerError> {
    let Some(account) = acts_for(tonk).await? else {
        return Ok(());
    };
    let the = SESSION_ACCOUNT
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("{SESSION_ACCOUNT}: {e}")))?;
    tonk.reactor
        .repository(space.repo_key())
        .branch(CONTENT_BRANCH)
        .overlay()
        .assert(RawClaim {
            the,
            of: Session::entity(),
            is: Value::Entity(account.this()),
            unique: true,
        })
        .write()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to say who {space} is for: {e}")))
}

/// A space's `main` as the host holds it: the snapshot of everything its
/// revision reaches, and the revision itself. `None` for a space with nothing
/// on `main` yet.
pub(crate) struct Snapshot {
    /// The CARv1 of every block and blob the revision reaches.
    pub(crate) content: Vec<u8>,
    /// The revision, as JSON.
    pub(crate) revision: Vec<u8>,
}

/// Snapshot `space`'s `main` for its own worker to seed from. Strict: a block
/// this replica does not hold fails the snapshot rather than leaving a hole.
pub(crate) async fn snapshot(
    tonk: &TonkState,
    space: &Did,
) -> Result<Option<Snapshot>, TonkWorkerError> {
    let repository = tonk
        .profile
        .space(space.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("space {space}: {e}")))?;
    let branch = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("{space} main: {e}")))?;
    let Some(revision) = branch.revision() else {
        return Ok(None);
    };
    let root = Blake3Hash::from(*revision.tree.hash());
    let content = codec::encode(
        repository
            .snapshot(revision.clone())
            .export()
            .perform(&tonk.operator),
        vec![root],
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("failed to snapshot {space}: {e}")))?;
    let revision = serde_json::to_vec(&revision)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to encode revision: {e}")))?;
    Ok(Some(Snapshot { content, revision }))
}

/// Seed the freshly mounted replica of `space` from the host's snapshot: store
/// its content, then publish its revision on `main`. A replica that already
/// has a revision is left alone, so a repeated seed never rewinds it.
pub(crate) async fn seed(
    tonk: &TonkState,
    space: &Did,
    content: &[u8],
    revision: &[u8],
) -> Result<(), TonkWorkerError> {
    let repository = tonk
        .profile
        .space(space.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("space {space}: {e}")))?;
    let branch = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("{space} main: {e}")))?;
    if branch.revision().is_some() {
        return Ok(());
    }
    let revision: Revision = serde_json::from_slice(revision)
        .map_err(|e| TonkWorkerError::Router(format!("malformed revision: {e}")))?;
    let items = codec::decode(content)
        .map_err(|e| TonkWorkerError::Router(format!("malformed snapshot: {e}")))?;
    let imported = repository
        .import(stream::iter(items))
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to store {space}: {e}")))?;
    branch
        .reset(revision)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to publish {space}: {e}")))?;
    log!("space worker: seeded {space} ({imported:?})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use dialog_repository::{RepositoryExt as _, Revision};
    use dialog_varsig::Did;
    use tower::ServiceExt;

    use super::{adopt, delegate, member_did, resume, seed, snapshot};
    use crate::TonkWorkerError;
    use crate::helpers::state::{test_state, test_state_without_root};
    use crate::router::join::{find_replica_for_subject, mount_replica};
    use crate::router::{RepositoryInfo, api_router_with_state};
    use crate::worker::TonkState;

    /// A worker standing in for a space origin's: a profile of its own. Unlike
    /// a real one it shares the host's storage, since a test runs in a single
    /// origin and every profile there mounts a space in the same place. So a
    /// space the host holds is never empty here; seeding is tested on a
    /// replica the host never mounted.
    async fn space_origin() -> TonkState {
        test_state_without_root().await
    }

    /// The `main` revision `tonk` holds for `space`, if any.
    async fn main_revision(tonk: &TonkState, space: &Did) -> Option<Revision> {
        let repository = tonk
            .profile
            .space(space.as_str())
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap()
            .revision()
    }

    /// A host with a space on it that has something on `main`.
    async fn host_with_space() -> (crate::router::AppState, Did) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/space-worker")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: RepositoryInfo = serde_json::from_slice(&body).unwrap();
        let doc = "attribute!: &probe-name\n  description: A name\n  the: xyz.tonk.probe/name\n  as: text\n  cardinality: one\n";
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{}/branch/main/evaluate",
                        info.name
                    ))
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from(doc))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        (state, info.name.parse().unwrap())
    }

    #[dialog_common::test]
    async fn it_hands_a_space_worker_its_space() {
        let (host, space) = host_with_space().await;
        let host = host.read().await;
        let worker = space_origin().await;

        let grant = delegate(&host, &space, &worker.profile.did())
            .await
            .unwrap();
        assert!(
            grant.remote.is_none(),
            "a space only on this device has no upstream"
        );
        adopt(&worker, &space, &grant.chain, None, &grant.account)
            .await
            .unwrap();
        assert!(
            main_revision(&worker, &space).await.is_some(),
            "the worker reads the space it was handed"
        );
        assert!(
            find_replica_for_subject(&worker, &space).await.unwrap(),
            "the worker lists the space, so its library is kept up to date"
        );
        assert_eq!(
            member_did(&worker).await.unwrap(),
            member_did(&host).await.unwrap(),
            "the worker acts for the account the person's profile acts for"
        );
        resume(&worker, &space).await.unwrap();
    }

    /// The accounts the roster of `space` names, as `tonk` reads it.
    async fn roster(tonk: &TonkState, space: &Did) -> Vec<String> {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::{Membership, prelude::DidExt as _};

        let session = tonk
            .reactor
            .repository(space.repo_key())
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let rows: Vec<Membership> = session
            .handle()
            .query()
            .select(Query::<Membership> {
                this: Term::var("this"),
                subject: Term::from(space.this()),
                member: Term::var("member"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| row.member.0.to_string())
            .collect()
    }

    /// A profile that signs in acts for another account, and tells the
    /// space's worker so. The roster entry made under the last account is
    /// the worker's to move: the person's profile holds no roster.
    #[dialog_common::test]
    async fn it_moves_its_roster_entry_to_the_account_it_is_told() {
        let (host, space) = host_with_space().await;
        let host = host.read().await;
        let worker = space_origin().await;
        let grant = delegate(&host, &space, &worker.profile.did())
            .await
            .unwrap();
        adopt(&worker, &space, &grant.chain, None, &grant.account)
            .await
            .unwrap();
        assert_eq!(
            roster(&worker, &space).await,
            vec![grant.account.to_string()]
        );

        let signed_in = space_origin().await.profile.did();
        adopt(&worker, &space, &grant.chain, None, &signed_in)
            .await
            .unwrap();
        assert_eq!(roster(&worker, &space).await, vec![signed_in.to_string()]);
    }

    #[dialog_common::test]
    async fn it_seeds_an_empty_replica_from_a_snapshot() {
        let (host, space) = host_with_space().await;
        let host = host.read().await;
        let worker = space_origin().await;
        // A replica nothing has mounted yet, standing in for the space as a
        // fresh origin holds it.
        let replica = space_origin().await.profile.did();
        mount_replica(&worker, &replica, None, None).await.unwrap();
        assert!(
            main_revision(&worker, &replica).await.is_none(),
            "a freshly mounted replica is empty"
        );

        let copy = snapshot(&host, &space)
            .await
            .unwrap()
            .expect("main has content");
        seed(&worker, &replica, &copy.content, &copy.revision)
            .await
            .unwrap();
        assert_eq!(
            main_revision(&worker, &replica).await,
            main_revision(&host, &space).await,
            "the replica holds the host's revision"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_delegation_for_another_worker() {
        let (host, space) = host_with_space().await;
        let host = host.read().await;
        let worker = space_origin().await;
        let other = space_origin().await;

        let grant = delegate(&host, &space, &other.profile.did()).await.unwrap();
        let refused = adopt(&worker, &space, &grant.chain, None, &grant.account).await;
        assert!(matches!(refused, Err(TonkWorkerError::Forbidden(_))));
    }

    #[dialog_common::test]
    async fn it_leaves_a_replica_with_content_alone() {
        let (host, space) = host_with_space().await;
        let host = host.read().await;
        let worker = space_origin().await;
        let grant = delegate(&host, &space, &worker.profile.did())
            .await
            .unwrap();
        adopt(&worker, &space, &grant.chain, None, &grant.account)
            .await
            .unwrap();
        let seeded = main_revision(&worker, &space).await;
        assert!(seeded.is_some());

        // Seeding again with anything at all must not rewind the replica.
        seed(&worker, &space, b"not a snapshot", b"not a revision")
            .await
            .unwrap();
        assert_eq!(main_revision(&worker, &space).await, seeded);
    }
}

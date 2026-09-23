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

use dialog_capability::Subject;
use dialog_common::Blake3Hash;
use dialog_effects::Use;
use dialog_repository::{RepositoryExt as _, Revision, codec};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, time::Timestamp};
use dialog_varsig::Did;
use futures_util::stream;
use tonk_common::log;

use super::create_invite::{ConfiguredRemoteRequirement, resolve_configured_remote_url_with};
use super::join::mount_replica;
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
    Ok(Grant {
        chain,
        expires,
        remote,
    })
}

/// Take up a delegation for `space`: save its chain to this worker's profile,
/// where every proof is looked up, and mount the space as a replica syncing
/// with `remote`. Refuses a chain for another space or another audience.
pub(crate) async fn adopt(
    tonk: &TonkState,
    space: &Did,
    chain: &[u8],
    remote: Option<&str>,
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
    log!("space worker: adopted {space} (remote: {remote:?})");
    Ok(())
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

    use super::{adopt, delegate, seed, snapshot};
    use crate::TonkWorkerError;
    use crate::helpers::state::{test_state, test_state_without_root};
    use crate::router::join::mount_replica;
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
        adopt(&worker, &space, &grant.chain, None).await.unwrap();
        assert!(
            main_revision(&worker, &space).await.is_some(),
            "the worker reads the space it was handed"
        );
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
        let refused = adopt(&worker, &space, &grant.chain, None).await;
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
        adopt(&worker, &space, &grant.chain, None).await.unwrap();
        let seeded = main_revision(&worker, &space).await;
        assert!(seeded.is_some());

        // Seeding again with anything at all must not rewind the replica.
        seed(&worker, &space, b"not a snapshot", b"not a revision")
            .await
            .unwrap();
        assert_eq!(main_revision(&worker, &space).await, seeded);
    }
}

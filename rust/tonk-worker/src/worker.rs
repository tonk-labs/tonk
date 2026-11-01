//! The worker's state: the profile, its operator, and what the router
//! shares between requests.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(target_arch = "wasm32")]
use crate::router::AppState;
#[cfg(target_arch = "wasm32")]
use crate::router::space_worker;
use crate::{bootstrap_profile, router::ViewBindings};
#[cfg(target_arch = "wasm32")]
use dialog_artifacts::Asset;
use dialog_peer::{Peer, Session};
use dialog_storage::provider::storage::Storage;
#[cfg(target_arch = "wasm32")]
use dialog_varsig::Did;
#[cfg(feature = "persona")]
use tokio::sync::Mutex;

/// Default storage type — filesystem on native.
#[cfg(not(target_arch = "wasm32"))]
pub type DefaultSpace = dialog_storage::provider::storage::NativeSpace;

/// Default storage type on WASI: the layers and credentials of the kernel's
/// host.
#[cfg(target_arch = "wasm32")]
pub type DefaultSpace = tonk_kernel_world::space::KernelSpace;

/// Concrete operator type for the default storage backend.
pub type DefaultOperator = Peer<DefaultSpace, Session>;

/// The profile this worker signs as: a peer acting with its own key.
pub type DefaultProfile = Peer<DefaultSpace>;

/// Application state containing the profile and operator.
pub struct TonkState {
    /// The user's persistent profile.
    pub profile: DefaultProfile,
    /// The operator derived from the profile — the key that signs
    /// presign invocations. Rotated by
    /// [`session`](crate::session) before its delegation lapses, so it
    /// is not stable for the life of the worker.
    pub operator: DefaultOperator,
    /// The branch of the profile repository this state is on: the
    /// account's branch when signed in, an upstream-less one when not.
    /// Read from `meta` at boot; a switch records a new active branch
    /// there and rebuilds the state, so it is fixed for one state's life.
    pub active_branch: String,
    /// The storage pool every space is mounted in. Held so a rotated
    /// operator can be built over the *same* pool: a replacement with
    /// its own would leave the reactor's cached repository and branch
    /// handles talking to the retired one.
    pub storage: Storage<DefaultSpace>,
    /// When the current operator's delegation stops being valid, unix
    /// seconds. Consulted by the sync drain, which rotates the session
    /// as this approaches.
    pub session_expires_at: u64,
    /// Display name the profile was opened under. `Profile` does
    /// not retain this internally, so we carry it here for routes
    /// that report it back to the UI (e.g. `GET /api/profile`).
    pub profile_name: String,
    /// Reactive layer: cached repository/branch handles and the
    /// query subscriptions registered against them. Routes that
    /// mutate a branch flow through `reactor.repository(r).branch(b)`
    /// so subscription broadcasts happen automatically.
    pub reactor: crate::Reactor,
    pub(crate) admission: crate::router::adopt::cache::AdmissionCache,
    // Also under `helpers`: the state fixture in `helpers::state`
    // constructs a `TonkState` literally, so every field it names has to
    // exist whenever that fixture compiles. Only this crate's own tests
    // READ it, hence the allow — under `helpers` alone it exists purely
    // to keep that struct literal valid.
    #[cfg(any(test, feature = "helpers"))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) reject_admission_content_reads: std::sync::atomic::AtomicBool,
    /// Terminal lifecycle latch for this worker generation. Once a verified
    /// successor installs, no later query reconnect may recreate an SSE stream
    /// on the outgoing worker, even after `registration.waiting` clears.
    pub(crate) retiring: Arc<AtomicBool>,
    /// View-iframe bindings keyed by service-worker Client ID.
    /// Behind its own interior lock so binding registration /
    /// lookup doesn't contend with profile/operator access on
    /// the outer state lock.
    pub view_bindings: ViewBindings,
    /// Registered command handlers — the typed-Rust effects fired by
    /// transient command concepts after a commit. Two vocabularies
    /// (profile / space), selected per dispatch by the triggering
    /// commit's origin; consulted by the transact path's post-commit
    /// dispatch.
    pub commands: crate::router::CommandProviders,
    /// Repositories with un-pushed local commits. A commit enqueues its repo;
    /// `POST /api/sync` (the page heartbeat) and the post-commit push drain
    /// reconcile it. See `router::sync::SyncQueue`.
    pub sync_queue: crate::router::SyncQueue,
    /// Liveness ledger: SW client → what it registered (site stamps) and
    /// whether we have observed it alive. The stale-client sweep reaps
    /// born-then-died clients from here. See [`crate::router::ClientRegistry`].
    pub clients: crate::router::ClientRegistry,
    /// Spaces whose seed this worker instance already checked against the
    /// shipped bundle. See [`crate::router::adopt::SeedUpgrades`].
    pub(crate) seed_upgrades: crate::router::adopt::SeedUpgrades,
    /// Whether each space renders on an origin of its own, where the
    /// space's own worker holds it. This worker, the person's profile's, then
    /// creates a space's identity and mounts nothing of it (see
    /// `router::space_directory`). So for a person's worker started by a
    /// site's script (see [`crate::device::spaces_have_origins`]).
    pub site_origins: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Routing keys the hidden account repository answers to, resolved lazily.
    /// Consulted by the middleware that keeps that repository off the generic
    /// HTTP surface, so it sits on the hot path for every repository request.
    /// See [`crate::router::AccountKeys`].
    pub account_keys: crate::router::AccountKeys,
    /// The profile-library bytes and branch revision most recently validated
    /// by this worker instance.
    pub(crate) profile_library: crate::router::ProfileLibraryCache,
    /// Handle to the fixed registry profile recording which profile is
    /// active and the roster of every profile this browser knows. Held so
    /// the profile-switching routes can validate, repoint, and annotate
    /// without re-deriving where the registry lives — and so tests can
    /// point it at a scratch registry instead of the real one.
    pub(crate) registry: crate::device::Registry,
    /// Serializes every replacement of the active profile, whether explicit
    /// or selected automatically by an account ceremony.
    #[cfg(feature = "persona")]
    pub(crate) profile_transition: Arc<Mutex<()>>,
    /// Monotonic active-profile context. Browser clients bind to the value
    /// they first use and become stale when a profile promotion advances it.
    pub(crate) context_generation: Arc<AtomicU64>,
}

impl TonkState {
    /// Whether each space's content is held by a worker on the space's own
    /// origin, and none of it by this one: this is the worker of a person's
    /// profile where sites have origins of their own. What a command here
    /// does to a space's content is then that worker's to do (see
    /// [`space_reach`](crate::router::space_reach)).
    pub(crate) fn spaces_elsewhere(&self) -> bool {
        self.site_origins.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// End a native host generation: enter the one-way retiring state and
    /// release every query stream.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn shutdown(&self) {
        self.retiring.store(true, Ordering::Release);
        self.reactor.shutdown();
    }

    /// Open the ordinary worker runtime in an explicit native storage directory.
    /// The caller owns authentication, process isolation and the directory's
    /// lifetime. No default user profile or process working directory is used.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn open_native(directory: &std::path::Path) -> Result<Self, crate::TonkWorkerError> {
        std::fs::create_dir_all(directory.join("spaces"))
            .map_err(|error| crate::TonkWorkerError::Internal(error.to_string()))?;
        let directory = directory
            .canonicalize()
            .map_err(|error| crate::TonkWorkerError::Internal(error.to_string()))?;
        let registry = crate::device::Registry {
            profile: "hosted".to_owned(),
            directory: dialog_effects::storage::Directory::At(
                directory.to_string_lossy().into_owned(),
            ),
            standing: Default::default(),
        };
        let storage = registry.storage().await?;
        let profile = registry.open_profile(&storage, "hosted").await?;
        boot_state(storage, "hosted".to_owned(), profile, registry).await
    }

    /// Open the worker runtime in a kernel on WASI, as the browser's worker
    /// opens it: the device's registry, and the profile this device signs as.
    /// Its storage and keys are the layers and credentials the kernel's host
    /// gives it (`tonk-kernel-world`).
    #[cfg(target_arch = "wasm32")]
    pub async fn open_kernel() -> Result<Self, crate::TonkWorkerError> {
        let registry = crate::device::Registry::device();
        let storage = registry.storage().await?;
        let (name, profile) = registry.open_active(&storage).await?;
        boot_state(storage, name, profile, registry).await
    }

    /// Take up a delegation for `space`, as a space's own worker does: the
    /// chain from the space to this worker's profile, as the person's profile
    /// signed it.
    #[cfg(target_arch = "wasm32")]
    pub async fn adopt_space(
        &self,
        space: &str,
        chain: &[u8],
    ) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        space_worker::adopt(self, &space, chain).await?;
        self.reactor.run_scheduled_polls(&self.operator).await;
        Ok(())
    }

    /// Delegate `space` to `audience`, the credential of the session of the
    /// space's own worker, as the person's persona grants what that worker
    /// claims: the chain from the space down to `audience`, encoded.
    #[cfg(all(target_arch = "wasm32", feature = "persona"))]
    pub async fn delegate_space(
        &self,
        space: &str,
        audience: &str,
    ) -> Result<Vec<u8>, crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        let audience: Did = audience
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("audience: {e:?}")))?;
        let grant = space_worker::delegate(self, &space, &audience).await?;
        // The space's own worker takes it up on this device with what it is
        // delegated: from now on this device holds it, as a replica.
        if !crate::router::join::find_replica_for_subject(self, &space).await? {
            crate::router::repository::record_initialized_replica_in_profile(self, &space)
                .await
                .map_err(|e| crate::TonkWorkerError::Internal(format!("list {space}: {e}")))?;
        }
        Ok(grant.chain)
    }

    /// Whether this worker took up a delegation before: it acts for the
    /// account the delegation named.
    #[cfg(target_arch = "wasm32")]
    pub async fn acts(&self) -> Result<bool, crate::TonkWorkerError> {
        Ok(crate::router::account::acts_for(self).await?.is_some())
    }

    /// Give `space`, which this worker holds, its first commit if it has
    /// none: the kernel it runs, the asset `kernel` (its BLAKE3 `hash` and
    /// `size`, which the space's store already holds), and the pin to it.
    #[cfg(target_arch = "wasm32")]
    pub async fn found_space(
        &self,
        space: &str,
        hash: [u8; 32],
        size: u64,
    ) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        let kernel = Asset::stored(hash, size);
        space_worker::found(self, &space, kernel).await?;
        self.reactor.run_scheduled_polls(&self.operator).await;
        Ok(())
    }

    /// What the space's own worker is to create `space`'s content from, as
    /// JSON, or `None` when there is nothing left to hand over: the seed this
    /// persona kept as it created the space.
    #[cfg(all(target_arch = "wasm32", feature = "persona"))]
    pub async fn pending_seed(
        &self,
        space: &str,
    ) -> Result<Option<String>, crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        crate::router::repository::pending_seed(self, &space)
            .await?
            .map(|seed| {
                serde_json::to_string(&seed)
                    .map_err(|e| crate::TonkWorkerError::Internal(e.to_string()))
            })
            .transpose()
    }

    /// The space's own worker created `space`'s content from its seed:
    /// there is nothing left to hand over.
    #[cfg(all(target_arch = "wasm32", feature = "persona"))]
    pub async fn settle_seed(&self, space: &str) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        crate::router::repository::settle_seed(self, &space).await
    }

    /// Create the content of `space`, which this worker holds, from `seed`
    /// (JSON, as [`pending_seed`] gives it), and record the kernel it runs,
    /// the asset `kernel` (its BLAKE3 `hash` and `size`) on top. Nothing is
    /// written over a `main` that has content.
    ///
    /// [`pending_seed`]: Self::pending_seed
    #[cfg(target_arch = "wasm32")]
    pub async fn seed_space(
        &self,
        space: &str,
        seed: &str,
        hash: [u8; 32],
        size: u64,
    ) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        let seed: crate::router::repository::SpaceSeed = serde_json::from_str(seed)
            .map_err(|e| crate::TonkWorkerError::Router(format!("seed: {e}")))?;
        crate::router::repository::create_content(self, &space, &seed)
            .await
            .map_err(|e| crate::TonkWorkerError::Internal(e.to_string()))?;
        space_worker::pin(self, &space, Asset::stored(hash, size)).await?;
        self.reactor.run_scheduled_polls(&self.operator).await;
        Ok(())
    }

    /// Create `space` as a copy of another space, from that space's
    /// `content` and `revision` (see [`snapshot_space`]) and `seed` (JSON, as
    /// [`pending_seed`] gives it), and record the kernel it runs, the asset
    /// `kernel` (its BLAKE3 `hash` and `size`) on top. Nothing is written over
    /// a `main` that has content.
    ///
    /// [`snapshot_space`]: Self::snapshot_space
    /// [`pending_seed`]: Self::pending_seed
    #[cfg(target_arch = "wasm32")]
    pub async fn copy_space(
        &self,
        space: &str,
        seed: &str,
        content: &[u8],
        revision: &[u8],
        hash: [u8; 32],
        size: u64,
    ) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        let seed: crate::router::repository::SpaceSeed = serde_json::from_str(seed)
            .map_err(|e| crate::TonkWorkerError::Router(format!("seed: {e}")))?;
        crate::router::repository::create_copy(self, &space, &seed, content, revision)
            .await
            .map_err(|e| crate::TonkWorkerError::Internal(e.to_string()))?;
        space_worker::pin(self, &space, Asset::stored(hash, size)).await?;
        self.reactor.run_scheduled_polls(&self.operator).await;
        Ok(())
    }

    /// `space`'s `main`, for another space to be made a copy of: the CARv1 of
    /// everything its revision reaches, and the revision as JSON. `None` for a
    /// space with nothing on `main` yet.
    #[cfg(target_arch = "wasm32")]
    pub async fn snapshot_space(
        &self,
        space: &str,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        Ok(space_worker::snapshot(self, &space)
            .await?
            .map(|snapshot| (snapshot.content, snapshot.revision)))
    }

    /// Put back what a restart lost about `space`, a space this worker took
    /// up before.
    #[cfg(target_arch = "wasm32")]
    pub async fn resume_space(&self, space: &str) -> Result<(), crate::TonkWorkerError> {
        let space: Did = space
            .parse()
            .map_err(|e| crate::TonkWorkerError::Router(format!("space: {e:?}")))?;
        space_worker::resume(self, &space).await?;
        self.reactor.run_scheduled_polls(&self.operator).await;
        Ok(())
    }

    /// Whether this worker generation has terminally begun retirement.
    pub(crate) fn is_retiring(&self) -> bool {
        self.retiring.load(Ordering::Acquire)
    }
}

// SAFETY: a kernel runs on one thread. The interior types hold handles to
// the host's resources, which are !Send/!Sync, but cross-thread access cannot
// occur in a single-threaded component.
#[cfg(target_arch = "wasm32")]
unsafe impl Send for TonkState {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for TonkState {}

/// Build a [`TonkState`] for an already-opened profile — the shared core
/// of worker boot and profile activation.
///
/// Opens a signing session over `storage`, constructs the reactive state,
/// and bootstraps the profile repo's meta branch (idempotent). The
/// registry's active-profile pointer is deliberately not touched here:
/// activation repoints it only after this succeeds, so a failed build
/// never strands the next boot on a broken profile.
pub(crate) async fn boot_state(
    storage: Storage<DefaultSpace>,
    profile_name: String,
    profile: DefaultProfile,
    registry: crate::device::Registry,
) -> Result<TonkState, crate::TonkWorkerError> {
    boot_state_with_profile_library(storage, profile_name, profile, registry, Default::default())
        .await
}

/// Build profile state while retaining the running worker generation's
/// acquired profile-library input across an in-memory profile switch.
pub(crate) async fn boot_state_with_profile_library(
    storage: Storage<DefaultSpace>,
    profile_name: String,
    profile: DefaultProfile,
    registry: crate::device::Registry,
    profile_library: crate::router::ProfileLibraryCache,
) -> Result<TonkState, crate::TonkWorkerError> {
    // A switch hands over the peer of the branch it leaves; the profile's
    // own branch is where every other branch's peer is derived from.
    let profile = if profile.state().name() == crate::router::repository::PROFILE_BRANCH {
        profile
    } else {
        registry
            .open_on(
                &storage,
                &profile_name,
                crate::router::repository::PROFILE_BRANCH,
            )
            .await?
    };
    let reactor = crate::Reactor::new(profile.credential().clone());
    // Session construction reads branch reference cells, but no longer
    // walks or retains delegation content. Hydrating after a construction
    // failure cannot repair entropy, signing, or local reference errors;
    // surface them without touching the profile's durable contents.
    let session = crate::session::open(&profile).await?;
    // Which branch the profile is on. Reading `meta` takes an operator,
    // so the bootstrap session opens on `main`; a profile that is on
    // another branch gets a session whose authority is that branch's.
    let active_branch = crate::router::profile::active_branch_name(&reactor, &session.operator)
        .await
        .unwrap_or_else(|| crate::router::repository::PROFILE_BRANCH.to_owned());
    let (profile, session) = if active_branch == crate::router::repository::PROFILE_BRANCH {
        (profile, session)
    } else {
        // Each account branch is its own account: the peer acting on it
        // keeps its records, custody and delegations in the branch, and
        // what the branch kept while branches shared one peer follows it.
        let branch = registry
            .open_on(&storage, &profile_name, &active_branch)
            .await?;
        let suffix = format!("-{active_branch}");
        tonk_account::peer::copy_site_secrets(&profile, &branch, |name| name.ends_with(&suffix))
            .await
            .map_err(|error| {
                crate::TonkWorkerError::Internal(format!(
                    "failed to move the branch's site secrets: {error}"
                ))
            })?;
        registry
            .migrate_branch_secrets(&branch, &profile_name, &active_branch)
            .await?;
        let session = crate::session::open_on(&branch, &active_branch).await?;
        (branch, session)
    };

    let state = TonkState {
        profile,
        operator: session.operator,
        active_branch,
        storage,
        session_expires_at: session.expires_at,
        profile_name,
        reactor,
        admission: Default::default(),
        #[cfg(any(test, feature = "helpers"))]
        reject_admission_content_reads: Default::default(),
        retiring: Arc::new(AtomicBool::new(false)),
        view_bindings: Default::default(),
        commands: crate::router::command_providers(),
        sync_queue: Default::default(),
        clients: Default::default(),
        seed_upgrades: Default::default(),
        // Known before anything runs, the chores of starting included: they
        // would mount spaces that are another worker's to hold.
        site_origins: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            crate::device::spaces_have_origins(),
        )),
        account_keys: Default::default(),
        profile_library,
        registry,
        #[cfg(feature = "persona")]
        profile_transition: Arc::new(Mutex::new(())),
        context_generation: Arc::new(AtomicU64::new(0)),
    };
    bootstrap_profile(&state).await.map_err(|e| {
        crate::TonkWorkerError::Internal(format!("failed to bootstrap profile meta: {e}"))
    })?;
    crate::router::identity::follow_signed_in_account(&state).await?;
    Ok(state)
}

/// Whether a page watches something this worker holds: a cached branch
/// holds a live subscriber. A kernel syncs on a timer while one does, as
/// the browser's worker's loop does; each drain sweeps the account too,
/// which is how a page waiting on an emailed link hears it was followed.
#[cfg(target_arch = "wasm32")]
pub async fn watched(state: &AppState) -> bool {
    has_live_subscribers(state).await
}

/// Whether any cached branch — named repositories or the profile — holds a
/// live subscriber. The signal that a page is watching, so the SW should
/// keep pulling upstream changes.
#[cfg(target_arch = "wasm32")]
async fn has_live_subscribers(state: &AppState) -> bool {
    let tonk = state.read().await;
    // The profile's own repository is among the cached ones.
    let repos = tonk.reactor.repos().read();
    for repo in repos.values() {
        for branch in repo.branches().read().values() {
            if !branch.subscriptions().lock().is_empty() {
                return true;
            }
        }
    }
    false
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_host_tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn native_worker_boots_the_shared_router_in_isolated_storage() {
        let directory =
            std::env::temp_dir().join(format!("tonk-worker-host-{}", rand::random::<u64>()));
        let state = TonkState::open_native(&directory)
            .await
            .expect("native worker boot");
        let did = state.profile.did().to_string();
        assert!(matches!(state.registry.space_location("example").directory,
            dialog_effects::storage::Directory::At(ref path) if path == &directory.canonicalize().unwrap().join("spaces").to_string_lossy()));
        let (router, _) = crate::api_router(state);
        let response = router
            .clone()
            .oneshot(
                ::axum::http::Request::builder()
                    .uri("/api/profile")
                    .body(::axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), ::axum::http::StatusCode::OK);
        let created = router
            .clone()
            .oneshot(
                ::axum::http::Request::builder()
                    .method("PUT")
                    .uri("/api/repository/host-test")
                    .header("content-type", "application/json")
                    .body(::axum::body::Body::from(r#"{"branch":{"main":{}}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = created.status();
        let bytes = ::axum::body::to_bytes(created.into_body(), 1_000_000)
            .await
            .unwrap();
        assert_eq!(
            status,
            ::axum::http::StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let repository: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let key = repository["name"].as_str().expect("routing key");
        let document = "concept!: &host-example\n  with:\n    title: { the: example.host/title, as: text, description: Test title }\nhost-example!:\n  this: urn:host:record\n  title: Shared worker\n";
        let evaluated = router
            .clone()
            .oneshot(
                ::axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/repository/{key}/branch/main/evaluate"))
                    .header("content-type", "text/plain")
                    .body(::axum::body::Body::from(document))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = evaluated.status();
        let bytes = ::axum::body::to_bytes(evaluated.into_body(), 1_000_000)
            .await
            .unwrap();
        assert_eq!(
            status,
            ::axum::http::StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        for document in [
            "library/install!:\n  component: tonk:library/notebook\n  time: 1.0\n",
            "notebook/named!:\n  this: urn:host:notebook\n  title: General commands\nblock/edit!:\n  subject: urn:host:block\n  notebook: urn:host:notebook\n  source: Written through the shared worker\n",
        ] {
            let response = router
                .clone()
                .oneshot(
                    ::axum::http::Request::builder()
                        .method("POST")
                        .uri(format!("/api/repository/{key}/branch/main/evaluate"))
                        .header("content-type", "text/plain")
                        .body(::axum::body::Body::from(document))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = ::axum::body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap();
            assert_eq!(
                status,
                ::axum::http::StatusCode::OK,
                "{}",
                String::from_utf8_lossy(&bytes)
            );
        }
        drop(router);
        let reopened = TonkState::open_native(&directory)
            .await
            .expect("reopen native worker");
        assert_eq!(reopened.profile.did().to_string(), did);
        let (router, _) = crate::api_router(reopened);
        let read = router
            .oneshot(
                ::axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/repository/{key}/branch/main/evaluate?transact=false"
                    ))
                    .header("content-type", "text/plain")
                    .body(::axum::body::Body::from("notebook/block:\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(read.status(), ::axum::http::StatusCode::OK);
        let bytes = ::axum::body::to_bytes(read.into_body(), 1_000_000)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Written through the shared worker"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

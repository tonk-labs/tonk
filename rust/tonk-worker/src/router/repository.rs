//! Repository create/update route.
//!
//! `PUT /api/repository/{repo}` always creates. If the repository already
//! exists, it responds with `412 Precondition Failed` when the request
//! carried `If-None-Match: *`, or `409 Conflict` otherwise.

use std::collections::HashMap;

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use axum_wasm_macros::wasm_compat;
use dialog_credentials::SignerCredential;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{
    RemoteRepository, Repository, RepositoryExt as _, Revision, SiteAddress, Upstream,
};
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{Branch as MetaBranch, Remote, Replica, SpaceStatus, TrackingBranch};

use super::AppState;
use crate::{Notification, RepositoryError, TonkWorkerError, broadcast, worker::TonkState};

/// Name of the meta branch every repository has alongside its
/// content branches. The meta branch stores schema concepts
/// describing the repository itself (see [`tonk_schema`]).
const META_BRANCH: &str = "meta";

/// Configuration for a single remote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteConfiguration {
    /// The remote's site address (serialized `SiteAddress`).
    pub address: SiteAddress,
    /// Optional subject DID for the remote repository. Defaults to
    /// this repository's DID if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Did>,
}

impl RemoteConfiguration {
    /// Build a remote config from its address.
    pub fn new(address: impl Into<SiteAddress>) -> Self {
        Self {
            address: address.into(),
            subject: None,
        }
    }

    /// Override the subject DID — by default the remote's subject
    /// is the same as the local repository's DID.
    pub fn subject(mut self, subject: Did) -> Self {
        self.subject = Some(subject);
        self
    }
}

/// Upstream wiring for a branch, pointing at a remote branch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamConfiguration {
    /// The remote's local name (e.g. `"origin"`).
    pub remote: String,
    /// The branch name on that remote.
    pub branch: String,
}

impl UpstreamConfiguration {
    /// Build an upstream config pointing at `{remote}/{branch}`.
    pub fn new(remote: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            branch: branch.into(),
        }
    }
}

/// Configuration / state for a single branch.
///
/// Same type is used for write (PUT body) and read (GET/PUT
/// response) — the server ignores `revision` on input and fills
/// it on output. Both fields serialize as `null` when absent so
/// the wire shape is consistent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchConfiguration {
    /// Upstream wiring, or `null` if the branch has no upstream.
    #[serde(default)]
    pub upstream: Option<UpstreamConfiguration>,
    /// The branch's current revision, or `null` if it has no
    /// commits. Server-populated; ignored on incoming PUT bodies.
    #[serde(default)]
    pub revision: Option<Revision>,
}

impl BranchConfiguration {
    /// Attach an upstream pointing at `{remote}/{branch}`.
    pub fn upstream(mut self, remote: impl Into<String>, branch: impl Into<String>) -> Self {
        self.upstream = Some(UpstreamConfiguration::new(remote, branch));
        self
    }
}

/// Configuration for creating/updating a repository.
///
/// Serialized as the body of `PUT /api/repository/{repo}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepositoryConfiguration {
    /// Remotes to create, keyed by local name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote: HashMap<String, RemoteConfiguration>,
    /// Branches to create, keyed by branch name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub branch: HashMap<String, BranchConfiguration>,
}

impl RepositoryConfiguration {
    /// Add (or replace) a remote entry.
    pub fn remote(mut self, name: impl Into<String>, config: RemoteConfiguration) -> Self {
        self.remote.insert(name.into(), config);
        self
    }

    /// Add (or replace) a branch entry.
    pub fn branch(mut self, name: impl Into<String>, config: BranchConfiguration) -> Self {
        self.branch.insert(name.into(), config);
        self
    }
}

/// Read-side view of a repository.
///
/// Returned by `GET /api/repository/{repo}` and `PUT
/// /api/repository/{repo}` (on create). The shape mirrors the write
/// configuration but adds the observable fields — identifier DIDs
/// and per-branch revision state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryInfo {
    /// The local repository name (the URL segment it's addressable at).
    pub name: String,
    /// The repository's own DID.
    pub subject: Did,
    /// The operator's DID (ephemeral session key).
    pub operator: Did,
    /// The profile's DID (long-lived identity).
    pub profile: Did,
    /// Branches probed so far. Today only `main` is probed if it
    /// exists; other branches don't appear even if they're on disk.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub branch: HashMap<String, BranchConfiguration>,
    /// Remotes referenced by probed branches. Today only the
    /// remote that `main.upstream` points at is included.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub remote: HashMap<String, RemoteConfiguration>,
}

/// Create a repository with optional remote and branch configuration.
///
/// Semantics:
/// - Always creates — if the repository exists, fails with `412
///   Precondition Failed` when `If-None-Match: *` was sent, or
///   `409 Conflict` otherwise.
/// - On success, delegates repository access to the current profile,
///   sets up any remotes from the body, creates each listed branch,
///   and wires up upstream tracking when specified.
/// - Returns `201 Created` with a [`RepositoryInfo`] body.
#[wasm_compat]
pub async fn put_repository(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Result<(StatusCode, Json<RepositoryInfo>), TonkWorkerError> {
    log!("PUT /api/repository/{}", name);

    let if_none_match_star = headers
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "*")
        .unwrap_or(false);

    // Parse body manually so JSON errors return our structured
    // `TonkWorkerError::Router` (JSON body) rather than axum's
    // default plain-text `JsonRejection`.
    let configuration = if body_bytes.is_empty() {
        RepositoryConfiguration::default()
    } else {
        serde_json::from_slice(&body_bytes)
            .map_err(|e| TonkWorkerError::Router(format!("Invalid request body: {}", e)))?
    };

    let tonk = state.write().await;

    // 1. Check if the repo already exists before attempting to
    // create. `.load()` returns Ok when the space is present; in
    // that case the PUT is rejected with 412 (if the caller sent
    // `If-None-Match: *`) or 409 (otherwise). Relying on
    // `.create()`'s own error signalling isn't enough because
    // it's possible for the space to exist while being
    // inconsistent enough that `.create()` would either succeed
    // (overwriting) or fail in a way we can't classify cleanly.
    if tonk
        .profile
        .repository(&name)
        .load()
        .perform(&tonk.operator)
        .await
        .is_ok()
    {
        let message = format!("Repository '{}' already exists", name);
        return Err(if if_none_match_star {
            TonkWorkerError::PreconditionFailed(message)
        } else {
            TonkWorkerError::Conflict(message)
        });
    }

    // 2. Create the repository and everything that comes with
    // it — delegation, remotes, branches, upstreams, meta facts.
    // This records the replica in the profile with `status: blank`
    // (see `record_replica_in_profile`), so the Hub card appears in
    // its installing state right away. The helper doesn't know about
    // HTTP; its errors are mapped to the right status via
    // `RepositoryError::into`.
    let repository = create_repository(&tonk, &name, &configuration).await?;
    let subject = repository.did();
    let info = build_repository_info(&tonk, &name, &repository).await;

    // 3. Seed asynchronously, then flip the replica to `initialized`.
    // Seeding the standard library is the slow part (~seconds of
    // prolly-tree commits); doing it inline would block this response
    // and starve the page's asset/Web Awesome loads on the single SW
    // thread. Instead we return now and seed in the background, then
    // stamp `status: initialized` so the Hub card settles. The reactor
    // re-polls the profile subscription on that commit, so the card
    // updates without the page polling.
    //
    // The spawned task takes an owned `AppState` (the lock is released
    // when `tonk` drops at the end of this scope) and re-acquires it.
    drop(tonk);
    let branches: Vec<String> = configuration.branch.keys().cloned().collect();
    spawn_seed(state, name, subject, branches);

    Ok((StatusCode::CREATED, Json(info)))
}

/// `CommandEnv` provides [`CreateSpace`] — the asserted-command path that
/// replaces a direct `PUT /api/repository/{name}`. The "New space" form
/// asserts a transient `CreateSpace`; the dispatcher calls this.
///
/// It does the same work as [`put_repository`]: record the replica
/// (`status: blank`) so the Hub shows it installing, create the
/// repository, seed the standard library, then flip the status to
/// `initialized`. An existing name is a no-op (logged) rather than an
/// error — there's no HTTP caller to receive a 409/412.
///
/// The provider re-locks the state through the env to do its (genuinely
/// multi-branch) IO and commits its own outcomes. `Output = ()`.
///
/// TODO(ucan): gate this by capability rather than by the mere existence
/// of this `Provider` impl. Long-term a command is a UCAN-like
/// invocation: the provider attempts the work and the operator's
/// capabilities decide whether each action is permitted.
///
/// TODO(stm): the existence check and the create are not atomic against
/// concurrent commands — two `CreateSpace` for the same name could both
/// pass the check. The transactional goal (read-set tracking + commit-
/// or-conflict) would close this.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[async_trait::async_trait(?Send)]
impl dialog_capability::Provider<tonk_schema::command::CreateSpace> for super::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::CreateSpace) {
        let name = command.name.0.clone();
        log!("command CreateSpace name={}", name);
        if let Err(error) = create_space_inner(self.state(), &name).await {
            log!("CreateSpace '{}' failed: {}", name, error);
        }
    }
}

/// The body of the [`CreateSpace`](tonk_schema::command::CreateSpace)
/// provider, split out so its `?` errors are logged once at the boundary.
/// Mirrors [`put_repository`] minus the HTTP shell.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn create_space_inner(state: &AppState, name: &str) -> Result<(), RepositoryError> {
    // The button asks for a `main`-branch space (the same config the old
    // `create_space` fetch sent).
    let configuration =
        RepositoryConfiguration::default().branch("main", BranchConfiguration::default());

    let (subject, branches) = {
        let tonk = state.write().await;

        // Already exists → no-op. Without an HTTP caller there's no
        // 409/412 to return; a duplicate command just does nothing.
        if tonk
            .profile
            .repository(name)
            .load()
            .perform(&tonk.operator)
            .await
            .is_ok()
        {
            log!("CreateSpace '{}': already exists, skipping", name);
            return Ok(());
        }

        // Create the repository (records the replica with status:blank).
        let repository = create_repository(&tonk, name, &configuration).await?;
        let subject = repository.did();
        let branches: Vec<String> = configuration.branch.keys().cloned().collect();
        (subject, branches)
    };

    // Seed + flip to initialized once the lock is released (seeding is
    // the slow part; holding the lock would stall the page).
    seed_and_initialize(state, name, &subject, &branches).await
}

/// Spawn the background seed + status flip for a freshly created
/// repository. Returns immediately; the work runs after the PUT
/// response is sent.
///
/// Native builds have no service-worker scope (and no `spawn_local`
/// runtime here), so they no-op — the seed/status path is browser-only.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn spawn_seed(state: AppState, name: String, subject: Did, branches: Vec<String>) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = seed_and_initialize(&state, &name, &subject, &branches).await {
            log!("Background seed for '{}' failed: {}", name, e);
        }
    });
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn spawn_seed(_state: AppState, _name: String, _subject: Did, _branches: Vec<String>) {}

/// Seed the standard library into every branch, then flip the
/// replica's status to `initialized`. Runs in the background after
/// `put_repository` has already responded.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn seed_and_initialize(
    state: &AppState,
    name: &str,
    subject: &Did,
    branches: &[String],
) -> Result<(), RepositoryError> {
    if !branches.is_empty() {
        let library = fetch_standard_library(STANDARD_LIBRARY_URL)
            .await
            .map_err(|e| RepositoryError::Internal(format!("fetch standard library: {e}")))?;
        let tonk = state.read().await;
        for branch_name in branches {
            seed_standard_library(&tonk, name, branch_name, &library)
                .await
                .map_err(|e| RepositoryError::Internal(format!("seed '{branch_name}': {e}")))?;
            log!(
                "Seeded standard library on '{}' branch '{}'",
                name,
                branch_name
            );
        }
        // The showcase demo (the "Trip to Pistoia" sample) lands only
        // in the default `home` repository, on top of the scaffold. It
        // resolves its bare concept references against the scaffold just
        // committed above. Every other repo gets the scaffold alone, so
        // its directory render has zero `workspace` instances and reads
        // as genuinely empty — the precondition for the launchpad view.
        if name == DEFAULT_REPOSITORY_NAME {
            let demo = fetch_standard_library(DEMO_LIBRARY_URL)
                .await
                .map_err(|e| RepositoryError::Internal(format!("fetch showcase demo: {e}")))?;
            for branch_name in branches {
                seed_standard_library(&tonk, name, branch_name, &demo)
                    .await
                    .map_err(|e| {
                        RepositoryError::Internal(format!("seed demo '{branch_name}': {e}"))
                    })?;
                log!(
                    "Seeded showcase demo on '{}' branch '{}'",
                    name,
                    branch_name
                );
            }
        }
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    } else {
        // Nothing to seed — the replica is immediately initialized.
        let tonk = state.read().await;
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    }
    log!("Repository '{}' initialized", name);
    Ok(())
}

/// URL of the served standard-library notation asset, copied into
/// the dist from `tonk-core/assets/library/core.yaml` by trunk. Seeded
/// onto each space's content branch. Only referenced from the
/// SW-scoped background seed path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const STANDARD_LIBRARY_URL: &str = "/library/core.yaml";

/// URL of the served showcase-demo notation asset (`demo.yaml`),
/// copied into the dist alongside `core.yaml`. Seeded on top of the
/// scaffold, but only into the default `home` repository, so every
/// other repo starts with zero workspace instances. Only referenced
/// from the SW-scoped background seed path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const DEMO_LIBRARY_URL: &str = "/library/demo.yaml";

/// The default repository created for a fresh profile. The showcase
/// demo is seeded only here; every other repository gets the scaffold
/// alone and renders empty until populated.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const DEFAULT_REPOSITORY_NAME: &str = "home";

/// URL of the lean profile library — only the `space` concept and the
/// Hub directory view. Seeded onto the profile's meta branch, which
/// backs nothing but the Hub, so it doesn't pay to write the full
/// workspace/board/sheet library it never reads. Only referenced from
/// the SW-scoped profile seed path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const PROFILE_LIBRARY_URL: &str = "/library/profile.yaml";

/// Fetch the standard-library notation document from the served
/// asset, sidestepping the HTTP cache so an edited library is seen
/// the moment it's re-copied into the dist (rather than a stale
/// cached copy). The fetch is issued from the service-worker scope,
/// so it bypasses the SW's own `onfetch` handler per spec.
///
/// A missing or unreadable library is a deployment fault, not a
/// client fault: surfaced as an internal error so repository
/// creation fails loudly rather than seeding an empty repo.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_standard_library(url: &str) -> Result<String, TonkWorkerError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestCache, RequestInit, Response};

    let init = RequestInit::new();
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(url, &init)
        .map_err(|e| TonkWorkerError::Internal(format!("standard library request: {e:?}")))?;

    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service-worker scope".to_owned()))?;
    let response: Response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| TonkWorkerError::Internal(format!("fetch {url}: {e:?}")))?;
    if !response.ok() {
        return Err(TonkWorkerError::Internal(format!(
            "fetch {url} returned HTTP {}",
            response.status()
        )));
    }
    let text = JsFuture::from(
        response
            .text()
            .map_err(|e| TonkWorkerError::Internal(format!("library text(): {e:?}")))?,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("library body: {e:?}")))?;
    text.as_string()
        .ok_or_else(|| TonkWorkerError::Internal("library body is not a string".to_owned()))
}

/// Seed a notation document into `branch` by running it through the
/// evaluate pipeline — the same `parse → analyze → commit` path as
/// the `/evaluate` route, which commits concept claims and `rule!:`
/// installs alike. A bad library is a deployment fault, surfaced as
/// an internal error.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn seed_standard_library(
    tonk: &TonkState,
    repo: &str,
    branch: &str,
    library: &str,
) -> Result<(), TonkWorkerError> {
    super::evaluate::evaluate_body(tonk, repo, branch, library.to_owned(), true)
        .await
        .map(|_| ())
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "failed to seed standard library on branch '{branch}': {e}"
            ))
        })
}

/// Build out a repository from a [`RepositoryConfiguration`].
///
/// Runs the full create-side pipeline in a single pass:
///
/// 1. `profile.repository(name).create()` — allocate a new
///    signer-owned repository in dialog-db.
/// 2. Delegate repository access to the profile and save the
///    delegation, so future operations authenticated by the
///    profile can reach the repo.
/// 3. Open the `meta` branch and start a transaction, seeded
///    with the [`Replica`] concept and a [`TonkBranch`] for the
///    meta branch itself.
/// 4. For each configured remote: create it at the dialog layer
///    *and* assert the corresponding [`TonkRemote`] concept on
///    the transaction. Concepts are kept keyed by remote name
///    so the upstream-linking step can find them.
/// 5. For each configured branch: open it at the dialog layer
///    and assert a [`TonkBranch`]. If the config names an
///    upstream, wire it at the dialog layer and assert the
///    corresponding [`TrackingBranch`].
/// 6. Commit the meta transaction — one commit containing
///    every concept, so the metadata lands atomically.
///
/// Interleaving dialog mutations with meta assertions keeps
/// both sides in lockstep and means we never have to
/// "reconstruct what we just built" as a second pass.
///
/// Returns the opened [`Repository<SignerCredential>`] so the
/// caller can still introspect it (e.g. to build a response
/// body) without a separate load. The caller is responsible
/// for existence-checking before calling — this function
/// assumes the name is free.
pub async fn create_repository(
    tonk: &TonkState,
    name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<Repository<SignerCredential>, RepositoryError> {
    // 1. Create the repository. Any failure here is genuinely
    // internal — we assume the caller has already confirmed the
    // name is free.
    let repository = tonk
        .profile
        .repository(name)
        .create()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to create repository '{}': {}", name, e))
        })?;
    log!("Repository created. DID: {}", repository.did());

    // 2. Delegate repo access to the profile. A freshly created
    // repository always has a signer credential, so `.access()`
    // is available directly. Splitting this delegation from
    // creation would leave the repo half-initialised if the save
    // fails; commit it immediately so the caller sees a clean
    // success or a clean failure.
    let delegation = repository
        .access()
        .claim(&repository)
        .delegate(tonk.profile.did())
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to delegate repo access to profile: {}", e))
        })?;

    tonk.profile
        .access()
        .save(delegation)
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to save repo delegation: {}", e)))?;

    // 3-7. Wire up the meta branch and register the replica.
    record_repository_meta(tonk, &repository, name, configuration).await?;

    Ok(repository)
}

/// Lay down the meta-branch facts and profile-side index for an
/// already-opened repository.
///
/// Steps 3-7 of the original `create_repository` pipeline, lifted
/// into a helper so both the local-create path
/// ([`create_repository`]) and the invite-claim path can share
/// it. Generic over the credential type because the claim path
/// uses a verifier-only [`Credential`] (the local replica has the
/// invited subject's DID but no signing key — the operator/profile
/// authority signs commits, not the repo credential).
///
/// Caller is responsible for steps 1 and 2 (creating the
/// repository in dialog, and persisting any access delegation —
/// either via `repository.access().claim().delegate()` for self-
/// owned repos or via `profile.access().save(invite_chain)` for
/// invited replicas).
pub async fn record_repository_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    // 3. Open the meta branch and start the single transaction
    // that will carry every concept describing the repository.
    // Seed it with the replica record and the meta branch's own
    // `Branch` fact — the meta branch is a real branch of this
    // replica, so it belongs in the enumeration like any other.

    let meta = repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to open meta branch: {}", e)))?;

    // Local replica of this repository
    let replica = Replica::new(tonk.profile.did(), repository.did(), name);

    let mut transaction = meta
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(META_BRANCH));

    // 4. Create remotes at the dialog layer and assert their
    // concepts on the same transaction. Stash each created
    // `RemoteRepository` alongside its `Remote` concept so the
    // branch loop below can resolve upstream references without
    // a second `.load()` round-trip against dialog — we just
    // created these remotes, so the data we'd load is still in
    // hand.
    let mut remotes: HashMap<String, (RemoteRepository, Remote)> =
        HashMap::with_capacity(configuration.remote.len());

    for (remote_name, remote_config) in &configuration.remote {
        // Subject defaults to the local repo's DID — that's the
        // existing `RemoteConfiguration` convention (remote
        // repository subject == local subject unless explicitly
        // overridden).
        let subject = remote_config
            .subject
            .clone()
            .unwrap_or_else(|| repository.did());

        let mut create = repository
            .remote(remote_name.as_str())
            .create(remote_config.address.clone());

        if remote_config.subject.is_some() {
            create = create.subject(subject.clone());
        }
        let remote = create.perform(&tonk.operator).await.map_err(|e| {
            RepositoryError::Internal(format!("Failed to create remote '{}': {}", remote_name, e))
        })?;

        log!("Remote '{}' created", remote_name);

        let concept = replica.remote(remote_name.as_str(), subject, &remote_config.address);
        transaction = transaction.assert(concept.clone());
        remotes.insert(remote_name.clone(), (remote, concept));
    }

    // 5. Open each branch at the dialog layer and assert its
    // `TonkBranch` concept. If the branch names an upstream,
    // wire it through dialog and assert a `TrackingBranch` link
    // on the same transaction. An upstream that references an
    // unknown remote is a user-facing configuration error —
    // surface it as `InvalidConfiguration` (400), not Internal.
    for (branch_name, settings) in &configuration.branch {
        let branch = repository
            .branch(branch_name.as_str())
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                RepositoryError::Internal(format!("Failed to open branch '{}': {}", branch_name, e))
            })?;

        transaction = transaction.assert(replica.branch(branch_name.as_str()));

        if let Some(upstream) = &settings.upstream {
            // Look up the remote we just created in step 4
            // instead of doing another `.load()` round-trip
            // against dialog. If the upstream names a remote
            // that wasn't in the configuration, that's a
            // user-facing configuration error (400), not an
            // internal failure.
            let (remote, concept) = remotes.get(&upstream.remote).ok_or_else(|| {
                RepositoryError::InvalidConfiguration(format!(
                    "Upstream for branch '{}' references unknown remote '{}'",
                    branch_name, upstream.remote
                ))
            })?;

            let target = remote
                .branch(upstream.branch.as_str())
                .open()
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to open remote branch '{}/{}': {}",
                        upstream.remote, upstream.branch, e
                    ))
                })?;

            branch
                .set_upstream(&target)
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to set upstream for branch '{}': {}",
                        branch_name, e
                    ))
                })?;
            log!(
                "Upstream for branch '{}' set to {}/{}",
                branch_name,
                upstream.remote,
                upstream.branch
            );

            // Mirror the upstream wiring on the meta side.
            // Both halves of the link need to land on the meta
            // branch: the remote-side `Branch` concept
            // (otherwise the upstream pointer has no target to
            // resolve to on read) and the `TrackingBranch` that
            // connects them.
            let tracked = concept.branch(upstream.branch.as_str());
            transaction = transaction
                .assert(tracked.clone())
                .assert(replica.branch(branch_name.as_str()).set_upstream(&tracked));
        }
    }

    // 6. Commit the meta transaction. Everything above has
    // already happened at the dialog layer; committing here
    // makes the schema view of it land atomically.
    let revision = transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!(
                "Failed to commit meta for repository '{}': {}",
                name, e
            ))
        })?;
    log!("Wrote meta facts for repository '{}'", name);

    // Notify listeners of `/api/repository/{name}` that the repo's
    // representation changed. The broadcast mirrors the endpoint
    // the data is served from; UIs subscribed on that path pick up
    // the change without a reload. Fires after the commit so
    // listeners only see durable state.
    broadcast(
        &format!("/api/repository/{name}"),
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    // 7. Record this replica in the profile repository's meta
    // branch so the profile keeps an index of every replica it
    // owns. Separate transaction — cross-repo atomicity isn't
    // available. The per-repo meta is already durable; a failure
    // here leaves the repo working but missing from the index,
    // which is recoverable.
    record_replica_in_profile(tonk, name, &repository.did()).await?;

    Ok(())
}

/// Assert a [`Replica`] concept for a newly created repository in
/// the profile repository's meta branch.
///
/// The profile repository serves as an index of every replica the
/// profile owns; this function adds one entry to that index.
/// Idempotent at the concept layer — re-asserting the same
/// `(profile, subject)` replica is a no-op.
async fn record_replica_in_profile(
    tonk: &TonkState,
    name: &str,
    subject: &Did,
) -> Result<(), RepositoryError> {
    let replica = Replica::new(tonk.profile.did(), subject.clone(), name);
    // Stamp the replica `blank`: the content branch has not been seeded
    // yet (seeding runs asynchronously after this response). The Hub
    // renders this as an installing card; `set_replica_status` flips it
    // to `initialized` once the seed completes.
    let status = SpaceStatus::new(replica.this().clone(), Replica::blank_status());

    // Write through the *reactor's* profile-repository handle, not a
    // fresh `Repository::from(&tonk.profile)`. The reactor caches the
    // profile repo and its meta-branch handle (opened the first time
    // the Hub queried, at boot); a commit through a separate handle
    // leaves that cached handle pinned at its old head, so the Hub —
    // which reads through the reactor — never sees this replica. Going
    // through the reactor advances the cached handle and re-polls its
    // subscriptions, so the new space appears in the Hub immediately.
    let revision = tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
        .transaction()
        .assert(replica)
        .assert(status)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!(
                "Failed to record replica '{}' in profile meta: {}",
                name, e
            ))
        })?;
    log!("Recorded replica '{}' in profile meta", name);

    // The profile repo's representation — what `GET /api/profile`
    // returns — now includes this replica, so tell listeners of
    // `/api/profile`.
    broadcast(
        "/api/profile",
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    Ok(())
}

/// Flip a replica's seeding [`Status`] by stamping a [`SpaceStatus`]
/// on its entity. `status` is cardinality-one, so the new value
/// supersedes the prior one. Goes through the reactor (like
/// [`record_replica_in_profile`]) so the Hub's subscription re-polls
/// and the card reflects the change.
///
/// The replica entity is re-derived from `(profile, subject)` — the
/// same hash `Replica::new` uses — so no read is needed to find it.
///
/// Only called from the background seed path, which is SW-only.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn set_replica_status(
    tonk: &TonkState,
    subject: &Did,
    status: tonk_schema::domain::replica::Status,
) -> Result<(), RepositoryError> {
    let entity = Replica::new(tonk.profile.did(), subject.clone(), "")
        .this()
        .clone();
    let stamp = SpaceStatus::new(entity, status);

    let revision = tonk
        .reactor
        .profile_repository()
        .branch(META_BRANCH)
        .transaction()
        .assert(stamp)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to set replica status: {}", e)))?;

    broadcast(
        "/api/profile",
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    Ok(())
}

/// Bootstrap the profile repository's meta branch.
///
/// Called on every worker startup. Asserts the profile's "self"
/// replica record (profile DID == subject DID, labeled with the
/// profile's name) and a [`MetaBranch`] concept for the meta
/// branch itself.
///
/// A no-op when the profile has already been bootstrapped — both
/// assertions are content-addressed (entity hashes depend only on
/// `(profile, subject)` / `(replica, name)`), so re-asserting the
/// same facts produces the same entities and attribute values and
/// the dialog layer deduplicates.
pub async fn bootstrap_profile_meta(
    tonk: &TonkState,
    profile_name: &str,
) -> Result<(), RepositoryError> {
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did, profile_name);

    // Write through the reactor's profile handle so the cached branch
    // state (which every read also goes through) advances on this
    // commit — see `record_replica_in_profile` for why a separate
    // `Repository::from` handle would leave the reader stale.
    tonk.reactor
        .profile_repository()
        .branch(META_BRANCH)
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(META_BRANCH))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to bootstrap profile meta: {}", e))
        })?;
    log!("Profile meta bootstrapped");

    // Seed the standard library onto the profile meta branch so a
    // `<tonk-display>` reading the profile (the Hub at `/`) can resolve
    // the library's concepts and views — the `space` model and its
    // directory view — there, the same way a named repo's content
    // branch carries them. Idempotent: re-evaluating the library
    // de-duplicates rather than minting fresh claims, so it's safe on
    // every boot. Fetch is only available in the SW scope; native
    // builds skip it (the Hub is a browser-only surface).
    seed_profile_library(tonk).await?;

    Ok(())
}

/// Fetch and seed the lean profile library onto the profile meta
/// branch. SW-only — the fetch needs a service-worker scope.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn seed_profile_library(tonk: &TonkState) -> Result<(), RepositoryError> {
    let library = fetch_standard_library(PROFILE_LIBRARY_URL)
        .await
        .map_err(|e| RepositoryError::Internal(format!("fetch profile library: {e}")))?;
    super::evaluate::evaluate_profile_body(tonk, META_BRANCH, library, true)
        .await
        .map(|_| ())
        .map_err(|e| {
            RepositoryError::Internal(format!("seed standard library on profile meta: {e}"))
        })
}

/// Native stub — no service-worker scope to fetch the served library.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn seed_profile_library(_tonk: &TonkState) -> Result<(), RepositoryError> {
    Ok(())
}

/// Load a repository by name and return its [`RepositoryInfo`].
///
/// Handler for `GET /api/repository/{repo}`. 404s when the
/// repository can't be loaded.
#[wasm_compat]
pub async fn get_repository(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<RepositoryInfo>, TonkWorkerError> {
    log!("GET /api/repository/{}", name);

    let tonk = state.read().await;

    let repository = tonk
        .profile
        .repository(&name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", name, e))
        })?;

    let info = build_repository_info(&tonk, &name, &repository).await;
    Ok(Json(info))
}

/// Return [`RepositoryInfo`] for the profile-as-repository.
///
/// Handler for `GET /api/profile/repository`. The profile lives
/// outside the named-repo namespace, so it has its own route.
/// Mirrors the data the `info.profile` field of
/// `GET /api/profile` carries — exposed separately so the UI can
/// `.refetch()` just the profile-as-repository view after
/// branch-level operations without re-fetching the full profile
/// payload (with its replica list).
#[wasm_compat]
pub async fn get_profile_repository(
    State(state): State<AppState>,
) -> Result<Json<RepositoryInfo>, TonkWorkerError> {
    log!("GET /api/profile/repository");

    let tonk = state.read().await;
    let repository = tonk
        .reactor
        .profile_repository()
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to acquire profile repository: {e}"))
        })?
        .repository();
    let info = build_repository_info(&tonk, &tonk.profile_name, &repository).await;
    Ok(Json(info))
}

/// Construct [`RepositoryInfo`] for an open repository by
/// reading the schema concepts off its `meta` branch.
///
/// The meta branch is the source of truth for which branches and
/// remotes belong to the repository. Opening the repository's
/// meta branch, running three queries, and joining the results
/// gives the full picture without having to probe individual
/// dialog-repository objects.
///
/// What each query finds:
///
/// - **Branches (all)** — every `Branch` concept on the meta
///   branch, local *and* remote-side. Grouped by `origin`:
///   origin == replica means local; origin == remote means
///   remote-side (used later to resolve upstream references to
///   a `(remote_name, branch_name)` pair).
/// - **Remotes (on replica)** — `Remote` concepts scoped to
///   this replica.
/// - **Tracking branches (on replica)** — `TrackingBranch`
///   concepts that link local branches to their upstream remote
///   branches.
///
/// Revisions still come from the dialog layer: for each local
/// branch, we open it and read `.revision()`. That's a handful
/// of sequential I/O calls but they're quick and the data
/// doesn't live in meta.
///
/// Repositories that predate the meta-branch writes show up as
/// empty here (no branches or remotes). That's fine — the
/// `subject` / `operator` / `profile` fields still surface, and
/// the UI can tell the repo is unpopulated.
pub(super) async fn build_repository_info<R>(
    tonk: &TonkState,
    name: &str,
    repository: &Repository<R>,
) -> RepositoryInfo
where
    R: Principal + Clone,
{
    let meta = match repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
    {
        Ok(meta) => meta,
        Err(e) => {
            log!("No meta branch for repository '{}': {}", name, e);
            return RepositoryInfo {
                name: name.to_string(),
                subject: repository.did(),
                operator: tonk.operator.did(),
                profile: tonk.profile.did(),
                branch: HashMap::new(),
                remote: HashMap::new(),
            };
        }
    };

    // Derive the replica entity the way `create_repository`
    // did — `(profile, subject)` hashed into an entity. We
    // don't query for `Replica` itself; nothing we return
    // depends on its name or attributes, and filtering
    // branches/remotes by `origin == replica.this()` is all we
    // need.
    let replica = Replica::new(tonk.profile.did(), repository.did(), name);
    let replica_entity = replica.this().clone();

    // Pull every branch on the meta branch, local and remote.
    // Keyed by entity so the upstream-resolution step can look
    // up any branch by its hash.
    let all_branches: Vec<MetaBranch> = match meta
        .query()
        .select(Query::<MetaBranch> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::var("origin"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log!("Branch query on meta failed for '{}': {:?}", name, e);
            Vec::new()
        }
    };
    let branches_by_entity: HashMap<_, _> = all_branches
        .iter()
        .map(|b| (b.this.clone(), b.clone()))
        .collect();

    // Pull remotes on this replica. Keyed by entity for the
    // same reason as branches — a tracking branch's upstream
    // points at a remote-side `Branch`, whose `origin` is a
    // `Remote.this`, and we want to go from that entity back to
    // the remote's name.
    let remote_concepts: Vec<Remote> = match meta
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(replica_entity.clone()),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log!("Remote query on meta failed for '{}': {:?}", name, e);
            Vec::new()
        }
    };
    let remotes_by_entity: HashMap<_, _> = remote_concepts
        .iter()
        .map(|r| (r.this.clone(), r.clone()))
        .collect();

    // Pull every tracking link on this replica. Keyed by the
    // local branch's entity so the branch-assembly step below
    // can find "does this branch track something?" in O(1).
    let tracking: Vec<TrackingBranch> = match meta
        .query()
        .select(Query::<TrackingBranch> {
            this: Term::var("this"),
            upstream: Term::var("upstream"),
            origin: Term::from(replica_entity.clone()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log!(
                "Tracking-branch query on meta failed for '{}': {:?}",
                name,
                e
            );
            Vec::new()
        }
    };
    let tracking_by_local: HashMap<_, _> = tracking
        .into_iter()
        .map(|t| (t.this.clone(), t.upstream))
        .collect();

    // Assemble the branch map. Iterate local branches only
    // (those whose origin is the replica), skipping any entity
    // that is also a `Remote` — `Query<Branch>` matches on the
    // `origin` + `name` attribute pair, which `Remote` shares
    // (`Remote` has the same pair plus `subject` + `address`),
    // so remote entities turn up as spurious branch hits. For
    // each real local branch, resolve its upstream (if any) by
    // looking up the tracked `Branch` entity, then the remote
    // that branch belongs to.
    let mut branches = HashMap::new();
    for branch in &all_branches {
        if branch.origin.0 != replica_entity {
            continue;
        }
        if remotes_by_entity.contains_key(&branch.this) {
            continue;
        }
        let upstream = tracking_by_local.get(&branch.this).and_then(|upstream| {
            let tracked_branch = branches_by_entity.get(&upstream.0)?;
            let remote = remotes_by_entity.get(&tracked_branch.origin.0)?;
            Some(UpstreamConfiguration::new(
                remote.name.0.clone(),
                tracked_branch.name.0.clone(),
            ))
        });

        let revision = match repository
            .branch(branch.name.0.as_str())
            .open()
            .perform(&tonk.operator)
            .await
        {
            Ok(opened) => opened.revision(),
            Err(e) => {
                log!(
                    "Failed to open branch '{}' of '{}' for revision: {}",
                    branch.name.0,
                    name,
                    e
                );
                None
            }
        };

        branches.insert(
            branch.name.0.clone(),
            BranchConfiguration { upstream, revision },
        );
    }

    // Assemble the remote map. Every remote concept scoped to
    // this replica becomes a `RemoteConfiguration`. The address
    // field comes back decoded from its dag-cbor bytes. The
    // `subject` field stays `None` when no subject override was
    // recorded — see `RemoteConfiguration.subject`'s "`None`
    // means same as local repo" convention.
    let mut remotes = HashMap::new();
    for remote in &remote_concepts {
        let address = match remote.address.decode() {
            Ok(address) => address,
            Err(e) => {
                log!(
                    "Failed to decode address for remote '{}' of '{}': {:?}",
                    remote.name.0,
                    name,
                    e
                );
                continue;
            }
        };
        // Emit `subject` only when it differs from the local
        // repo's own DID; matches the write-side convention
        // (see `RemoteConfiguration.subject`). If the stored
        // value isn't a parseable `Did` for some reason we
        // drop the field rather than fail the whole response.
        let subject = match remote.subject.0.to_string().parse::<Did>() {
            Ok(did) if did != repository.did() => Some(did),
            _ => None,
        };
        remotes.insert(
            remote.name.0.clone(),
            RemoteConfiguration { address, subject },
        );
    }

    RepositoryInfo {
        name: name.to_string(),
        subject: repository.did(),
        operator: tonk.operator.did(),
        profile: tonk.profile.did(),
        branch: branches,
        remote: remotes,
    }
}

/// Idempotently ensure an existing repository carries the remotes and
/// branch upstreams named in `configuration`.
///
/// The dialog-layer mutations are probed before they run — a remote
/// is created only when [`load`](dialog_repository) reports it
/// missing, and an upstream is set only when the branch isn't already
/// tracking it — because `create` errors on a duplicate remote and
/// `set_upstream` would otherwise reset the branch's sync divergence
/// base. The meta-branch concept assertions are content-addressed, so
/// they're re-asserted unconditionally (a no-op when already present).
///
/// Generic over the credential type for the same reason as
/// [`record_repository_meta`]: the operator/profile authority signs
/// the commits, not the repository credential.
async fn ensure_remote_config<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    if configuration.remote.is_empty() && configuration.branch.is_empty() {
        return Ok(());
    }

    let meta = repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to open meta branch: {}", e)))?;

    let replica = Replica::new(tonk.profile.did(), repository.did(), name);
    let mut transaction = meta.transaction().assert(replica.clone());

    // Ensure each configured remote exists at the dialog layer, then
    // mirror it on the meta branch. A remote that already exists is
    // loaded rather than recreated — `create` errors on a duplicate.
    let mut remotes: HashMap<String, Remote> = HashMap::with_capacity(configuration.remote.len());
    for (remote_name, remote_config) in &configuration.remote {
        let subject = remote_config
            .subject
            .clone()
            .unwrap_or_else(|| repository.did());

        match repository
            .remote(remote_name.as_str())
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(_) => log!("Remote '{}' already present; left as-is", remote_name),
            Err(_) => {
                let mut create = repository
                    .remote(remote_name.as_str())
                    .create(remote_config.address.clone());
                if remote_config.subject.is_some() {
                    create = create.subject(subject.clone());
                }
                create.perform(&tonk.operator).await.map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to create remote '{}': {}",
                        remote_name, e
                    ))
                })?;
                log!("Remote '{}' created", remote_name);
            }
        }

        let concept = replica.remote(remote_name.as_str(), subject, &remote_config.address);
        transaction = transaction.assert(concept.clone());
        remotes.insert(remote_name.clone(), concept);
    }

    // Wire each configured branch's upstream. The branch is opened
    // (created on first open if absent), its upstream set only when it
    // isn't already tracking the requested remote branch, and the
    // tracking link mirrored on the meta branch.
    for (branch_name, settings) in &configuration.branch {
        let Some(upstream) = &settings.upstream else {
            continue;
        };

        let branch = repository
            .branch(branch_name.as_str())
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                RepositoryError::Internal(format!("Failed to open branch '{}': {}", branch_name, e))
            })?;

        // The upstream's remote must be one named in this request —
        // mirrors the create path, where an upstream can only
        // reference a remote in the same configuration.
        let concept = remotes.get(&upstream.remote).ok_or_else(|| {
            RepositoryError::InvalidConfiguration(format!(
                "Upstream for branch '{}' references remote '{}', which is not in the request",
                branch_name, upstream.remote
            ))
        })?;

        let already_tracking = matches!(
            branch.upstream(),
            Some(Upstream::Remote { ref remote, branch: ref tracked, .. })
                if *remote == upstream.remote && *tracked == upstream.branch
        );

        if already_tracking {
            log!(
                "Branch '{}' already tracks {}/{}; left as-is",
                branch_name,
                upstream.remote,
                upstream.branch
            );
        } else {
            let remote = repository
                .remote(upstream.remote.as_str())
                .load()
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to load remote '{}' for upstream: {}",
                        upstream.remote, e
                    ))
                })?;
            let target = remote
                .branch(upstream.branch.as_str())
                .open()
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to open remote branch '{}/{}': {}",
                        upstream.remote, upstream.branch, e
                    ))
                })?;
            branch
                .set_upstream(&target)
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to set upstream for branch '{}': {}",
                        branch_name, e
                    ))
                })?;
            log!(
                "Branch '{}' now tracks {}/{}",
                branch_name,
                upstream.remote,
                upstream.branch
            );
        }

        // Mirror the upstream on the meta branch (idempotent): the
        // local branch, the remote-side tracked branch, and the
        // tracking link between them.
        let tracked = concept.branch(upstream.branch.as_str());
        transaction = transaction
            .assert(replica.branch(branch_name.as_str()))
            .assert(tracked.clone())
            .assert(replica.branch(branch_name.as_str()).set_upstream(&tracked));
    }

    let revision = transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!(
                "Failed to commit meta for repository '{}': {}",
                name, e
            ))
        })?;

    // Mirror the create path: tell listeners of the repository's
    // representation that its remotes/branches changed.
    broadcast(
        &format!("/api/repository/{name}"),
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    Ok(())
}

/// Attach remotes (and branch upstreams) to an **existing**
/// repository — the opt-in counterpart to wiring a remote at create
/// time.
///
/// `POST /api/repository/{repo}/remote`. The body is a
/// [`RepositoryConfiguration`] — the same shape `PUT` accepts — so a
/// caller advertises the remote and the branch that tracks it exactly
/// as it would at creation:
///
/// ```json
/// { "remote": { "origin": { "address": … } },
///   "branch": { "main": { "upstream": { "remote": "origin", "branch": "main" } } } }
/// ```
///
/// Idempotent: a remote that already exists keeps its address and
/// subject (it is not recreated), and a branch already tracking the
/// requested upstream is left untouched (so its sync divergence base
/// isn't reset). Calling twice is a safe no-op.
///
/// Why this is opt-in rather than baked into `create_space`: the
/// access-service remote is useful for exercising the sync/invite
/// loop now, but production provisions sync differently. Keeping the
/// attach an explicit, isolated action means prod swaps this one call
/// instead of unpicking it from the create path, and a freshly
/// created repo stays local until something explicitly gives it a
/// remote.
#[wasm_compat]
pub async fn attach_remote(
    State(state): State<AppState>,
    Path(name): Path<String>,
    body_bytes: Bytes,
) -> Result<Json<RepositoryInfo>, TonkWorkerError> {
    log!("POST /api/repository/{}/remote", name);

    let configuration: RepositoryConfiguration = if body_bytes.is_empty() {
        RepositoryConfiguration::default()
    } else {
        serde_json::from_slice(&body_bytes)
            .map_err(|e| TonkWorkerError::Router(format!("Invalid request body: {e}")))?
    };

    let tonk = state.write().await;

    let repository = tonk
        .profile
        .repository(&name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", name, e))
        })?;

    ensure_remote_config(&tonk, &repository, &name, &configuration).await?;

    let info = build_repository_info(&tonk, &name, &repository).await;
    Ok(Json(info))
}

/// Seed-split regression tests: the scaffold (`core.yaml`) makes a
/// repository renderable but seeds zero instances, and the showcase
/// (`demo.yaml`) layers the demo content on top, resolving its bare
/// concept references against the committed scaffold.
///
/// These embed the real assets via `include_str!` and seed them
/// through [`evaluate_body`] — the same `parse → analyze → commit`
/// path the worker runs at creation, minus the served-asset fetch
/// (unavailable in the wasm test scope, which is why
/// [`fetch_standard_library`] is bypassed here).
///
/// wasm32-only — `evaluate_body` and the worker test `TonkState` are
/// built from the service-worker harness.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use axum::Router;
    use dialog_remote_ucan_s3::UcanAddress;
    use dialog_repository::SiteAddress;

    use super::{
        BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    };
    use crate::router::evaluate::evaluate_body;
    use crate::router::{AppState, CreateInviteResponse, api_router_with_state, tests::test_state};

    /// The scaffold and showcase notation, embedded at compile time.
    const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");
    const DEMO: &str = include_str!("../../../tonk-core/assets/library/demo.yaml");

    /// Create a fresh repo and return its router + wrapped state. PUTs
    /// a branchless `{}` so the worker seeds nothing — the test drives
    /// seeding / attaching itself. The `main` branch is created on
    /// first write. Tolerates `412` from IndexedDB state surviving a
    /// prior run in the same browser session.
    async fn fresh_repo(repo: &str) -> (Router, AppState) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("if-none-match", "*")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::CREATED || status == StatusCode::PRECONDITION_FAILED,
            "expected 201 or 412 from PUT /api/repository/{repo}, got {status}",
        );
        (app, state)
    }

    /// Seed a notation document into the repo's `main` branch.
    async fn seed(state: &AppState, repo: &str, document: &str) {
        let guard = state.read().await;
        evaluate_body(&guard, repo, "main", document.to_owned(), true)
            .await
            .unwrap_or_else(|e| panic!("seed failed: {e}"));
    }

    /// Run a query document and return the number of result rows in
    /// its single match block (zero if the query matched nothing).
    async fn count(state: &AppState, repo: &str, query: &str) -> usize {
        let guard = state.read().await;
        let response = evaluate_body(&guard, repo, "main", query.to_owned(), false)
            .await
            .unwrap_or_else(|e| panic!("query failed: {e}"));
        response
            .matches_after
            .first()
            .map(|block| block.results.len())
            .unwrap_or(0)
    }

    /// The scaffold alone defines the `workspace` concept — so the
    /// directory route can resolve it — but seeds zero `workspace`
    /// instances. That empty render is the precondition for the
    /// launchpad.
    #[dialog_common::test]
    async fn it_seeds_scaffold_without_showcase_instances() {
        let repo = "test-seed-scaffold-only";
        let (_app, state) = fresh_repo(repo).await;
        seed(&state, repo, CORE).await;

        assert_eq!(
            count(&state, repo, "workspace:\n").await,
            0,
            "scaffold-only repo must have zero workspace instances",
        );
    }

    /// The showcase, seeded on top of the scaffold, resolves its bare
    /// concept references (`workspace`, `person`, …) against the
    /// committed scaffold and lands the demo instances — what the
    /// default `home` repo gets. Guards the cross-document resolution
    /// the split depends on.
    #[dialog_common::test]
    async fn it_seeds_showcase_on_top_of_scaffold() {
        let repo = "test-seed-showcase";
        let (_app, state) = fresh_repo(repo).await;
        seed(&state, repo, CORE).await;
        seed(&state, repo, DEMO).await;

        assert!(
            count(&state, repo, "workspace:\n").await >= 1,
            "showcase must seed at least one workspace instance",
        );
        assert_eq!(
            count(&state, repo, "person:\n").await,
            2,
            "showcase must seed the Alice and Bob person instances",
        );
    }

    /// A `RepositoryConfiguration` that attaches an `origin` remote at
    /// `endpoint` and points `main` at `origin/main` — the shape the
    /// launchpad sends to make a `create_space` repo sync-capable.
    fn origin_config(endpoint: &str) -> RepositoryConfiguration {
        let address = SiteAddress::from(UcanAddress::new(endpoint));
        RepositoryConfiguration::default()
            .remote("origin", RemoteConfiguration::new(address))
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            )
    }

    /// POST a remote-attach config to `repo` and decode the resulting
    /// `RepositoryInfo`.
    async fn attach(app: &Router, repo: &str, config: &RepositoryConfiguration) -> RepositoryInfo {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "attach should return 200"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("decode RepositoryInfo: {e}"))
    }

    /// Attaching the access-service remote to an existing, remote-less
    /// repo wires `origin` and points `main` at `origin/main`.
    #[dialog_common::test]
    async fn it_attaches_a_remote_and_tracks_main() {
        let repo = "test-attach-remote";
        let (app, _state) = fresh_repo(repo).await;

        let info = attach(&app, repo, &origin_config("https://example.test/ucan/")).await;

        assert!(
            info.remote.contains_key("origin"),
            "attach must register the origin remote; got {:?}",
            info.remote.keys().collect::<Vec<_>>(),
        );
        let main = info
            .branch
            .get("main")
            .expect("attach must surface the main branch");
        let upstream = main
            .upstream
            .as_ref()
            .expect("main must have an upstream after attach");
        assert_eq!(upstream.remote, "origin");
        assert_eq!(upstream.branch, "main");
    }

    /// Attach is idempotent: a second call on an already-wired repo
    /// succeeds and leaves a single `origin` still tracking
    /// `origin/main` (no duplicate-remote error, no reset).
    #[dialog_common::test]
    async fn it_attaches_remote_idempotently() {
        let repo = "test-attach-remote-idempotent";
        let (app, _state) = fresh_repo(repo).await;
        let config = origin_config("https://example.test/ucan/");

        attach(&app, repo, &config).await;
        let info = attach(&app, repo, &config).await;

        assert!(info.remote.contains_key("origin"));
        let upstream = info
            .branch
            .get("main")
            .and_then(|b| b.upstream.as_ref())
            .expect("main must still track an upstream after a second attach");
        assert_eq!(upstream.remote, "origin");
        assert_eq!(upstream.branch, "main");
    }

    /// After attach, a minted invite carries the `remote=` endpoint —
    /// the whole point of the opt-in remote, so `slide join` has
    /// something to pull from. Before attach the repo is remote-less
    /// and the invite carries no remote.
    #[dialog_common::test]
    async fn it_mints_an_invite_with_a_remote_after_attach() {
        let repo = "test-attach-then-invite";
        let (app, _state) = fresh_repo(repo).await;

        attach(&app, repo, &origin_config("https://example.test/ucan/")).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/invite"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "invite mint should succeed"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let invite: CreateInviteResponse =
            serde_json::from_slice(&body).unwrap_or_else(|e| panic!("decode invite: {e}"));

        let has_remote = invite.url().query_pairs().any(|(key, _)| key == "remote");
        assert!(
            has_remote,
            "invite minted after attach must embed a remote= param; url was {}",
            invite.url(),
        );
    }
}

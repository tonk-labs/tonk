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
use dialog_repository::{
    RemoteRepository, Repository, RepositoryExt as _, Revision, SiteAddress, Upstream,
};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{Remote, Replica};

use super::AppState;
use crate::{RepositoryError, TonkWorkerError, worker::TonkState};

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
    // The helper doesn't know about HTTP; its errors are mapped
    // to the right status via `RepositoryError::into`.
    let repository = create_repository(&tonk, &name, &configuration).await?;

    // 3. Respond with the current state of the repository.
    let info = build_repository_info(&tonk, &name, &repository).await;
    Ok((StatusCode::CREATED, Json(info)))
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
            transaction = transaction.assert(
                replica
                    .branch(branch_name.as_str())
                    .set_upstream(&concept.branch(upstream.branch.as_str())),
            );
        }
    }

    // 6. Commit the meta transaction. Everything above has
    // already happened at the dialog layer; committing here
    // makes the schema view of it land atomically.
    transaction
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

    Ok(repository)
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

/// Construct [`RepositoryInfo`] for an open repository.
///
/// Probes `main` for its upstream and revision. If `main.upstream`
/// points at a remote, loads that remote and includes its address
/// in the `remote` map. Other branches / other remotes are *not*
/// surfaced today — those need the meta-branch schema to enumerate.
async fn build_repository_info<R>(
    tonk: &crate::worker::TonkState,
    name: &str,
    repository: &dialog_repository::Repository<R>,
) -> RepositoryInfo
where
    R: dialog_varsig::Principal + Clone,
{
    let mut branches = HashMap::new();
    let mut remotes = HashMap::new();

    if let Ok(main) = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
    {
        // Only remote upstreams can be represented as
        // `UpstreamConfiguration` today (it always names a remote).
        // Local upstreams are reported as "no upstream" here; they
        // need a richer response shape if/when we start using them.
        let upstream = match main.upstream() {
            Some(Upstream::Remote { remote, branch, .. }) => {
                Some(UpstreamConfiguration::new(remote, branch))
            }
            _ => None,
        };

        // If main's upstream is remote, populate the remote map.
        if let Some(Upstream::Remote { remote, .. }) = main.upstream()
            && let Ok(remote_repo) = repository
                .remote(remote.as_str())
                .load()
                .perform(&tonk.operator)
                .await
        {
            let remote_addr = remote_repo.address();
            let repo_did_str = repository.did().to_string();
            let remote_subject = remote_addr.subject().clone();
            let subject = (remote_subject.as_str() != repo_did_str).then_some(remote_subject);
            remotes.insert(
                remote.to_string(),
                RemoteConfiguration {
                    address: remote_addr.site().clone(),
                    subject,
                },
            );
        }

        branches.insert(
            "main".to_string(),
            BranchConfiguration {
                upstream,
                revision: main.revision(),
            },
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

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
use dialog_repository::{RepositoryExt as _, Revision, SiteAddress, Upstream};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use super::home;
use crate::TonkWorkerError;

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

    // 2. Create the repository. Any failure here is genuinely
    // internal — we already confirmed it doesn't exist.
    let repository = tonk
        .profile
        .repository(&name)
        .create()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to create repository '{}': {}", name, e))
        })?;
    log!("Repository created. DID: {}", repository.did());

    // 3. Delegate repo access to the profile. A freshly created
    // repository always has a signer credential, so `.access()` is
    // available directly. This used to live in
    // `TonkServiceWorker::new`; it belongs with creation, and if
    // either half of it fails we fail the whole request — a repo
    // without a working delegation chain is half-initialised and
    // the caller needs to know.
    let delegation = repository
        .access()
        .claim(&repository)
        .delegate(tonk.profile.did())
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to delegate repo access to profile: {}", e))
        })?;

    tonk.profile
        .access()
        .save(delegation)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to save repo delegation: {}", e)))?;

    // 4. Create any remotes listed in the body.
    for (name, remote) in &configuration.remote {
        let mut create = repository
            .remote(name.as_str())
            .create(remote.address.clone());

        // If subject is specified, it means remote repository subject is
        // different from the local one so we need to set it explicitly.
        if let Some(subject) = remote.subject.clone() {
            create = create.subject(subject);
        }

        create.perform(&tonk.operator).await.map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to create remote '{}': {}", name, e))
        })?;
        log!("Remote '{}' created", name);
    }

    // 5. Open each branch listed in the body (opens-or-creates) and
    // optionally wire its upstream. Missing from the body = don't
    // create; present with `{}` = create without upstream.
    for (name, settings) in &configuration.branch {
        let branch = repository
            .branch(name.as_str())
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", name, e))
            })?;

        if let Some(upstream) = &settings.upstream {
            let remote = repository
                .remote(upstream.remote.as_str())
                .load()
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Router(format!(
                        "Upstream references unknown remote '{}': {}",
                        upstream.remote, e
                    ))
                })?;

            let target = remote
                .branch(upstream.branch.as_str())
                .open()
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!(
                        "Failed to open remote branch '{}/{}': {}",
                        upstream.remote, upstream.branch, e
                    ))
                })?;

            branch
                .set_upstream(&target)
                .perform(&tonk.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!(
                        "Failed to set upstream for branch '{}': {}",
                        name, e
                    ))
                })?;
            log!(
                "Upstream for branch '{}' set to {}/{}",
                name,
                upstream.remote,
                upstream.branch
            );
        }
    }

    // 6. Register the new repo in home (the profile's meta-index).
    // Self-registration of home works because by this point home
    // exists locally — we just created it.
    home::register_repo(&tonk, &name).await?;

    // 7. Respond with the current state of the repository.
    let info = build_repository_info(&tonk, &name, &repository).await;
    Ok((StatusCode::CREATED, Json(info)))
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
pub(super) async fn build_repository_info<R>(
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
        subject: repository
            .did()
            .to_string()
            .parse()
            .expect("repository DID is always a valid Did"),
        operator: tonk.operator.did(),
        profile: tonk.profile.did(),
        branch: branches,
        remote: remotes,
    }
}

//! Repository create route.
//!
//! `PUT /api/repository/{repo}` always creates a fresh repository. The
//! repository's identity is its credential's `did:key`; the `{repo}`
//! path segment is only a display label. Every create mints a new
//! identity, so there is never a create-time collision — two spaces may
//! share a label. The response carries the new repository's routing key
//! (the DID suffix), which the UI routes by.

use std::collections::HashMap;

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use axum_wasm_macros::wasm_compat;
use dialog_credentials::{Ed25519Signer, SignerCredential};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{
    RemoteRepository, Repository, RepositoryExt as _, Revision, SiteAddress, Upstream,
};
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{
    Branch as MetaBranch, Remote, Replica, RepositoryName, SpaceStatus, TrackingBranch,
};

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
    /// The repository's routing key (the DID suffix it's addressable
    /// at). The URL segment routes resolve through this; identity, not
    /// label.
    pub name: String,
    /// The user-typed display label, read from the repository's own
    /// `tonk/repository` name on its content branch (the cross-device
    /// source of truth). Distinct from `name`: two spaces may share a
    /// label, but each has a unique routing key.
    pub label: String,
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
/// - Always creates a fresh repository with a freshly minted identity.
///   The `{repo}` path segment is the display label; the repository's
///   routing key is its credential's DID suffix. There is no create-time
///   collision — two spaces may share a label.
/// - On success, delegates repository access to the current profile,
///   sets up any remotes from the body, creates each listed branch,
///   and wires up upstream tracking when specified.
/// - Returns `201 Created` with a [`RepositoryInfo`] body whose `name`
///   is the new routing key.
#[wasm_compat]
pub async fn put_repository(
    State(state): State<AppState>,
    Path(display_name): Path<String>,
    _headers: HeaderMap,
    body_bytes: Bytes,
) -> Result<(StatusCode, Json<RepositoryInfo>), TonkWorkerError> {
    log!("PUT /api/repository/{}", display_name);

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

    // Create the repository and everything that comes with it —
    // delegation, remotes, branches, upstreams, meta facts. This
    // records the replica in the profile with `status: blank` (see
    // `record_replica_in_profile`), so the Hub card appears in its
    // installing state right away. The display label is seeded into the
    // repository's own `tonk/repository` concept; the routing key is the
    // new repository's DID suffix, derived from the returned handle.
    let repository = create_repository(&tonk, &display_name, &configuration).await?;
    let subject = repository.did();
    let key = subject.repo_key().to_owned();
    let info = build_repository_info(&tonk, &key, &repository).await;

    // Seed asynchronously, then flip the replica to `initialized`.
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
    spawn_seed(state, display_name, key, subject, branches);

    Ok((StatusCode::CREATED, Json(info)))
}

/// The form-event attribute carrying the optional sync URL — the
/// `remote` input on the `space/create` and `space/enable-sync` forms.
/// Kept in sync with those notation commands' `remote` field `the:`.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const REMOTE_ATTR: &str = "dom.event.current-target.elements.remote/value";

/// Read the optional remote URL from a transient's facts, tolerating
/// both `Value::String` and `Value::Entity`.
///
/// A URL like `http://host/ucan/` round-trips through JSON, and the
/// worker's untagged `Value` deserialization picks `Entity` for any
/// string containing a `:` — so a `String`-typed concept field never
/// decodes a URL (that's the bug a `remote: String` field hit). Reading
/// the artifact directly sidesteps the concept decode and accepts either
/// representation. Empty/whitespace → `None` (a local-only space).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn remote_from_facts(facts: &crate::reactor::EntityFacts) -> Option<String> {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| artifact.the.to_string() == REMOTE_ATTR)
        .and_then(|artifact| match &artifact.is {
            Value::String(url) => Some(url.clone()),
            Value::Entity(uri) => Some(uri.to_string()),
            _ => None,
        })
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Command handler for the "New space" form (`space/create`) and the
/// topbar's "Enable sync" form (`space/enable-sync`).
///
/// `CreateSpace` is matched **name-only** so it keeps decoding against an
/// older, frozen profile descriptor (see [`CreateSpace`]). The optional
/// sync URL is read straight from the transient's facts by
/// [`remote_from_facts`] — not as a concept field, both because a
/// required field would break the frozen-descriptor match and because a
/// URL deserializes as `Value::Entity`, which a `String` field can't
/// decode.
///
/// The repository is **always created** with a freshly minted identity
/// (`create_space_inner` returns its routing key), then, if a remote was
/// given, attached best-effort via [`enable_sync_inner`] to that key. So
/// the same handler serves both forms: the Hub "New space" form and the
/// topbar "Enable sync" form — both post the same `name`(+`remote`)
/// shape, and the handler keys on the shared `name` attribute. The
/// user-typed `name` is only a display label; two spaces may share it. A
/// remote/auth failure leaves a working local space, retryable from the
/// topbar.
///
/// A custom handler (not a plain `Provider<CreateSpace>`) is required
/// because the provider only receives the decoded command, never the
/// facts the remote must be read from.
///
/// [`CreateSpace`]: tonk_schema::command::CreateSpace
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct CreateSpaceHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl CreateSpaceHandler {
    /// Cache `CreateSpace`'s trigger attributes (its `name` field) so the
    /// registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::CreateSpace::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for CreateSpaceHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::CreateSpace::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock), then
        // hand owned values + an env clone to the `'static` future.
        let name = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::CreateSpace::decode(entity, facts))
            .map(|command| command.name.0);
        // The optional remote is read from the facts directly (tolerating
        // the URL's `Value::Entity` representation), not via a concept.
        let remote = remote_from_facts(facts);
        let env = env.clone();

        Box::pin(async move {
            let Some(name) = name else {
                return;
            };
            log!("command CreateSpace name={} remote={:?}", name, remote);

            // 1. Always create local-only first, so the space appears
            //    whether or not a remote was given (and never vanishes on
            //    a remote failure). The create mints a fresh identity and
            //    returns its routing key.
            let key = match create_space_inner(env.state(), &name).await {
                Ok(key) => key,
                Err(error) => {
                    log!("CreateSpace '{}' failed: {}", name, error);
                    return;
                }
            };

            // 2. If the form carried a remote, attach it best-effort to
            //    the identity just created. A failure here just leaves it
            //    local-only — retryable from the topbar's Enable sync.
            //    (`remote_from_facts` already dropped empty/blank URLs.)
            if let Some(remote) = remote
                && let Err(error) = enable_sync_inner(env.state(), &key, &remote).await
            {
                log!("CreateSpace '{}': remote attach failed: {}", key, error);
            }
        })
    }
}

/// Post-commit handler for the [`Invite`] command.
///
/// When the share form's `<form onsubmit=tonk:invite>` commits a
/// transient [`Invite`], this handler generates a fresh membership
/// keypair, delegates the *origin* repository's access to its DID,
/// base58-encodes the resulting delegation chain, and asserts a durable
/// [`Authorization`] fact keyed by that DID on the repository's content
/// branch (`main`). It then asserts the private seed as a [`Credential`]
/// into the reactor's session overlay (never replicated). The share view
/// joins the two via `tonk:invitation` and assembles the final URL.
///
/// The repository is not a command field: it is read from
/// [`CommandEnv::origin`](crate::router::CommandEnv::origin) (the branch
/// the commit landed in), so the form needs no `data-subject` stamp.
///
/// A custom handler (not a plain `Provider<Invite>`) is required because
/// it reads durable repository state the decoded command alone does not
/// carry, writes to the reactor's session overlay, and targets the repo
/// from the origin rather than a command field.
///
/// [`Invite`]: tonk_schema::command::Invite
/// [`Authorization`]: tonk_schema::command::Authorization
/// [`Credential`]: tonk_schema::command::Credential
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct InviteHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl InviteHandler {
    /// Cache `Invite`'s trigger attributes (its `time` field) so the
    /// registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::Invite::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for InviteHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::Invite::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock) only to
        // confirm this is an `Invite` command — it carries no payload the
        // handler needs (the repo comes from the origin, the keypair is
        // minted here).
        let is_invite = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::Invite::decode(entity, facts))
            .is_some();
        let env = env.clone();

        Box::pin(async move {
            if !is_invite {
                return;
            }
            // The repository to delegate is read from the origin — the
            // branch the commit landed in — not from a command field.
            let repo_name = env.origin().repo.clone();
            log!("command Invite repo={}", repo_name);

            if let Err(error) = run_invite(&env, &repo_name).await {
                log!("Invite for repo '{}' failed: {}", repo_name, error);
            }
        })
    }
}

/// Generate a membership keypair, delegate `repo_name`'s access to it,
/// assert the public [`Authorization`] on the content branch, and assert
/// the private seed as a [`Credential`] into the reactor's session
/// overlay (so it stays out of replicated storage).
///
/// Split out from [`InviteHandler::run`] so the `?` early-return funnels
/// into the single `log!` there — the command future itself returns `()`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_invite(
    env: &crate::router::CommandEnv,
    repo_name: &str,
) -> Result<(), TonkWorkerError> {
    use dialog_artifacts::Entity;
    use dialog_varsig::Principal as _;
    use tonk_schema::command::{Authorization, Credential};
    use tonk_schema::domain::authorization::{Proof, Remote as AuthorizationRemote};
    use tonk_schema::domain::credential::Seed;

    // Mint a fresh membership keypair. Its private seed becomes the
    // invite URL's `#` fragment; its public DID is the audience the repo
    // access is delegated to. The browser never sees this DID.
    let (signer, seed_bytes) = super::create_invite::generate_ephemeral().await?;
    let membership_did = signer.did();
    let seed = bs58::encode(seed_bytes).into_string();

    let tonk = env.state().read().await;

    let repository = tonk
        .profile
        .repository(repo_name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{repo_name}' not found: {e}"))
        })?;

    // Both facts are keyed by the repository's *subject* DID — the entity
    // the share view already addresses (`entity={subject}`) — not the
    // membership DID. So the membership DID is only the delegation
    // audience; the subject is the join key for authorization + credential.
    let subject_entity = repository
        .did()
        .to_string()
        .parse::<Entity>()
        .map_err(|e| {
            TonkWorkerError::Internal(format!("repository subject is not a valid entity: {e}"))
        })?;

    let delegation: dialog_ucan::UcanDelegation = tonk
        .profile
        .access()
        .claim(&repository)
        .delegate(membership_did)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;

    // base58-encode the delegation chain — the `?access=` parameter the
    // view reads back and assembles into the final URL.
    let chain_bytes = delegation.into_chain().to_bytes().map_err(|e| {
        TonkWorkerError::Internal(format!("failed to serialize delegation chain: {e}"))
    })?;
    let proof = bs58::encode(&chain_bytes).into_string();

    // Optional sync remote endpoint, stored as a ready-to-append URL query
    // suffix (`&remote=<percent-encoded-url>`) — or empty when the repo is
    // local-only. The share view appends it verbatim between `?access=…`
    // and the `#seed`, so a recipient on another device knows where to
    // pull from. It is a suffix (not a bare URL) because the view template
    // can't conditionally include a parameter, and `Invite::parse_url`
    // rejects an empty `remote=`, so "no remote" must append *nothing*.
    let remote = match super::create_invite::resolve_remote_url(&tonk, &repository).await? {
        Some(url) => {
            let encoded: String =
                url::form_urlencoded::byte_serialize(url.as_str().as_bytes()).collect();
            format!("&remote={encoded}")
        }
        None => String::new(),
    };

    let authorization = Authorization {
        this: subject_entity.clone(),
        proof: Proof(proof),
        remote: AuthorizationRemote(remote),
    };

    // Acquire the content branch through the reactor so the durable
    // commit and the overlay write target the same cached branch the
    // share view reads from.
    let session = tonk
        .reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to acquire content branch: {e}")))?;

    // Write the private seed into the session overlay, then schedule a
    // poll of this branch so the change propagates even though it never
    // commits durably. The seed never reaches replicated storage; clearing
    // first keeps exactly one live credential.
    session.state.clear_overlay();
    session.state.assert_overlay(Credential {
        this: subject_entity,
        seed: Seed(seed),
    });
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&session.state));

    // Assert the public authorization durably — committed **through the
    // reactor** so its cached branch sees the fact. The commit schedules
    // its own poll on the same branch; the dispatcher's drain coalesces it
    // with the overlay write above into a single re-evaluation that fans
    // the now-complete invitation out to the share view.
    tonk.reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(authorization)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to commit authorization fact: {e}"))
        })?;

    log!("Minted invitation for repo '{}'", repo_name);
    Ok(())
}

/// Build the [`RepositoryConfiguration`] for a space with a single
/// `main` branch, optionally synced to `remote`.
///
/// An empty (or whitespace-only) `remote` yields a local-only space —
/// the historical [`CreateSpace`](tonk_schema::command::CreateSpace)
/// behaviour. A non-empty `remote` is wired as the `origin` remote with
/// `main` tracking `origin/main`, so the space syncs from creation —
/// the same shape `init()` builds for `home`.
///
/// The URL is interpreted as a UCAN access-service endpoint (the only
/// remote scheme the UI offers): the topbar's default-service button
/// fills it with the worker origin + `/ucan/`, and a user may type any
/// other UCAN endpoint.
///
/// Shared by [`enable_sync_inner`] (called for both the create and
/// enable-sync forms) so they produce an identical remote shape.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn space_config(remote: &str) -> RepositoryConfiguration {
    use dialog_remote_ucan_s3::UcanAddress;

    let remote = remote.trim();
    if remote.is_empty() {
        return RepositoryConfiguration::default().branch("main", BranchConfiguration::default());
    }
    let address = SiteAddress::from(UcanAddress::new(remote));
    RepositoryConfiguration::default()
        .remote("origin", RemoteConfiguration::new(address))
        .branch(
            "main",
            BranchConfiguration::default().upstream("origin", "main"),
        )
}

/// Create a space local-only, split out so its `?` errors are logged
/// once at the boundary. Mirrors [`put_repository`] minus the HTTP shell.
/// Always creates a fresh repository with a minted identity; `name` is
/// only its display label. Returns the new routing key (the DID suffix)
/// so the caller can attach a remote to the identity it just created.
///
/// A sync remote is never wired here — it would make a remote/auth
/// failure abort the whole create, so the space never appears.
/// [`CreateSpaceHandler`] attaches the remote separately, after this.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn create_space_inner(state: &AppState, name: &str) -> Result<String, RepositoryError> {
    // A local-only `main`-branch space (the same config the button asks
    // for); a remote is attached afterwards by the handler.
    let configuration =
        RepositoryConfiguration::default().branch("main", BranchConfiguration::default());

    let (subject, key, branches) = {
        let tonk = state.write().await;

        // Create the repository (records the replica with status:blank).
        // `name` is the display label; the identity is freshly minted.
        let repository = create_repository(&tonk, name, &configuration).await?;
        let subject = repository.did();
        let key = subject.repo_key().to_owned();
        let branches: Vec<String> = configuration.branch.keys().cloned().collect();
        (subject, key, branches)
    };

    // Seed + flip to initialized once the lock is released (seeding is
    // the slow part; holding the lock would stall the page).
    seed_and_initialize(state, name, &key, &subject, &branches).await?;
    Ok(key)
}

/// Attach a sync remote to a space, idempotently, via
/// [`ensure_remote_config`] — the same helper [`attach_remote`] uses, so
/// the in-app path and the HTTP route converge on one implementation.
///
/// Called by [`CreateSpaceHandler`] after the repository exists (created
/// or pre-existing), for both the Hub "New space" and topbar "Enable
/// sync" forms. A missing repository or empty URL is a no-op (logged),
/// not an error.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn enable_sync_inner(
    state: &AppState,
    key: &str,
    remote: &str,
) -> Result<(), RepositoryError> {
    if remote.trim().is_empty() {
        // Submitted with no URL — nothing to attach.
        log!("enable sync '{}': empty remote, nothing to attach", key);
        return Ok(());
    }
    let configuration = space_config(remote);

    let tonk = state.write().await;
    // A missing repository is a no-op, not an error — defensive against a
    // stale key (e.g. an enable-sync form whose hidden repo field didn't
    // populate). The create path always runs `create_space_inner` first,
    // so the repo is present by the time this is reached on that path.
    let repository = match tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(error) => {
            log!(
                "enable sync '{}': repository not present, skipping ({})",
                key,
                error
            );
            return Ok(());
        }
    };

    ensure_remote_config(&tonk, &repository, key, &configuration).await
}

/// Spawn the background seed + status flip for a freshly created
/// repository. Returns immediately; the work runs after the PUT
/// response is sent.
///
/// Native builds have no service-worker scope (and no `spawn_local`
/// runtime here), so they no-op — the seed/status path is browser-only.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn spawn_seed(
    state: AppState,
    display_name: String,
    key: String,
    subject: Did,
    branches: Vec<String>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = seed_and_initialize(&state, &display_name, &key, &subject, &branches).await
        {
            log!("Background seed for '{}' failed: {}", key, e);
        }
    });
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn spawn_seed(
    _state: AppState,
    _display_name: String,
    _key: String,
    _subject: Did,
    _branches: Vec<String>,
) {
}

/// Seed the standard library into every branch, then flip the
/// replica's status to `initialized`. Runs in the background after
/// `put_repository` has already responded.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn seed_and_initialize(
    state: &AppState,
    display_name: &str,
    key: &str,
    subject: &Did,
    branches: &[String],
) -> Result<(), RepositoryError> {
    if !branches.is_empty() {
        let library = fetch_standard_library(STANDARD_LIBRARY_URL)
            .await
            .map_err(|e| RepositoryError::Internal(format!("fetch standard library: {e}")))?;
        // The showcase demo (the "Trip to Pistoia" sample) lands only in
        // the default `home` repository, on top of the scaffold; every
        // other repo gets the scaffold alone, so its directory render has
        // zero sheet instances and reads as genuinely empty — the
        // precondition for the launchpad view.
        let demo = if display_name == DEFAULT_REPOSITORY_NAME {
            Some(
                fetch_standard_library(DEMO_LIBRARY_URL)
                    .await
                    .map_err(|e| RepositoryError::Internal(format!("fetch showcase demo: {e}")))?,
            )
        } else {
            None
        };

        // Seed the whole scaffold in ONE commit per branch: the standard
        // library, the showcase demo (home only), AND the repository's own
        // name, concatenated into a single notation document evaluated
        // once. Splitting these across commits made the name land a beat
        // after the scaffold, so the Hub card briefly rendered the
        // library's default "Untitled" before the real name arrived — a
        // visible flash. Concatenating lets the rule engine saturate over
        // the whole document at once: the explicit `tonk/repository` name
        // is present when the library's default-name rule evaluates, so
        // its `unless` guard suppresses "Untitled" and only the real name
        // is ever committed.
        let name_body = repository_name_body(subject, display_name)?;
        let tonk = state.read().await;
        for branch_name in branches {
            let mut body = library.clone();
            if let Some(demo) = &demo {
                body.push('\n');
                body.push_str(demo);
            }
            body.push('\n');
            body.push_str(&name_body);
            seed_standard_library(&tonk, key, branch_name, &body)
                .await
                .map_err(|e| RepositoryError::Internal(format!("seed '{branch_name}': {e}")))?;
            log!(
                "Seeded scaffold + name on '{}' branch '{}'",
                key,
                branch_name
            );
        }
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    } else {
        // Nothing to seed — the replica is immediately initialized.
        let tonk = state.read().await;
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    }
    log!("Repository '{}' initialized", key);
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
/// other repo starts with zero sheet instances. Only referenced
/// from the SW-scoped background seed path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const DEMO_LIBRARY_URL: &str = "/library/demo.yaml";

/// The default display label for the repository created for a fresh
/// profile. The showcase demo is seeded only when the create carries
/// this label; every other repository gets the scaffold alone and
/// renders empty until populated. This is a label, not an address —
/// the repository's identity is still its minted DID.
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

/// Build the notation document asserting the repository's own
/// `tonk/repository` name, keyed by the subject DID. Concatenated into
/// the scaffold seed body (see [`seed_and_initialize`]) so the name lands
/// in the same commit as the library that defines the `tonk/repository`
/// concept it instantiates — no separate commit, no "Untitled" flash.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn repository_name_body(subject: &Did, display_name: &str) -> Result<String, RepositoryError> {
    // `name` is a JSON string so any character in the user-typed label
    // (quotes, colons, newlines) is carried verbatim rather than
    // breaking the notation.
    let name = serde_json::to_string(display_name)
        .map_err(|e| RepositoryError::Internal(format!("encode repository name: {e}")))?;
    Ok(format!(
        "tonk/repository!:\n  this: {subject}\n  name: {name}\n",
        subject = subject.as_str(),
    ))
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
    display_name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<Repository<SignerCredential>, RepositoryError> {
    // 1. Generate the repository's credential up front so its
    // `did:key` is its stable identity. The repository's routing
    // and storage key is that DID's suffix (`did.repo_key()`); the
    // user-typed `display_name` is only a label, seeded later into the
    // repository's own `tonk/repository` concept. Generating the signer
    // first (rather than letting `.create()` mint one) is what lets the
    // name derive from the DID instead of the other way around.
    let signer = Ed25519Signer::generate()
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to generate signer: {}", e)))?;
    let did = signer.did();
    let key = did.repo_key();

    let repository = tonk
        .profile
        .repository(key)
        .create()
        .with_credential(signer)
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to create repository '{}': {}", key, e))
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

    // 3-7. Wire up the meta branch and register the replica. The
    // replica is a name-less membership index; its identity (`subject`)
    // is the repository DID. The `display_name` is only threaded for log
    // context — the name itself is seeded into the repository's own
    // `tonk/repository` concept by the caller's seed step.
    record_repository_meta(tonk, &repository, display_name, configuration).await?;

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
    display_name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    // The repository's routing/storage key is its DID suffix; the
    // `display_name` is only used for log context here.
    let did = repository.did();
    let key = did.repo_key();

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

    // Local replica of this repository. The display name is not stored
    // here — it lives in the repository's own `tonk/repository` concept
    // on its content branch (seeded into the scaffold body, see `repository_name_body`).
    let replica = Replica::new(tonk.profile.did(), repository.did());

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
                key, e
            ))
        })?;
    log!("Wrote meta facts for repository '{}'", key);

    // Notify listeners of `/api/repository/{key}` that the repo's
    // representation changed. The broadcast mirrors the endpoint
    // the data is served from (keyed by the DID suffix); UIs
    // subscribed on that path pick up the change without a reload.
    // Fires after the commit so listeners only see durable state.
    broadcast(
        &format!("/api/repository/{key}"),
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
    record_replica_in_profile(tonk, display_name, &repository.did()).await?;

    // Drain the polls scheduled by the meta and profile-index commits
    // above so subscribers (e.g. the Hub on the profile meta branch) see
    // the new replica.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

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
    display_name: &str,
    subject: &Did,
) -> Result<(), RepositoryError> {
    let replica = Replica::new(tonk.profile.did(), subject.clone());
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
                display_name, e
            ))
        })?;
    log!("Recorded replica '{}' in profile meta", display_name);

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
/// Called from the background seed path and the join handler.
async fn set_replica_status(
    tonk: &TonkState,
    subject: &Did,
    status: tonk_schema::domain::replica::Status,
) -> Result<(), RepositoryError> {
    let entity = Replica::new(tonk.profile.did(), subject.clone())
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

    // Drain the poll the status commit scheduled so the Hub's profile
    // meta subscription reflects the new status.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    broadcast(
        "/api/profile",
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    Ok(())
}

/// Mark a replica `initialized` — its card settles from "Installing…" to
/// the resolved name. Used by the join path: a joined replica has no
/// local seed step (its content arrives over the pull), so it would
/// otherwise sit at the `blank` status `record_repository_meta` stamps,
/// stuck on an installing card forever.
pub(super) async fn mark_replica_initialized(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), RepositoryError> {
    set_replica_status(tonk, subject, Replica::initialized_status()).await
}

/// Bootstrap the profile repository's meta branch.
///
/// Called on every worker startup. Asserts the profile's "self"
/// replica record (profile DID == subject DID) and a [`MetaBranch`]
/// concept for the meta branch itself.
///
/// A no-op when the profile has already been bootstrapped — both
/// assertions are content-addressed (entity hashes depend only on
/// `(profile, subject)` / `(replica, name)`), so re-asserting the
/// same facts produces the same entities and attribute values and
/// the dialog layer deduplicates.
pub async fn bootstrap_profile_meta(tonk: &TonkState) -> Result<(), RepositoryError> {
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);

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

    // Drain the poll the bootstrap commit scheduled.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

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

/// The branch a repository's own `tonk/repository` name is seeded onto.
/// Spaces have a single content branch (`main`); the seed writes the
/// name there (see `repository_name_body`).
const CONTENT_BRANCH: &str = "main";

/// Read a repository's display label from its own `tonk/repository`
/// concept on its content branch, keyed by the subject DID.
///
/// This is the single source of truth for the name: it lives with the
/// repository and syncs across devices, so a rename on any device is
/// visible everywhere the content branch syncs. Falls back to the
/// routing `key` when the content branch can't be opened or carries no
/// name yet (a freshly created repo before its name is seeded).
async fn repository_label<R>(tonk: &TonkState, repository: &Repository<R>, key: &str) -> String
where
    R: Principal + Clone,
{
    let content = match repository
        .branch(CONTENT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
    {
        Ok(content) => content,
        Err(e) => {
            log!(
                "No '{}' branch for repository '{}' label: {}",
                CONTENT_BRANCH,
                key,
                e
            );
            return key.to_string();
        }
    };

    match content
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(repository.did().this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .next()
            .map(|row| row.name.0)
            .unwrap_or_else(|| key.to_string()),
        Err(e) => {
            log!("tonk/repository label query failed for '{}': {:?}", key, e);
            key.to_string()
        }
    }
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
    key: &str,
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
            log!("No meta branch for repository '{}': {}", key, e);
            return RepositoryInfo {
                name: key.to_string(),
                label: key.to_string(),
                subject: repository.did(),
                operator: tonk.operator.did(),
                profile: tonk.profile.did(),
                branch: HashMap::new(),
                remote: HashMap::new(),
            };
        }
    };

    // Derive the replica entity from `(profile, subject)` — the same
    // hash `create_repository` used. Used below to scope the remote and
    // tracking-branch queries on the meta branch.
    let replica = Replica::new(tonk.profile.did(), repository.did());
    let replica_entity = replica.this().clone();

    // Read the display label from the repository's own `tonk/repository`
    // concept on its content branch, keyed by the subject DID. The name
    // lives with the repository (not in the profile's replica index), so
    // it stays current on every device that syncs the content branch.
    // Falls back to the routing `key` when no name has been seeded yet.
    let label = repository_label(tonk, repository, key).await;

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
            log!("Branch query on meta failed for '{}': {:?}", key, e);
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
            log!("Remote query on meta failed for '{}': {:?}", key, e);
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
                key,
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
                    key,
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
                    key,
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
        name: key.to_string(),
        label,
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

    let replica = Replica::new(tonk.profile.did(), repository.did());
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

    // Drain the poll the meta commit scheduled.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    // Mirror the create path: tell listeners of the repository's
    // representation that its remotes/branches changed.
    broadcast(
        &format!("/api/repository/{name}"),
        &Notification {
            branch: META_BRANCH.to_string(),
            revision,
        },
    );

    // The upstream was just published on *this* loaded handle, but the
    // reactor caches a separate branch handle (opened earlier, e.g. when
    // the standard library was seeded) whose `upstream` cell predates it.
    // Sync reads through that cached handle, so without reconciling it the
    // pull would fail with `BranchHasNoUpstream` even though the upstream
    // is durable. Refresh each branch we wired so the cached handle
    // reflects it.
    for (branch_name, settings) in &configuration.branch {
        if settings.upstream.is_some() {
            tonk.reactor
                .refresh_branch(name, branch_name, &tonk.operator)
                .await
                .map_err(|e| {
                    RepositoryError::Internal(format!(
                        "Failed to refresh cached branch '{}' after wiring upstream: {}",
                        branch_name, e
                    ))
                })?;
        }
    }

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
/// The pure remote-shape builder shared by the create and attach paths.
/// Native — no browser/service-worker scope needed.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod space_config_tests {
    use super::space_config;

    #[test]
    fn it_builds_a_local_only_config_for_an_empty_remote() {
        let config = space_config("");
        assert!(
            config.remote.is_empty(),
            "an empty remote must leave the space local-only"
        );
        let main = config.branch.get("main").expect("main branch present");
        assert!(
            main.upstream.is_none(),
            "a local-only space's main branch must have no upstream"
        );
    }

    #[test]
    fn it_treats_a_whitespace_remote_as_local_only() {
        let config = space_config("   ");
        assert!(config.remote.is_empty());
        assert!(config.branch.get("main").unwrap().upstream.is_none());
    }

    #[test]
    fn it_wires_origin_and_tracks_main_for_a_remote_url() {
        let config = space_config("https://example.test/ucan/");
        assert!(
            config.remote.contains_key("origin"),
            "a remote URL must register the origin remote"
        );
        let upstream = config
            .branch
            .get("main")
            .and_then(|b| b.upstream.as_ref())
            .expect("main must track an upstream when a remote is given");
        assert_eq!(upstream.remote, "origin");
        assert_eq!(upstream.branch, "main");
    }
}

/// The optional-remote reader the create/enable handler uses. Native.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod remote_from_facts_tests {
    use super::remote_from_facts;
    use dialog_artifacts::{Artifact, Changes, Entity, Instruction, Statement, Value};
    use dialog_query::the;

    const URL: &str = "http://127.0.0.1:8080/ucan/";

    fn artifacts(changes: Changes) -> Vec<Artifact> {
        changes
            .into_instructions()
            .into_iter()
            .map(|instruction| match instruction {
                Instruction::Assert(artifact)
                | Instruction::Replace(artifact)
                | Instruction::Retract(artifact) => artifact,
            })
            .collect()
    }

    /// Seed the always-present `name` fact (the create form's required field).
    fn name_fact(changes: &mut Changes, of: &Entity) {
        the!("dom.event.current-target.elements.name/value")
            .of(of.clone())
            .is("test".to_string())
            .assert(changes);
    }

    #[test]
    fn it_reads_a_string_remote() {
        let of: Entity = "did:key:zCreate".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        // `.is(String)` produces a `Value::String` — the relative-path case.
        the!("dom.event.current-target.elements.remote/value")
            .of(of)
            .is(URL.to_string())
            .assert(&mut changes);
        assert_eq!(remote_from_facts(&artifacts(changes)).as_deref(), Some(URL));
    }

    #[test]
    fn it_reads_an_entity_remote() {
        // A URL deserializes as `Value::Entity` (any string with a `:`) —
        // exactly the case a `String`-typed concept field couldn't decode,
        // which is why the handler reads the artifact directly.
        let url_value: Value = serde_json::from_str(&format!("\"{URL}\"")).unwrap();
        let url = match url_value {
            Value::Entity(entity) => entity,
            other => panic!("URL should deserialize as Entity, got {other:?}"),
        };
        let of: Entity = "did:key:zCreate".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        // `.is(Entity)` produces a `Value::Entity` — the URL case.
        the!("dom.event.current-target.elements.remote/value")
            .of(of)
            .is(url)
            .assert(&mut changes);
        assert_eq!(remote_from_facts(&artifacts(changes)).as_deref(), Some(URL));
    }

    #[test]
    fn it_returns_none_without_a_remote_fact() {
        let of: Entity = "did:key:zLocal".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        assert!(remote_from_facts(&artifacts(changes)).is_none());
    }

    #[test]
    fn it_treats_a_blank_remote_as_none() {
        let of: Entity = "did:key:zBlank".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        the!("dom.event.current-target.elements.remote/value")
            .of(of)
            .is("   ".to_string())
            .assert(&mut changes);
        assert!(remote_from_facts(&artifacts(changes)).is_none());
    }
}

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

    /// Create a fresh repo and return its router, wrapped state, and
    /// minted routing key. PUTs a branchless `{}` so the worker seeds
    /// nothing — the test drives seeding / attaching itself. The `main`
    /// branch is created on first write. `label` is only a display
    /// name; every create mints a fresh identity, so runs never collide.
    async fn fresh_repo(label: &str) -> (Router, AppState, String) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{label}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "expected 201 from PUT /api/repository/{label}, got {status}",
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: RepositoryInfo = serde_json::from_slice(&body).unwrap();
        (app, state, info.name)
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

    /// The scaffold alone defines the `workspace/sheet` concept — so the
    /// directory route can resolve it — but seeds zero sheet instances.
    /// That empty render is the precondition for the launchpad.
    #[dialog_common::test]
    async fn it_seeds_scaffold_without_showcase_instances() {
        let (_app, state, repo) = fresh_repo("test-seed-scaffold-only").await;
        let repo = repo.as_str();
        seed(&state, repo, CORE).await;

        assert_eq!(
            count(&state, repo, "workspace/sheet:\n").await,
            0,
            "scaffold-only repo must have zero sheet instances",
        );
    }

    /// The showcase, seeded on top of the scaffold, resolves its bare
    /// concept references (`workspace/sheet`, `person`, …) against the
    /// committed scaffold and lands the demo instances — what the
    /// default `home` repo gets. Guards the cross-document resolution
    /// the split depends on.
    #[dialog_common::test]
    async fn it_seeds_showcase_on_top_of_scaffold() {
        let (_app, state, repo) = fresh_repo("test-seed-showcase").await;
        let repo = repo.as_str();
        seed(&state, repo, CORE).await;
        seed(&state, repo, DEMO).await;

        assert!(
            count(&state, repo, "workspace/sheet:\n").await >= 1,
            "showcase must seed at least one sheet instance",
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
        let (app, _state, repo) = fresh_repo("test-attach-remote").await;
        let repo = repo.as_str();

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
        let (app, _state, repo) = fresh_repo("test-attach-remote-idempotent").await;
        let repo = repo.as_str();
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
        let (app, _state, repo) = fresh_repo("test-attach-then-invite").await;
        let repo = repo.as_str();

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

    /// Regression: the reactor caches a branch handle the first time it's
    /// touched — e.g. when the standard library is seeded — capturing its
    /// `upstream` cell *before* any remote is attached. Attaching a remote
    /// later sets the upstream on a freshly loaded handle; the cached
    /// handle must be reconciled, or sync (which reads through the cache)
    /// fails with `BranchHasNoUpstream` even though the upstream is durable.
    #[dialog_common::test]
    async fn it_reconciles_the_cached_branch_handle_after_attach() {
        use dialog_repository::Upstream;

        let (app, state, repo) = fresh_repo("test-attach-refreshes-cache").await;
        let repo = repo.as_str();

        // Seed through the reactor so `main` is cached with no upstream —
        // the state real space creation leaves behind before sync is on.
        seed(&state, repo, CORE).await;

        attach(&app, repo, &origin_config("https://example.test/ucan/")).await;

        // The cached handle that sync reads must now report the upstream.
        let guard = state.read().await;
        let session = guard
            .reactor
            .repository(repo)
            .branch("main")
            .acquire(&guard.operator)
            .await
            .expect("acquire cached main");
        let upstream = session
            .handle()
            .upstream()
            .expect("cached main must report the upstream after attach");
        assert!(
            matches!(
                upstream,
                Upstream::Remote { ref remote, ref branch, .. }
                    if remote == "origin" && branch == "main"
            ),
            "cached main must track origin/main, got {upstream:?}",
        );
    }

    /// Reconciling the cached handle swaps in a fresh `BranchState`, but it
    /// must carry the live subscriptions across so in-flight SSE streams
    /// don't silently freeze on the discarded handle.
    #[dialog_common::test]
    async fn it_keeps_live_subscriptions_when_refreshing_a_branch() {
        use std::sync::Arc;

        use dialog_query::{ConceptQuery, Query};
        use tonk_schema::meta::Name;

        let (app, state, repo) = fresh_repo("test-attach-keeps-subscriptions").await;
        let repo = repo.as_str();
        seed(&state, repo, CORE).await;

        // Register a subscription on the cached `main` and note which
        // `BranchState` it landed on. Hold the subscriber so its receiver
        // (and the paired sender in the state) stays connected.
        let subscriber;
        let before_ptr;
        {
            let guard = state.read().await;
            let session = guard
                .reactor
                .repository(repo)
                .branch("main")
                .acquire(&guard.operator)
                .await
                .expect("acquire cached main");
            subscriber = session
                .subscribe(ConceptQuery::from(Query::<Name>::default()))
                .expect("subscribe");
            before_ptr = Arc::as_ptr(&session.state);
        }

        attach(&app, repo, &origin_config("https://example.test/ucan/")).await;

        let guard = state.read().await;
        let session = guard
            .reactor
            .repository(repo)
            .branch("main")
            .acquire(&guard.operator)
            .await
            .expect("re-acquire main");
        assert!(
            !std::ptr::eq(before_ptr, Arc::as_ptr(&session.state)),
            "refresh must swap in a fresh BranchState",
        );
        assert_eq!(
            session.state.subscriptions().lock().len(),
            1,
            "the live subscription must survive the refresh",
        );
        drop(subscriber);
    }
}

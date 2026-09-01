//! Repository create route.
//!
//! `PUT /api/repository/{repo}` always creates a fresh repository. The
//! repository's identity is its credential's `did:key`; the `{repo}`
//! path segment is only a display label. Every create mints a new
//! identity, so there is never a create-time collision — two spaces may
//! share a label. The response carries the new repository's routing key
//! (the DID suffix), which the UI routes by.

use dialog_capability::Subject;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_effects::Use;
use std::collections::HashMap;

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use axum_wasm_macros::wasm_compat;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{
    RemoteRepository, Repository, RepositoryExt as _, Revision, SiteAddress, Upstream,
};
use dialog_ucan::UcanDelegation;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::prefix::SPACE_ROOT_SITE_PREFIX;
use tonk_common::log;
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{
    Branch as MetaBranch, Invitation, InvitedVia, MemberName, MemberRole, Membership, Remote,
    RemoteExecution, Replica, RepositoryName, SeedKind, SpaceStatus, TrackingBranch,
};
use url::Url;
use zeroize::Zeroizing;

use super::AppState;
use crate::{Notification, RepositoryError, TonkWorkerError, broadcast, worker::TonkState};

/// Name of the device-local meta branch every *space* repository has
/// alongside its content branch. It stores local bookkeeping — the
/// local [`Replica`] record, remotes config, and branch enumeration —
/// that must never replicate (see [`tonk_schema`]).
pub(crate) const META_BRANCH: &str = "meta";

/// The single branch the *profile* repository lives on. The profile
/// has no content/meta split (its whole state is device-local hub
/// bookkeeping), so it uses `main` like any repository's default
/// branch rather than a separate meta branch.
const PROFILE_BRANCH: &str = "main";

/// Configuration for a single remote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteConfiguration {
    /// The remote's site address (serialized `SiteAddress`).
    pub address: SiteAddress,
    /// Optional subject DID for the remote repository. Defaults to
    /// this repository's DID if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Did>,
    /// Explicit immutable-artifact relay for invitation revocations.
    #[serde(
        default,
        rename = "revocationUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub revocation_url: Option<Url>,
}

impl RemoteConfiguration {
    /// Build a remote config from its address.
    pub fn new(address: impl Into<SiteAddress>) -> Self {
        Self {
            address: address.into(),
            subject: None,
            revocation_url: None,
        }
    }

    /// Override the subject DID — by default the remote's subject
    /// is the same as the local repository's DID.
    pub fn subject(mut self, subject: Did) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Attach the explicit immutable-artifact relay.
    pub fn revocation_url(mut self, revocation_url: Url) -> Self {
        self.revocation_url = Some(revocation_url);
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

/// One member of a repository, assembled from the roster facts on
/// the meta branch. `did` is the member profile's did:key URI (the
/// meta entity, used directly as a `<tonk-sigil>` seed). `invited_by`
/// is the inviter's did:key, which the UI resolves to a name against
/// the member list; `None` for the founder and self-invites.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    /// The member profile's did:key URI.
    pub did: String,
    /// The member's published display name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this member is the active profile.
    pub is_self: bool,
    /// The inviter's did:key, when provenance was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invited_by: Option<String>,
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
    /// The repository's members, read from the synced content branch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberInfo>,
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

    // A space created with a remote in this one shot is not escrowed for
    // cross-device restore: the account-holder create flow attaches its
    // remote through `enable_sync_inner` (which does escrow it), never
    // this path, so only non-UI callers reach here with a remote. Backing
    // it up would need the sync URL recovered from the parsed
    // configuration; left as a follow-up. Fails open — the space works
    // locally, it just will not follow the user to another device.

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

/// The `tonk:enable-sync` transient's target space, read from the raw facts.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ENABLE_SYNC_SPACE_ATTR: &str = "xyz.tonk.enable-sync/space";

/// The `tonk:enable-sync` transient's endpoint, read from the raw facts.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ENABLE_SYNC_REMOTE_ATTR: &str = "xyz.tonk.enable-sync/remote";

/// Marker asking the handler to mint once the remote is attached.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ENABLE_SYNC_SHARE_ATTR: &str = "xyz.tonk.enable-sync/share";

/// Read a fact's value as a string, tolerating both the `String` and
/// `Entity` representations — a URL or a DID round-trips through JSON as an
/// `Entity` (any `:`-bearing string does), so a single-representation read
/// would silently miss them. Mirrors [`remote_from_facts`].
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn text_fact(facts: &crate::reactor::EntityFacts, attribute: &str) -> Option<String> {
    text_fact_any_target(facts, attribute)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn text_fact_any_target(facts: &crate::reactor::EntityFacts, attribute: &str) -> Option<String> {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| artifact.the.to_string() == attribute)
        .and_then(|artifact| match &artifact.is {
            Value::String(text) => Some(text.clone()),
            Value::Entity(entity) => Some(entity.to_string()),
            _ => None,
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The default display label for a space created without a user-typed
/// name. The create forms carry it in a hidden `name` input (the wizard
/// no longer asks for a name up front); the handler uniquifies it
/// against the existing space labels via [`next_untitled_label`], and
/// the user renames the space later (the FAB's inline editable /
/// `tonk/rename-repository`).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const UNTITLED: &str = "Untitled";

/// Pick the first free untitled label: `Untitled`, then `Untitled 2`,
/// `Untitled 3`, … — the smallest ordinal no existing label already
/// uses. Only exact `Untitled` / `Untitled <n>` labels count as taken;
/// anything else (user-typed names, key fallbacks) is ignored.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn next_untitled_label<I>(existing: I) -> String
where
    I: IntoIterator<Item = String>,
{
    let taken: std::collections::HashSet<u64> = existing
        .into_iter()
        .filter_map(|label| {
            let label = label.trim();
            if label == UNTITLED {
                return Some(1);
            }
            label
                .strip_prefix(UNTITLED)
                .and_then(|rest| rest.strip_prefix(' '))
                .and_then(|ordinal| ordinal.parse::<u64>().ok())
                .filter(|ordinal| *ordinal >= 2)
        })
        .collect();
    let mut ordinal = 1;
    while taken.contains(&ordinal) {
        ordinal += 1;
    }
    if ordinal == 1 {
        UNTITLED.to_string()
    } else {
        format!("{UNTITLED} {ordinal}")
    }
}

/// The display labels of every space the profile owns, read from each
/// repository's own `tonk/repository` concept (the same source the Hub
/// renders). Used by the create handler to uniquify the untitled label.
///
/// Best-effort: a replica whose repo can't be loaded is skipped (its
/// [`repository_label`] key fallback wouldn't match the untitled
/// pattern anyway), so a single broken space never blocks a create.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn existing_space_labels(state: &AppState) -> Vec<String> {
    use tonk_schema::domain::replica::Profile as ProfileEntity;

    let tonk = state.read().await;
    let profile_entity = tonk.profile.did().this();

    let meta = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(meta) => meta,
        Err(e) => {
            log!("existing_space_labels: profile meta acquire failed: {e}");
            return Vec::new();
        }
    };

    let rows: Vec<Replica> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(profile_entity.clone())),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    let mut labels = Vec::new();
    for replica in rows {
        if replica.kind != Replica::repository_kind() {
            continue;
        }
        let Ok(did) = replica.subject.0.to_string().parse::<Did>() else {
            continue;
        };
        let key = did.repo_key().to_owned();
        match tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(repository) => labels.push(repository_label(&tonk, &repository, &key).await),
            Err(e) => log!("existing_space_labels: repository '{key}' not loadable: {e}"),
        }
    }
    labels
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
/// `name` is only a display label; two spaces may share it. The create
/// wizard doesn't ask for one — its hidden input carries the
/// [`UNTITLED`] sentinel, which the handler uniquifies against the
/// existing space labels ([`next_untitled_label`]) so consecutive
/// creates read "Untitled", "Untitled 2", …. Once the space is created
/// and seeded, the handler posts a `navigate` message back to the
/// originating client so the creator lands inside the new space. A
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

            // The create wizard no longer asks for a name: its hidden
            // `name` input carries the `Untitled` sentinel (a blank name
            // from an older form gets the same treatment). Uniquify it
            // against the existing space labels so consecutive creates
            // read "Untitled", "Untitled 2", … — the user renames later.
            let name = if name.trim().is_empty() || name.trim() == UNTITLED {
                next_untitled_label(existing_space_labels(env.state()).await)
            } else {
                name
            };
            log!("command CreateSpace name={} remote={:?}", name, remote);

            // The space's seed is custodied under the account before the
            // space exists. A linked device whose root record predates the
            // encryption key asks the originating page for a passkey
            // assertion here, outside the state lock, and resumes once the
            // page has saved the key.
            if let Err(error) = super::custody::ensure_recipient(env.state(), env.client()).await {
                log!("CreateSpace '{}' refused: {}", name, error);
                return;
            }

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

            // 2. The space is created and seeded — drop the creator into
            //    it. Same page-capability channel as the join redirect: a
            //    `{ type: "navigate", href }` posted to the originating
            //    client. Fired before the remote attach so the navigation
            //    doesn't wait on the network; the attach continues in the
            //    worker regardless.
            let href = format!("/space/{key}");
            crate::router::navigate::notify_navigate(env.client(), &href);

            // 3. If the form carried a remote, attach it best-effort to
            //    the identity just created. A failure here just leaves it
            //    local-only — retryable from the topbar's Enable sync.
            //    (`remote_from_facts` already dropped empty/blank URLs.)
            // A blank remote used to mean local-only, which the account
            // directory now advertises account-wide as a space no other
            // device can ever replicate. With an ACTIVE account, the
            // account's own sync remote is the natural default — the
            // same access service the account DB syncs through; the
            // relay resolves from the remote's origin as usual.
            //
            // Without one, no default: the access service serves only an
            // active customer's subjects, so defaulting a remote here
            // would wire an upstream that 403s on every presign. The
            // space stays local until the user asks to share it, which
            // is where provisioning belongs.
            // The endpoint comes from the account's own registration
            // fact, not from the signed descriptor and not from the
            // page's `https://{origin}/ucan/` guess: registration is
            // where the account learned which access service it is a
            // customer of, so that is the one answer every attach path
            // reads.
            let remote = match remote {
                Some(remote) => Some(remote),
                None => {
                    let tonk = env.state().read().await;
                    if super::customer::is_active(&tonk).await {
                        account_sync_remote(&tonk).await
                    } else {
                        None
                    }
                }
            };
            if let Some(remote) = remote
                && let Err(error) = enable_sync_inner(env.state(), &key, &remote).await
            {
                log!("CreateSpace '{}': remote attach failed: {}", key, error);
            }
        })
    }
}

/// The FAB's routeless share claim's target-space attribute — the
/// `xyz.tonk.invite/space` fact asserted alongside the `tonk:invite`
/// transient. Kept in sync with
/// [`tonk_schema::domain::command::invite::Space`]'s derived attribute.
///
/// NOT a matched field on [`tonk_schema::command::Invite`]: every existing
/// space's `tonk:invite` descriptor is frozen without it, and a required
/// field would make those transients silently fail to match (the transient
/// commits, no handler runs) — see that type's doc and
/// `docs/evolving-command-concepts.md`, which records the same mistake with
/// `CreateSpace.remote`.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const INVITE_SPACE_ATTR: &str = "xyz.tonk.invite/space";

/// Read the target space DID from a `tonk:invite` transient's facts,
/// opportunistically — mirrors [`remote_from_facts`].
///
/// `Some` when the FAB's newer profile-dispatched share claim named its
/// target explicitly (asserted as either a `Value::Entity` DID or a
/// `Value::String`, tolerating both representations like `remote_from_facts`
/// does). `None` for an older claim carrying no such fact — the handler
/// falls back to the dispatch origin in that case.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn invite_space_from_facts(facts: &crate::reactor::EntityFacts) -> Option<String> {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| artifact.the.to_string() == INVITE_SPACE_ATTR)
        .and_then(|artifact| match &artifact.is {
            Value::String(space) => Some(space.clone()),
            Value::Entity(entity) => Some(entity.to_string()),
            _ => None,
        })
        .map(|space| space.trim().to_string())
        .filter(|space| !space.is_empty())
}

/// Post-commit handler for the [`Invite`] command.
///
/// When the FAB's share control (`<tonk-share>`) dispatches a transient
/// [`Invite`], this handler generates a fresh membership keypair, delegates
/// the *target* repository's access to its DID, base58-encodes the
/// resulting delegation chain, and asserts a durable [`Authorization`] fact
/// keyed by that DID on the repository's content branch (`main`). It then
/// asserts the private seed as a [`Credential`] into the reactor's session
/// overlay (never replicated). The share view joins the two via
/// `tonk:invitation` and assembles the final URL.
///
/// The repository is read from the command's `space` field, not
/// [`CommandEnv::origin`](crate::router::CommandEnv::origin): `Invite` is
/// dispatched routeless from the FAB's own profile-branch context (see
/// `tonk-fab::logic::invite_claim_json`), where the origin repo is always
/// empty — mirroring [`PauseSyncHandler`] and [`RenameRepositoryHandler`].
///
/// A custom handler (not a plain `Provider<Invite>`) is required because
/// it reads durable repository state the decoded command alone does not
/// carry and writes to the reactor's session overlay.
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
        use tonk_schema::prelude::DidExt as _;

        // Read the target space off the facts opportunistically (NOT a
        // matched `Invite` field — see `invite_space_from_facts`) so the
        // FAB's routeless, profile-dispatched share claim can name its
        // target. Fall back to the dispatch origin when the fact is
        // absent — the shape every existing space's frozen `tonk:invite`
        // descriptor still dispatches, mirroring `PauseSyncHandler`/
        // `RenameRepositoryHandler` for the named-target case and
        // `ProfileRenameHandler` for the origin fallback.
        let repo_name = invite_space_from_facts(facts)
            .and_then(|space| space.parse::<dialog_varsig::Did>().ok())
            .map(|did| did.repo_key().to_owned())
            .unwrap_or_else(|| env.origin().repo.clone());

        // The triggering click's timestamp, echoed onto a refusal so a
        // later resubscribe can tell this refusal from a replay of an
        // older one — see `publish_share_blocked`.
        let time = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::Invite::decode(entity, facts))
            .map(|command| command.time.0)
            .unwrap_or_default();
        let env = env.clone();

        Box::pin(async move {
            if repo_name.is_empty() {
                log!("Invite: no target space (no fact, empty origin), skipping");
                return;
            }
            log!("command Invite repo={}", repo_name);

            // A pass that attached a remote leaves the space ready but
            // unminted, so run once more. Bounded to a single retry: the
            // second pass either mints or refuses for a reason attaching
            // cannot fix.
            let outcome = run_invite(&env, &repo_name, time).await;
            if let Ok(RunInvite::Attached) = outcome
                && let Err(error) = run_invite(&env, &repo_name, time).await
            {
                log!(
                    "Invite for repo '{}' failed after attaching: {}",
                    repo_name,
                    error
                );
            }
            if let Err(error) = outcome {
                log!("Invite for repo '{}' failed: {}", repo_name, error);
            }
        })
    }
}

/// Attach a sync remote to an existing space, then mint an invite when the
/// transient asks for one.
///
/// The share control dispatches this when a user accepts the offer to turn
/// sync on after a refused share. Minting from inside the handler is what
/// makes that a single click: the control needs no completion signal for the
/// attach, because success reaches it as a new invite link on the
/// subscription it already holds — the same path an ordinary mint takes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct EnableSyncHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl EnableSyncHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::EnableSync::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for EnableSyncHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::EnableSync::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;
        use tonk_schema::prelude::DidExt as _;

        let time = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::EnableSync::decode(entity, facts))
            .map(|command| command.time.0)
            .unwrap_or_default();
        let space = text_fact(facts, ENABLE_SYNC_SPACE_ATTR);
        let remote = text_fact(facts, ENABLE_SYNC_REMOTE_ATTR);
        let share = text_fact(facts, ENABLE_SYNC_SHARE_ATTR).is_some();
        let env = env.clone();

        Box::pin(async move {
            use dialog_artifacts::Entity;

            let Some(space) = space else {
                log!("EnableSync: missing space, skipping");
                return;
            };
            // An absent remote means "wherever this account syncs" — the
            // page no longer derives an endpoint from its own origin,
            // which it could not even do reliably: a sealed guest's
            // document is `about:srcdoc`, so it had to be told its own
            // origin by the portal bridge first, and a share before that
            // arrived did nothing at all.
            let remote = match remote {
                Some(remote) => remote,
                None => {
                    let tonk = env.state().read().await;
                    match account_sync_remote(&tonk).await {
                        Some(remote) => remote,
                        None => {
                            log!("EnableSync: no remote given and the account names no provider");
                            return;
                        }
                    }
                }
            };
            let Ok(did) = space.parse::<dialog_varsig::Did>() else {
                log!("EnableSync: '{}' is not a DID", space);
                return;
            };
            let key = did.repo_key().to_owned();
            log!("command EnableSync repo={} share={}", key, share);

            if let Err(error) = enable_sync_inner(env.state(), &key, &remote).await {
                log!("EnableSync '{}' failed: {}", key, error);
                if share {
                    let subject = match space.parse::<Entity>() {
                        Ok(entity) => entity,
                        Err(e) => {
                            log!("EnableSync: '{}' is not an entity: {}", space, e);
                            return;
                        }
                    };
                    publish_share_blocked(
                        env.state(),
                        &key,
                        subject,
                        "attach-failed",
                        &format!("Could not turn on sync: {error}"),
                        time,
                    )
                    .await;
                }
                return;
            }

            if share && let Err(error) = run_invite(&env, &key, time).await {
                log!("EnableSync '{}': mint after attach failed: {}", key, error);
            }
        })
    }
}

/// Generate a membership keypair, delegate `repo_name`'s access to it,
/// assert the public [`Authorization`] on the content branch, and assert
/// the private seed as a [`Credential`] into the reactor's session
/// overlay (so it stays out of replicated storage).
///
/// `time` is the triggering `tonk:invite` transient's timestamp — unused
/// on the mint path, but threaded through so a refusal (see
/// [`publish_share_blocked`]) can echo the click it answers.
///
/// Split out from [`InviteHandler::run`] so the `?` early-return funnels
/// into the single `log!` there — the command future itself returns `()`.
/// What one pass of [`run_invite`] settled.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
enum RunInvite {
    /// Minted, refused, or otherwise finished — nothing more to do.
    Settled,
    /// The space had no remote and one was just attached, so a second
    /// pass can now mint. Returned rather than recursing: re-entering
    /// an async fn from inside itself needs boxing for no gain.
    Attached,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_invite(
    env: &crate::router::CommandEnv,
    repo_name: &str,
    time: f64,
) -> Result<RunInvite, TonkWorkerError> {
    use dialog_artifacts::Entity;
    use dialog_varsig::Principal as _;
    use tonk_schema::command::{Authorization, Credential};
    use tonk_schema::domain::authorization::{Proof, Remote as AuthorizationRemote};
    use tonk_schema::domain::credential::{Link, Seed};
    use tonk_schema::{Invitation, InvitationExecution};

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
    require_real_space(&tonk, &repository.did()).await?;

    // Both facts are keyed by the repository's *subject* DID — the entity
    // the share view already addresses (`entity={subject}`) — not the
    // membership DID.
    let subject_entity = repository
        .did()
        .to_string()
        .parse::<Entity>()
        .map_err(|e| {
            TonkWorkerError::Internal(format!("repository subject is not a valid entity: {e}"))
        })?;

    if super::account::provider(&tonk).await.is_none() {
        log!("Invite for repo '{}' refused: account required", repo_name);
        drop(tonk);
        publish_share_blocked(
            env.state(),
            repo_name,
            subject_entity,
            tonk_worker_api::share::BLOCKED_ACCOUNT_REQUIRED,
            "Create an account or log in before sharing this space.",
            time,
        )
        .await;
        return Ok(RunInvite::Settled);
    }

    // Resolve the sync endpoint BEFORE minting anything. An invite with no
    // remote lands its recipient in a space that can never fill, so there is
    // nothing worth generating key material for. Refusing here also means a
    // refusal costs no delegation and rotates no credential.
    let remote_execution = match super::create_invite::resolve_remote_url(&tonk, &repository)
        .await?
    {
        super::create_invite::RemoteRequirement::Ready(execution) => execution,
        super::create_invite::RemoteRequirement::Refused(reason) => {
            // Say WHY there is no remote. "Attach one" is the right
            // offer only when a provider exists to attach to.
            let reason = super::create_invite::explain_refusal(&tonk, reason).await;
            log!("Invite for repo '{}' refused: {}", repo_name, reason.code());
            let subject = repository.did().to_string();
            drop(tonk);

            // Whether to issue a link or get an account first is the
            // worker's call, not the caller's. A share that needs an
            // account is not a failure the control should interpret
            // and repair — it is this handler's next step, so it
            // asks for the account itself and the share resumes when
            // the account facts land.
            //
            // Not awaited: registration may take a ceremony, an
            // email round trip, or never finish, and a handler held
            // open across that is held open forever.
            if reason.code() == tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT
                && let Some(client) = env.client()
                && let Err(error) = super::navigate::request_account_link(client, &subject).await
            {
                log!("Invite: could not ask the page to link an account: {error}");
            }

            // `not-synced` is not a refusal either: the account has a
            // provider, this space simply has no remote yet, and
            // attaching one is this handler's next step rather than a
            // question for the caller. Sharing a local-only space is
            // exactly the moment it earns its remote.
            //
            // Without this the click had nowhere to go. The control's
            // own prompt for this case was removed when the worker took
            // the decision over, so the share refused, nothing attached,
            // and the button span until it timed out.
            if reason.code() == tonk_worker_api::share::BLOCKED_NOT_SYNCED {
                let provider = {
                    let tonk = env.state().read().await;
                    super::customer::provider_address(&tonk).await
                };
                match provider {
                    Some(remote) => {
                        log!("Invite for repo '{repo_name}': attaching {remote} before minting");
                        match enable_sync_inner(env.state(), repo_name, &remote).await {
                            // Attached. Report it and let the caller mint:
                            // re-entering `run_invite` here would be async
                            // recursion, which needs boxing for no gain.
                            Ok(()) => return Ok(RunInvite::Attached),
                            Err(error) => {
                                log!("Invite for repo '{repo_name}': attach failed: {error}")
                            }
                        }
                    }
                    None => log!("Invite for repo '{repo_name}': the account names no provider"),
                }
            }

            publish_share_blocked(
                env.state(),
                repo_name,
                subject_entity,
                reason.code(),
                reason.detail(),
                time,
            )
            .await;
            return Ok(RunInvite::Settled);
        }
    };

    // A share is a promise the recipient can actually pull, and an
    // upstream can outlive its provisioning (a space created before the
    // account had an active customer keeps its remote while the service
    // refuses every presign). Ensure the consumer row exists before any
    // key material is minted: `/provider/add` is idempotent — an already
    // provided consumer answers `ConsumerProvided`, treated as success —
    // so every share self-heals that half-state. Best effort like the
    // enable-sync attach: a foreign remote (self-hosted, a test server)
    // is not our access service, and refusing the mint over it would
    // make those unshareable.
    match space_root_prefix(&tonk, &repository.did()).await {
        Ok(prefix) => {
            if let Err(error) =
                super::customer::provision_consumer(&tonk, &repository.did(), &prefix, None).await
            {
                log!("Invite for repo '{repo_name}': provisioning skipped: {error}");
            }
        }
        Err(error) => {
            log!("Invite for repo '{repo_name}': no prefix to provision with: {error}");
        }
    }

    // A ready-to-append URL query suffix (`&remote=<percent-encoded-url>`).
    // The share view appends it verbatim between `?access=…` and the `#seed`.
    let encoded_access: String =
        url::form_urlencoded::byte_serialize(remote_execution.access_url.as_str().as_bytes())
            .collect();
    let remote = format!("&remote={encoded_access}");

    // Mint a fresh membership keypair. Its private seed becomes the invite
    // URL's `#` fragment; its public DID is the audience the repo access is
    // delegated to. The browser never sees this DID.
    let (signer, seed_bytes) = super::create_invite::generate_ephemeral().await?;
    let membership_did = signer.did();
    let seed = bs58::encode(seed_bytes).into_string();

    let delegation: dialog_ucan::UcanDelegation = tonk
        .profile
        .access()
        .claim(Subject::from(repository.did()).attenuate(Use))
        .delegate(membership_did)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;

    // Derive the invitation record from the chain as minted — before it's
    // serialized away — so the meta-branch roster carries this invite. The
    // claim side self-heals a missing record, but the mint should write its
    // own. Guaranteed `Some`: the delegation is scoped to the repo subject.
    let chain = delegation.into_chain();
    let invitation =
        Invitation::from_chain(&chain).expect("invite delegation is scoped to a specific subject");
    let execution = InvitationExecution::new(&invitation, "open");

    // base58-encode the delegation chain — the `?access=` parameter the
    // view reads back and assembles into the final URL.
    let chain_bytes = chain.to_bytes().map_err(|e| {
        TonkWorkerError::Internal(format!("failed to serialize delegation chain: {e}"))
    })?;
    let proof = bs58::encode(&chain_bytes).into_string();

    // Assemble the invite URL the recipient opens. Built here rather than
    // concatenated in the view template so there is exactly one definition
    // of an invite URL, and so it can be shortened — an async round-trip a
    // template can't make.
    let link = invite_url(&proof, &remote, &seed).await;

    let authorization = Authorization {
        this: subject_entity.clone(),
        proof: Proof(proof),
        remote: AuthorizationRemote(remote),
    };

    // Write the private seed and the assembled URL into the session overlay
    // and schedule a poll of this branch so the change propagates even
    // though it never commits durably. Neither reaches replicated storage:
    // the URL carries the seed in its `#` fragment, so it is exactly as
    // secret as the seed and lives on the same overlay-only concept.
    // `Credential` is cardinality-one keyed on the subject, so asserting
    // supersedes any prior credential in place — no whole-overlay clear,
    // which would also drop the tab's `tonk:site` fact and collapse the
    // share view to "not found".
    tonk.reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .overlay()
        .assert(Credential {
            this: subject_entity.clone(),
            seed: Seed(seed),
            link: Link(link.clone()),
        })
        // The same answer in the shape the share control subscribes to:
        // one row per space whose `status` says where the invite has got
        // to, carrying the url once there is one. `Credential` keeps the
        // seed beside it for readers that need both; this is what a view
        // renders. See `plan/share-intent.md`.
        .assert(tonk_schema::command::InviteState::granted(
            subject_entity,
            link,
        ))
        .write()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to write credential overlay: {e}"))
        })?;

    // Ensure the self-identity overlay (`state:self`) is present so the
    // topbar identity chip renders. The overlay builder above no longer
    // clears the whole overlay (which previously wiped `state:self` and the
    // tab's `tonk:site`), so this is a guarantee, not a recovery: if no
    // sync-status poll has stamped it yet, this fills it in.
    crate::router::sync::publish_self_identity(&tonk, repo_name, CONTENT_BRANCH).await;

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

    // Record the invitation on the repo's content branch — the durable
    // roster half of the invite (the URL with its secret fragment is never
    // stored). Mirrors the HTTP `create_invite` route so both mint paths
    // leave the same roster fact for the claim side to match against, and
    // routes through the *reactor's* cached handle for the same reason the
    // `Authorization` commit above does: a commit on a separately-opened
    // handle would leave the cached one pinned at a stale head.
    tonk.reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(invitation)
        .assert(execution)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to record invitation: {e}")))?;

    super::create_invite::retain_invite_authority(&tonk, repo_name, &chain).await?;

    log!("Minted invitation for repo '{}'", repo_name);
    Ok(RunInvite::Settled)
}

/// Record why a share click could not mint, on the space's content-branch
/// session overlay, keyed by the subject.
///
/// Overlay-only, exactly like the `Credential` a successful mint writes: a
/// refusal is this device's answer to this click, not a property of the space,
/// and it must never replicate. The write schedules a poll, so the dispatcher's
/// drain fans it out to the share control's subscription in the same pass as a
/// successful mint would have been.
///
/// `time` echoes the refused command's timestamp. The fact is cardinality-one
/// on the subject, so it lingers and replays on every resubscribe; the echo is
/// what lets the control tell this refusal from a replay of an older one, which
/// is why the fact never needs retracting.
/// The `invite:*` status a refusal code becomes.
///
/// Pinned by `it_keeps_a_repairable_refusal_open`.
///
/// Only reasons nothing can repair are terminal. `not-synced` and
/// `needs-account` are answered by attaching a remote or making an
/// account, so the request stays open rather than reporting a failure
/// the user is in the middle of fixing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn invite_status_for(code: &str) -> &'static str {
    use tonk_schema::command::InviteState;
    use tonk_worker_api::share;
    match code {
        share::BLOCKED_SUSPENDED => InviteState::SUSPENDED,
        share::BLOCKED_UNSHAREABLE_REMOTE => InviteState::UNSHAREABLE,
        // Repairable, or an attach that can be retried.
        _ => InviteState::REQUESTED,
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish_share_blocked(
    state: &AppState,
    repo_name: &str,
    subject: dialog_artifacts::Entity,
    code: &str,
    detail: &str,
    time: f64,
) {
    use tonk_schema::command::ShareBlocked;
    use tonk_schema::domain::share;

    let tonk = state.read().await;
    if let Err(error) = tonk
        .reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .overlay()
        .assert(ShareBlocked {
            this: subject.clone(),
            blocked: share::Blocked(code.to_owned()),
            detail: share::Detail(detail.to_owned()),
            time: share::Time(time),
        })
        // The same refusal in the shape the share control subscribes to.
        // Only a terminal reason becomes a terminal status: a refusal
        // the user can repair (no account yet, no remote yet) leaves the
        // request open, because the click has not finished failing — it
        // is waiting on something. See `plan/share-intent.md`.
        .assert(tonk_schema::command::InviteState::denied(
            subject,
            invite_status_for(code),
        ))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("failed to publish share refusal for '{repo_name}': {error}");
    }
}

/// Assemble the invite URL a recipient opens, shortened when the
/// shortcut service answers.
///
/// The long form is `{origin}/join?access={proof}{remote}#{seed}`, where
/// `remote` is already a ready-to-append `&remote=…` suffix. It is never
/// empty: a repo with no shareable remote is refused before any of this
/// runs, because an invite that carries no remote strands its recipient in
/// a space that can never fill. This is the shape the share view used to
/// concatenate from three overlay fields; building it here gives it one
/// definition and lets it be shortened.
///
/// Shortening is best-effort: a failed `PUT /@` (offline, no service
/// deployed, a non-2xx) logs and yields the long URL, which is fully
/// functional. Minting must not fail because a convenience failed.
///
/// The origin comes from the service worker's own scope, which is the only
/// origin that can serve the shortcut's relative redirect back — and the
/// origin the recipient will actually load. It is read here rather than
/// taken from the page because a sealed guest's `window.location.origin` is
/// the opaque `"null"`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn invite_url(proof: &str, remote: &str, seed: &str) -> String {
    let long = long_invite_url(worker_origin().as_deref(), proof, remote, seed);

    match super::create_invite::shorten(&long).await {
        Ok(short) => short,
        Err(e) => {
            log!("invite shortcut failed; using the full URL: {e}");
            long
        }
    }
}

/// The service worker's own origin, or `None` outside a worker scope.
///
/// Split out so [`long_invite_url`] stays pure and testable: the browser
/// test harness runs in a *window*, never a `ServiceWorkerGlobalScope`, so
/// a test driving `invite_url` could only ever reach the no-origin branch.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(super) fn worker_origin() -> Option<String> {
    use wasm_bindgen::JsCast;

    js_sys::global()
        .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
        .ok()
        .map(|global| global.location().origin())
        .filter(|origin| !origin.is_empty())
}

/// Assemble the long (un-shortened) invite URL.
///
/// With an origin: `{origin}/join?access={proof}{remote}#{seed}`. Without
/// one there is no worker scope to read (and so no service to shorten
/// against either), so it falls back to the same base the HTTP mint path
/// defaults to — still a well-formed, redeemable invite.
///
/// `remote` is already a ready-to-append `&remote=…` suffix. It is never
/// empty: `run_invite` refuses to mint at all when the repository has no
/// shareable remote (see `RemoteRefusal`), so by the time this runs the
/// repo has one. The seed is the fragment and never the query: it must not
/// reach a server, and the shortcut service is handed only the path + query.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn long_invite_url(origin: Option<&str>, proof: &str, remote: &str, seed: &str) -> String {
    match origin {
        Some(origin) => format!("{origin}/join?access={proof}{remote}#{seed}"),
        None => {
            log!("invite: no worker origin; using the default base");
            format!(
                "{}?access={proof}{remote}#{seed}",
                tonk_invite::DEFAULT_BASE_URL
            )
        }
    }
}

/// Post-commit handler for the [`PauseSync`] command.
///
/// Toggles auto-sync for the *origin* space: reads the durable
/// [`ReplicaSyncEnabled`] preference at the `state:here` singleton, flips it
/// (`active` ⇄ `paused`, defaulting an absent fact to "pause"), and commits the
/// new value on the origin's content branch. On pause it stamps `sync:paused`
/// into the live-status overlay so the chip and banner update at once; on
/// resume it leaves the overlay for the next status sweep (which resumes now
/// that the gate is open).
///
/// The preference lives on the space's content branch — not the profile meta —
/// so the sealed-guest chip can read it (it can only reach the branch the
/// `<tonk-portal>` is mounted under) and so the service worker's background
/// sweep can gate on it (the same branch it syncs). Keyed on `state:here`, the
/// same singleton the live status uses, so both fold into one chip
/// subscription.
///
/// A custom handler (not a plain `Provider<PauseSync>`) because it reads and
/// writes durable branch state the decoded command doesn't carry and targets
/// the repo from the origin rather than a command field — like
/// [`InviteHandler`].
///
/// [`PauseSync`]: tonk_schema::command::PauseSync
/// [`ReplicaSyncEnabled`]: tonk_schema::ReplicaSyncEnabled
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct PauseSyncHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl PauseSyncHandler {
    /// Cache `PauseSync`'s trigger attributes (its `time` field) so the
    /// registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::PauseSync::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for PauseSyncHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::PauseSync::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;
        use tonk_schema::prelude::DidExt as _;

        // Decode synchronously to read the target space off the command — the
        // handler flips THAT space's replica, not the dispatch origin's, so the
        // command can be dispatched from the profile branch. The repo key is
        // the space DID's suffix; a space's content branch is always `main`.
        let target = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::PauseSync::decode(entity, facts))
            .and_then(|command| {
                command
                    .space
                    .0
                    .to_string()
                    .parse::<dialog_varsig::Did>()
                    .ok()
            })
            .map(|did| did.repo_key().to_owned());
        let env = env.clone();

        Box::pin(async move {
            let Some(repo) = target else {
                log!("PauseSync: no/unparseable target space, skipping");
                return;
            };
            let branch = CONTENT_BRANCH.to_string();
            log!("command PauseSync repo={} branch={}", repo, branch);

            if let Err(error) = run_pause_sync(&env, &repo, &branch).await {
                log!("PauseSync for repo '{}' failed: {}", repo, error);
            }
        })
    }
}

/// Post-commit handler for the [`ProfileRename`] command.
///
/// Fired when the topbar identity chip's `<tonk-editable>` commits a
/// transient [`ProfileRename`]. It persists the new display name as a
/// durable [`ProfileName`] override on the profile's meta branch, then
/// re-stamps the self member's [`MemberName`] on every space the profile
/// belongs to so all of its rosters reflect the new name at once.
///
/// The new name is the only payload (read from `currentTarget.value`);
/// the spaces to re-stamp come from the profile's replica index on the
/// meta branch, and the [`CommandEnv::origin`](crate::router::CommandEnv::origin)
/// space is also used to refresh the self-identity overlay. An
/// empty/whitespace name is a no-op — a member can't
/// blank their own name out.
///
/// A custom handler (not a plain `Provider<ProfileRename>`) because it
/// writes durable branch state the decoded command doesn't carry and
/// targets the repo from the origin rather than a command field — like
/// [`InviteHandler`]/[`PauseSyncHandler`].
///
/// [`ProfileRename`]: tonk_schema::command::ProfileRename
/// [`ProfileName`]: tonk_schema::ProfileName
/// [`MemberName`]: tonk_schema::MemberName
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct ProfileRenameHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl ProfileRenameHandler {
    /// Cache `ProfileRename`'s trigger attributes (its `name` field) so
    /// the registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::ProfileRename::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for ProfileRenameHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::ProfileRename::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock), then
        // hand the owned new name + an env clone to the `'static` future.
        let name = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::ProfileRename::decode(entity, facts))
            .map(|command| command.name.0);
        let key = env.origin().repo.clone();
        let env = env.clone();

        Box::pin(async move {
            let Some(name) = name else {
                return;
            };
            let name = name.trim();
            // Don't let a member blank their own name out.
            if name.is_empty() {
                return;
            }
            log!("command ProfileRename repo={} name={}", key, name);

            if let Err(error) = run_profile_rename(&env, name).await {
                log!("ProfileRename for repo '{}' failed: {}", key, error);
            }
        })
    }
}

/// Persist the display-name override on the profile meta branch and
/// re-stamp `MemberName` on every space's content branch.
///
/// Split out from [`ProfileRenameHandler::run`] so the `?` early-return
/// funnels into the single `log!` there — the command future itself
/// returns `()`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_profile_rename(
    env: &crate::router::CommandEnv,
    name: &str,
) -> Result<(), TonkWorkerError> {
    let tonk = env.state().read().await;
    crate::router::account_state::rename_display_name(&tonk, name).await?;

    // Prompt an immediate push so peers see the new name without waiting for
    // the heartbeat. Linked and unlinked paths both queue their durable writes
    // before this compatibility notification.
    drop(tonk);
    crate::router::join::notify_sync(env.client());
    Ok(())
}

/// Outcome of a rename, surfaced rather than swallowed.
///
/// `PauseSyncHandler` logs and returns on a missing replica. Rename must not:
/// a silently-dropped rename looks successful to the user, which is the
/// failure class this whole design attacks.
///
/// Compiled for the wasm handler that uses it and for native tests (see
/// [`rename_outcome`]) — never for a plain native build, where it would sit
/// unused and trip the `-D warnings` dead-code lint.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RenameOutcome {
    /// The rename committed.
    Renamed,
    /// The rename did not commit; the caller must not treat this as success.
    Failed,
}

/// Map a rename result to an outcome the chip can reflect.
///
/// Pure and native-testable — the handler around it is wasm-gated, so this is
/// the seam where the "do not swallow a failed rename" decision is pinned.
/// Any error is `Failed`: `RepositoryError` carries no `NotFound` variant, so
/// an absent replica arrives as `Internal` from the acquire, and the chip's
/// response is the same either way — revert, do not show a phantom success.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub(crate) fn rename_outcome(result: Result<(), RepositoryError>) -> RenameOutcome {
    match result {
        Ok(()) => RenameOutcome::Renamed,
        Err(_) => RenameOutcome::Failed,
    }
}

/// Post-commit handler for the [`RenameRepository`] command.
///
/// The space-side `tonk/rename-repository` rule (`core.yaml`) binds the
/// command's `subject` to `?this` and asserts the new name directly — but
/// that rule lives on the space's OWN branch, so it can never see a claim
/// dispatched from the profile branch. This handler is the worker-side
/// replacement: it reads the target `space` off the command (like
/// [`PauseSyncHandler`]) rather than the dispatch origin, so the FAB's name
/// chip can dispatch from the profile branch with nothing seeded per-space.
///
/// [`RenameRepository`]: tonk_schema::command::RenameRepository
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct RenameRepositoryHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl RenameRepositoryHandler {
    /// Cache `RenameRepository`'s trigger attributes so the registry indexes
    /// this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::RenameRepository::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for RenameRepositoryHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::RenameRepository::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;
        use tonk_schema::prelude::DidExt as _;

        // Decode synchronously to read the target space off the command — the
        // handler renames THAT repository, not the dispatch origin's, so the
        // command can be dispatched from the profile branch.
        let decoded = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::RenameRepository::decode(entity, facts));
        let env = env.clone();

        Box::pin(async move {
            let Some(command) = decoded else { return };
            let Ok(did) = command.space.0.to_string().parse::<dialog_varsig::Did>() else {
                log!("RenameRepository: unparseable target space, skipping");
                return;
            };
            // `repo_key()` is the FULL DID, not a suffix.
            let repo = did.repo_key().to_owned();
            log!("command RenameRepository repo={}", repo);

            let result = run_rename_repository(&env, &repo, &command.name.0).await;
            let failure_detail = result.as_ref().err().map(ToString::to_string);
            if rename_outcome(result) == RenameOutcome::Failed {
                log!(
                    "RenameRepository for repo '{}' failed: {}",
                    repo,
                    failure_detail.unwrap_or_default()
                );
            }
        })
    }
}

/// Assert the repository's own [`RepositoryName`] on its content branch,
/// keyed by the subject DID — the same fact the space-side
/// `tonk/rename-repository` rule used to write. Split out from
/// [`RenameRepositoryHandler::run`] so the caller funnels every failure
/// through [`rename_outcome`] rather than a bare `?`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_rename_repository(
    env: &crate::router::CommandEnv,
    repo: &str,
    name: &str,
) -> Result<(), RepositoryError> {
    use tonk_schema::prelude::DidExt as _;

    let tonk = env.state().read().await;

    // The durable key: the repository's own subject DID, read straight off
    // the branch handle rather than re-parsed from `repo` (they're the same
    // DID either way).
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("{repo}/{CONTENT_BRANCH} not found: {e}"))
        })?;
    let subject = session.handle().of().this();

    log!("RenameRepository repo={} name={}", repo, name);

    // Commit the new name through the reactor so subscriptions re-poll. `name`
    // is cardinality-one, so the assert supersedes the prior value — the same
    // fact the standard-library rule wrote.
    tonk.reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(RepositoryName {
            this: subject,
            name: tonk_schema::domain::repo::Name(name.to_string()),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("failed to commit repository name: {e}")))?;

    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    // Mirror the new name into the account directory so devices that
    // have not replicated this space still label it correctly.
    if let Ok(subject) = repo.parse::<Did>()
        && let Err(error) = tonk
            .reactor
            .profile_repository()
            .branch(PROFILE_BRANCH)
            .transaction()
            .assert(tonk_schema::SpaceName::new(&subject, name))
            .commit()
            .perform(&tonk.operator)
            .await
    {
        log!("RenameRepository directory mirror skipped: {error}");
    }
    Ok(())
}

/// Post-commit handler for the [`RemoveSpace`] command.
///
/// Fired when the user confirms a Hub row's delete overlay. Removal is
/// device-local and ordered so the visible state commits first and
/// cleanup is best-effort behind it — see [`remove_space_inner`].
///
/// A custom handler (not a plain `Provider<RemoveSpace>`) for the same
/// reason as [`CreateSpaceHandler`]: the work needs the profile handle,
/// the reactor cache, and storage, reached through state rather than
/// carried by the decoded command.
///
/// `run` refuses any transient whose origin repo is non-empty. This is
/// the first *destructive* command reachable through shape-matched
/// cross-branch dispatch: `dom.event.current-target.dataset/remove` is
/// just an attribute name, so the same-shaped fact committed on ANY
/// content branch — a joined space's own notation, or a same-origin
/// POST to that repo's `/transact` — would otherwise let it name and
/// delete any space by DID, regardless of where the command actually
/// fired. The Hub's delete form commits on the profile branch, whose
/// origin `repo` is always empty (`transact_profile` in `transact.rs`
/// never names a repo — the same reasoning `transact_profile`'s
/// sealed-guest check relies on), so refusing a non-empty origin is
/// exactly "only the Hub can fire this."
///
/// [`RemoveSpace`]: tonk_schema::command::RemoveSpace
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct RemoveSpaceHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl RemoveSpaceHandler {
    /// Cache `RemoveSpace`'s trigger attributes (its `subject` field) so
    /// the registry indexes this handler under them.
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::RemoveSpace::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for RemoveSpaceHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::RemoveSpace::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;

        // Decode synchronously (the caller still holds the lock), then
        // hand the owned subject + an env clone to the `'static` future.
        let subject = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::RemoveSpace::decode(entity, facts))
            .map(|command| command.subject.0);
        let env = env.clone();

        Box::pin(async move {
            let Some(subject) = subject else {
                return;
            };
            // See the type doc: only the profile branch (empty origin
            // repo) may fire this. A non-empty origin means the fact came
            // from a content branch — matched by shape, not by who asked —
            // so it is ignored rather than trusted to remove anything.
            if !env.origin().repo.is_empty() {
                log!(
                    "RemoveSpace ignored: origin '{}' is not the profile branch",
                    env.origin().repo
                );
                return;
            }
            log!("command RemoveSpace subject={}", subject);
            let subject: Did = match subject.to_string().parse() {
                Ok(did) => did,
                Err(error) => {
                    log!("RemoveSpace: '{}' is not a DID: {}", subject, error);
                    return;
                }
            };
            if let Err(error) = remove_space_inner(env.state(), &subject).await {
                log!("RemoveSpace '{}' failed: {}", subject, error);
            }
        })
    }
}

/// Remove a space device-locally, in three ordered steps:
///
/// 1. Retract its replica record from the profile meta branch
///    ([`remove_replica_from_profile`]) — the Hub row's source of
///    truth, so the space disappears immediately. This is the commit
///    point; everything after is cleanup.
/// 2. Evict the repository from the reactor cache
///    ([`Reactor::evict`](crate::Reactor::evict)) and forget it in the
///    sync work-queue ([`SyncQueue::forget`](crate::router::SyncQueue::forget)).
///    The background sync sweep unions the reactor cache with the dirty
///    set (see `drain_sync`), so both must drop the repo — a leftover
///    dirty stamp alone would resurrect it on the next drain even after
///    eviction.
/// 3. Delete local storage ([`delete_space_storage`]) — best-effort
///    and outside the state lock; a failure only orphans invisible
///    bytes, so it is logged, never surfaced. Re-evicted once more
///    afterward (see below) since the unlocked delete leaves a window
///    for a concurrent drain to re-acquire the repo.
///
/// The self-replica (subject == profile) is refused: its row is hidden
/// chrome in the Hub, and deleting the profile's own storage would take
/// every space with it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn remove_space_inner(
    state: &AppState,
    subject: &Did,
) -> Result<(), RepositoryError> {
    {
        let tonk = state.write().await;
        if let Err(error) = require_real_space(&tonk, subject).await
            && replica_still_recorded(&tonk, subject).await?
        {
            return Err(RepositoryError::Internal(error.to_string()));
        }
        remove_replica_from_profile(&tonk, subject).await?;
        // Drain the poll the retraction scheduled so the Hub's meta
        // subscription reflects the removal (mirrors set_replica_status).
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        tonk.reactor.evict(subject.repo_key());
        // Same repo, same lock: a dirty stamp left in the sync queue would
        // otherwise survive eviction and, on the next drain, get folded
        // into the pull set that resurrects the reactor cache entry.
        tonk.sync_queue.forget(subject.repo_key());
    }
    // Storage cleanup after the lock is released — the delete awaits
    // browser IO and must not stall other requests.
    //
    // A space's storage is keyed by routing key alone, with no profile
    // prefix, so two profiles replicating one space SHARE its storage.
    // With more than one profile on this browser the delete is skipped
    // (the replica rows above are still removed): the failure mode is
    // leaked storage, never data loss — blocks are re-fetchable for
    // sync-enabled spaces. A precise guard that consults the other
    // profiles' replica indexes is deferred. An unreadable roster skips
    // too, since sharing can't be ruled out.
    let other_profiles = {
        let tonk = state.read().await;
        match tonk
            .registry
            .read_roster(&tonk.storage, &tonk.operator)
            .await
        {
            Ok(roster) => roster.len() > 1,
            Err(error) => {
                log!("profile roster unreadable before storage delete: {error}");
                true
            }
        }
    };
    if other_profiles {
        log!(
            "keeping storage for '{}': another profile on this browser may replicate it",
            subject.repo_key()
        );
    } else {
        let _ =
            wasm_bindgen_futures::JsFuture::from(delete_space_storage(subject.repo_key())).await;
    }

    // The delete ran unlocked, so a concurrent `drain_sync` could have
    // reached in and re-acquired the repo (e.g. to pull) while it was in
    // flight — resurrecting the cache entry and, since the IDB open races
    // the delete, potentially recreating an empty database right behind
    // it. Re-evict now that the delete has settled to drop any such
    // handle.
    {
        let tonk = state.write().await;
        tonk.reactor.evict(subject.repo_key());
    }
    Ok(())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn require_real_space(tonk: &TonkState, subject: &Did) -> Result<(), TonkWorkerError> {
    let entity = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();
    let meta = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open profile meta: {error}")))?;
    let rows: Vec<Replica> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::from(entity),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("replica kind query: {error:?}")))?;
    if rows
        .iter()
        .any(|replica| replica.kind == Replica::repository_kind())
    {
        Ok(())
    } else {
        Err(TonkWorkerError::Forbidden(
            "system replicas are ineligible for user-space controls".to_string(),
        ))
    }
}

/// Retract every fact keyed on `subject`'s replica entity from the
/// profile repository's meta branch — the reverse of
/// [`record_replica_in_profile`]. Selecting the entity's actual claims
/// (rather than re-asserting typed concepts to retract) sweeps every
/// stamp regardless of vintage — the `Replica` fields, `SpaceStatus`,
/// a migration's `SpaceKind`, a legacy `name` — without knowing their
/// current values.
///
/// Reads and writes through the reactor's cached profile handle for the
/// same reason `record_replica_in_profile` does: the Hub reads through
/// that handle, so a commit on a separate handle would be invisible to
/// it. Broadcasts `/api/profile` like the record path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn remove_replica_from_profile(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), RepositoryError> {
    use dialog_artifacts::ArtifactSelector;
    use futures_util::StreamExt as _;

    let entity = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();

    let meta = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("open profile meta: {e}")))?;

    // Removal is account-wide: profile main is shared account state, so
    // EVERY device's replica row for this subject is swept, not just
    // this device's — a surviving foreign row would resurrect the
    // directory entry through the next sweep's backfill. This device's
    // derived entity rides along in case its row is gone but stray
    // stamps remain.
    let rows: Vec<Replica> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::from(tonk_schema::domain::replica::Subject(subject.this())),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("replica rows query: {e:?}")))?;
    let mut entities: Vec<dialog_artifacts::Entity> =
        rows.into_iter().map(|row| row.this).collect();
    if !entities.contains(&entity) {
        entities.push(entity);
    }

    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction();
    let mut found = false;
    for row_entity in entities {
        let stream = meta
            .handle()
            .claims()
            .select(ArtifactSelector::new().of(row_entity))
            .perform(&tonk.operator)
            .await
            .map_err(|e| RepositoryError::Internal(format!("select replica claims: {e}")))?;
        tokio::pin!(stream);
        while let Some(artifact) = stream.next().await {
            let artifact = artifact
                .map_err(|e| RepositoryError::Internal(format!("read replica claim: {e}")))?
                .to_owned()
                .map_err(|e| RepositoryError::Internal(format!("read replica claim: {e}")))?;
            found = true;
            transaction = transaction.retract(super::claim::RawClaim {
                the: artifact.the,
                of: artifact.of,
                is: artifact.is,
                unique: false,
            });
        }
    }

    // The account-level directory entry hangs on the repository's own
    // entity, so it needs its own sweep — filtered to the space
    // namespace, because other facts may key on that entity too.
    // Removing it is what makes "delete space" account-wide: every
    // device's Hub lists the directory, not this device's replica row.
    let directory = meta
        .handle()
        .claims()
        .select(ArtifactSelector::new().of(subject.this()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("select directory claims: {e}")))?;
    tokio::pin!(directory);
    while let Some(artifact) = directory.next().await {
        let artifact = artifact
            .map_err(|e| RepositoryError::Internal(format!("read directory claim: {e}")))?
            .to_owned()
            .map_err(|e| RepositoryError::Internal(format!("read directory claim: {e}")))?;
        if !artifact.the.to_string().starts_with("xyz.tonk.space/") {
            continue;
        }
        found = true;
        transaction = transaction.retract(super::claim::RawClaim {
            the: artifact.the,
            of: artifact.of,
            is: artifact.is,
            unique: false,
        });
    }
    if !found {
        // Nothing recorded — a stale row or a repeated submit. Not an
        // error: the desired end state (no record) already holds.
        log!("remove replica: no facts for {} in profile meta", subject);
        return Ok(());
    }

    let revision = transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("retract replica record: {e}")))?;

    broadcast(
        "/api/profile",
        &Notification {
            branch: PROFILE_BRANCH.to_string(),
            revision,
        },
    );
    Ok(())
}

/// Delete a space's local storage: its IndexedDB database (archive,
/// memory, credential, certificate object stores) and, best-effort, an
/// OPFS blob subtree at `current/<key>` — the path dialog-storage's
/// FileSystem provider would use under its `Directory::Current`
/// mapping, if a `WebSpace` wired one up. At the currently pinned
/// dialog-storage revision it doesn't: the web space keeps everything
/// in the IndexedDB database, so the OPFS removal below is a
/// forward-compatible no-op that quietly settles via its `catch` when
/// the directory doesn't exist. The database name is exactly the
/// routing key.
///
/// Inline JS rather than web-sys: `deleteDatabase` and recursive
/// `removeEntry` have no plumbing here, and the whole operation is two
/// promise chains. Never rejects — each half settles on error/absence.
/// `onblocked` also resolves: the worker's own pooled connection closes
/// itself on the `versionchange` the delete fires (see
/// [`crate::patch_idb_versionchange`]), after which the browser
/// completes the delete; waiting for the completion event would hang if
/// another tab pins the database open.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function delete_space_storage(name) {
    const database = new Promise((resolve) => {
        const request = indexedDB.deleteDatabase(name);
        request.onsuccess = request.onerror = request.onblocked = () => resolve();
    });
    const blobs = navigator.storage.getDirectory()
        .then((root) => root.getDirectoryHandle('current'))
        .then((dir) => dir.removeEntry(name, { recursive: true }))
        .catch(() => {});
    return Promise.all([database, blobs]);
}
"#)]
extern "C" {
    /// Delete the IndexedDB database and OPFS blob directory for a
    /// space's routing key. Resolves once both halves settle; never
    /// rejects.
    fn delete_space_storage(name: &str) -> js_sys::Promise;
}

/// Delete the storage a legacy hidden account repository left behind.
/// Its content synced with the same remote profile main now follows, so
/// everything it held is recoverable by pulling.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn delete_legacy_storage(key: &str) {
    let _ = wasm_bindgen_futures::JsFuture::from(delete_space_storage(key)).await;
}

/// Toggle the durable `enabled` preference on the replica and publish the
/// matching live status to the chip's overlay.
///
/// The preference is a per-replica boolean keyed on this device's replica
/// entity (`(profile, subject)`), committed on the space content branch — the
/// branch the SW syncs. The chip reads the `status` overlay (`state:here`, same
/// branch), so the command also publishes status on BOTH pause and resume so
/// the chip reflects the change immediately.
///
/// Split out from [`PauseSyncHandler::run`] so the `?` early-return funnels
/// into the single `log!` there.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn run_pause_sync(
    env: &crate::router::CommandEnv,
    repo: &str,
    branch: &str,
) -> Result<(), TonkWorkerError> {
    use tonk_schema::ReplicaSyncEnabled;

    let tonk = env.state().read().await;

    // The durable key: this device's replica entity, derived from `(profile,
    // subject)` — the subject DID comes straight off the branch handle.
    let session = tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("{repo}/{branch} not found: {e}")))?;
    let subject = session.handle().of().clone();
    require_real_space(&tonk, &subject).await?;
    let replica = Replica::new(tonk.profile.did(), subject).this().clone();

    // Toggle: read the current preference (absent → enabled, so a first click
    // pauses), flip it.
    let was_enabled = super::sync::is_sync_enabled(&tonk, repo, branch).await;
    let now_enabled = !was_enabled;
    log!(
        "PauseSync repo={} {} -> {}",
        repo,
        if was_enabled { "enabled" } else { "paused" },
        if now_enabled { "enabled" } else { "paused" }
    );

    // Commit the new preference durably on the content branch, keyed on the
    // replica entity. Through the reactor so subscriptions re-poll. `enabled` is
    // cardinality-one, so the assert supersedes the prior value.
    tonk.reactor
        .repository(repo)
        .branch(branch)
        .transaction()
        .assert(ReplicaSyncEnabled::new(replica, now_enabled))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to commit sync preference: {e}")))?;

    // Update the chip's status overlay on the space branch — on both pause and
    // resume. On pause we stamp `paused` (a paused replica runs no sweep to
    // publish it). On resume we stamp `pending`; the controller's next status
    // sweep settles it to the real state (idle / local / offline).
    if now_enabled {
        super::sync::publish_sync_status_attr(&tonk, repo, branch, Replica::pending_status()).await;
    } else {
        super::sync::publish_paused_status(&tonk, repo, branch).await;
    }

    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
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
fn space_config(remote: &str) -> Result<RepositoryConfiguration, RepositoryError> {
    use dialog_remote_ucan_s3::UcanAddress;

    let remote = remote.trim();
    if remote.is_empty() {
        return Ok(
            RepositoryConfiguration::default().branch("main", BranchConfiguration::default())
        );
    }
    let address = SiteAddress::from(UcanAddress::new(remote));
    Ok(RepositoryConfiguration::default()
        .remote("origin", RemoteConfiguration::new(address))
        .branch(
            "main",
            BranchConfiguration::default().upstream("origin", "main"),
        ))
}

/// Where a space on this account syncs, when nothing named a remote.
///
/// The account's recorded provider is the authority — the access service
/// names it in the registration receipt. It is written by whatever last
/// talked to the service, though, and a space created in the moment
/// after activation can beat that write; the account descriptor names
/// the same deployment and is recorded at link time, so it answers while
/// the fact catches up rather than leaving the space local-only on a
/// race.
///
/// Shared by both creation paths so they cannot disagree about where a
/// space syncs.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn account_sync_remote(tonk: &TonkState) -> Option<String> {
    super::account_state::account_remote(tonk).await.ok()
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
///
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
    let configuration = space_config(remote)?;

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

    // Provision before attaching. Creation only provisions when there is
    // an active customer to provision under, so a space created during
    // onboarding has no consumer row — and an upstream attached without
    // one syncs to `subject is provisioned by an active customer (the
    // subject is not provisioned)` on every presign. This is where a
    // local-only space earns its remote, so it is where the consumer row
    // has to be created.
    //
    // Best effort, not fatal. A remote is not necessarily OUR access
    // service — a self-hosted endpoint or a test server is attached the
    // same way, and `/provider/add` against our own service is beside
    // the point there. Refusing the attach on a failed provision would
    // make those unreachable, so the attach proceeds and a space that
    // does need provisioning surfaces it as a refused presign rather
    // than a refused attach.
    match space_root_prefix(&tonk, &repository.did()).await {
        Ok(prefix) => {
            if let Err(error) =
                super::customer::provision_consumer(&tonk, &repository.did(), &prefix, None).await
            {
                log!("enable sync '{key}': provisioning skipped: {error}");
            }
        }
        Err(error) => {
            log!("enable sync '{key}': no root delegation to consent with: {error}")
        }
    }

    let effective = ensure_remote_config(&tonk, &repository, key, &configuration).await?;

    // Mirror the EFFECTIVE mount configuration into the account
    // directory so other devices adopt what this device actually
    // syncs against: an already-configured upstream is preserved, so
    // the request's (possibly repair-supplied) address must not
    // overwrite it there.
    record_space_mount(&tonk, &repository.did(), &effective, None).await;

    Ok(())
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

/// Whether `subject` still has a recorded [`Replica`] on the profile's
/// meta branch. The replica entity is content-derived from `(profile,
/// subject)` — the same hash [`Replica::new`] uses (see
/// [`set_replica_status`]) — so its presence is checked directly rather
/// than searched for.
///
/// Guards [`seed_and_initialize`] against a `RemoveSpace` landing
/// mid-seed: [`remove_replica_from_profile`] retracts exactly this
/// record, so its absence means the space was removed while this seed
/// was in flight (either on the awaited create path or the detached
/// [`spawn_seed`] path).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn replica_still_recorded(tonk: &TonkState, subject: &Did) -> Result<bool, RepositoryError> {
    let entity = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();
    let meta = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("open profile meta: {e}")))?;
    let rows: Vec<Replica> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::from(entity),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("replica query: {e}")))?;
    Ok(!rows.is_empty())
}

/// If `subject`'s replica record is gone (see
/// [`replica_still_recorded`]), evict the repo from the reactor cache
/// (a mid-seed removal already evicted once, but the seed may have
/// re-acquired it since) and log; the caller returns early without
/// seeding or stamping. `stage` names the point being skipped, for the
/// log line.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn bail_if_space_removed(
    tonk: &TonkState,
    subject: &Did,
    key: &str,
    stage: &str,
) -> Result<bool, RepositoryError> {
    if replica_still_recorded(tonk, subject).await? {
        return Ok(false);
    }
    log!(
        "seed '{}': replica record gone (space removed mid-seed), skipping {}",
        key,
        stage
    );
    tonk.reactor.evict(key);
    Ok(true)
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
    // The seed can run long after the replica record was asserted (the
    // detached `spawn_seed` path, or just a slow library fetch on the
    // awaited create path), leaving a window for the user to remove the
    // space before it lands. Without this guard the seed would re-insert
    // the evicted reactor cache entry, recreate the just-deleted database
    // with seeded content, and re-stamp `SpaceStatus` on a retracted
    // entity. Checked again below, right before each status flip, since
    // removal can also land in the gap opened by the fetch/seed loop.
    {
        let tonk = state.read().await;
        if bail_if_space_removed(&tonk, subject, key, "seed").await? {
            return Ok(());
        }
    }

    if !branches.is_empty() {
        // The scaffold and the repository's name go in as ONE body, so the
        // rule engine saturates over the whole document in a single commit
        // per branch (the name flash fix).
        let scaffold = fetch_standard_library(STANDARD_LIBRARY_URL)
            .await
            .map_err(|e| {
                RepositoryError::Internal(format!("fetch '{STANDARD_LIBRARY_URL}': {e}"))
            })?;

        let name_body = repository_name_body(subject, display_name)?;
        let tonk = state.read().await;
        for branch_name in branches {
            let body = format!("{scaffold}\n{name_body}");
            seed_standard_library(&tonk, key, branch_name, &body)
                .await
                .map_err(|e| RepositoryError::Internal(format!("seed '{branch_name}': {e}")))?;
            log!(
                "Seeded scaffold + name on '{}' branch '{}'",
                key,
                branch_name
            );
        }
        // Cheap re-check right before stamping: the fetch/seed loop above
        // awaited, opening another window for a removal to land.
        if bail_if_space_removed(&tonk, subject, key, "status stamp").await? {
            return Ok(());
        }
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    } else {
        let tonk = state.read().await;
        if bail_if_space_removed(&tonk, subject, key, "status stamp").await? {
            return Ok(());
        }
        set_replica_status(&tonk, subject, Replica::initialized_status()).await?;
    }
    log!("Repository '{}' initialized", key);
    Ok(())
}

/// URL of the served standard-library notation asset, copied into
/// the dist from `tonk-core/assets/library/core.yaml` by trunk. Seeded
/// onto each space's content branch. Only referenced from the
/// SW-scoped background seed path, so it is wasm-only: the native tests
/// that also read it went with the template libraries.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const STANDARD_LIBRARY_URL: &str = "/library/core.yaml";

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
) -> Result<Repository, RepositoryError> {
    // A space always delegates to an ACCOUNT: the passkey-derived root
    // once one is persisted, else this device's onboarding account,
    // which is a real account custodied locally rather than by WebAuthn
    // (`plan/onboarding-accreditation.md`).
    //
    // It used to fall back to the profile's own device key, which made a
    // pre-account space differ in shape from every other one and left
    // `adopt_profile_spaces` to reconcile the difference at sign-in.
    // Delegating to an account from the start means enrolling a passkey
    // is an account key ROTATION, the same operation a compromised
    // passkey needs, rather than a bespoke migration.
    let owner = match super::identity::local_root(tonk).await {
        Ok(root) => root.root_did,
        Err(TonkWorkerError::RootRequired) => {
            // Minting the grant here as well as the account: the device
            // signs on the account's behalf, so a space delegated to an
            // account this device cannot prove for would be unusable.
            crate::onboarding::grant_device(tonk)
                .await
                .map_err(|error| {
                    RepositoryError::Internal(format!("failed to grant the device: {error}"))
                })?;
            crate::onboarding::did(tonk)
                .await
                .map_err(|error| {
                    RepositoryError::Internal(format!(
                        "failed to open the onboarding account: {error}"
                    ))
                })?
                .ok_or_else(|| {
                    RepositoryError::Internal(
                        "the onboarding account did not materialise".to_string(),
                    )
                })?
        }
        Err(error) => {
            return Err(RepositoryError::Internal(format!(
                "failed to load local root: {error}"
            )));
        }
    };

    // 1. Generate the repository's credential up front so its
    // `did:key` is its stable identity. The repository's routing
    // and storage key is that DID's suffix (`did.repo_key()`); the
    // user-typed `display_name` is only a label, seeded later into the
    // repository's own `tonk/repository` concept. Generating the signer
    // first (rather than letting `.create()` mint one) is what lets the
    // name derive from the DID instead of the other way around.
    // The seed is drawn here rather than inside `generate`, so it can be
    // sealed to the account below; the signer imports from it the same
    // way an account root does (non-extractable on the web target), so
    // the credential the repository stores is the shape it always was.
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut())
        .map_err(|e| RepositoryError::Internal(format!("Failed to generate signer: {}", e)))?;
    let signer = Ed25519Signer::import(&*seed)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to generate signer: {}", e)))?;
    let did = signer.did();
    let key = did.repo_key();

    // The seed sealed to the account is the ONLY copy of the space secret
    // that outlives this function: the repository stores the verifier, and
    // every later act on the space proves through `space -> account ->
    // device`, the way a joined replica does. So the custody row lands
    // before anything else does, and a seed that cannot be custodied is a
    // space that is not created.
    if !super::account_state::custody_seed(tonk, &did, SeedKind::Space, seed).await {
        return Err(RepositoryError::Internal(
            "the space seed could not be custodied under the account".to_string(),
        ));
    }

    let verifier: Ed25519Verifier = did.to_string().parse().map_err(|e| {
        RepositoryError::Internal(format!("space DID is not a valid Ed25519 did:key: {e:?}"))
    })?;
    let space_credential = Subject::from(tonk.profile.did())
        .attenuate(Space::new(key))
        .create(Credential::from(verifier))
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to create repository '{}': {}", key, e))
        })?;
    let repository = Repository::from(space_credential);
    log!("Repository created. DID: {}", repository.did());

    // 2. Delegate subject-specific authority to the owner key, from the
    //    signer this function still holds.
    let minter = Repository::from(signer);
    let delegation = minter
        .access()
        .claim(&minter)
        .delegate(owner.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to delegate repo access to profile: {}", e))
        })?;

    let prefix = delegation.into_chain();

    tonk.profile
        .access()
        .save(UcanDelegation(prefix.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("Failed to save repo delegation: {}", e)))?;
    // The same authority, retained into the account space. The profile's own
    // access branch above is what makes this space usable HERE; the account is
    // what makes it recoverable on the next device, since a device regains
    // access by pulling the account rather than by fetching an artifact.
    super::account_state::retain_space_delegation(tonk, &prefix).await;

    // The billing half of the same act: provision the new space as a
    // consumer of the access service, depositing the powerline as its
    // consent. Best effort for the same reason retain is — a space is
    // usable the moment its delegations exist locally.
    //
    // Only for an ACTIVE customer. A device has an account from first
    // boot (the onboarding account), so "an account exists" says nothing
    // about whether the access service will serve this subject: until
    // the user enrols an email and confirms it, `/provider/add` refuses
    // and the space would be left wired to a remote that answers 403 on
    // every presign. A space created in that window is local-only by
    // design, and the share button provisions it on demand.
    if super::customer::is_active(tonk).await {
        if let Err(error) =
            super::customer::provision_or_defer(tonk, &repository.did(), &prefix, None).await
        {
            log!("consumer provisioning skipped: {error}");
        }
    } else {
        log!(
            "space '{}' created local-only: no active customer to provision it under",
            repository.did()
        );
    }

    let prefix_bytes = prefix.to_bytes().map_err(|error| {
        RepositoryError::Internal(format!(
            "Failed to serialize space root delegation: {error}"
        ))
    })?;
    tonk.profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{}", repository.did()))
        .save(prefix_bytes)
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            RepositoryError::Internal(format!("Failed to persist space root delegation: {error}"))
        })?;

    // 3-7. Wire up the meta branch and register the replica. The
    // replica is a name-less membership index; its identity (`subject`)
    // is the repository DID. The `display_name` is only threaded for log
    // context — the name itself is seeded into the repository's own
    // `tonk/repository` concept by the caller's seed step.
    // The opener of a freshly created repo is its founder.
    record_repository_meta(
        tonk,
        &repository,
        display_name,
        configuration,
        MemberRole::FOUNDER,
    )
    .await?;

    Ok(repository)
}

/// Load the exact provider-neutral `space → root` prefix persisted at creation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn space_root_prefix(
    tonk: &TonkState,
    subject: &Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let bytes = tonk
        .profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{subject}"))
        .load::<Vec<u8>>()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            if crate::credential::is_missing(&error) {
                TonkWorkerError::NotFound(
                    "space root delegation is not persisted on this device".to_string(),
                )
            } else {
                TonkWorkerError::Internal(format!("failed to load space root delegation: {error}"))
            }
        })?;
    DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        TonkWorkerError::Internal(format!("stored space root delegation is invalid: {error}"))
    })
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
///
/// Does not touch the content-branch roster — see
/// [`record_repository_meta`] for the wrapper that also records
/// membership.
pub(crate) async fn record_replica_local_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    _display_name: &str,
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

    // Membership is NOT recorded here. The meta branch is device-local
    // and never replicates, so a roster on it would only ever show the
    // local profile. The shared roster lives on the content branch (see
    // `record_membership_on_content`), written by the create + claim paths.
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

        let remote = match repository
            .remote(remote_name.as_str())
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(remote) => {
                if remote.address().subject() != &subject
                    || remote.address().site() != &remote_config.address
                {
                    return Err(RepositoryError::InvalidConfiguration(format!(
                        "Remote '{}' is already configured differently",
                        remote_name
                    )));
                }
                remote
            }
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
                })?
            }
        };

        log!("Remote '{}' prepared", remote_name);

        let concept = replica.remote(remote_name.as_str(), subject, &remote_config.address);
        transaction = transaction.assert(concept.clone());
        if let Some(revocation_url) = &remote_config.revocation_url {
            transaction =
                transaction.assert(RemoteExecution::new(&concept, revocation_url.as_str()));
        }
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

    Ok(())
}

/// Prepare repository-local metadata, then expose the replica in the profile
/// index with its initial installing status.
pub(crate) async fn record_replica_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    display_name: &str,
    configuration: &RepositoryConfiguration,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    record_replica_local_meta(tonk, repository, display_name, configuration).await?;
    record_replica_visibility(
        tonk,
        display_name,
        &repository.did(),
        Replica::blank_status(),
    )
    .await?;
    record_space_mount(tonk, &repository.did(), configuration, Some(display_name)).await;
    // Only on the creation path: `record_space_mount` also runs for
    // joined spaces, and a founding stamp there would claim this
    // account made a space it was merely invited to.
    record_space_founded(tonk, &repository.did()).await;
    super::adopt::stamp_space_locality(tonk, &repository.did()).await;
    Ok(())
}

/// Stamp who founded a space and when, onto its directory entity.
///
/// Best effort, like the mount record beside it: a space is usable the
/// moment its delegations exist, and a missing founding stamp costs a
/// Hub label rather than access.
async fn record_space_founded(tonk: &TonkState, subject: &Did) {
    let at = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(tonk_schema::SpaceFounded::new(
            subject,
            &tonk.profile.did(),
            at,
        ));
    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("stamp space founding for '{subject}': {error}");
    }
}

/// Anchor wrapper so branch/remote concepts can hang off the space's
/// directory entity (`subject.this()`), giving every device the same
/// derived entities — the account-level mirror of the per-replica meta
/// records.
struct DirectoryAnchor(dialog_artifacts::Entity);

impl AsRef<dialog_artifacts::Entity> for DirectoryAnchor {
    fn as_ref(&self) -> &dialog_artifacts::Entity {
        &self.0
    }
}

/// Mirror a space's remote/branch configuration — and optionally its
/// display name — into the account directory as plain facts on
/// directory-anchored entities, so any device can rebuild the full
/// [`RepositoryConfiguration`] from the account DB and mount the space
/// identically, non-default setups included. Individually updatable
/// like all facts; no serialized blob.
pub(crate) async fn record_space_mount(
    tonk: &TonkState,
    subject: &Did,
    configuration: &RepositoryConfiguration,
    display_name: Option<&str>,
) {
    use tonk_schema::domain::remote::Address as RemoteAddress;

    let anchor_entity = subject.this();
    let anchor = DirectoryAnchor(anchor_entity.clone());
    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction();
    if let Some(name) = display_name {
        transaction = transaction.assert(tonk_schema::SpaceName::new(subject, name));
    }
    let mut remote_concepts: HashMap<String, Remote> = HashMap::new();
    for (name, remote_config) in &configuration.remote {
        let target = remote_config
            .subject
            .clone()
            .unwrap_or_else(|| subject.clone());
        let concept = Remote::at(
            &anchor_entity,
            target,
            RemoteAddress::encode(&remote_config.address),
            name.as_str(),
        );
        transaction = transaction.assert(concept.clone());
        if let Some(relay) = &remote_config.revocation_url {
            transaction = transaction.assert(RemoteExecution::new(&concept, relay.as_str()));
        }
        remote_concepts.insert(name.clone(), concept);
    }
    for (branch_name, settings) in &configuration.branch {
        let local = MetaBranch::new(&anchor, branch_name.as_str());
        transaction = transaction.assert(local.clone());
        if let Some(upstream) = &settings.upstream
            && let Some(remote_concept) = remote_concepts.get(&upstream.remote)
        {
            let remote_branch = MetaBranch::new(remote_concept, upstream.branch.as_str());
            transaction = transaction
                .assert(remote_branch.clone())
                .assert(TrackingBranch::new(&local, &remote_branch));
        }
    }
    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("record space mount for '{subject}': {error}");
    }
}

/// Expose a fully prepared replica and its initialized status in one profile
/// branch commit. Repository-local metadata and content must already be usable.
///
/// This is the visibility commit: until it lands, the replica exists in
/// storage but is not in the profile index, so it never appears in the
/// Hub and nothing can navigate to it.
pub(crate) async fn record_initialized_replica_in_profile(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), RepositoryError> {
    record_replica_visibility(
        tonk,
        subject.repo_key(),
        subject,
        Replica::initialized_status(),
    )
    .await
}

/// Lay down the meta-branch facts and profile index, then record the
/// opening profile's membership on the content branch. The two halves
/// are split so the join/restore mount can reuse the meta half without
/// the roster write (restore must not stamp a role — see the restore
/// path).
pub async fn record_repository_meta<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    display_name: &str,
    configuration: &RepositoryConfiguration,
    role_uri: &str,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    record_replica_meta(tonk, repository, display_name, configuration).await?;
    record_membership_on_content(tonk, repository, repository.did().repo_key(), role_uri).await
}

/// Assert the opening profile's [`Membership`] + [`MemberRole`] +
/// [`MemberName`] on the repository's content branch.
///
/// The roster lives on the content branch (`main`) because that branch
/// syncs across replicas; the meta branch is local-only, so a roster
/// written there never converges. Runs on every path
/// [`record_repository_meta`] serves: on create the opener is the
/// `tonk:founder`; on join the claimer is a `tonk:member`. The member
/// is resolved via [`crate::router::account::member_did`] — the
/// account root when this profile is linked, else the device DID — so
/// a founder/member row converges across every device on the same
/// account. The membership entity is content-derived from `(member,
/// subject)`, so a repeat is a no-op; `role`/`name` are cardinality-one
/// stamps.
///
/// `key` is the repository's routing key (the `{repo}` param) so the
/// write goes through the *reactor's* cached `main` handle.
pub(crate) async fn record_membership_on_content<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    key: &str,
    role_uri: &str,
) -> Result<(), RepositoryError>
where
    C: Principal + Clone,
{
    // The opening profile is a member of this repository, stamped with
    // its role (founder on create, member on join) and named with the
    // name their profile was opened under. Keyed on the account root
    // when this profile is linked, so a founder/member row converges
    // across every device on the same account.
    let member = crate::router::account::member_did(tonk)
        .await
        .map_err(|error| match error {
            TonkWorkerError::RootRequired => RepositoryError::RootRequired,
            error => RepositoryError::Internal(error.to_string()),
        })?;
    let membership = Membership::new(member, repository.did());
    let role = if role_uri == MemberRole::FOUNDER {
        MemberRole::founder(membership.this().clone())
    } else {
        MemberRole::member(membership.this().clone())
    };
    let display_name = crate::router::profile_name::resolve_display_name(tonk).await;
    let member_name = MemberName::new(membership.this().clone(), display_name);

    // Write through the *reactor's* cached content-branch handle, not a
    // fresh `repository.branch().open()`. Background sync pulls/publishes
    // through the reactor's cached `main` handle; a commit through a
    // separate handle leaves that cached handle pinned at its old head, so
    // a later pull compares against a stale base version and the CAS fails
    // forever (`VersionMismatch`), wedging all `main` sync. Going through
    // the reactor advances the cached handle and re-polls its subscriptions.
    tonk.reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(membership)
        .assert(role)
        .assert(member_name)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to record membership on content: {}", e))
        })?;

    Ok(())
}

/// Assert a [`Replica`] concept for a newly created repository in
/// the profile repository's meta branch.
///
/// The profile repository serves as an index of every replica the
/// profile owns; this function adds one entry to that index.
/// Idempotent at the concept layer — re-asserting the same
/// `(profile, subject)` replica is a no-op.
async fn record_replica_visibility(
    tonk: &TonkState,
    display_name: &str,
    subject: &Did,
    status: tonk_schema::domain::replica::Status,
) -> Result<(), RepositoryError> {
    let replica = Replica::new(tonk.profile.did(), subject.clone());
    // The account-level directory entry rides the same commit: the
    // replica row is this device's mount, the `Space` entry is the one
    // row per space every device's Hub lists.
    let directory = tonk_schema::Space::new(subject, status.clone());
    let status = SpaceStatus::new(replica.this().clone(), status);

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
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(replica)
        .assert(status)
        .assert(directory)
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
            branch: PROFILE_BRANCH.to_string(),
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
/// Called from the background seed path, which only runs in the worker.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn set_replica_status(
    tonk: &TonkState,
    subject: &Did,
    status: tonk_schema::domain::replica::Status,
) -> Result<(), RepositoryError> {
    let entity = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();
    let directory = tonk_schema::Space::new(subject, status.clone());
    let stamp = SpaceStatus::new(entity, status);

    let revision = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(stamp)
        .assert(directory)
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
            branch: PROFILE_BRANCH.to_string(),
            revision,
        },
    );

    Ok(())
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
pub async fn bootstrap_profile(tonk: &TonkState) -> Result<(), RepositoryError> {
    let profile_did = tonk.profile.did();
    let replica = Replica::new(profile_did.clone(), profile_did);

    // Write through the reactor's profile handle so the cached branch
    // state (which every read also goes through) advances on this
    // commit — see `record_replica_in_profile` for why a separate
    // `Repository::from` handle would leave the reader stale.
    tonk.reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(PROFILE_BRANCH))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            RepositoryError::Internal(format!("Failed to bootstrap profile branch: {}", e))
        })?;
    log!("Profile branch bootstrapped");

    // Stamp a durable display name (the deterministic petname) when none is
    // stored yet, so the FAB's sealed profile-branch `<tonk-display
    // model="tonk:profile/name">` resolves a name for a never-renamed member.
    // Rename-safe: a no-op once any `ProfileName` override exists.
    crate::router::profile_name::ensure_display_name(tonk).await?;

    // Seed the standard library onto the profile meta branch so a
    // `<tonk-display>` reading the profile (the Hub at `/`) can resolve
    // the library's concepts and views — the `space` model and its
    // directory view — there, the same way a named repo's content
    // branch carries them. Idempotent: re-evaluating the library
    // de-duplicates rather than minting fresh claims, so it's safe on
    // every boot. Fetch is only available in the SW scope; native
    // builds skip it (the Hub is a browser-only surface).
    //
    // Best-effort: this runs again on every boot and profile
    // activation, so a failed fetch (an offline worker restart, a
    // harness that serves no library) costs a degraded Hub until the
    // next attempt — not a worker that refuses to boot or a profile
    // switch that dies half-way.
    if let Err(error) = seed_profile_library(tonk).await {
        log!("profile library seed skipped: {error}");
    }

    // Drain the poll the bootstrap commit scheduled.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    Ok(())
}

/// Fetch and seed the lean profile library onto the profile
/// branch. SW-only — the fetch needs a service-worker scope.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn seed_profile_library(tonk: &TonkState) -> Result<(), RepositoryError> {
    let library = fetch_standard_library(PROFILE_LIBRARY_URL)
        .await
        .map_err(|e| RepositoryError::Internal(format!("fetch profile library: {e}")))?;
    super::evaluate::evaluate_profile_body(tonk, PROFILE_BRANCH, library, true)
        .await
        .map(|_| ())
        .map_err(|e| {
            RepositoryError::Internal(format!("seed standard library on profile branch: {e}"))
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

    // First use of a directory-listed space this device has not
    // replicated mounts it on demand — same lazy adoption the query
    // route performs, so a second device can address a space straight
    // from the synced account directory. A no-op for mounted repos.
    // The outcome rides the not-found error: a swallowed mount failure
    // turns an explainable miss into a bare 404.
    let mount = match super::adopt::ensure_space_mounted(&tonk, &name).await {
        Ok(true) => None,
        Ok(false) => Some("the account directory holds no mountable record for it".to_string()),
        Err(error) => {
            log!("on-demand mount of '{}' failed: {error}", name);
            Some(format!(
                "mounting it from the account directory failed: {error}"
            ))
        }
    };
    let repository = tonk
        .profile
        .repository(&name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            let mount = mount
                .as_deref()
                .map(|note| format!(" ({note})"))
                .unwrap_or_default();
            TonkWorkerError::NotFound(format!("Repository '{}' not found{}: {}", name, mount, e))
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
/// meta branch, running four queries, and joining the results
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
/// - **Roster** — `Membership` rows (who belongs), joined with
///   `MemberName` (published display names) and `InvitedVia` →
///   `Invitation` (who invited whom), assembled into `members`.
///   These are read from the *content* branch, not meta: the roster
///   lives there so it syncs across replicas.
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
                members: Vec::new(),
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
    let remote_executions: Vec<RemoteExecution> = match meta
        .query()
        .select(Query::<RemoteExecution> {
            this: Term::var("this"),
            revocation_url: Term::var("revocation_url"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log!(
                "Remote-execution query on meta failed for '{}': {:?}",
                key,
                e
            );
            Vec::new()
        }
    };
    let execution_by_remote: HashMap<_, _> = remote_executions
        .into_iter()
        .filter_map(|execution| {
            Url::parse(&execution.revocation_url.0)
                .ok()
                .map(|url| (execution.this, url))
        })
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
            RemoteConfiguration {
                address,
                subject,
                revocation_url: execution_by_remote.get(&remote.this).cloned(),
            },
        );
    }

    // Pull the roster from the content branch — it lives there (not on
    // meta) so it syncs across replicas. If the content branch can't be
    // opened, leave the roster empty, consistent with the per-query
    // log-and-empty-vec fallbacks below.
    let (memberships, member_names, invited_via, invitations) = match repository
        .branch(CONTENT_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
    {
        Ok(content) => {
            // `Membership` is the spine — one row per member;
            // `MemberName`, `InvitedVia`, and `Invitation` are joined in
            // below to attach the display name and inviter provenance.
            let memberships: Vec<Membership> = match content
                .query()
                .select(Query::<Membership> {
                    this: Term::var("this"),
                    subject: Term::from(repository.did().this()),
                    member: Term::var("member"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    log!("Membership query on content failed for '{}': {:?}", key, e);
                    Vec::new()
                }
            };
            // `MemberName`/`InvitedVia` carry no subject; they are scoped
            // implicitly by the join below on the membership entity, which
            // the subject-scoped `Membership` query already filtered.
            let member_names: Vec<MemberName> = match content
                .query()
                .select(Query::<MemberName> {
                    this: Term::var("this"),
                    name: Term::var("name"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    log!("MemberName query on content failed for '{}': {:?}", key, e);
                    Vec::new()
                }
            };
            let invited_via: Vec<InvitedVia> = match content
                .query()
                .select(Query::<InvitedVia> {
                    this: Term::var("this"),
                    invitation: Term::var("invitation"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    log!("InvitedVia query on content failed for '{}': {:?}", key, e);
                    Vec::new()
                }
            };
            let invitations: Vec<Invitation> = match content
                .query()
                .select(Query::<Invitation> {
                    this: Term::var("this"),
                    subject: Term::from(repository.did().this()),
                    inviter: Term::var("inviter"),
                    audience: Term::var("audience"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    log!("Invitation query on content failed for '{}': {:?}", key, e);
                    Vec::new()
                }
            };
            (memberships, member_names, invited_via, invitations)
        }
        Err(e) => {
            log!("No content branch for repository '{}' roster: {}", key, e);
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        }
    };

    // membership entity -> display name
    let names_by_membership: HashMap<_, _> = member_names
        .iter()
        .map(|n| (n.this.clone(), n.name.0.clone()))
        .collect();
    // invitation entity -> inviter did:key
    let inviter_by_invitation: HashMap<_, _> = invitations
        .iter()
        .map(|i| (i.this.clone(), i.inviter.0.to_string()))
        .collect();
    // membership entity -> inviter did:key, via the provenance stamp
    let inviter_by_membership: HashMap<_, _> = invited_via
        .iter()
        .filter_map(|v| {
            inviter_by_invitation
                .get(&v.invitation.0)
                .map(|inviter| (v.this.clone(), inviter.clone()))
        })
        .collect();

    let self_entity = crate::router::account::member_did(tonk)
        .await
        .ok()
        .map(|member| member.this());
    let mut members: Vec<MemberInfo> = memberships
        .iter()
        .map(|m| MemberInfo {
            did: m.member.0.to_string(),
            name: names_by_membership.get(&m.this).cloned(),
            is_self: self_entity.as_ref() == Some(&m.member.0),
            invited_by: inviter_by_membership.get(&m.this).cloned(),
        })
        .collect();
    // Deterministic order: self first, then named members
    // alphabetically, unnamed last, did as the stable tiebreak.
    members.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then_with(|| a.name.is_none().cmp(&b.name.is_none()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.did.cmp(&b.did))
    });

    RepositoryInfo {
        name: key.to_string(),
        label,
        subject: repository.did(),
        operator: tonk.operator.did(),
        profile: tonk.profile.did(),
        branch: branches,
        remote: remotes,
        members,
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
) -> Result<RepositoryConfiguration, RepositoryError>
where
    C: Principal + Clone,
{
    // What actually took effect: existing remotes are preserved rather
    // than rewritten, so the caller must mirror THIS into the account
    // directory, not the request.
    let mut effective = configuration.clone();
    if configuration.remote.is_empty() && configuration.branch.is_empty() {
        return Ok(effective);
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

        // What the meta mirror should describe. An existing remote is left
        // alone at the dialog layer, so the mirror has to follow the remote
        // that is really there and not the one the request asked for —
        // otherwise a caller that names a remote only to reach the
        // `revocationUrl` beside it (the share prompt's relay repair) would
        // silently rewrite its address to whatever origin that caller
        // happened to be served from.
        let (subject, address) = match repository
            .remote(remote_name.as_str())
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(existing) => {
                log!("Remote '{}' already present; left as-is", remote_name);
                let address = existing.address();
                (address.subject().clone(), address.site().clone())
            }
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
                (subject, remote_config.address.clone())
            }
        };

        if let Some(effective_remote) = effective.remote.get_mut(remote_name) {
            effective_remote.address = address.clone();
            effective_remote.subject = Some(subject.clone());
        }
        let concept = replica.remote(remote_name.as_str(), subject, &address);
        transaction = transaction.assert(concept.clone());
        if let Some(revocation_url) = &remote_config.revocation_url {
            transaction =
                transaction.assert(RemoteExecution::new(&concept, revocation_url.as_str()));
        }
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
    // Deliver the fresh snapshots the refresh scheduled for the rebound
    // subscriptions: without this drain a live view over a just-wired
    // branch waits for a commit that a quiet space never makes.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    Ok(effective)
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

    // Provision before attaching, for the same reason
    // [`enable_sync_inner`] does: a space created without an active
    // customer has no consumer row, and an upstream without one syncs to
    // a refused presign. Best effort here rather than fatal — this route
    // is also how a space is pointed at a remote that is not the
    // account's access service (a self-hosted endpoint, a test server),
    // where `/provider/add` against our own service is beside the point.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if !configuration.remote.is_empty() {
        match space_root_prefix(&tonk, &repository.did()).await {
            Ok(prefix) => {
                if let Err(error) =
                    super::customer::provision_consumer(&tonk, &repository.did(), &prefix, None)
                        .await
                {
                    log!("attach remote '{name}': provisioning skipped: {error}");
                }
            }
            Err(error) => {
                log!("attach remote '{name}': no root delegation to consent with: {error}")
            }
        }
    }

    ensure_remote_config(&tonk, &repository, &name, &configuration).await?;

    let info = build_repository_info(&tonk, &name, &repository).await;
    Ok(Json(info))
}

/// Scaffold regression tests: `core.yaml` makes a repository renderable
/// but seeds zero instances, so a fresh space opens on the blank canvas
/// and everything else is authored into it afterwards.
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
        let config = space_config("").unwrap();
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
        let config = space_config("   ").unwrap();
        assert!(config.remote.is_empty());
        assert!(config.branch.get("main").unwrap().upstream.is_none());
    }

    #[test]
    fn it_wires_origin_and_tracks_main_for_a_remote_url() {
        let config = space_config("https://example.test/ucan/").unwrap();
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

/// The create form and this handler must name the same remote attribute.
///
/// The handler reads it raw (not through the typed `CreateSpace`
/// decode) so an older, frozen profile descriptor still triggers it. That
/// tolerance cuts both ways: a renamed attribute on either side doesn't
/// fail — the fact simply never matches, the field reads as absent, and
/// the space is created missing the remote with nothing logged. Pin both
/// sides against the seeded document. Native.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod form_attribute_tests {
    use super::REMOTE_ATTR;

    /// The document the worker seeds onto a profile branch, embedded for
    /// the same reason `tests/standard_library.rs` embeds it: CI runs from
    /// a `cargo nextest archive`, which carries no sibling data files.
    const PROFILE_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

    /// The create form carries no remote, and the handler is fine with
    /// that.
    ///
    /// It used to: the Hub filled a hidden input from
    /// `<tonk-default-remote auto>`, and this test pinned the two
    /// spellings together. Then a space stopped earning its remote at
    /// creation — the worker resolves where a space syncs from the
    /// account's own registration, so a space made before anyone
    /// registers stays local until it is shared. A form that names a
    /// remote would wire one anyway, which is the behaviour
    /// `it_creates_a_local_only_space_from_the_hub_wizard` refuses.
    ///
    /// `REMOTE_ATTR` stays readable so a frozen older descriptor that
    /// still declares the field keeps working; it is simply no longer
    /// where the answer comes from.
    #[test]
    fn it_declares_no_remote_on_the_create_form() {
        assert!(
            !PROFILE_LIBRARY.contains(REMOTE_ATTR),
            "profile.yaml declares `the: {REMOTE_ATTR}` again — a space \
             would wire a remote at creation instead of earning one when \
             it is shared",
        );
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

/// The opportunistic invite-target reader `InviteHandler` uses. Native.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod invite_space_from_facts_tests {
    use super::invite_space_from_facts;
    use dialog_artifacts::{Artifact, Changes, Entity, Instruction, Statement, Value};
    use dialog_query::the;

    const DID: &str = "did:key:zTargetSpace";

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

    /// Seed the always-present `time` fact (every `tonk:invite` transient
    /// carries it, matched or not).
    fn time_fact(changes: &mut Changes, of: &Entity) {
        the!("dom.event/time-stamp")
            .of(of.clone())
            .is(1.0)
            .assert(changes);
    }

    #[test]
    fn it_reads_an_entity_space() {
        // A DID deserializes as `Value::Entity` (any string with a `:`) —
        // the FAB's routeless share claim asserts it this way.
        let did_value: Value = serde_json::from_str(&format!("\"{DID}\"")).unwrap();
        let did = match did_value {
            Value::Entity(entity) => entity,
            other => panic!("DID should deserialize as Entity, got {other:?}"),
        };
        let of: Entity = "did:key:zInviteCommand".parse().expect("entity");
        let mut changes = Changes::new();
        time_fact(&mut changes, &of);
        the!("xyz.tonk.invite/space")
            .of(of)
            .is(did)
            .assert(&mut changes);
        assert_eq!(
            invite_space_from_facts(&artifacts(changes)).as_deref(),
            Some(DID),
        );
    }

    #[test]
    fn it_reads_a_string_space() {
        let of: Entity = "did:key:zInviteCommand".parse().expect("entity");
        let mut changes = Changes::new();
        time_fact(&mut changes, &of);
        the!("xyz.tonk.invite/space")
            .of(of)
            .is(DID.to_string())
            .assert(&mut changes);
        assert_eq!(
            invite_space_from_facts(&artifacts(changes)).as_deref(),
            Some(DID),
        );
    }

    #[test]
    fn it_returns_none_without_a_space_fact() {
        // The shape every existing space's frozen `tonk:invite` descriptor
        // dispatches — the handler must fall back to the dispatch origin.
        let of: Entity = "did:key:zInviteCommand".parse().expect("entity");
        let mut changes = Changes::new();
        time_fact(&mut changes, &of);
        assert!(invite_space_from_facts(&artifacts(changes)).is_none());
    }

    #[test]
    fn it_treats_a_blank_space_as_none() {
        let of: Entity = "did:key:zInviteCommand".parse().expect("entity");
        let mut changes = Changes::new();
        time_fact(&mut changes, &of);
        the!("xyz.tonk.invite/space")
            .of(of)
            .is("   ".to_string())
            .assert(&mut changes);
        assert!(invite_space_from_facts(&artifacts(changes)).is_none());
    }
}

/// The pure untitled-label picker the create handler uses. Native.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod next_untitled_label_tests {
    use super::next_untitled_label;

    fn labels(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|label| label.to_string()).collect()
    }

    #[test]
    fn it_starts_at_bare_untitled() {
        assert_eq!(next_untitled_label(labels(&[])), "Untitled");
    }

    #[test]
    fn it_ignores_named_spaces() {
        assert_eq!(
            next_untitled_label(labels(&["pictures", "notes"])),
            "Untitled",
        );
    }

    #[test]
    fn it_numbers_from_two_after_the_bare_label() {
        assert_eq!(
            next_untitled_label(labels(&["Untitled", "pictures"])),
            "Untitled 2",
        );
    }

    #[test]
    fn it_fills_the_smallest_gap() {
        assert_eq!(
            next_untitled_label(labels(&["Untitled", "Untitled 3"])),
            "Untitled 2",
        );
        assert_eq!(
            next_untitled_label(labels(&["Untitled 2", "Untitled 3"])),
            "Untitled",
        );
    }

    #[test]
    fn it_counts_past_a_dense_run() {
        assert_eq!(
            next_untitled_label(labels(&["Untitled", "Untitled 2", "Untitled 3"])),
            "Untitled 4",
        );
    }

    #[test]
    fn it_ignores_near_misses() {
        // Prefixes without the ` <n>` shape, or with a non-ordinal
        // suffix, are user-typed names — not part of the sequence.
        assert_eq!(
            next_untitled_label(labels(&[
                "Untitled draft",
                "Untitled2",
                "Untitled 0",
                "untitled",
            ])),
            "Untitled",
        );
    }

    #[test]
    fn it_trims_surrounding_whitespace() {
        assert_eq!(next_untitled_label(labels(&["  Untitled  "])), "Untitled 2");
    }
}

/// The pure library-URL selector. Native.
/// The rename result → outcome mapping. Native.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod rename_outcome_tests {
    use super::{RenameOutcome, rename_outcome};
    use crate::RepositoryError;

    #[dialog_common::test]
    fn it_maps_a_failed_rename_to_failed_rather_than_success() {
        // `PauseSyncHandler` logs and returns on a missing replica. Rename must
        // not: a silently-dropped rename looks successful to the user, which is
        // the exact failure class this design attacks.
        // `RepositoryError` has no `NotFound` variant — an absent replica
        // surfaces as `Internal` from the acquire.
        let outcome = rename_outcome(Err(RepositoryError::Internal("no such replica".into())));
        assert_eq!(outcome, RenameOutcome::Failed);
    }

    #[dialog_common::test]
    fn it_maps_a_successful_rename_to_renamed() {
        assert_eq!(rename_outcome(Ok(())), RenameOutcome::Renamed);
    }
}

/// wasm32-only — `evaluate_body` and the worker test `TonkState` are
/// built from the service-worker harness.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A refusal the user is in the middle of fixing keeps the request
    /// open.
    ///
    /// `needs-account` and `not-synced` both end in a link once the
    /// thing they name arrives, so reporting them as terminal would stop
    /// the control on `failed` while the share is still going.
    #[dialog_common::test]
    fn it_keeps_a_repairable_refusal_open() {
        use super::invite_status_for;
        use tonk_schema::command::InviteState;
        use tonk_worker_api::share;

        assert_eq!(
            invite_status_for(share::BLOCKED_NEEDS_ACCOUNT),
            InviteState::REQUESTED,
            "the worker is off getting an account; the share has not failed",
        );
        assert_eq!(
            invite_status_for(share::BLOCKED_NOT_SYNCED),
            InviteState::REQUESTED,
            "attaching a remote still ends in a link",
        );
        // Terminal: nothing the user or the worker does next helps.
        assert_eq!(
            invite_status_for(share::BLOCKED_SUSPENDED),
            InviteState::SUSPENDED,
        );
        assert_eq!(
            invite_status_for(share::BLOCKED_UNSHAREABLE_REMOTE),
            InviteState::UNSHAREABLE,
        );
    }

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use axum::Router;
    use dialog_remote_ucan_s3::UcanAddress;
    use dialog_repository::SiteAddress;

    use super::{
        BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
        existing_space_labels,
    };
    use crate::router::evaluate::evaluate_body;
    use crate::router::tests::{content_invitations, put_repo, put_repo_info};
    use crate::router::{AppState, CreateInviteResponse, api_router_with_state, tests::test_state};

    /// The seed sealed to the account is the only copy of a created
    /// space's secret: the repository stores the verifier, the space still
    /// proves for the operator through `space -> account -> device`, and
    /// opening the custodied seed with the account key re-derives exactly
    /// the space's signer.
    #[dialog_common::test]
    async fn it_creates_a_space_with_a_public_key_and_custodies_its_seed() {
        use dialog_capability::Subject;
        use dialog_effects::Use;
        use dialog_query::{Output as _, Query, Term};
        use dialog_repository::RepositoryExt as _;
        use dialog_varsig::Principal as _;
        use tonk_schema::prelude::DidExt as _;

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "public-key-space").await;
        let tonk = state.read().await;
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert!(
            repository.try_access().is_none(),
            "the repository stores only the verifier",
        );
        let subject = repository.did();

        tonk.profile
            .access()
            .prove(Subject::from(subject.clone()).attenuate(Use))
            .audience(&tonk.operator)
            .perform(&tonk.operator)
            .await
            .expect("the space proves through the account without its own key");

        let branch = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let principals: Vec<tonk_schema::SecretPrincipal> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretPrincipal> {
                this: Term::from(subject.this()),
                kind: Term::var("kind"),
                seed: Term::var("seed"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(principals.len(), 1, "one sealed space principal");
        assert_eq!(
            principals[0].kind.0.to_string(),
            tonk_schema::SeedKind::SPACE
        );

        let rows: Vec<tonk_schema::SecretMessage> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretMessage> {
                this: Term::from(principals[0].seed.0.clone()),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "the principal names a real message");
        let sealed = tonk_identity::sealed::Sealed::decode(&rows[0].message.0).unwrap();
        let account = tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new(
            crate::router::tests::test_root_seed(&tonk.profile_name),
        ));
        let opened = account
            .secret()
            .reveal(&sealed, &subject)
            .expect("the account key opens the custodied seed");
        let reissued = dialog_credentials::Ed25519Signer::import(&*opened)
            .await
            .unwrap();
        assert_eq!(reissued.did(), subject, "the seed derives the space's key");
    }

    /// The scaffold notation, embedded at compile time.
    const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

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

    /// A profile holding one space, signed out of its account.
    ///
    /// The local profile-name override belongs to this state: with an account
    /// attached, a rename adopts the account's display name instead. Signing
    /// out is how a device reaches it while still holding spaces — creating
    /// them without an account is what the account gate refuses.
    #[dialog_common::test]
    async fn rename_mirrors_the_name_into_the_account_directory() {
        use dialog_query::{Output as _, Query, Term};

        let (app, state, key) = fresh_repo("rename-directory-mirror").await;
        attach(
            &app,
            &key,
            &origin_config("https://sync.example.test/ucan/"),
        )
        .await;

        let env = crate::router::CommandEnv::new(state.clone(), Default::default());
        super::run_rename_repository(&env, &key, "renamed-garden")
            .await
            .unwrap();

        let tonk = state.read().await;
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let main = tonk
            .reactor
            .profile_repository()
            .branch(super::PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let names: Vec<tonk_schema::SpaceName> = main
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceName> {
                this: Term::from(tonk_schema::prelude::DidExt::this(&subject)),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(
            names.first().map(|row| row.name.0.as_str()),
            Some("renamed-garden"),
            "the rename lands in the account directory so unreplicated \
             devices can label the space"
        );
    }

    #[dialog_common::test]
    async fn enable_sync_records_the_preserved_upstream_in_the_directory() {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::domain::remote::Origin as RemoteOrigin;

        let (app, state, key) = fresh_repo("preserved-directory-upstream").await;
        attach(
            &app,
            &key,
            &origin_config("https://actual-sync.example.test/ucan/"),
        )
        .await;

        super::enable_sync_inner(&state, &key, "https://form-repair.example.test/ucan/")
            .await
            .unwrap();

        let tonk = state.read().await;
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let main = tonk
            .reactor
            .profile_repository()
            .branch(super::PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let remotes: Vec<super::Remote> = main
            .handle()
            .query()
            .select(Query::<super::Remote> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(RemoteOrigin::from(tonk_schema::prelude::DidExt::this(
                    &subject,
                ))),
                subject: Term::var("subject"),
                address: Term::var("address"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let addresses: Vec<String> = remotes
            .iter()
            .filter_map(|row| {
                tonk_schema::domain::remote::Address::decode(&row.address)
                    .ok()
                    .map(|address| format!("{address:?}"))
            })
            .collect();
        assert!(
            addresses
                .iter()
                .any(|address| address.contains("actual-sync.example.test")),
            "the directory records the PRESERVED configured upstream, not \
             the form-supplied repair URL: {addresses:?}"
        );
    }

    async fn fresh_repo_signed_out(label: &str) -> (Router, AppState, String) {
        let (app, state, key) = fresh_repo(label).await;
        {
            let tonk = state.read().await;
            crate::router::account::detach_test_account(&tonk)
                .await
                .expect("the test account detaches");
        }
        (app, state, key)
    }

    /// A freshly created repo reports exactly its founder as a member,
    /// named, marked `is_self`, with no inviter.
    #[dialog_common::test]
    async fn it_reports_the_founder_in_members() {
        let (_app, state, key) = fresh_repo("test-members-founder").await;

        let info = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            super::build_repository_info(&tonk, &key, &repository).await
        };

        assert_eq!(info.members.len(), 1, "exactly the founder");
        let founder = &info.members[0];
        assert!(founder.is_self, "founder is the active profile");
        assert!(founder.invited_by.is_none(), "founder has no inviter");
        assert!(founder.name.is_some(), "founder is named");
    }

    /// All `Replica` rows on the profile meta branch (any kind), read
    /// through the reactor's cached profile handle — the same handle
    /// the Hub and the removal path use.
    async fn profile_replicas(state: &AppState) -> Vec<tonk_schema::Replica> {
        use dialog_query::{Output as _, Query, Term};
        let tonk = state.read().await;
        let meta = tonk
            .reactor
            .profile_repository()
            .branch(super::PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("profile meta acquires");
        meta.handle()
            .query()
            .select(Query::<tonk_schema::Replica> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                profile: Term::var("profile"),
                kind: Term::var("kind"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("replica query")
    }

    /// Removing a space retracts its replica record from the profile
    /// meta branch and evicts the repo from the reactor cache (which is
    /// what drops it from the background sync sweep).
    #[dialog_common::test]
    async fn it_removes_a_space_from_the_profile_index() {
        use tonk_schema::prelude::DidExt as _;

        let (_app, state, key) = fresh_repo("test-remove-space").await;

        let subject: dialog_varsig::Did = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            repository.did()
        };
        let recorded = profile_replicas(&state).await;
        assert!(
            recorded.iter().any(|r| r.subject.0 == subject.this()),
            "the fresh repo must be recorded before removal"
        );

        super::remove_space_inner(&state, &subject)
            .await
            .expect("remove succeeds");

        let remaining = profile_replicas(&state).await;
        assert!(
            !remaining.iter().any(|r| r.subject.0 == subject.this()),
            "the replica record must be gone after removal"
        );
        {
            let tonk = state.read().await;
            assert!(
                !tonk.reactor.repos().read().contains_key(&key),
                "the repo must be evicted from the reactor cache"
            );
        }

        // Idempotent: a repeated submit (e.g. a double-click before the
        // Hub row disappears) finds no replica record and no cached repo —
        // `remove_replica_from_profile`'s "nothing recorded" branch and a
        // no-op `evict` — and is a logged no-op, not an error.
        super::remove_space_inner(&state, &subject)
            .await
            .expect("a repeated remove is a no-op, not an error");
    }

    /// The self-replica (subject == profile) is refused: deleting the
    /// profile's own storage would take every space with it.
    #[dialog_common::test]
    async fn it_refuses_to_remove_the_self_replica() {
        use tonk_schema::prelude::DidExt as _;

        let (_app, state, _key) = fresh_repo("test-remove-self").await;

        // The harness never runs the worker boot path — and can't call
        // `bootstrap_profile`, whose library fetch needs a real
        // service-worker registration — so seed just the self-replica
        // record the assertion below expects, mirroring the bootstrap's
        // own transaction.
        {
            let tonk = state.read().await;
            let profile_did = tonk.profile.did();
            let replica = super::Replica::new(profile_did.clone(), profile_did);
            tonk.reactor
                .profile_repository()
                .branch(super::PROFILE_BRANCH)
                .transaction()
                .assert(replica.clone())
                .assert(replica.branch(super::PROFILE_BRANCH))
                .commit()
                .perform(&tonk.operator)
                .await
                .expect("seed self-replica");
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
        }

        let profile_did = {
            let tonk = state.read().await;
            tonk.profile.did()
        };
        super::remove_space_inner(&state, &profile_did)
            .await
            .expect_err("removing the self-replica must fail");

        let remaining = profile_replicas(&state).await;
        assert!(
            remaining.iter().any(|r| r.subject.0 == profile_did.this()),
            "the self-replica record must survive"
        );
    }

    /// Account replicas survive removal and fail the shared guard used by
    /// pause, invite, and other direct user-space controls.
    #[dialog_common::test]
    async fn it_refuses_user_space_controls_for_the_account_replica() {
        use dialog_credentials::Ed25519Signer;
        use dialog_varsig::Principal as _;
        use tonk_schema::prelude::DidExt as _;

        let (_app, state, _key) = fresh_repo("test-account-controls").await;
        let account = Ed25519Signer::import(&[74; 32]).await.unwrap().did();
        {
            let tonk = state.read().await;
            tonk.reactor
                .profile_repository()
                .branch(super::PROFILE_BRANCH)
                .transaction()
                .assert(super::Replica::account(tonk.profile.did(), account.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .expect("seed account replica");
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;

            super::require_real_space(&tonk, &account)
                .await
                .expect_err("account replica must fail the user-space guard");
        }

        super::remove_space_inner(&state, &account)
            .await
            .expect_err("account replica must not be removable");
        let remaining = profile_replicas(&state).await;
        assert!(
            remaining
                .iter()
                .any(|replica| replica.subject.0 == account.this()
                    && replica.kind == super::Replica::account_kind()),
            "the account replica must survive refused controls"
        );
    }

    /// Build a one-entity transient `ProfileRename{this, name, marker}`
    /// batch — the facts the identity chip's `<tonk-editable>` commit
    /// asserts. Mirrors how `command::tests::ping_transient` hand-builds a
    /// command transient via `the!`, carrying both the `name`
    /// (`current-target/value`) and the `marker`
    /// (`current-target.dataset/rename`) so it decodes as a `ProfileRename`.
    fn profile_rename_transient(of: &str, name: &str) -> dialog_artifacts::Changes {
        use dialog_artifacts::{Entity, Statement};
        use dialog_query::the;

        let entity: Entity = of.parse().expect("entity URI");
        let mut changes = dialog_artifacts::Changes::new();
        the!("dom.event.current-target/value")
            .of(entity.clone())
            .is(name.to_string())
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/rename")
            .of(entity)
            .is("tonk:profile".parse::<Entity>().expect("marker URI"))
            .assert(&mut changes);
        changes
    }

    /// Read the self member's stamped name off the space's content
    /// branch.
    async fn self_member_name(state: &AppState, key: &str) -> Option<String> {
        let tonk = state.read().await;
        use dialog_repository::RepositoryExt as _;
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(key)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repo loads");
        let info = super::build_repository_info(&tonk, key, &repository).await;
        info.members
            .into_iter()
            .find(|m| m.is_self)
            .and_then(|m| m.name)
    }

    /// A `profile/rename` command persists the display-name override AND
    /// re-stamps the self member's `MemberName` on the current space.
    #[dialog_common::test]
    async fn it_persists_the_override_and_restamps_the_current_space() {
        let (_app, state, key) = fresh_repo_signed_out("test-profile-rename").await;

        // Drive the transient command through the real dispatcher, scoped
        // to the space's content branch — mirrors
        // `command::tests::it_dispatches_every_matched_command_in_a_batch`.
        let changes = profile_rename_transient("did:key:zRenameCmd", "brave-lynx");
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: key.clone(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        // Override is on the profile meta branch.
        {
            let tonk = state.read().await;
            assert_eq!(
                crate::router::profile_name::resolve_display_name(&tonk).await,
                "brave-lynx",
                "the override is persisted on the profile meta branch",
            );
        }

        // The current space's roster now reads the re-stamped name.
        assert_eq!(
            self_member_name(&state, &key).await.as_deref(),
            Some("brave-lynx"),
            "the self member's MemberName is re-stamped on the space",
        );
    }

    /// A `profile/rename` re-stamps the self member's `MemberName` on
    /// EVERY space the profile belongs to, not just the one in focus when
    /// the rename was issued.
    #[dialog_common::test]
    async fn it_restamps_member_name_across_all_spaces() {
        let (app, state, key_a) = fresh_repo("rename-all-a").await;

        // A second space in the same profile/state. Both are created before
        // signing out, because creating one is exactly what the account gate
        // refuses afterwards.
        let key_b = {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/repository/rename-all-b")
                        .method("PUT")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let info: RepositoryInfo = serde_json::from_slice(&body).unwrap();
            info.name
        };
        {
            let tonk = state.read().await;
            crate::router::account::detach_test_account(&tonk)
                .await
                .expect("the test account detaches");
        }

        // Rename while focused on space A.
        let changes = profile_rename_transient("did:key:zRenameAll", "brave-lynx");
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: key_a.clone(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        assert_eq!(
            self_member_name(&state, &key_a).await.as_deref(),
            Some("brave-lynx"),
            "the focused space's roster is restamped",
        );
        assert_eq!(
            self_member_name(&state, &key_b).await.as_deref(),
            Some("brave-lynx"),
            "the non-focused space's roster is also restamped",
        );
    }

    /// A rename fired from the FAB carries an EMPTY origin repo (it lands on
    /// the profile branch, not a space). The self-identity overlay
    /// (`state:self`) the topbar chip reads must still be re-stamped on the
    /// space — regression for a rename that persisted the name but left the
    /// chip stale because step 3 tried to acquire the empty-named origin repo.
    #[dialog_common::test]
    async fn it_stamps_the_self_identity_overlay_with_an_empty_origin() {
        use dialog_query::{Output as _, Query, Term};

        let (_app, state, key) = fresh_repo_signed_out("rename-empty-origin").await;

        // Realistic origin: a profile-branch rename command has no repo.
        let changes = profile_rename_transient("did:key:zRenameChip", "brave-lynx");
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: String::new(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        // The topbar chip's overlay on the space now carries the new name.
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let entity: dialog_artifacts::Entity =
            tonk_schema::Replica::SELF_STATE_HERE.parse().unwrap();
        let rows: Vec<tonk_schema::ProfileIdentity> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::ProfileIdentity> {
                this: Term::from(entity),
                did: Term::var("did"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();

        assert_eq!(rows.len(), 1, "one state:self overlay row on the space");
        assert_eq!(
            rows[0].name.0, "brave-lynx",
            "the chip overlay reflects the new name despite the empty origin repo",
        );
    }

    /// An empty/whitespace name is a no-op: the prior name stands (a
    /// member can't blank their own name out).
    #[dialog_common::test]
    async fn it_ignores_a_whitespace_only_rename() {
        let (_app, state, key) = fresh_repo("test-profile-rename-empty").await;

        // The founder is already named at create time; capture it.
        let before = self_member_name(&state, &key)
            .await
            .expect("founder is named");

        let changes = profile_rename_transient("did:key:zRenameEmpty", "   ");
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: key.clone(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        assert_eq!(
            self_member_name(&state, &key).await,
            Some(before),
            "a whitespace-only rename leaves the name unchanged",
        );
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

    /// The FAB space-rename must PERSIST. The repository banner writes a
    /// transient `tonk/rename-repository` command (its `subject` is the repo's
    /// own DID, `name` the typed value); the standard-library rule fires on
    /// commit and asserts the durable `tonk/repository` name
    /// (`xyz.tonk.repo/name`, keyed by the subject DID) on the content branch.
    /// The banner then reads that fact back, so a refresh keeps the new name
    /// rather than reverting.
    ///
    /// Regression guard: the rename flows entirely through the standard
    /// library on the content branch, but the branch query that drives the
    /// rule's fixpoint also carries dialog's auto-injected session/replica
    /// facts. A drift in those injected names breaks the rule's evaluation
    /// context, so the command commits nothing and the name silently reverts —
    /// exactly the FAB "rename dropped on refresh" symptom.
    #[dialog_common::test]
    async fn it_persists_a_space_rename() {
        use dialog_repository::RepositoryExt as _;

        let (_app, state, key) = fresh_repo("test-space-rename").await;
        let repo = key.as_str();
        seed(&state, repo, CORE).await;

        // Fire the FAB's rename command: a transient `tonk/rename-repository`
        // whose `subject` is the repository's own DID (the banner stamps it
        // from `data-subject`) and whose `name` is the new value. Evaluating
        // with `transact=true` commits it, which fires the library rule.
        let rename =
            format!("tonk/rename-repository!:\n  subject: {key}\n  name: \"brave-lynx\"\n");
        seed(&state, repo, &rename).await;

        // Read the name back exactly as the Hub/banner does — through
        // `repository_label`, which queries `tonk/repository` on the content
        // branch keyed by the subject DID.
        let label = {
            let tonk = state.read().await;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(repo)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            super::repository_label(&tonk, &repository, repo).await
        };

        assert_eq!(
            label, "brave-lynx",
            "a space rename persists to xyz.tonk.repo/name; got {label:?}",
        );
    }

    /// The lean scaffold (core alone) carries the blank canvas concept,
    /// not the sheets workspace. The blank model resolves to exactly one
    /// instance — the repo's own subject — which the blank-canvas view
    /// binds to render the lean, no-template default.
    #[dialog_common::test]
    async fn it_seeds_blank_scaffold() {
        let (_app, state, repo) = fresh_repo("test-seed-blank-scaffold").await;
        let repo = repo.as_str();
        seed(&state, repo, CORE).await;

        // The lean scaffold carries the blank canvas concept, not the
        // sheets workspace. `blank:` resolves to the repo subject (its
        // sole `dialog.replica/subject`-derived attribute); a
        // `workspace/sheet:` query would fault on an unresolved concept.
        assert_eq!(
            count(&state, repo, "blank:\n").await,
            1,
            "blank scaffold resolves the blank model to the repo subject",
        );
        assert_eq!(
            count(&state, repo, "share/blocked:\n").await,
            0,
            "local-only fallback model is seeded before a refusal exists",
        );
    }

    /// The empty-state canvas keeps the pending label only while the invite
    /// request is unanswered. A refusal resolves the nested model and renders
    /// the explicit local-only notice instead of spinning forever.
    #[dialog_common::test]
    fn it_routes_refused_agent_links_to_the_local_only_notice() {
        assert!(
            CORE.contains("slot=\"no-entity\"") && CORE.contains("model=tonk:share/blocked"),
            "agent-link fallback should query the share refusal",
        );
        assert!(
            !CORE.contains("agent link &middot; paste into your agent"),
            "the rendered state should provide its own single label",
        );
        assert!(
            CORE.contains("tonk-display > [slot][hidden]"),
            "inactive pending and refusal slots should not survive a ready result",
        );
        assert!(CORE.contains("sharing unavailable"));
        assert!(
            !CORE.contains("Use connect in the condition banner"),
            "the refusal must not prescribe a repair that is absent or inappropriate"
        );
        assert!(
            CORE.contains("<p>{detail}</p>"),
            "the worker-owned complete sentence is the only refusal body"
        );
    }

    /// Regression guard for the dialog-injected replica identity fact the
    /// standard library queries. Dialog materializes this device's replica
    /// identity under the reserved `dialog.` namespace, and the blank canvas
    /// resolves its `subject` from `dialog.replica/subject`. When dialog
    /// renamed that attribute from `dialog.origin/subject` to
    /// `dialog.replica/subject`, a stale name in the library silently unbound
    /// the field: `blank:` resolved zero rows instead of one, so the space's
    /// content rendered an empty "Concept mismatch: subject: _" instead of the
    /// canvas.
    ///
    /// `it_seeds_blank_scaffold` above already asserts the count is 1; this
    /// test pins the *reason* — the attribute name must track what dialog
    /// injects — so a future dialog rename fails here with a clear message
    /// rather than as a blank space in the browser. This is the exact failure
    /// mode a native `cargo test` cannot catch: the fact exists only on a real
    /// branch, materialized by the reactor, which only runs on wasm.
    #[dialog_common::test]
    async fn it_binds_the_dialog_injected_replica_subject() {
        let (_app, state, repo) = fresh_repo("test-replica-subject-binds").await;
        let repo = repo.as_str();
        seed(&state, repo, CORE).await;

        // `blank:` resolves `tonk:blank`, whose sole `with` field
        // (`subject`) reads `dialog.replica/subject`. A zero means that
        // attribute name drifted from what dialog injects for the replica.
        assert_eq!(
            count(&state, repo, "blank:\n").await,
            1,
            "blank resolves its subject via dialog.replica/subject",
        );
    }

    /// A `RepositoryConfiguration` that attaches an `origin` remote at
    /// `endpoint` and points `main` at `origin/main` — the shape the
    /// launchpad sends to make a `create_space` repo sync-capable.
    fn origin_config(endpoint: &str) -> RepositoryConfiguration {
        let address = SiteAddress::from(UcanAddress::new(endpoint));
        RepositoryConfiguration::default()
            .remote(
                "origin",
                // A remote an invite can embed has to name the relay its
                // revocations get published to, or the mint refuses it.
                RemoteConfiguration::new(address)
                    .revocation_url("https://relay.example.test/revocations".parse().unwrap()),
            )
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
    /// the whole point of the opt-in remote, so `tonk join` has
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
                    // The link's prefix comes from the request origin, which
                    // the browser-to-axum conversion stamps on every real
                    // request; a hand-built one has to supply it.
                    .extension(
                        crate::axum::RequestOrigin::parse("https://local.example/invite")
                            .expect("valid origin"),
                    )
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
        let mut subscriber;
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
                .subscribe(ConceptQuery::from(Query::<Name>::default()), None)
                .expect("subscribe");
            before_ptr = Arc::as_ptr(&session.state);
        }
        // Drain whatever subscribing itself delivered, so anything that
        // arrives next can only be the refresh's own doing.
        while subscriber.receiver.try_recv().is_ok() {}

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
        // Surviving is not enough: the rebound subscription's retained
        // result was discarded with the old engine, so it must be handed
        // a fresh snapshot NOW. Left waiting for the next commit, a live
        // view over a just-wired quiet space showed its loading state
        // indefinitely — the share flow's infinite loader.
        assert!(
            subscriber.receiver.try_recv().is_ok(),
            "the rebound subscription must be handed a fresh snapshot",
        );
        drop(subscriber);
    }

    /// Creating a repository records its creator as a member on the
    /// repo's content branch, stamped with the founder role.
    #[dialog_common::test]
    async fn it_records_the_founder_membership_on_create() {
        let (_app, state, key) = fresh_repo("test-founder-membership").await;

        let memberships = crate::router::tests::content_memberships(&state, &key).await;
        // Keyed on the local root, not the device that created it, so the
        // row converges across every device holding the same root.
        let (root_entity, device_entity) = {
            let guard = state.read().await;
            use tonk_schema::prelude::DidExt as _;
            (
                crate::router::identity::root_did(&guard)
                    .await
                    .expect("the test profile has a local root")
                    .this(),
                guard.profile.did().this(),
            )
        };
        // Every create mints a fresh routing key, so the repo is brand
        // new: exactly the founder's membership.
        assert_eq!(memberships.len(), 1, "exactly the founder membership");
        assert_eq!(memberships[0].member.0, root_entity);
        assert_ne!(
            memberships[0].member.0, device_entity,
            "no device-keyed row was written",
        );

        // The creator's membership is stamped `founder`.
        let roles = crate::router::tests::content_member_roles(&state, &key).await;
        let role = roles
            .iter()
            .find(|r| r.this == *memberships[0].this())
            .expect("founder role stamped on create");
        assert_eq!(role.role.0.to_string(), tonk_schema::MemberRole::FOUNDER);
    }

    /// Creating a space stamps who founded it and when, onto the
    /// account-directory entity the Hub renders.
    #[dialog_common::test]
    async fn it_stamps_space_founding_on_create() {
        use dialog_query::{Output as _, Query, Term};
        use dialog_varsig::Did;
        use tonk_schema::prelude::DidExt as _;

        let (_app, state, key) = fresh_repo("test-space-founding").await;

        let guard = state.read().await;
        let subject: Did = key.parse().expect("the repository is named by its DID");
        let profile_entity = guard.profile.did().this();

        let branch = guard
            .reactor
            .profile_repository()
            .branch(super::PROFILE_BRANCH)
            .acquire(&guard.operator)
            .await
            .expect("profile branch opens");
        let rows: Vec<tonk_schema::SpaceFounded> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceFounded> {
                this: Term::from(subject.this()),
                founded_at: Term::var("founded_at"),
                founded_by: Term::var("founded_by"),
            })
            .perform(&guard.operator)
            .try_vec()
            .await
            .expect("founding query runs");

        assert_eq!(rows.len(), 1, "exactly one founding stamp");
        assert_eq!(
            rows[0].founded_by.0, profile_entity,
            "the founding device is recorded, not just the account",
        );
        assert!(rows[0].founded_at.0 > 0, "a real timestamp");
    }

    /// Creating a repository names the creator on the content branch.
    #[dialog_common::test]
    async fn it_records_the_founder_name_on_create() {
        let (_app, state, key) = fresh_repo("test-founder-name").await;

        let names = crate::router::tests::content_member_names(&state, &key).await;
        let memberships = crate::router::tests::content_memberships(&state, &key).await;
        assert_eq!(names.len(), 1, "exactly the founder's name");
        assert_eq!(names[0].this, memberships[0].this);
        assert!(!names[0].name.0.is_empty(), "a non-empty display name");
    }

    /// Build a one-entity transient `Invite{this, time, marker}` batch —
    /// the facts the share form's submit event asserts. Carries both the
    /// `time-stamp` (`dom.event/time-stamp`) and the `marker`
    /// (`dom.event.current-target.dataset/invite`) so it decodes as an
    /// `Invite` command and not a `PauseSync` (identical `{this, time}`
    /// shape otherwise).
    fn invite_transient(of: &str) -> dialog_artifacts::Changes {
        use dialog_artifacts::{Entity, Statement};
        use dialog_query::the;

        let entity: Entity = of.parse().expect("entity URI");
        let mut changes = dialog_artifacts::Changes::new();
        the!("dom.event/time-stamp")
            .of(entity.clone())
            .is(1.0_f64)
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/invite")
            .of(entity)
            .is("tonk:invite".parse::<Entity>().expect("marker URI"))
            .assert(&mut changes);
        changes
    }

    /// Dispatching a `tonk:invite` command clears the overlay (to rotate
    /// the credential) but MUST re-stamp `state:self` so the topbar chip
    /// retains the member's identity data. Without the re-stamp the chip
    /// goes blank until the next sync_status poll (~20 s).
    #[dialog_common::test]
    async fn it_restamps_state_self_after_invite_clears_the_overlay() {
        use dialog_query::{Output as _, Query, Term};

        let (app, state, key) = fresh_repo("test-invite-restamps-self").await;

        // Attach a remote first — `run_invite` refuses to mint (and never
        // reaches the credential overlay write this test exercises) against
        // a repo whose `main` has no upstream.
        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(
                    "https://sync.example.test/ucan/",
                ))),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let attach = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            attach.status(),
            StatusCode::OK,
            "remote attach should succeed"
        );

        // Prime state:self so we have something to lose.
        {
            let tonk = state.read().await;
            crate::router::sync::publish_self_identity(&tonk, &key, "main").await;
        }

        // Drive the invite command through the real dispatcher — same path
        // the share modal takes.
        let changes = invite_transient("did:key:zInviteCmd");
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: key.clone(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        // state:self must still be present on the overlay after run_invite's
        // clear_overlay + re-stamp sequence.
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("acquire main");

        let entity: dialog_artifacts::Entity =
            tonk_schema::Replica::SELF_STATE_HERE.parse().unwrap();
        let rows: Vec<tonk_schema::ProfileIdentity> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::ProfileIdentity> {
                this: Term::from(entity),
                did: Term::var("did"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();

        assert_eq!(
            rows.len(),
            1,
            "state:self must be re-stamped after invite clears the overlay",
        );
    }

    /// The refusal class follows the account's registration state.
    ///
    /// Same space, same missing upstream, three different remedies: an
    /// account that is served can attach one, an enrolled account is
    /// waiting on its email, and an unregistered one has to register.
    #[dialog_common::test]
    async fn it_names_the_refusal_by_registration_state() {
        use crate::router::create_invite::{RemoteRefusal, explain_refusal};
        use tonk_account::customer::CustomerStatus;

        let (_app, state, _key) = fresh_repo("test-refusal-by-state").await;
        let tonk = state.read().await;

        assert_eq!(
            explain_refusal(&tonk, RemoteRefusal::NotSynced)
                .await
                .code(),
            "needs-account",
            "nothing registered, so the remedy is to register",
        );

        crate::router::customer::record_test_customer(&tonk, CustomerStatus::Registered)
            .await
            .expect("the customer records");
        assert_eq!(
            explain_refusal(&tonk, RemoteRefusal::NotSynced)
                .await
                .code(),
            "needs-activation",
            "enrolled but unconfirmed: the remedy is in the inbox",
        );

        crate::router::customer::record_test_customer(&tonk, CustomerStatus::Active)
            .await
            .expect("the customer records");
        assert_eq!(
            explain_refusal(&tonk, RemoteRefusal::NotSynced)
                .await
                .code(),
            "not-synced",
            "served, so attaching a remote is the remedy after all",
        );

        // A refusal that already knows its cause is left alone.
        assert_eq!(
            explain_refusal(&tonk, RemoteRefusal::UnshareableRemote)
                .await
                .code(),
            "unshareable-remote",
        );
    }

    /// A share click on a space with no upstream mints nothing and leaves a
    /// refusal on the overlay instead.
    ///
    /// The class says WHY there is no upstream. This profile has never
    /// registered, so there is no provider to attach one to and the
    /// remedy is to register — not "turn on sync", which would offer an
    /// attach with nothing to attach to.
    #[dialog_common::test]
    async fn it_refuses_to_mint_without_a_remote() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-refuse-mint").await;

        run_invite_with_time(&state, &key, 1234.0).await;

        let blocked = share_blocked_rows(&state, &key).await;
        assert_eq!(blocked.len(), 1, "one refusal recorded");
        assert_eq!(blocked[0].0, "needs-account");
        assert_eq!(blocked[0].2, 1234.0, "echoes the command's timestamp");

        let invitations = content_invitations(&state, &key).await;
        assert!(
            invitations.is_empty(),
            "a refused mint records no invitation"
        );
    }

    /// The command path is what the FABB drives. It must answer a raced or
    /// stale share click with an account refusal and mint no authority.
    #[dialog_common::test]
    async fn it_refuses_to_mint_without_an_attached_account() {
        let (app, state, key) = fresh_repo_signed_out("test-account-required-mint").await;
        let _ = post_remote(&app, &key, "https://access.example.test/ucan/", None).await;

        run_invite_with_time(&state, &key, 4321.0).await;

        let blocked = share_blocked_rows(&state, &key).await;
        assert_eq!(blocked.len(), 1, "one refusal recorded");
        assert_eq!(
            blocked[0].0,
            tonk_worker_api::share::BLOCKED_ACCOUNT_REQUIRED
        );
        assert_eq!(
            blocked[0].1,
            "Create an account or log in before sharing this space."
        );
        assert_eq!(blocked[0].2, 4321.0, "echoes the command's timestamp");
        assert!(
            content_invitations(&state, &key).await.is_empty(),
            "an unattached profile records no invitation"
        );
    }

    /// POST a remote config to `key`, exactly as the topbar and the share
    /// prompt's confirm do. Unlike [`attach_remote`] it names no relay
    /// unless asked, so a test can produce the pre-in-band-revocation shape:
    /// a space that syncs but cannot mint.
    async fn post_remote(
        app: &Router,
        key: &str,
        endpoint: &str,
        relay: Option<&str>,
    ) -> RepositoryInfo {
        use dialog_remote_ucan_s3::UcanAddress;

        let mut remote = RemoteConfiguration::new(SiteAddress::from(UcanAddress::new(endpoint)));
        if let Some(relay) = relay {
            remote = remote.revocation_url(relay.parse().unwrap());
        }
        let config = RepositoryConfiguration::default()
            .remote("origin", remote)
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/remote"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&config).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "remote attach succeeds");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// A second attach does not repoint a remote that is already there.
    ///
    /// The share prompt builds its endpoint from the page's origin, which
    /// need not be the origin the space actually syncs through, and dialog
    /// leaves an existing remote as-is — so the meta mirror has to keep
    /// describing the remote that is really there rather than adopting the
    /// caller's.
    ///
    /// This used to also assert that a remote carrying no revocation relay
    /// refused the mint. Revocations travel in-band on `/ucan/` now, so
    /// there is no relay to be missing and nothing produces that refusal;
    /// minting without one is the ordinary case, asserted here.
    #[dialog_common::test]
    async fn it_does_not_repoint_a_remote_that_is_already_attached() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-relay-repair").await;
        let _ = post_remote(&app, &key, "https://access.example.test/ucan/", None).await;

        run_invite_with_time(&state, &key, 11.0).await;

        assert!(
            share_blocked_rows(&state, &key).await.is_empty(),
            "a remote without a relay is no longer a refusal",
        );
        assert_eq!(
            content_invitations(&state, &key).await.len(),
            1,
            "so the mint records its invitation",
        );

        let info = post_remote(
            &app,
            &key,
            "https://a-different-origin.example.test/ucan/",
            None,
        )
        .await;

        let address = serde_json::to_string(&info.remote["origin"].address).unwrap();
        assert!(
            address.contains("https://access.example.test/ucan/"),
            "a second attach must not repoint the remote, got {address}",
        );
    }

    /// Drive `run_invite` with a fixed timestamp, the way a `tonk:invite`
    /// transient would.
    async fn run_invite_with_time(state: &AppState, repo: &str, time: f64) {
        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        let _ = super::run_invite(&env, repo, time).await;
    }

    /// Read back every `ShareBlocked` row on the repo's content branch overlay
    /// as `(blocked, detail, time)`.
    async fn share_blocked_rows(state: &AppState, repo: &str) -> Vec<(String, String, f64)> {
        use dialog_query::{Output as _, Term};
        use tonk_schema::command::ShareBlocked;

        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .repository(repo)
            .branch(super::CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("content branch opens");
        let rows: Vec<ShareBlocked> = branch
            .handle()
            .query()
            .select(dialog_query::Query::<ShareBlocked> {
                this: Term::var("this"),
                blocked: Term::var("blocked"),
                detail: Term::var("detail"),
                time: Term::var("time"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("share-blocked query");
        rows.into_iter()
            .map(|row| (row.blocked.0, row.detail.0, row.time.0))
            .collect()
    }

    /// Attaching a remote through the command targets the EXISTING space. The
    /// `space/enable-sync` command in `core.yaml` shares `CreateSpace`'s trigger
    /// attribute and so mints a new space instead; this guards against that.
    #[dialog_common::test]
    async fn it_attaches_the_remote_to_the_existing_space() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (key, subject) = put_repo_info(&app, "test-enable-sync").await;
        let before = existing_space_labels(&state).await.len();

        dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", false, 1.0).await;

        assert_eq!(
            existing_space_labels(&state).await.len(),
            before,
            "no new space was created"
        );
        assert!(
            has_remote_upstream(&state, &key).await,
            "the existing space now tracks origin/main"
        );
    }

    /// A space created while the customer is not `Active` wires no
    /// remote, even though an account exists.
    ///
    /// A device has an account from first boot (the onboarding account),
    /// so "an account exists" says nothing about whether the access
    /// service will serve this subject. Until the user enrols and
    /// confirms an email, `/provider/add` refuses and a wired upstream
    /// would answer `subject is provisioned by an active customer (the
    /// subject is not provisioned)` on every presign. The space is
    /// local-only by design; the share button attaches sync later.
    #[dialog_common::test]
    async fn it_creates_a_space_local_only_before_the_customer_is_active() {
        let (app, state, key) = fresh_repo("test-inactive-no-remote").await;
        {
            let tonk = state.read().await;
            crate::router::customer::record_test_customer(
                &tonk,
                tonk_account::customer::CustomerStatus::Registered,
            )
            .await
            .expect("the customer record saves");
        }
        let _ = &app;

        assert!(
            !has_remote_upstream(&state, &key).await,
            "a space created before activation must track no upstream",
        );
    }

    /// The gate reads what the account actually proved, not merely that
    /// one exists: enrolled-but-unconfirmed is as unservable as no
    /// registration at all.
    ///
    /// One account per state, because the facts are monotone — an
    /// activation is never unmade by a later enrollment answer, which is
    /// the race the three-fact shape exists to prevent. Reusing one
    /// account across states would assert the opposite.
    #[dialog_common::test]
    async fn it_treats_an_enrolled_but_unconfirmed_customer_as_inactive() {
        use tonk_account::customer::CustomerStatus;

        let (_app, registered, _k1) = fresh_repo("test-registered-inactive").await;
        {
            let tonk = registered.read().await;
            crate::router::customer::record_test_customer(&tonk, CustomerStatus::Registered)
                .await
                .expect("the customer record saves");
            assert!(
                !crate::router::customer::is_active(&tonk).await,
                "a Registered customer awaits email activation and is not servable",
            );
        }

        let (_app, suspended, _k2) = fresh_repo("test-suspended-inactive").await;
        {
            let tonk = suspended.read().await;
            crate::router::customer::record_test_customer(&tonk, CustomerStatus::Suspended)
                .await
                .expect("the customer record saves");
            assert!(
                !crate::router::customer::is_active(&tonk).await,
                "a Suspended customer is not servable",
            );
        }

        let (_app, active, _k3) = fresh_repo("test-active-servable").await;
        {
            let tonk = active.read().await;
            crate::router::customer::record_test_customer(&tonk, CustomerStatus::Active)
                .await
                .expect("the customer record saves");
            assert!(
                crate::router::customer::is_active(&tonk).await,
                "an Active customer is the one state the service serves",
            );
        }
    }

    /// The account's provider is read from the registration fact, so
    /// every device on the account attaches spaces to the same one.
    ///
    /// It used to be re-derived per call site — from the signed account
    /// descriptor in the worker, and from `https://{origin}/ucan/` in the
    /// page's hidden form field — so two paths could disagree about
    /// where a space syncs. Recording it where registration happens is
    /// what makes that one answer.
    #[dialog_common::test]
    async fn it_reads_the_provider_from_the_registration_fact() {
        let (_app, state, _key) = fresh_repo("test-recorded-remote").await;

        let tonk = state.read().await;
        assert!(
            crate::router::customer::provider_address(&tonk)
                .await
                .is_none(),
            "an account that never registered records no provider",
        );

        crate::router::customer::record_test_customer(
            &tonk,
            tonk_account::customer::CustomerStatus::Active,
        )
        .await
        .expect("the customer record saves");

        assert_eq!(
            crate::router::customer::provider_address(&tonk)
                .await
                .as_deref(),
            Some("https://example.test/ucan/"),
            "the provider registration recorded is what attach paths read",
        );
    }

    /// Recording an enrollment with no provider must not make the
    /// registration fact unreadable.
    ///
    /// A concept resolves only when every field is present, so writing
    /// `provider` as an empty string risks asserting nothing for it and
    /// dropping the whole row — which reads back as "never registered"
    /// however many times the status is written afterwards.
    #[dialog_common::test]
    async fn it_reads_a_registration_recorded_without_a_provider() {
        use crate::router::customer::{Registration, record_customer_status, registration};
        use tonk_account::customer::CustomerStatus;

        let (_app, state, _key) = fresh_repo("test-empty-provider-row").await;
        let tonk = state.read().await;

        record_customer_status(&tonk, CustomerStatus::Registered, "who@example.test", None)
            .await
            .expect("the status records");
        assert_eq!(
            registration(&tonk).await,
            Registration::AwaitingActivation {
                email: "who@example.test".to_owned(),
            },
            "a registration recorded before activation must still read back",
        );

        // And the later activation write must be visible through it.
        record_customer_status(
            &tonk,
            CustomerStatus::Active,
            "who@example.test",
            Some("https://hub.test/ucan/"),
        )
        .await
        .expect("the status records");
        assert_eq!(
            registration(&tonk).await,
            Registration::Served {
                provider: "https://hub.test/ucan/".to_owned(),
            },
            "activation must promote the row a provider-less write created",
        );
    }

    /// Activation carries its provider, so "active with no address"
    /// cannot arise.
    ///
    /// The old shape wrote a status string and an address as separate
    /// fields, so a space created between the two writes came up
    /// local-only and the user was told to confirm an email they had
    /// already confirmed. `tonk:account/active` carries both or neither:
    /// there is no in-between to fall into.
    #[dialog_common::test]
    async fn it_records_no_activation_without_a_provider_to_serve_from() {
        use crate::router::customer::{
            Registration, is_active, record_customer_status, registration,
        };
        use tonk_account::customer::CustomerStatus;

        let (_app, state, _key) = fresh_repo("test-active-no-provider").await;
        let tonk = state.read().await;

        // An activation answer that names no provider records the
        // registration and withholds the activation, rather than
        // claiming served with nowhere to serve from.
        record_customer_status(&tonk, CustomerStatus::Active, "who@example.test", None)
            .await
            .expect("the status records");

        assert!(
            matches!(
                registration(&tonk).await,
                Registration::AwaitingActivation { .. }
            ),
            "no provider means nothing was activated",
        );
        assert!(!is_active(&tonk).await);

        // The answer that names one activates.
        record_customer_status(
            &tonk,
            CustomerStatus::Active,
            "who@example.test",
            Some("https://service.example/ucan/"),
        )
        .await
        .expect("the status records");

        assert!(
            matches!(registration(&tonk).await, Registration::Served { .. }),
            "an activation with a provider is served",
        );
        assert!(is_active(&tonk).await);
    }

    /// Registration reads as one of four states, and the provider
    /// address is what separates them.
    ///
    /// The service names a provider only once it serves the customer, so
    /// "has an address" IS "finished registering". That is what lets the
    /// share flow tell "confirm your email" from "register from
    /// scratch" without asking the service.
    #[dialog_common::test]
    async fn it_reads_how_far_registration_got() {
        use crate::router::customer::{Registration, registration};
        use tonk_account::customer::CustomerStatus;

        let (_app, state, _key) = fresh_repo("test-registration-states").await;
        let tonk = state.read().await;

        assert_eq!(
            registration(&tonk).await,
            Registration::Unregistered,
            "an account that never enrolled has registered nothing",
        );

        // Enrollment records the address but no provider: the service
        // withholds one until the emailed link is confirmed.
        crate::router::customer::record_customer_status(
            &tonk,
            CustomerStatus::Registered,
            "customer@example.test",
            None,
        )
        .await
        .expect("the status records");
        assert_eq!(
            registration(&tonk).await,
            Registration::AwaitingActivation {
                email: "customer@example.test".to_owned(),
            },
            "an enrolled account with no provider is still awaiting its email",
        );
        assert!(
            !crate::router::customer::is_active(&tonk).await,
            "awaiting activation is not served, so nothing may attach a remote",
        );

        // Activation is where the provider lands.
        crate::router::customer::record_customer_status(
            &tonk,
            CustomerStatus::Active,
            "customer@example.test",
            Some("https://hub.test/ucan/"),
        )
        .await
        .expect("the status records");
        assert_eq!(
            registration(&tonk).await,
            Registration::Served {
                provider: "https://hub.test/ucan/".to_owned(),
            },
            "an activated account names the provider its spaces attach to",
        );
        assert!(crate::router::customer::is_active(&tonk).await);

        // Suspension is terminal, and outranks a recorded provider: no
        // email confirms it away.
        crate::router::customer::record_customer_status(
            &tonk,
            CustomerStatus::Suspended,
            "customer@example.test",
            Some("https://hub.test/ucan/"),
        )
        .await
        .expect("the status records");
        assert_eq!(
            registration(&tonk).await,
            Registration::Suspended,
            "a suspended account is refused regardless of its recorded provider",
        );
        assert!(!crate::router::customer::is_active(&tonk).await);
    }

    /// Enable-sync still attaches when provisioning cannot run.
    ///
    /// A remote is not necessarily our access service, and the service
    /// may simply be unreachable. Refusing the attach on a failed
    /// provision would make a self-hosted endpoint unattachable, so the
    /// attach proceeds regardless — the gate is on the CREATE default,
    /// not on an explicit request to sync.
    #[dialog_common::test]
    async fn it_attaches_sync_even_when_provisioning_cannot_run() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (key, subject) = put_repo_info(&app, "test-attach-without-provision").await;

        dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", false, 1.0).await;

        assert!(
            has_remote_upstream(&state, &key).await,
            "an explicit enable-sync attaches even with no reachable service to provision against",
        );
    }

    /// Without the `share` marker the handler attaches and stops.
    #[dialog_common::test]
    async fn it_mints_only_when_asked_to_share() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (key, subject) = put_repo_info(&app, "test-enable-sync-no-share").await;

        dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", false, 1.0).await;

        // Assert the handler RAN before asserting what it declined to do.
        // Without this, deleting the handler outright would leave the
        // transient matching nothing, and the emptiness check below would
        // still pass -- proving only that no invitation appeared from thin
        // air.
        assert!(
            has_remote_upstream(&state, &key).await,
            "the handler ran and attached the remote"
        );
        assert!(
            content_invitations(&state, &key).await.is_empty(),
            "attach-only records no invitation"
        );
    }

    /// With the marker, the attach is followed by a mint — the single-click path.
    #[dialog_common::test]
    async fn it_mints_after_attaching_when_asked_to_share() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (key, subject) = put_repo_info(&app, "test-enable-sync-share").await;

        dispatch_enable_sync(&state, &subject, "https://example.test/ucan/", true, 1.0).await;

        assert_eq!(
            content_invitations(&state, &key).await.len(),
            1,
            "the attach is followed by exactly one mint"
        );
    }

    /// Build the `tonk:enable-sync` transient the FAB dispatches and run it
    /// through `dispatch`, the way `/transact` does after a commit. Going through
    /// `dispatch` (not the handler directly) means this also covers registration
    /// and trigger matching.
    async fn dispatch_enable_sync(
        state: &AppState,
        subject: &str,
        remote: &str,
        share: bool,
        time: f64,
    ) {
        use dialog_artifacts::{Changes, Statement};
        use dialog_query::{Entity, the};

        let of: Entity = "tonk:enable-sync-test".parse().expect("entity URI");
        let mut changes = Changes::new();
        the!("dom.event/time-stamp")
            .of(of.clone())
            .is(time)
            .assert(&mut changes);
        the!("dom.event.current-target.dataset/enable-sync")
            .of(of.clone())
            .is("tonk:enable-sync".parse::<Entity>().expect("marker entity"))
            .assert(&mut changes);
        the!("xyz.tonk.enable-sync/space")
            .of(of.clone())
            .is(subject.parse::<Entity>().expect("subject entity"))
            .assert(&mut changes);
        the!("xyz.tonk.enable-sync/remote")
            .of(of.clone())
            .is(remote.to_string())
            .assert(&mut changes);
        the!("xyz.tonk.enable-sync/revocation-url")
            .of(of.clone())
            .is("https://relay.example.test/revocations".to_string())
            .assert(&mut changes);
        if share {
            the!("xyz.tonk.enable-sync/share")
                .of(of)
                .is("tonk:share".parse::<Entity>().expect("share entity"))
                .assert(&mut changes);
        }

        crate::router::dispatch(state, crate::router::CommandOrigin::default(), changes).await;
    }

    /// Whether the repo's `main` tracks a remote upstream — the exact condition
    /// `resolve_remote_url_with` probes.
    async fn has_remote_upstream(state: &AppState, repo: &str) -> bool {
        use dialog_repository::{RepositoryExt as _, Upstream};

        let tonk = state.read().await;
        let Ok(repository) = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
        else {
            return false;
        };
        let Ok(main) = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
        else {
            return false;
        };
        matches!(main.upstream(), Some(Upstream::Remote { .. }))
    }

    /// Build a one-entity transient `RemoveSpace{this, subject}` batch —
    /// the facts the Hub's delete-confirm form asserts. Mirrors
    /// `profile_rename_transient`: the `data-remove` marker attribute
    /// (`dom.event.current-target.dataset/remove`) carries the target
    /// subject DID as its value and is the command's whole payload (see
    /// `tonk_schema::command::RemoveSpace`).
    fn remove_space_transient(of: &str, subject: &dialog_varsig::Did) -> dialog_artifacts::Changes {
        use dialog_artifacts::{Entity, Statement};
        use dialog_query::the;
        use tonk_schema::prelude::DidExt as _;

        let entity: Entity = of.parse().expect("entity URI");
        let mut changes = dialog_artifacts::Changes::new();
        the!("dom.event.current-target.dataset/remove")
            .of(entity)
            .is(subject.this())
            .assert(&mut changes);
        changes
    }

    /// `RemoveSpace` is refused unless it fired on the profile branch
    /// (empty origin repo) — the gate closing the finding that a
    /// same-shaped `dom.event.current-target.dataset/remove` fact
    /// committed on ANY content branch (a joined space's own notation, or
    /// a same-origin POST to that repo's `/transact`) could otherwise name
    /// and delete any space by DID. Fired here with a non-empty origin, as
    /// that cross-branch dispatch would produce; the replica record must
    /// survive untouched.
    #[dialog_common::test]
    async fn it_ignores_remove_space_from_a_non_profile_origin() {
        use tonk_schema::prelude::DidExt as _;

        let (_app, state, key) = fresh_repo("test-remove-non-profile-origin").await;

        let subject: dialog_varsig::Did = {
            let tonk = state.read().await;
            use dialog_repository::RepositoryExt as _;
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("repo loads");
            repository.did()
        };

        let changes = remove_space_transient("did:key:zRemoveWrongOrigin", &subject);
        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin {
                repo: "somerepo".to_string(),
                branch: "main".to_string(),
                client: None,
            },
            changes,
        )
        .await;

        let remaining = profile_replicas(&state).await;
        assert!(
            remaining.iter().any(|r| r.subject.0 == subject.this()),
            "a RemoveSpace fired from a non-profile origin must not remove the replica",
        );
    }

    /// The invite URL puts the seed in the fragment and the delegation in
    /// the query, on the worker's own origin.
    ///
    /// Driven through [`long_invite_url`] directly rather than through the
    /// mint: the test harness's worker scope reports no `location.origin`,
    /// so a mint always takes the no-origin fallback and the branch that
    /// actually runs in production would never be exercised.
    ///
    /// The fragment split is the load-bearing part. The seed must never
    /// reach a server, and shortening PUTs only the path + query — so a
    /// seed that slipped into the query would be uploaded to the shortcut
    /// service in plaintext.
    #[dialog_common::test]
    async fn it_builds_the_invite_url_on_the_worker_origin() {
        let url = super::long_invite_url(
            Some("https://tonk.example"),
            "PROOF",
            "&remote=https%3A%2F%2Fhub%2Fucan%2F",
            "SEED",
        );

        assert_eq!(
            url,
            "https://tonk.example/join\
             ?access=PROOF&remote=https%3A%2F%2Fhub%2Fucan%2F#SEED",
        );

        // The secret is the fragment, never the query — everything before
        // `#` is what a shortcut PUT would upload.
        let (sent, fragment) = url.split_once('#').expect("the seed must be a fragment");
        assert_eq!(fragment, "SEED");
        assert!(
            !sent.contains("SEED"),
            "the seed must not appear in the path or query: {sent}",
        );
    }

    /// A local-only repo has no sync endpoint, so the invite carries no
    /// `&remote=`. The suffix is empty rather than absent-and-malformed:
    /// `Invite::parse_url` rejects an empty `remote=`, so "no remote" has
    /// to append *nothing*.
    #[dialog_common::test]
    async fn it_omits_the_remote_for_a_local_only_repo() {
        let url = super::long_invite_url(Some("https://tonk.example"), "PROOF", "", "SEED");
        assert_eq!(url, "https://tonk.example/join?access=PROOF#SEED");
        assert!(!url.contains("remote="));
    }

    /// Outside a worker scope there is no origin to build on (and no
    /// service to shorten against), so the URL falls back to the default
    /// base — still well-formed and redeemable, never a broken link.
    #[dialog_common::test]
    async fn it_falls_back_to_the_default_base_without_an_origin() {
        let url = super::long_invite_url(None, "PROOF", "", "SEED");
        assert!(
            url.starts_with(tonk_invite::DEFAULT_BASE_URL),
            "expected the default base, got {url}",
        );
        assert!(
            url.ends_with("#SEED"),
            "the seed must still be the fragment"
        );
        assert!(url.contains("access=PROOF"));
    }
}

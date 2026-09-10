//! Repository create route.
//!
//! `PUT /api/repository/{repo}` always creates a fresh repository. The
//! repository's identity is its credential's `did:key`; the `{repo}`
//! path segment is only a display label. Every create mints a new
//! identity, so there is never a create-time collision — two spaces may
//! share a label. The response carries the new repository's routing key
//! (the DID suffix), which the UI routes by.

use dialog_capability::Subject;
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

/// The attribute carrying the optional sync URL on a `space/create` or
/// `space/enable-sync` transient. Kept in step with those notation
/// commands' `remote` field `the:`.
const REMOTE_ATTR: &str = "xyz.tonk.command.create-space/remote";

/// The same field before the command took its own namespace: the DOM
/// read path that filled it. Still asserted by any branch seeded before
/// the migration, so both are read.
const LEGACY_REMOTE_ATTR: &str = "dom.event.current-target.elements.remote/value";

/// Read the optional remote URL from a transient's facts, tolerating
/// both `Value::String` and `Value::Entity`.
///
/// A URL like `http://host/ucan/` round-trips through JSON, and the
/// worker's untagged `Value` deserialization picks `Entity` for any
/// string containing a `:` — so a `String`-typed concept field never
/// decodes a URL (that's the bug a `remote: String` field hit). Reading
/// the artifact directly sidesteps the concept decode and accepts either
/// representation. Empty/whitespace → `None` (a local-only space).
///
/// This is why `remote` is still not a field on [`CreateSpace`] even
/// after the command took its own namespace: the obstacle is how a URL
/// is *represented*, not how the command is *shaped*, and the two are
/// separate problems.
///
/// [`CreateSpace`]: tonk_schema::command::CreateSpace
fn remote_from_facts(facts: &crate::reactor::EntityFacts) -> Option<String> {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| {
            let the = artifact.the.to_string();
            the == REMOTE_ATTR || the == LEGACY_REMOTE_ATTR
        })
        .and_then(|artifact| match &artifact.is {
            Value::String(url) => Some(url.clone()),
            Value::Entity(uri) => Some(uri.to_string()),
            _ => None,
        })
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// The `space/create` transient's optional `open` flag.
///
/// Absent (or false) means create only — the space appears in the Hub and
/// the person stays where they are. Present and true means create and
/// navigate, which is what the Hub's own create form asks for.
///
/// Read from the raw facts rather than declared on [`CreateSpace`] for
/// the same reason the remote is: the command is matched name-only, so a
/// declared field would make every create that omits it fail to decode.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
const OPEN_ATTR: &str = "xyz.tonk.command.create-space/open";

/// Whether the create should navigate the caller into the new space.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
fn open_from_facts(facts: &crate::reactor::EntityFacts) -> bool {
    use dialog_artifacts::Value;

    facts
        .iter()
        .find(|artifact| artifact.the.to_string() == OPEN_ATTR)
        .map(|artifact| match &artifact.is {
            Value::Boolean(open) => *open,
            // A form field arrives as text; treat anything but an
            // explicit falsehood as asking to open, since carrying the
            // field at all is the request.
            Value::String(text) => !matches!(text.trim(), "" | "false" | "0"),
            Value::Entity(uri) => uri.to_string() != "case:false",
            _ => false,
        })
        .unwrap_or(false)
}

/// The `tonk:enable-sync` transient's target space, read from the raw facts.
const ENABLE_SYNC_SPACE_ATTR: &str = "xyz.tonk.enable-sync/space";

/// The `tonk:enable-sync` transient's endpoint, read from the raw facts.
const ENABLE_SYNC_REMOTE_ATTR: &str = "xyz.tonk.enable-sync/remote";

/// Marker asking the handler to mint once the remote is attached.
const ENABLE_SYNC_SHARE_ATTR: &str = "xyz.tonk.enable-sync/share";

/// Read a fact's value as a string, tolerating both the `String` and
/// `Entity` representations — a URL or a DID round-trips through JSON as an
/// `Entity` (any `:`-bearing string does), so a single-representation read
/// would silently miss them. Mirrors [`remote_from_facts`].
fn text_fact(facts: &crate::reactor::EntityFacts, attribute: &str) -> Option<String> {
    text_fact_any_target(facts, attribute)
}

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
const UNTITLED: &str = "Untitled";

/// Pick the first free untitled label: `Untitled`, then `Untitled 2`,
/// `Untitled 3`, … — the smallest ordinal no existing label already
/// uses. Only exact `Untitled` / `Untitled <n>` labels count as taken;
/// anything else (user-typed names, key fallbacks) is ignored.
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

/// The full decode surface of a `space/create` (or legacy topbar
/// `space/enable-sync`) transient: the typed [`CreateSpace`] command —
/// current or legacy shape — plus the optional sync URL read straight
/// from the raw facts by [`remote_from_facts`].
///
/// `CreateSpace` is matched **name-only** so it keeps decoding against an
/// older, frozen profile descriptor (see [`CreateSpace`]). The remote is
/// NOT a concept field, both because a required field would break the
/// frozen-descriptor match and because a URL deserializes as
/// `Value::Entity`, which a `String` field can't decode — this wrapper's
/// hand-written [`Decode`](crate::reactor::Decode) is what lets the
/// typed provider still see it.
///
/// [`CreateSpace`]: tonk_schema::command::CreateSpace
pub(crate) struct CreateSpaceRequest {
    /// The decoded command (a legacy shape arrives converted).
    command: tonk_schema::command::CreateSpace,
    /// The optional sync URL, read from the raw facts.
    remote: Option<String>,
}

impl crate::reactor::Decode for CreateSpaceRequest {
    fn trigger_attributes() -> Vec<String> {
        crate::reactor::Migrated::<
            tonk_schema::command::CreateSpace,
            tonk_schema::command::legacy::CreateSpace,
        >::new()
        .trigger_attributes()
        .to_vec()
    }

    fn decode(
        _this: dialog_artifacts::Entity,
        facts: &crate::reactor::EntityFacts,
    ) -> Option<Self> {
        let command = crate::reactor::Migrated::<
            tonk_schema::command::CreateSpace,
            tonk_schema::command::legacy::CreateSpace,
        >::new()
        .decode(facts)?;
        Some(Self {
            command,
            remote: remote_from_facts(facts),
        })
    }
}

impl dialog_capability::Command for CreateSpaceRequest {
    type Input = Self;
    type Output = ();
}

/// Run the "New space" form (`space/create`).
///
/// The repository is **always created** with a freshly minted identity
/// (`create_space_inner` returns its routing key), then, if a remote was
/// given, attached best-effort via [`enable_sync_inner`] to that key.
/// The `name` is only a display label; two spaces may share it. The
/// create wizard doesn't ask for one — its hidden input carries the
/// [`UNTITLED`] sentinel, which is uniquified against the existing space
/// labels ([`next_untitled_label`]) so consecutive creates read
/// "Untitled", "Untitled 2", …. Once the space is created and seeded, a
/// `navigate` message goes back to the originating client so the creator
/// lands inside the new space. A remote/auth failure leaves a working
/// local space, retryable from the topbar.
///
/// Only the profile branch may mint: creating a space is a profile-space
/// capability, and a content branch asserting the same shape (including
/// the legacy topbar `space/enable-sync` form, which used to mint a
/// fresh space as a side effect) is refused rather than trusted.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<CreateSpaceRequest> for crate::router::CommandEnv {
    async fn execute(&self, request: CreateSpaceRequest) {
        execute_create_space(self.clone(), request).await
    }
}

async fn execute_create_space(env: crate::router::CommandEnv, request: CreateSpaceRequest) {
    let name = request.command.name.0;
    let remote = request.remote;
    if !env.from_profile() {
        log!(
            "CreateSpace ignored: origin '{}' is not the profile branch — \
                 a space cannot mint spaces",
            env.origin().repo
        );
        return;
    }

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

    crate::router::navigate::notify_analytics(
        env.client(),
        tonk_worker_api::AnalyticsEvent::SpaceCreated { space: key.clone() },
    );

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
const INVITE_SPACE_ATTR: &str = "xyz.tonk.invite/space";

/// Read the target space DID from a `tonk:invite` transient's facts,
/// opportunistically — mirrors [`remote_from_facts`].
///
/// `Some` when the FAB's newer profile-dispatched share claim named its
/// target explicitly (asserted as either a `Value::Entity` DID or a
/// `Value::String`, tolerating both representations like `remote_from_facts`
/// does). `None` for an older claim carrying no such fact — the handler
/// falls back to the dispatch origin in that case.
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

/// The full decode surface of a `tonk:invite` transient: the typed
/// [`Invite`] command — current or legacy shape — plus the optional
/// target space read from the raw facts by [`invite_space_from_facts`]
/// (NOT a matched `Invite` field — every existing space's frozen
/// `tonk:invite` descriptor lacks it).
///
/// [`Invite`]: tonk_schema::command::Invite
pub(crate) struct InviteRequest {
    /// The decoded command (a legacy shape arrives converted).
    command: tonk_schema::command::Invite,
    /// The fact-named target space, when the FAB's routeless share
    /// claim named one.
    space: Option<String>,
}

impl crate::reactor::Decode for InviteRequest {
    fn trigger_attributes() -> Vec<String> {
        crate::reactor::Migrated::<
            tonk_schema::command::Invite,
            tonk_schema::command::legacy::Invite,
        >::new()
        .trigger_attributes()
        .to_vec()
    }

    fn decode(
        _this: dialog_artifacts::Entity,
        facts: &crate::reactor::EntityFacts,
    ) -> Option<Self> {
        let command = crate::reactor::Migrated::<
            tonk_schema::command::Invite,
            tonk_schema::command::legacy::Invite,
        >::new()
        .decode(facts)?;
        Some(Self {
            command,
            space: invite_space_from_facts(facts),
        })
    }
}

impl dialog_capability::Command for InviteRequest {
    type Input = Self;
    type Output = ();
}

/// Run the [`Invite`] command.
///
/// When the FAB's share control (`<tonk-share>`) dispatches a transient
/// [`Invite`], this provider generates a fresh membership keypair, delegates
/// the *target* repository's access to its DID, base58-encodes the
/// resulting delegation chain, and asserts a durable [`Authorization`] fact
/// keyed by that DID on the repository's content branch (`main`). It then
/// asserts the private seed as a [`Credential`] into the reactor's session
/// overlay (never replicated). The share view joins the two via
/// `tonk:invitation` and assembles the final URL.
///
/// The target is the fact-named space when the FAB's routeless,
/// profile-dispatched share claim named one, else the dispatch origin
/// (the shape every existing space's frozen `tonk:invite` descriptor
/// still dispatches). A content branch naming a DIFFERENT space is
/// refused — see [`CommandEnv::may_target_space`](crate::router::CommandEnv::may_target_space).
///
/// [`Invite`]: tonk_schema::command::Invite
/// [`Authorization`]: tonk_schema::command::Authorization
/// [`Credential`]: tonk_schema::command::Credential
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<InviteRequest> for crate::router::CommandEnv {
    async fn execute(&self, request: InviteRequest) {
        execute_invite(self.clone(), request).await
    }
}

async fn execute_invite(env: crate::router::CommandEnv, request: InviteRequest) {
    use tonk_schema::prelude::DidExt as _;

    let repo_name = request
        .space
        .and_then(|space| space.parse::<dialog_varsig::Did>().ok())
        .map(|did| did.repo_key().to_owned())
        .unwrap_or_else(|| env.origin().repo.clone());
    // The triggering click's timestamp, echoed onto a refusal so a
    // later resubscribe can tell this refusal from a replay of an
    // older one — see `publish_share_blocked`.
    let time = request.command.time.0;

    if repo_name.is_empty() {
        log!("Invite: no target space (no fact, empty origin), skipping");
        return;
    }
    if !env.may_target_space(&repo_name) {
        log!(
            "Invite ignored: origin '{}' may not mint an invite for '{}'",
            env.origin().repo,
            repo_name
        );
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
}

/// The full decode surface of a `tonk:enable-sync` transient: the typed
/// [`EnableSync`] command — current or legacy shape — plus its target
/// `space`, optional `remote` endpoint, and `share` marker, all read
/// from the raw facts (a DID or URL round-trips through JSON as a
/// `Value::Entity`, which a `String` concept field can't decode — see
/// [`text_fact`]).
///
/// [`EnableSync`]: tonk_schema::command::EnableSync
pub(crate) struct EnableSyncRequest {
    /// The decoded command (a legacy shape arrives converted).
    command: tonk_schema::command::EnableSync,
    /// The target space DID, read from the raw facts.
    space: Option<String>,
    /// The endpoint to attach; absent means "wherever this account
    /// syncs".
    remote: Option<String>,
    /// Whether to mint an invite once attached.
    share: bool,
}

impl crate::reactor::Decode for EnableSyncRequest {
    fn trigger_attributes() -> Vec<String> {
        crate::reactor::Migrated::<
            tonk_schema::command::EnableSync,
            tonk_schema::command::legacy::EnableSync,
        >::new()
        .trigger_attributes()
        .to_vec()
    }

    fn decode(
        _this: dialog_artifacts::Entity,
        facts: &crate::reactor::EntityFacts,
    ) -> Option<Self> {
        let command = crate::reactor::Migrated::<
            tonk_schema::command::EnableSync,
            tonk_schema::command::legacy::EnableSync,
        >::new()
        .decode(facts)?;
        Some(Self {
            command,
            space: text_fact(facts, ENABLE_SYNC_SPACE_ATTR),
            remote: text_fact(facts, ENABLE_SYNC_REMOTE_ATTR),
            share: text_fact(facts, ENABLE_SYNC_SHARE_ATTR).is_some(),
        })
    }
}

/// Mint an account-scoped handoff for the originating space.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AgentHandoff> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::AgentHandoff) {
        if let Err(error) = run_agent_handoff(self).await {
            log!("agent handoff failed: {error}");
        }
    }
}

async fn publish_agent_handoff(
    tonk: &TonkState,
    repo: &str,
    subject: &Did,
    account: &Did,
    status: String,
    link: String,
) -> Result<(), TonkWorkerError> {
    use tonk_schema::prelude::DidExt as _;
    tonk.reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .overlay()
        .assert(tonk_schema::command::AgentHandoffState {
            this: subject.this(),
            status: status.into(),
            link: link.into(),
            account: account.this().into(),
        })
        .write()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to publish handoff: {error}"))
        })?;
    Ok(())
}

async fn run_agent_handoff(env: &crate::router::CommandEnv) -> Result<(), TonkWorkerError> {
    let repo = &env.origin().repo;
    let subject = {
        let tonk = env.state().read().await;
        let repository = tonk
            .profile
            .repository(repo)
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
        let subject = repository.did();
        require_real_space(&tonk, &subject).await?;
        if super::account::provider(&tonk).await.is_none() {
            return publish_agent_handoff(
                &tonk,
                repo,
                &subject,
                &tonk.profile.did(),
                "Create an account or sign in to connect an agent. Open share and choose ‘log in to share’ to get started, then return here to copy your prompt.".into(),
                String::new(),
            )
            .await;
        }
        publish_agent_handoff(
            &tonk,
            repo,
            &subject,
            &tonk.profile.did(),
            "Generating account-scoped handoff…".into(),
            String::new(),
        )
        .await?;
        subject
    };
    let origin = crate::axum::RequestOrigin::parse(
        &worker_origin().unwrap_or_else(|| "https://tonk.network".into()),
    )
    .map_err(|error| TonkWorkerError::Internal(format!("invalid handoff origin: {error:?}")))?;
    let minted =
        super::create_invite::create_agent_handoff(env.state().clone(), repo.clone(), origin).await;
    let tonk = env.state().read().await;
    match minted {
        Ok((response, expected)) => {
            let current = super::identity::local_root(&tonk).await?;
            if current.root_did != expected.root_did || current.bytes != expected.bytes {
                return publish_agent_handoff(
                    &tonk,
                    repo,
                    &subject,
                    &current.root_did,
                    "Account changed; generate a new handoff.".into(),
                    String::new(),
                )
                .await;
            }
            publish_agent_handoff(
                &tonk,
                repo,
                &subject,
                &expected.root_did,
                "ready".into(),
                response.url().to_string(),
            )
            .await
        }
        Err(error) => {
            publish_agent_handoff(
                &tonk,
                repo,
                &subject,
                &tonk.profile.did(),
                format!("Could not create an agent handoff: {error}"),
                String::new(),
            )
            .await
        }
    }
}

impl dialog_capability::Command for EnableSyncRequest {
    type Input = Self;
    type Output = ();
}

/// Run the `tonk:enable-sync` command: attach a sync remote to an
/// existing space, then mint an invite when the transient asks for one.
///
/// The share control dispatches this when a user accepts the offer to turn
/// sync on after a refused share. Minting from inside the provider is what
/// makes that a single click: the control needs no completion signal for the
/// attach, because success reaches it as a new invite link on the
/// subscription it already holds — the same path an ordinary mint takes.
///
/// The target is the fact-named space; a content branch naming a
/// DIFFERENT space is refused, and the target must be a real user space
/// — see [`CommandEnv::may_target_space`](crate::router::CommandEnv::may_target_space)
/// and [`require_real_space`].
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<EnableSyncRequest> for crate::router::CommandEnv {
    async fn execute(&self, request: EnableSyncRequest) {
        execute_enable_sync(self.clone(), request).await
    }
}

async fn execute_enable_sync(env: crate::router::CommandEnv, request: EnableSyncRequest) {
    use dialog_artifacts::Entity;
    use tonk_schema::prelude::DidExt as _;

    let time = request.command.time.0;
    let share = request.share;
    let Some(space) = request.space else {
        log!("EnableSync: missing space, skipping");
        return;
    };
    // An absent remote means "wherever this account syncs" — the
    // page no longer derives an endpoint from its own origin,
    // which it could not even do reliably: a sealed guest's
    // document is `about:srcdoc`, so it had to be told its own
    // origin by the portal bridge first, and a share before that
    // arrived did nothing at all.
    let remote = match request.remote {
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
    if !env.may_target_space(&key) {
        log!(
            "EnableSync ignored: origin '{}' may not attach a remote to '{}'",
            env.origin().repo,
            key
        );
        return;
    }
    {
        // Only a real user space takes a remote from this command —
        // never the profile's own hidden replica or a system repo.
        let tonk = env.state().read().await;
        if let Err(error) = require_real_space(&tonk, &did).await {
            log!("EnableSync '{}' refused: {}", key, error);
            return;
        }
    }
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
enum RunInvite {
    /// Minted, refused, or otherwise finished — nothing more to do.
    Settled,
    /// The space had no remote and one was just attached, so a second
    /// pass can now mint. Returned rather than recursing: re-entering
    /// an async fn from inside itself needs boxing for no gain.
    Attached,
}

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
                log!("Invite: could not ask the page to add an account: {error}");
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
    // refuses every presign). Whether the consumer row exists is read
    // from the account db first: the `SpaceProvider` fact is written
    // when `/provider/add` succeeds and retracted when the gate stops
    // serving the subject, so a provisioned space mints its link with
    // no registration call at all. Only a space with no record runs the
    // ceremony for owned authority — a legacy space provisioned before the
    // fact existed, or one whose earlier attempt failed. Joined authority
    // keeps its existing provider; this account need not have its record.
    // Success for an owned space records the fact,
    // so it runs once, not per share. Best effort like the enable-sync
    // attach: a foreign remote (self-hosted, a test server) is not our
    // access service, and refusing the mint over it would make those
    // unshareable.
    match if super::customer::space_provider_recorded(&tonk, &repository.did()).await {
        Ok(())
    } else {
        provision_space_consumer(&tonk, &repository.did()).await
    } {
        Ok(()) => {}
        // Our own service said no, and waiting will not change the
        // answer. A link minted anyway points at a space the service
        // will not serve — the recipient meets "you don't have this
        // space" — so the share is refused with the reason instead.
        Err(error @ TonkWorkerError::Upstream { .. })
            if remote_is_own_service(remote_execution.access_url.as_str())
                && !super::customer::is_retryable(&error) =>
        {
            log!("Invite for repo '{repo_name}': the service refused to provision: {error}");
            drop(tonk);
            publish_share_blocked(
                env.state(),
                repo_name,
                subject_entity,
                tonk_worker_api::share::BLOCKED_NOT_PROVISIONED,
                &error.to_string(),
                time,
            )
            .await;
            return Ok(RunInvite::Settled);
        }
        Err(error) => {
            log!("Invite for repo '{repo_name}': provisioning skipped: {error}");
        }
    }

    // The endpoint rides inside the signed chain (`home.address` meta), so
    // the URL carries no `&remote=` suffix any more. The suffix slot stays
    // empty rather than removed: `tonk:authorization.remote` is a required
    // field of the seeded concept, and the URL assembler treats an empty
    // suffix as absent.
    let remote = String::new();

    // Mint a fresh membership keypair. Its private seed becomes the invite
    // URL's `#` fragment; its public DID is the audience the repo access is
    // delegated to. The browser never sees this DID.
    let (signer, seed_bytes) = super::create_invite::generate_ephemeral().await?;
    let membership_did = signer.did();
    let seed = bs58::encode(seed_bytes).into_string();

    // The leaf is signed with the space's upstream in its `home.address`
    // meta and — when one has hydrated here — its display name in
    // `space.name`, so both ride inside the signed grant: the endpoint
    // because the grant and the address must not be swappable
    // independently, the name as the invitation's historical fact ("you
    // were invited to a space called X", true after any rename).
    let mut meta = tonk_invite::home_address_meta(&remote_execution.access_url);
    if let Some(name) = repository_display_name(&tonk, &repository, repo_name).await {
        meta.extend(tonk_invite::space_name_meta(&name));
    }
    let delegation: dialog_ucan::UcanDelegation = tonk
        .profile
        .access()
        .claim(Subject::from(repository.did()).attenuate(Use))
        .delegate(membership_did)
        .meta(meta)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;
    let chain = delegation.into_chain();

    // Derive the invitation record from the chain as minted — before it's
    // serialized away — so the meta-branch roster carries this invite. The
    // claim side self-heals a missing record, but the mint should write its
    // own. Guaranteed `Some`: the delegation is scoped to the repo subject.
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
    // template can't make. The display name needs no URL carrier: it
    // rides in the chain's signed `space.name` meta, inside `access=`.
    let link = invite_url(
        &proof,
        &remote,
        &seed,
        repo_name,
        &remote_execution.access_url,
    )
    .await?;

    let authorization = Authorization {
        this: subject_entity.clone(),
        proof: Proof(proof),
        remote: AuthorizationRemote(remote),
    };
    let subject_entity_for_short = subject_entity.clone();
    let subject_entity_for_short_state = subject_entity.clone();

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
            seed: Seed(seed.clone()),
            link: Link(link.clone()),
        })
        .write()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to write credential overlay: {e}"))
        })?;

    // The same answer in the shape the share control subscribes to, on
    // PROFILE main rather than the space: one row per space whose
    // `status` says where the invite has got to, carrying the url once
    // there is one. `Credential` above keeps the seed beside it on the
    // space for readers that need both; this is what a view renders.
    //
    // On profile main because the Hub renders one share control per row,
    // and a control subscribed to the space made merely LISTING spaces
    // query into each one — which mounts it (`query.rs` adopts on first
    // use), so opening the Hub replicated the whole account. The state is
    // this device's view of a click it made; nothing about it needs the
    // space's branch. See `plan/share-intent.md`.
    publish_invite_state(
        &tonk,
        tonk_schema::command::InviteState::granted(subject_entity, link.clone()),
    )
    .await;

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

    crate::router::navigate::notify_analytics(
        env.client(),
        tonk_worker_api::AnalyticsEvent::SpaceShared {
            space: repo_name.to_owned(),
        },
    );
    log!("Minted invitation for repo '{}'", repo_name);

    // The mint is complete and the LONG link is what the share control
    // copies — fan it out NOW, before any shortening network. Minting an
    // invocation must never wait on a convenience round-trip.
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    drop(tonk);

    // Best-effort shortening, off the critical path and outside the
    // state lock: PUT the target and probe the stored shortcut (HEAD —
    // the landing URL is the whole answer). A host that provides no
    // shortening, or answers non-conformingly, leaves the long link
    // standing; a conforming answer supersedes the overlay credential
    // in place, and the dispatcher's drain broadcasts the update.
    match super::create_invite::shorten(&link).await {
        Ok(short) if short != link => {
            let tonk = env.state().read().await;
            if let Err(error) = tonk
                .reactor
                .repository(repo_name)
                .branch(CONTENT_BRANCH)
                .overlay()
                .assert(Credential {
                    this: subject_entity_for_short.clone(),
                    seed: Seed(seed),
                    link: Link(short.clone()),
                })
                .write()
                .perform(&tonk.operator)
                .await
            {
                log!("short link overlay update failed; the long link stands: {error}");
            }
            // And the row the share control actually reads, on profile
            // main. Without this the clipboard settles on the long URL:
            // the control resolves from the profile row, which would
            // still carry the pre-shortening link.
            publish_invite_state(
                &tonk,
                tonk_schema::command::InviteState::granted(subject_entity_for_short_state, short),
            )
            .await;
        }
        Ok(_) => {}
        Err(error) => {
            log!("invite shortcut failed; using the full URL: {error}");
        }
    }
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

async fn publish_share_blocked<'a>(
    state: &'a AppState,
    repo_name: &'a str,
    subject: dialog_artifacts::Entity,
    code: &'a str,
    detail: &'a str,
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
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("failed to publish share refusal for '{repo_name}': {error}");
    }

    // The same refusal in the shape the share control subscribes to, on
    // profile main for the reason the grant is. Only a terminal reason
    // becomes a terminal status: a refusal the user can repair (no
    // account yet, no remote yet) leaves the request open, because the
    // click has not finished failing — it is waiting on something.
    publish_invite_state(
        &tonk,
        tonk_schema::command::InviteState::denied(subject, invite_status_for(code)),
    )
    .await;
}

/// Publish the share control's view of an invite onto PROFILE main's
/// overlay, keyed on the space's subject.
///
/// Overlay because the url carries the membership seed in its fragment
/// and must not reach storage, and because the row is a per-session view
/// of one click rather than a durable record — the durable record of a
/// minted invite is `Invitation`, on the space.
async fn publish_invite_state(tonk: &TonkState, state: tonk_schema::command::InviteState) {
    let main = match tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("invite state: open profile main: {error}");
            return;
        }
    };
    main.state.assert_overlay(state);
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&main.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Assemble the invite URL a recipient opens, shortened when the
/// shortcut service answers.
///
/// The long form is
/// `{origin}/join?access={proof}{remote}&tonk_channel=reshare&tonk_space={hash}#{seed}`.
/// `remote` is the legacy ready-to-append `&remote=…` suffix and is empty when
/// the signed delegation carries the endpoint itself. The share pipeline has
/// already refused a space with no usable remote. This is the shape the share
/// view used to concatenate from three overlay fields; building it here gives
/// it one definition and lets it be shortened.
///
/// Shortening is best-effort: a failed `PUT /@` (offline, no service
/// deployed, a non-2xx, a non-conforming answer) logs and yields the long
/// URL, which is fully functional. Minting must not fail because a
/// convenience failed.
async fn invite_url(
    proof: &str,
    remote: &str,
    seed: &str,
    space_key: &str,
    access_url: &Url,
) -> Result<String, TonkWorkerError> {
    // The link lives on the host serving the SPACE — the origin derived
    // from its access endpoint (the same derivation the CLI uses) — not
    // on whatever surface happened to mint it. That host is where the
    // space's members already sync, and it is the origin whose
    // same-origin shortcut store can answer the short link's relative
    // redirect. There is no fallback base: `run_invite` has already
    // refused a space without a usable remote (that is what the share
    // bar's login/attach prompts are), so an endpoint that yields no
    // origin here is a bug worth failing on, not a case to paper over
    // with a link rooted somewhere the space is not served.
    //
    // No network here, deliberately: this is on the mint's critical
    // path, and the long URL is complete. Shortening is a later,
    // best-effort pass (`run_invite` runs it after the link has been
    // delivered, outside the state lock).
    let base = tonk_invite::base_url_for_remote(access_url.as_str()).map_err(|error| {
        TonkWorkerError::Internal(format!(
            "the space's access endpoint yields no invite base: {error:#}"
        ))
    })?;
    Ok(long_invite_url(&base, proof, remote, seed, space_key))
}

/// The service worker's own origin, or `None` outside a worker scope.
///
/// No longer part of the invite base — a minted link lives on the host
/// serving the space, not the surface that minted it (see [`invite_url`])
/// — but still what worker-relative endpoints (`/api/…` calls the worker
/// makes to its own deployment) resolve against.
pub(super) fn worker_origin() -> Option<String> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;

        js_sys::global()
            .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
            .ok()
            .map(|global| global.location().origin())
            .filter(|origin| !origin.is_empty())
    }
    // A native host serves no origin of its own; the access-service
    // address must come from recorded facts (an account provider) or
    // host configuration, so "derive it from where I am serving" has no
    // native answer. Callers already treat `None` as "the service is
    // unknown" and refuse or degrade visibly.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        None
    }
}

/// Assemble the long (un-shortened) invite URL.
///
/// `base` is the resolved `…/join` base on the host serving the space —
/// required, because a space with no serving host was refused before the
/// mint ever got here (see [`invite_url`]). The URL then receives an
/// organic channel and hashed space token before being returned.
///
/// `remote` is already a ready-to-append `&remote=…` suffix and is empty for a
/// modern delegation whose signed metadata names the shareable remote (see
/// `RemoteRefusal`). The seed is the fragment and never the query: it must not
/// reach a server, and the shortcut service is handed only the path + query.
///
/// The space's display name needs no slot here: it rides in the chain's
/// signed `space.name` meta, inside the `access=` parameter itself.
fn long_invite_url(base: &str, proof: &str, remote: &str, seed: &str, space_key: &str) -> String {
    let base = format!("{base}?access={proof}{remote}#{seed}");
    match tonk_analytics::launch::space_referral_url(&base, space_key) {
        Ok(url) => url,
        Err(error) => {
            // Referral metadata must never turn a valid authority grant into
            // a failed share. The base above is still a complete invite.
            log!("invite: could not add referral attribution: {error}");
            base
        }
    }
}

/// Run the [`PauseSync`] command.
///
/// Toggles auto-sync for the space the command NAMES: reads the durable
/// [`ReplicaSyncEnabled`] preference at the `state:here` singleton, flips it
/// (`active` ⇄ `paused`, defaulting an absent fact to "pause"), and commits the
/// new value on that space's content branch. On pause it stamps `sync:paused`
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
/// Naming the target is what lets the FAB dispatch this from the profile
/// branch; a content branch naming a DIFFERENT space is refused — see
/// [`CommandEnv::may_target_space`](crate::router::CommandEnv::may_target_space).
///
/// [`PauseSync`]: tonk_schema::command::PauseSync
/// [`ReplicaSyncEnabled`]: tonk_schema::ReplicaSyncEnabled
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::PauseSync> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::PauseSync) {
        use tonk_schema::prelude::DidExt as _;

        // The repo key is the space DID's suffix; a space's content
        // branch is always `main`.
        let Some(repo) = command
            .space
            .0
            .to_string()
            .parse::<dialog_varsig::Did>()
            .ok()
            .map(|did| did.repo_key().to_owned())
        else {
            log!("PauseSync: no/unparseable target space, skipping");
            return;
        };
        if !self.may_target_space(&repo) {
            log!(
                "PauseSync ignored: origin '{}' may not toggle sync for '{}'",
                self.origin().repo,
                repo
            );
            return;
        }
        let branch = CONTENT_BRANCH.to_string();
        log!("command PauseSync repo={} branch={}", repo, branch);

        if let Err(error) = run_pause_sync(self, &repo, &branch).await {
            log!("PauseSync for repo '{}' failed: {}", repo, error);
        }
    }
}

/// Run the [`ProfileRename`] command.
///
/// Fired when the topbar identity chip's `<tonk-editable>` commits a
/// transient [`ProfileRename`]. It persists the new display name as a
/// durable [`ProfileName`] override on the profile's meta branch, then
/// re-stamps the self member's [`MemberName`] on every space the profile
/// belongs to so all of its rosters reflect the new name at once.
///
/// The new name is the only payload; the spaces to re-stamp come from
/// the profile's replica index on the meta branch. The target is always
/// THE PROFILE — never a space named by a field or the origin — so no
/// origin constraint applies. An empty/whitespace name is a no-op — a
/// member can't blank their own name out.
///
/// [`ProfileRename`]: tonk_schema::command::ProfileRename
/// [`ProfileName`]: tonk_schema::ProfileName
/// [`MemberName`]: tonk_schema::MemberName
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ProfileRename>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ProfileRename) {
        let name = command.name.0;
        let name = name.trim();
        // Don't let a member blank their own name out.
        if name.is_empty() {
            return;
        }
        let key = self.origin().repo.clone();
        log!("command ProfileRename repo={} name={}", key, name);

        if let Err(error) = run_profile_rename(self, name).await {
            log!("ProfileRename for repo '{}' failed: {}", key, error);
        }
    }
}

/// Persist the display-name override on the profile meta branch and
/// re-stamp `MemberName` on every space's content branch.
///
/// Split out from [`ProfileRenameHandler::run`] so the `?` early-return
/// funnels into the single `log!` there — the command future itself
/// returns `()`.
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
pub(crate) fn rename_outcome(result: Result<(), RepositoryError>) -> RenameOutcome {
    match result {
        Ok(()) => RenameOutcome::Renamed,
        Err(_) => RenameOutcome::Failed,
    }
}

/// Run the [`RenameRepository`] command.
///
/// The space-side `tonk/rename-repository` rule (`core.yaml`) binds the
/// command's `subject` to `?this` and asserts the new name directly — but
/// that rule lives on the space's OWN branch, so it can never see a claim
/// dispatched from the profile branch. This provider is the worker-side
/// replacement: it reads the target `space` off the command (like
/// [`PauseSync`](tonk_schema::command::PauseSync)) rather than the dispatch
/// origin, so the FAB's name chip can dispatch from the profile branch with
/// nothing seeded per-space. A content branch naming a DIFFERENT space is
/// refused, and the target must be a real user space — see
/// [`CommandEnv::may_target_space`](crate::router::CommandEnv::may_target_space)
/// and [`require_real_space`].
///
/// [`RenameRepository`]: tonk_schema::command::RenameRepository
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::RenameRepository>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::RenameRepository) {
        use tonk_schema::prelude::DidExt as _;

        let Ok(did) = command.space.0.to_string().parse::<dialog_varsig::Did>() else {
            log!("RenameRepository: unparseable target space, skipping");
            return;
        };
        // `repo_key()` is the FULL DID, not a suffix.
        let repo = did.repo_key().to_owned();
        if !self.may_target_space(&repo) {
            log!(
                "RenameRepository ignored: origin '{}' may not rename '{}'",
                self.origin().repo,
                repo
            );
            return;
        }
        {
            // Only a real user space is renameable from this command —
            // never the profile's own hidden replica or a system repo.
            let tonk = self.state().read().await;
            if let Err(error) = require_real_space(&tonk, &did).await {
                log!("RenameRepository '{}' refused: {}", repo, error);
                return;
            }
        }
        log!("command RenameRepository repo={}", repo);

        let result = run_rename_repository(self, &repo, &command.name.0).await;
        let failure_detail = result.as_ref().err().map(ToString::to_string);
        if rename_outcome(result) == RenameOutcome::Failed {
            log!(
                "RenameRepository for repo '{}' failed: {}",
                repo,
                failure_detail.unwrap_or_default()
            );
        }
    }
}

/// Assert the repository's own [`RepositoryName`] on its content branch,
/// keyed by the subject DID — the same fact the space-side
/// `tonk/rename-repository` rule used to write. Split out from
/// [`RenameRepositoryHandler::run`] so the caller funnels every failure
/// through [`rename_outcome`] rather than a bare `?`.
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

/// Run the [`RemoveSpace`] command: the user confirmed a Hub row's
/// delete overlay. Removal is device-local and ordered so the visible
/// state commits first and cleanup is best-effort behind it — see
/// [`remove_space_inner`].
///
/// Execution refuses any transient whose origin repo is non-empty. This
/// is the first *destructive* command reachable through shape-matched
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
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::RemoveSpace> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::RemoveSpace) {
        let subject = command.subject.0;
        // See the doc above: only the profile branch (empty origin
        // repo) may fire this. A non-empty origin means the fact came
        // from a content branch — matched by shape, not by who asked —
        // so it is ignored rather than trusted to remove anything.
        if !self.origin().repo.is_empty() {
            log!(
                "RemoveSpace ignored: origin '{}' is not the profile branch",
                self.origin().repo
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
        // A space this account PROVIDES has a copy on Tonk services,
        // and dropping the local replica alone would strand it there.
        //
        // `space_provider_recorded` is the whole difference between
        // LEAVING a space and destroying it for everyone: the fact
        // exists only when THIS account hosts the space, so a space
        // someone else provides falls straight through to the local
        // removal below and its hosted copy is untouched.
        //
        // No passkey: deprovisioning signs `/provider/remove` with this
        // device's own authority. The passkey belongs to deleting the
        // ACCOUNT, which is a different command.
        {
            let tonk = self.state().read().await;
            // The worker's own origin: a command has no request behind
            // it to carry one, and the access service that provides the
            // space is the one this worker is served from.
            let origin = super::customer::service_origin();
            if super::customer::space_provider_recorded(&tonk, &subject).await
                && let Ok(origin) = origin
                && let Err(error) =
                    super::customer::deprovision_consumer(&tonk, &origin, &subject).await
            {
                // Reported, not fatal: the local removal still runs, so
                // the row disappears and the service copy is reconciled
                // by the next sweep rather than blocking the person.
                log!(
                    "RemoveSpace '{}' could not be deprovisioned: {}",
                    subject,
                    error
                );
            }
        }
        if let Err(error) = remove_space_inner(self.state(), &subject).await {
            log!("RemoveSpace '{}' failed: {}", subject, error);
        }
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
        let _admission_mutation = tonk.admission.mutation(subject.repo_key());
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
        delete_space_storage_for(subject.repo_key()).await;
    }

    // The delete ran unlocked, so a concurrent `drain_sync` could have
    // reached in and re-acquired the repo (e.g. to pull) while it was in
    // flight — resurrecting the cache entry and, since the IDB open races
    // the delete, potentially recreating an empty database right behind
    // it. Re-evict now that the delete has settled to drop any such
    // handle.
    {
        let tonk = state.write().await;
        let _admission_mutation = tonk.admission.mutation(subject.repo_key());
        tonk.reactor.evict(subject.repo_key());
    }
    Ok(())
}

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

/// Delete a space's local storage by routing key, on whatever this host
/// uses for it. Best-effort on every host: a failure orphans invisible
/// bytes, never loses visible state (the replica retraction has already
/// committed by the time this runs).
///
/// On the web this is the IndexedDB/OPFS delete above. Natively
/// dialog-storage exposes no way to unmount-and-delete a space from a
/// `Storage<NativeSpace>` pool yet, so the bytes stay behind — the same
/// leaked-bytes outcome the web path deliberately accepts when another
/// profile shares the storage. Logged so the leak is visible; grows a
/// real implementation when dialog-storage grows the capability.
async fn delete_space_storage_for(key: &str) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let _ = wasm_bindgen_futures::JsFuture::from(delete_space_storage(key)).await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        log!("space '{key}' removed; its local storage is left behind (no native delete yet)");
    }
}

/// Delete the storage a legacy hidden account repository left behind.
/// Its content synced with the same remote profile main now follows, so
/// everything it held is recoverable by pulling.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn delete_legacy_storage(key: &str) {
    delete_space_storage_for(key).await;
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
async fn enable_sync_inner(
    state: &AppState,
    key: &str,
    remote: &str,
) -> Result<(), RepositoryError> {
    let tonk = state.write().await;
    enable_sync_for_repository(&tonk, key, remote).await
}

/// Attach the account provider to one existing repository.
///
/// This is the lock-free core shared by the form handler and the
/// post-reconcile local-space sweep. The caller must already know that an
/// account is ready before using it as an automatic transition; the form path
/// remains explicitly callable and reports the provider's refusal.
async fn enable_sync_for_repository(
    tonk: &TonkState,
    key: &str,
    remote: &str,
) -> Result<(), RepositoryError> {
    if remote.trim().is_empty() {
        // Submitted with no URL — nothing to attach.
        log!("enable sync '{}': empty remote, nothing to attach", key);
        return Ok(());
    }
    let configuration = space_config(remote)?;

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
    // Best effort ONLY for a remote that is not our access service — a
    // self-hosted endpoint or a test server is attached the same way,
    // and `/provider/add` against our own service is beside the point
    // there. But when the remote IS our service and it refuses with an
    // answer waiting will not change, attaching anyway wires the space
    // to an upstream that refuses every presign terminally: the sync
    // loop hammers it forever and a link handed out against it answers
    // "you don't have this space". That refusal fails the attach.
    match provision_space_consumer(tonk, &repository.did()).await {
        Ok(()) => {}
        Err(error) if remote_is_own_service(remote) && !super::customer::is_retryable(&error) => {
            return Err(RepositoryError::Internal(format!(
                "enable sync '{key}': the service refused to provision this space, and \
                 attaching its own remote anyway would wire the space to an upstream \
                 that refuses every request: {error}"
            )));
        }
        Err(error) => {
            log!("enable sync '{key}': provisioning skipped: {error}");
        }
    }

    let effective = ensure_remote_config(tonk, &repository, key, &configuration).await?;

    // Mirror the EFFECTIVE mount configuration into the account
    // directory so other devices adopt what this device actually
    // syncs against: an already-configured upstream is preserved, so
    // the request's (possibly repair-supplied) address must not
    // overwrite it there.
    record_space_mount(tonk, &repository.did(), &effective, None).await;

    Ok(())
}

/// Attach `remote` only when `key` is genuinely local-only: its replica meta
/// names no remote at all.
///
/// A branch with no upstream is not sufficient evidence — a repository may
/// already carry a foreign or partially configured remote. The account sweep
/// must leave every such repository untouched. Returns whether this call made
/// the local-only -> account-hosted transition.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(super) async fn attach_account_remote_if_local(
    tonk: &TonkState,
    key: &str,
    remote: &str,
) -> Result<bool, RepositoryError> {
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
                "account remote reconcile '{}': repository not present, skipping ({})",
                key,
                error
            );
            return Ok(false);
        }
    };
    if repository_has_any_remote(tonk, &repository, key).await? {
        return Ok(false);
    }

    enable_sync_for_repository(tonk, key, remote).await?;
    Ok(true)
}

/// Whether the repository's local meta branch names any remote, regardless of
/// whether a content branch currently tracks it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn repository_has_any_remote<C>(
    tonk: &TonkState,
    repository: &Repository<C>,
    key: &str,
) -> Result<bool, RepositoryError>
where
    C: Principal + Clone,
{
    let meta = repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            RepositoryError::Internal(format!(
                "Failed to open meta branch for repository '{key}': {error}"
            ))
        })?;
    let replica = Replica::new(tonk.profile.did(), repository.did());
    let remotes: Vec<Remote> = meta
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(replica.this().clone()),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            RepositoryError::Internal(format!(
                "Failed to read remotes for repository '{key}': {error:?}"
            ))
        })?;
    Ok(!remotes.is_empty())
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
    let _admission_mutation = tonk.admission.mutation(key);
    tonk.reactor.evict(key);
    Ok(true)
}

/// Seed the standard library into every branch, then flip the
/// replica's status to `initialized`. Runs in the background after
/// `put_repository` has already responded.
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
        let version = seed_version(&scaffold);
        let tonk = state.read().await;
        for branch_name in branches {
            // The record names the commit that installs the library. That
            // commit STAGES, so its version is minted and authoritative
            // before the record is written; the record then chains on and
            // one publish makes both visible. Recording separately would
            // name the record's own commit instead, and the seed's claims
            // — which route provenance and an upgrade both read — would
            // sit in a revision nothing pointed at.
            let body = format!("{scaffold}\n{name_body}");
            // A fresh space has no predecessor, and nothing to replace.
            let record = |minted: &dialog_artifacts::history::Version| {
                seed_record_facts(
                    &version,
                    STANDARD_LIBRARY_URL,
                    SEED_NONE,
                    SEED_NONE,
                    &encode_seed_version(minted),
                )
            };
            super::evaluate::evaluate_body_recording(&tonk, key, branch_name, body, &record)
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
const STANDARD_LIBRARY_URL: &str = "/library/core.yaml";

/// URL of the lean profile library — only the `space` concept and the
/// Hub directory view. Seeded onto the profile's meta branch, which
/// backs nothing but the Hub, so it doesn't pay to write the full
/// workspace/board/sheet library it never reads. Only referenced from
/// the SW-scoped profile seed path.
const PROFILE_LIBRARY_URL: &str = "/library/profile.yaml";

/// The seed a space is running: both halves, joined on the seed entity.
///
/// `seed/available` carries identity and source; `seed/installed` adds
/// the version its install commit landed at. A space that has fetched an
/// update has the first without the second for THAT seed, which is why
/// the version is read separately rather than assumed present.
struct InstalledSeed {
    /// The seed's entity — the hash of its bytes.
    seed: dialog_artifacts::Entity,
    /// Where those bytes were fetched from.
    source: String,
    /// The version of the commit that installed it.
    version: String,
}

/// Read the seed a space is running, if it recorded one.
async fn read_installed_seed(
    tonk: &TonkState,
    session: &crate::reactor::BranchSession,
) -> Result<Option<InstalledSeed>, String> {
    use dialog_query::{Output as _, Query, Term};

    let installed: Vec<tonk_schema::SeedInstalled> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::SeedInstalled> {
            this: Term::var("this"),
            prior: Term::var("prior"),
            version: Term::var("version"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let Some(current) = installed.into_iter().next() else {
        return Ok(None);
    };

    let available: Vec<tonk_schema::SeedAvailable> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::SeedAvailable> {
            this: Term::from(current.this.clone()),
            source: Term::var("source"),
            replaces: Term::var("replaces"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let Some(source) = available.into_iter().next() else {
        // The install half without its identity half. A seed is always
        // written as both, so this means the record was damaged.
        return Err(format!("seed {} records no source", current.this));
    };

    Ok(Some(InstalledSeed {
        seed: current.this,
        source: source.source.0,
        version: current.version.0,
    }))
}

/// Check whether a newer seed is waiting for the space the command names.
///
/// Target-agnostic: the source fetch resolves through
/// `fetch_standard_library`, which already answers natively from the
/// embedded libraries, so a CLI or a test can check for an update too.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::CheckUpdate> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::CheckUpdate) {
        let key = command.space.0.to_string();
        if !self.may_target_space(&key) {
            log!("CheckUpdate '{key}': refused from another space's branch");
            return;
        }
        let Ok(subject) = key.parse::<dialog_varsig::Did>() else {
            log!("CheckUpdate: '{key}' is not a space DID");
            return;
        };
        let tonk = self.state().read().await;
        // The command's own entity marks the check in flight, so a
        // marker stranded by a crashed worker names the check that left
        // it rather than being an anonymous flag.
        let check = command.this.clone();
        if let Err(error) = check_seed_update(&tonk, &subject, check).await {
            log!("CheckUpdate '{subject}': {error}");
        }
    }
}

/// Drop a space's invite row once its link has reached the clipboard.
///
/// The row lives in profile main's overlay, and the url in it carries a
/// membership seed. Cleared by dropping the entity's overlay facts
/// rather than retracting: an overlay retract records a tombstone beside
/// the assertion instead of removing it, so the row — and the seed —
/// would still read back.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ForgetInvite> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ForgetInvite) {
        let space = command.space.0.clone();
        if !self.may_target_space(&space.to_string()) {
            log!("ForgetInvite '{space}': refused from another space's branch");
            return;
        }
        let tonk = self.state().read().await;
        let main = match tonk
            .reactor
            .profile_repository()
            .branch(PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
        {
            Ok(main) => main,
            Err(error) => {
                log!("ForgetInvite: open profile main: {error}");
                return;
            }
        };
        main.state
            .retain_overlay_entities(|overlaid| overlaid != &space);
        tonk.reactor
            .schedule_poll(std::sync::Arc::clone(&main.state));
        tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    }
}

/// Look for a newer seed without installing one.
///
/// Records the check on this device's REPLICA — the entity already
/// pairing this profile with this subject, on the profile meta branch,
/// which never replicates. That is the right scope: one device checking
/// says nothing about another, and keying on the space alone would let
/// two devices overwrite each other's answer.
///
/// What a seed IS, by contrast, is global: an available seed is asserted
/// on the space's own content branch, so one member's check informs
/// everyone rather than each device rediscovering the same bytes.
///
/// The check is the cheap half of [`upgrade_seed`] — a fetch and a hash —
/// so an affordance can offer the update and leave installing it to the
/// user.
pub(crate) async fn check_seed_update(
    tonk: &TonkState,
    subject: &Did,
    check: dialog_artifacts::Entity,
) -> Result<(), RepositoryError> {
    let replica = Replica::new(tonk.profile.did(), subject.clone())
        .this()
        .clone();

    // Announce the check BEFORE the fetch, so a view can show it running
    // rather than only ever learning the outcome.
    stamp_checking(tonk, replica.clone(), check.clone()).await;

    let outcome = run_seed_check(tonk, subject).await;

    // The in-flight marker is retracted either way; what lands beside it
    // is what differs. The SAME check entity, so the retraction matches
    // the fact that was asserted.
    clear_checking(tonk, replica.clone(), check).await;
    match outcome {
        Ok(found) => {
            stamp_check_failure(tonk, replica.clone(), None).await;
            if let Some(found) = found {
                publish_available_seed(tonk, subject, &found).await;
            }
        }
        Err(error) => {
            log!("update check '{subject}': {error}");
            stamp_check_failure(tonk, replica.clone(), Some(&error)).await;
        }
    }
    stamp_checked(tonk, replica).await;
    Ok(())
}

/// A seed the check found waiting, and the installed one it supersedes.
struct FoundSeed {
    /// The waiting seed's entity — the hash of the fetched bytes.
    seed: String,
    /// Where those bytes came from.
    source: String,
    /// The installed seed it would replace.
    replaces: dialog_artifacts::Entity,
}

/// Fetch the space's own source and compare it against what is installed.
///
/// `Some` when the fetched bytes hash to something other than the
/// installed seed, `None` when the space is already current, and `Err`
/// with a message meant for the person when the check could not run.
///
/// A space with no install record is not an error: nothing names its
/// definitions, so an upgrade could not withdraw them. The absence of an
/// install fact is itself the answer, so no failure is recorded for it.
async fn run_seed_check(tonk: &TonkState, subject: &Did) -> Result<Option<FoundSeed>, String> {
    let key = subject.repo_key();
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| format!("could not open this space: {e}"))?;

    let Some(current) = read_installed_seed(tonk, &session)
        .await
        .map_err(|e| format!("could not read the seed record: {e}"))?
    else {
        return Ok(None);
    };

    let library = fetch_standard_library(&current.source)
        .await
        .map_err(|e| format!("could not fetch {}: {e}", current.source))?;

    let seed = seed_version(&library);
    if seed == current.seed.to_string() {
        return Ok(None);
    }
    Ok(Some(FoundSeed {
        seed,
        source: current.source,
        replaces: current.seed,
    }))
}

/// Assert a waiting seed on the space's own content branch.
///
/// Durable and global, unlike the per-device check facts: the bytes exist
/// for everyone, so one member's check spares the rest a fetch. Only
/// `seed/available` is asserted — the install-specific half stays absent
/// until something installs it, so a waiting seed can never be mistaken
/// for a running one.
async fn publish_available_seed(tonk: &TonkState, subject: &Did, found: &FoundSeed) {
    let Ok(seed) = found.seed.parse() else {
        log!("update check: '{}' is not an entity", found.seed);
        return;
    };
    let key = subject.repo_key();
    let fact = tonk_schema::SeedAvailable {
        this: seed,
        source: tonk_schema::domain::seed::Source(found.source.clone()),
        replaces: tonk_schema::domain::seed::Replaces(found.replaces.clone()),
    };
    let commit = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(fact)
        .commit()
        .perform(&tonk.operator)
        .await;
    if let Err(error) = commit {
        log!("update check: record available seed: {error}");
    }
}

/// Stamp the in-flight marker on this device's replica.
///
/// Presence is the state, so clearing means retracting the attribute
/// rather than writing a "done" value — see [`clear_checking`], which
/// needs the SAME check entity this stored.
async fn stamp_checking(
    tonk: &TonkState,
    replica: dialog_artifacts::Entity,
    check: dialog_artifacts::Entity,
) {
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(tonk_schema::ReplicaChecking {
            this: replica,
            checking: tonk_schema::domain::check::Checking(check),
        });
    commit_replica_stamp(tonk, transaction).await;
}

/// Drop the in-flight marker [`stamp_checking`] left.
///
/// A retraction matches on the VALUE as well as the entity and
/// attribute, so this has to name the same `check` that was asserted.
/// Passing anything else (the replica entity, say) retracts a fact that
/// was never stored and leaves the real marker standing, so a settled
/// check still reads as running.
async fn clear_checking(
    tonk: &TonkState,
    replica: dialog_artifacts::Entity,
    check: dialog_artifacts::Entity,
) {
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .retract(tonk_schema::ReplicaChecking {
            this: replica,
            checking: tonk_schema::domain::check::Checking(check),
        });
    commit_replica_stamp(tonk, transaction).await;
}

/// Record when this device's check completed.
async fn stamp_checked(tonk: &TonkState, replica: dialog_artifacts::Entity) {
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(tonk_schema::ReplicaChecked {
            this: replica,
            checked: tonk_schema::domain::check::Checked(js_sys::Date::now()),
        });
    commit_replica_stamp(tonk, transaction).await;
}

/// Record why this device's check failed, or clear a previous failure.
async fn stamp_check_failure(
    tonk: &TonkState,
    replica: dialog_artifacts::Entity,
    failure: Option<&str>,
) {
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction();
    let transaction = match failure {
        Some(failure) => transaction.assert(tonk_schema::ReplicaCheckFailure {
            this: replica,
            failure: tonk_schema::domain::check::Failure(failure.to_owned()),
        }),
        None => transaction.retract(tonk_schema::ReplicaCheckFailure {
            this: replica,
            failure: tonk_schema::domain::check::Failure(String::new()),
        }),
    };
    commit_replica_stamp(tonk, transaction).await;
}

/// Commit one replica stamp to the profile meta branch.
///
/// Durable rather than overlay: a check's answer should survive a worker
/// restart, and replica records never replicate, so this stays device
/// local without being ephemeral.
async fn commit_replica_stamp(
    tonk: &TonkState,
    transaction: crate::reactor::TransactionBuilder<'_>,
) {
    match transaction.commit().perform(&tonk.operator).await {
        Ok(revision) => broadcast(
            "/api/profile",
            &Notification {
                branch: PROFILE_BRANCH.to_string(),
                revision,
            },
        ),
        Err(error) => log!("update check: stamp replica: {error}"),
    }
}

/// Bring a space's seed up to the one this worker ships, if it is behind.
///
/// One atomic batch: the previous seed's assertions are retracted and the
/// new library installed together. A retract followed by an assert of the
/// same fact keeps it, citing what it overrode, so the overlap between
/// two seeds survives untouched — only what the old seed had and the new
/// one does not actually goes.
///
/// A space whose seed already matches is left alone, which is the common
/// case: this runs on every mount.
pub(crate) async fn upgrade_seed(tonk: &TonkState, key: &str) -> Result<bool, RepositoryError> {
    // Replicas exist under either key spelling — legacy mounts used the
    // bare suffix, newer ones the full did:key URI — so try the given
    // spelling and fall back to the other before reporting a miss.
    let session = match tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(first) => {
            let alternate = match key.strip_prefix("did:key:") {
                Some(suffix) => suffix.to_string(),
                None => format!("did:key:{key}"),
            };
            tonk.reactor
                .repository(&alternate)
                .branch(CONTENT_BRANCH)
                .acquire(&tonk.operator)
                .await
                .map_err(|_| RepositoryError::Internal(format!("open '{key}': {first}")))?
        }
    };

    // Older onboarding builds recorded their composite inputs as core.yaml.
    // Replaying that library over the imported app replaces its home alias.
    // Snapshot spaces have no single replaceable library; preserve their
    // authored state, including custom home aliases and agent-page changes.
    if super::onboarding_space::has_welcome_snapshot(tonk, key)
        .await
        .map_err(|e| RepositoryError::Internal(format!("read welcome marker: {e}")))?
    {
        return Ok(false);
    }

    let current = read_installed_seed(tonk, &session)
        .await
        .map_err(|e| RepositoryError::Internal(format!("read seed record: {e}")))?;

    let Some(current) = current else {
        // No record: a space seeded before this worker tracked one. Its
        // definitions are whatever it was created with and nothing names
        // them, so an upgrade would have to guess what to withdraw.
        log!("seed upgrade: '{key}' predates the seed record, leaving it alone");
        return Ok(false);
    };

    // Re-fetch the space's OWN source, not the shipped one. A space on a
    // custom seed follows that seed; comparing against `core.yaml` would
    // force it onto the built-in library on its next mount.
    let source = current.source.clone();
    let library = fetch_standard_library(&source)
        .await
        .map_err(|e| RepositoryError::Internal(format!("fetch '{source}': {e}")))?;
    let shipped = seed_version(&library);
    if current.seed.to_string() == shipped {
        return Ok(false);
    }

    let retract = prior_seed_retractions(tonk, &session, &current.version).await?;
    log!(
        "seed upgrade: '{key}' moves to {shipped}, withdrawing {} claims",
        retract.len()
    );

    // The record names the commit that installs the new library. That
    // commit stages, so the version is minted before the record is
    // written rather than predicted.
    let prior = current.seed.to_string();
    let record = |minted: &dialog_artifacts::history::Version| {
        seed_record_facts(
            &shipped,
            &source,
            &prior,
            &prior,
            &encode_seed_version(minted),
        )
    };

    // Retractions and the new library are one staged commit; the record
    // naming it chains on, and a single publish makes both visible. A
    // retract followed by an assert of the same fact keeps it, so what
    // both seeds carry survives while what only the old one had goes.
    super::evaluate::evaluate_with_retractions(
        tonk,
        key,
        CONTENT_BRANCH,
        library,
        retract,
        &record,
    )
    .await
    .map_err(|e| RepositoryError::Internal(format!("upgrade seed '{key}': {e}")))?;
    Ok(true)
}

/// The routes the seed installed at `version`.
///
/// Read from that commit's own history rather than recorded separately:
/// a route the seed installed is a claim it asserted, so the changelog
/// already names them. The router asks this to settle an
/// equal-specificity tie — a route the seed installed loses to one the
/// space authored.
pub(crate) async fn seed_routes(
    tonk: &TonkState,
    session: &dialog_reactor::BranchSession,
    version: &str,
) -> Result<std::collections::HashSet<String>, RepositoryError> {
    use futures_util::StreamExt as _;

    let Some(version) = decode_seed_version(version) else {
        return Ok(std::collections::HashSet::new());
    };

    let history = session.handle().history(&tonk.operator).await;
    let records = history.select(version);
    tokio::pin!(records);

    let mut routes = std::collections::HashSet::new();
    while let Some(record) = records.next().await {
        let (_, record) =
            record.map_err(|e| RepositoryError::Internal(format!("read seed history: {e}")))?;
        if !record.is_assertion() {
            continue;
        }
        let claim = record.claim();
        if claim.the.as_str() == "xyz.tonk.route/path" {
            routes.insert(claim.of.to_string());
        }
    }
    Ok(routes)
}

/// Retract everything the seed installed at `version` asserted, as claims
/// ride the same batch that installs its replacement.
///
/// A revision's history is a changelog: every claim it wrote, with its
/// polarity. Inverting only its ASSERTIONS is load-bearing — a seed
/// install also carries the retractions of the seed before it, and
/// replaying those inverted would restore the version before last.
async fn prior_seed_retractions(
    tonk: &TonkState,
    session: &dialog_reactor::BranchSession,
    version: &str,
) -> Result<Vec<super::claim::RawClaim>, RepositoryError> {
    use futures_util::StreamExt as _;

    let Some(version) = decode_seed_version(version) else {
        return Err(RepositoryError::Internal(format!(
            "seed version '{version}' does not decode"
        )));
    };

    let history = session.handle().history(&tonk.operator).await;
    let records = history.select(version);
    tokio::pin!(records);

    let mut claims = Vec::new();
    while let Some(record) = records.next().await {
        let (_, record) =
            record.map_err(|e| RepositoryError::Internal(format!("read seed history: {e}")))?;
        if !record.is_assertion() {
            continue;
        }
        let claim = record.claim();
        claims.push(super::claim::RawClaim {
            the: claim.the.clone(),
            of: claim.of.clone(),
            is: claim.is.clone(),
            unique: false,
        });
    }
    Ok(claims)
}

/// A revision's version, encoded for the record.
///
/// The version's KEY BYTES, not its entity: the entity is a blake3 hash
/// with no way back, while the key bytes round-trip through
/// `Version::from_key_bytes`. Storing the entity meant hunting the branch
/// log for a matching revision, which only works while the install is
/// still recent.
pub(crate) fn encode_seed_version(version: &dialog_artifacts::history::Version) -> String {
    use base58::ToBase58 as _;

    version.key_bytes().to_base58()
}

/// The inverse of [`encode_seed_version`].
fn decode_seed_version(encoded: &str) -> Option<dialog_artifacts::history::Version> {
    use base58::FromBase58 as _;

    let bytes = encoded.from_base58().ok()?;
    dialog_artifacts::history::Version::from_key_bytes(&bytes).ok()
}

/// The facts recording a seed install: identity and source
/// (`seed/available`), plus what it replaced and the version it committed
/// at (`seed/installed`).
///
/// Two concepts on ONE entity. `seed/available` says the seed exists and
/// where its bytes came from — true of a seed a check merely found, which
/// is why it carries no install fields. `seed/installed` adds them, and
/// its presence is what "this space is running it" means.
///
/// The version is the whole record of WHAT it installed. A revision's
/// history is a changelog — every claim it wrote, with its polarity — so
/// an upgrade reads the prior seed's version and inverts its assertions
/// rather than consulting a per-component tag. Tagging each definition
/// meant naming heads whose identity is content-derived, which the source
/// cannot do.
///
/// The version names the commit these facts are asserted ALONGSIDE, not
/// the one they ride in: the library stages first, its minted version is
/// read off the batch, and this record commits as the next link. One
/// publish makes both visible, so a reader never sees a library without
/// its record.
pub(crate) fn seed_record_facts(
    seed: &str,
    url: &str,
    prior: &str,
    replaces: &str,
    version: &str,
) -> Vec<dialog_artifacts::Instruction> {
    use dialog_artifacts::Statement as _;

    let (Ok(seed), Ok(prior), Ok(replaces)) = (
        seed.parse::<dialog_artifacts::Entity>(),
        prior.parse::<dialog_artifacts::Entity>(),
        replaces.parse::<dialog_artifacts::Entity>(),
    ) else {
        log!("seed record: '{seed}', '{prior}' or '{replaces}' is not an entity");
        return Vec::new();
    };

    let mut changes = dialog_artifacts::Changes::new();
    tonk_schema::SeedAvailable {
        this: seed.clone(),
        source: tonk_schema::domain::seed::Source(url.to_owned()),
        replaces: tonk_schema::domain::seed::Replaces(replaces),
    }
    .assert(&mut changes);
    tonk_schema::SeedInstalled {
        this: seed,
        prior: tonk_schema::domain::seed::Prior(prior),
        version: tonk_schema::domain::seed::Version(version.to_owned()),
    }
    .assert(&mut changes);
    changes.into_instructions()
}

/// The entity naming a seed version: `seed:{hash}` over the bytes actually
/// fetched.
///
/// The hash IS the identity, so two devices installing the same seed derive
/// the same entity and converge. It is taken over what was fetched rather
/// than read from a build manifest, so it cannot disagree with the bytes
/// that were installed.
///
/// The source it came from rides `xyz.tonk.seed/source` alongside, since a
/// custom seed is a different URL at the same shape.
fn seed_version(source: &str) -> String {
    format!("seed:{}", blake3::hash(source.as_bytes()).to_hex())
}

/// The entity standing for "no seed yet" — what a first install records as
/// its predecessor, so `prior` is present on every seed rather than absent
/// on the first.
const SEED_NONE: &str = "seed:none";

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
pub(super) async fn fetch_standard_library(url: &str) -> Result<String, TonkWorkerError> {
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

/// The native sibling of the fetch above: the same documents the
/// service worker fetches from its served assets are compiled in from
/// `tonk-core/assets/library/` — the identical files the dist copies,
/// and the same embedding the CLI uses (`tonk-cli/src/site.rs`).
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(super) async fn fetch_standard_library(url: &str) -> Result<String, TonkWorkerError> {
    match url {
        STANDARD_LIBRARY_URL => {
            Ok(include_str!("../../../tonk-core/assets/library/core.yaml").to_owned())
        }
        PROFILE_LIBRARY_URL => {
            Ok(include_str!("../../../tonk-core/assets/library/profile.yaml").to_owned())
        }
        "/library/onboarding-agent.yaml" => {
            Ok(include_str!("../../../tonk-core/assets/library/onboarding-agent.yaml").to_owned())
        }
        "/library/onboarding-demos.yaml" => {
            Ok(include_str!("../../../tonk-core/assets/library/onboarding-demos.yaml").to_owned())
        }
        "/library/onboarding.yaml" => {
            Ok(include_str!("../../../tonk-core/assets/library/onboarding.yaml").to_owned())
        }
        other => Err(TonkWorkerError::Internal(format!(
            "no embedded library for '{other}'"
        ))),
    }
}

/// Seed a notation document into `branch` by running it through the
/// evaluate pipeline — the same `parse → analyze → commit` path as
/// the `/evaluate` route, which commits concept claims and `rule!:`
/// installs alike. A bad library is a deployment fault, surfaced as
/// an internal error.
pub(super) async fn seed_standard_library(
    tonk: &TonkState,
    repo: &str,
    branch: &str,
    library: &str,
) -> Result<(), TonkWorkerError> {
    // Onboarding composes a scaffold, a named repository, an agent supplement,
    // and an imported application snapshot. These bytes are not core.yaml and
    // must not advertise it as an upgrade source. Ordinary space creation uses
    // seed_and_initialize, which records the actual seed separately.
    super::evaluate::seed_on_branch(
        tonk,
        tonk.reactor.repository(repo).branch(branch),
        library.to_owned(),
    )
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
pub(super) fn repository_name_body(
    subject: &Did,
    display_name: &str,
) -> Result<String, RepositoryError> {
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

/// Provision owned `subject` under this profile's account, repairing a stale
/// consent first. Indirect joined authority keeps its existing provider.
///
/// A space created before sign-in mints its consent to the account the
/// profile held THEN — the onboarding account — and the stored chain
/// does not change when the real account arrives. Presenting it earns
/// `consent was issued to <onboarding>, not the invoking customer`, and
/// treating that refusal as skippable wired spaces to remotes that then
/// refused every presign terminally, with an invite already handed out:
/// the recipient met "you don't have this space" while this device's
/// sync hammered an unprovisionable upstream. The rotation sweep is the
/// repair — it re-issues from the space's own key — so when the stored
/// audience is not the current root it runs HERE, before anything is
/// presented, rather than whenever the next boot chore gets to it.
pub(crate) async fn provision_space_consumer(
    tonk: &TonkState,
    subject: &Did,
) -> Result<(), TonkWorkerError> {
    let held = match space_root_prefix(tonk, subject).await {
        // A joined prefix is `space -> ... -> account`, not the direct
        // `space -> account` consent used to provision an owned space.
        // `/provider/add` consumes its FIRST proof, whose audience belongs
        // to the inviter, and correctly refuses it for this customer.
        // Leave that provider alone. This is not an access check: minting
        // and sync still prove their authority through the full chain.
        Ok(prefix) if prefix.proofs().nth(1).is_some() => return Ok(()),
        Ok(prefix) => match super::identity::root_did(tonk).await {
            Ok(root) if prefix.audience() != &root => None,
            _ => Some(prefix),
        },
        // A space created before sign-in may have persisted no prefix at
        // all — there was no account to delegate to. The rotation sweep
        // below mints and installs it from the sealed seed.
        Err(TonkWorkerError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };
    let prefix = match held {
        Some(current) => current,
        None => {
            log!("{subject}: consent missing or audienced to a retired account; re-issuing");
            super::rotation::rotate_from_onboarding(tonk).await;
            space_root_prefix(tonk, subject).await?
        }
    };
    super::customer::provision_consumer(tonk, subject, &prefix, None).await
}

/// Whether `remote` is this deployment's own access service — the one
/// party whose provisioning refusal is authoritative for it. A foreign
/// remote (self-hosted, a test server) is attached and shared without
/// asking our service's opinion.
pub(super) fn remote_is_own_service(remote: &str) -> bool {
    let Ok(own) = super::customer::service_origin() else {
        return false;
    };
    url::Url::parse(remote)
        .map(|remote| remote.origin() == own.origin())
        .unwrap_or(false)
}

/// Load the provider-neutral `space → … → root` prefix saved at creation or join.
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
    let _admission_mutation = tonk.admission.mutation(key);

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
        .publish()
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

/// Mirror one repository-authored display name into the account directory.
///
/// Joined spaces can become visible before their content (and therefore their
/// name) has downloaded. The account sweep calls this after later pulls so an
/// initially nameless Hub row repairs itself without remounting the space.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn record_space_name(tonk: &TonkState, subject: &Did, display_name: &str) {
    let transaction = tonk
        .reactor
        .profile_repository()
        .branch(PROFILE_BRANCH)
        .transaction()
        .assert(tonk_schema::SpaceName::new(subject, display_name));
    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("record space name for '{subject}': {error}");
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
        // The queryable twin of the `home.address` the space's grants
        // carry in signed meta: the UCAN endpoint the content branch
        // syncs through, on the directory entity.
        if branch_name == "main"
            && let Some(upstream) = &settings.upstream
            && let Some(remote_config) = configuration.remote.get(&upstream.remote)
            && let SiteAddress::Ucan(ucan) = &remote_config.address
        {
            transaction =
                transaction.assert(tonk_schema::SpaceHomeAddress::new(subject, ucan.endpoint()));
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
pub(super) async fn set_replica_status(
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

/// Fetch and seed the lean profile library onto the profile branch —
/// on every target, since the fetch reads the embedded assets natively.
async fn seed_profile_library(tonk: &TonkState) -> Result<(), RepositoryError> {
    let library = fetch_standard_library(PROFILE_LIBRARY_URL)
        .await
        .map_err(|e| RepositoryError::Internal(format!("fetch profile library: {e}")))?;
    let version = seed_version(&library);
    // The record names the commit that installs the library — see the
    // create path for why recording separately breaks provenance.
    let record = |minted: &dialog_artifacts::history::Version| {
        seed_record_facts(
            &version,
            PROFILE_LIBRARY_URL,
            SEED_NONE,
            SEED_NONE,
            &encode_seed_version(minted),
        )
    };
    super::evaluate::evaluate_profile_body_recording(tonk, PROFILE_BRANCH, library, &record)
        .await
        .map(|_| ())
        .map_err(|e| {
            RepositoryError::Internal(format!("seed standard library on profile branch: {e}"))
        })
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
        Ok(true) => {
            super::adopt::schedule_seed_upgrade(&tonk, state.clone(), &name).await;
            None
        }
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
async fn repository_label<'a, R>(
    tonk: &'a TonkState,
    repository: &'a Repository<R>,
    key: &'a str,
) -> String
where
    R: Principal + Clone,
{
    repository_display_name(tonk, repository, key)
        .await
        .unwrap_or_else(|| key.to_string())
}

/// Read the repository-authored display name without inventing a routing-key
/// fallback. Account-directory reconciliation uses absence to mean "content
/// has not hydrated far enough yet" and retries after later pulls.
pub(super) async fn repository_display_name<R>(
    tonk: &TonkState,
    repository: &Repository<R>,
    key: &str,
) -> Option<String>
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
            return None;
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
        Ok(rows) => rows.into_iter().next().map(|row| row.name.0),
        Err(e) => {
            log!("tonk/repository label query failed for '{}': {:?}", key, e);
            None
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
    #[cfg(test)]
    assert!(
        !tonk
            .reject_admission_content_reads
            .load(std::sync::atomic::Ordering::Relaxed),
        "unexpected content projection during admission",
    );
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
pub(super) async fn ensure_remote_config<C>(
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
    let _admission_mutation = tonk.admission.mutation(repository.did().as_str());
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
        .publish()
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
    use super::{LEGACY_REMOTE_ATTR, REMOTE_ATTR};

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
    /// Both spellings stay readable so a frozen older descriptor that
    /// still declares the field keeps working; neither is where the
    /// answer comes from any more.
    #[test]
    fn it_declares_no_remote_on_the_create_form() {
        for attribute in [REMOTE_ATTR, LEGACY_REMOTE_ATTR] {
            assert!(
                !PROFILE_LIBRARY.contains(attribute),
                "profile.yaml declares `the: {attribute}` again — a space \
                 would wire a remote at creation instead of earning one \
                 when it is shared",
            );
        }
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

    /// A create that says nothing about opening does NOT navigate.
    ///
    /// The default matters more than the flag: every caller that is not
    /// the Hub's own form — a script, an agent, a future affordance —
    /// gets a space in the Hub without the page being yanked out from
    /// under whoever is using it.
    #[test]
    fn it_does_not_open_a_space_by_default() {
        let of: Entity = "did:key:zCreate".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        assert!(
            !super::open_from_facts(&artifacts(changes)),
            "a create carrying no `open` field creates only"
        );
    }

    /// The Hub's form passes `open` as a hidden input, so it arrives as
    /// text rather than a boolean.
    #[test]
    fn it_opens_a_space_when_the_form_asks() {
        let of: Entity = "did:key:zCreate".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        the!("xyz.tonk.command.create-space/open")
            .of(of)
            .is("true".to_string())
            .assert(&mut changes);
        assert!(
            super::open_from_facts(&artifacts(changes)),
            "the Hub's hidden `open=true` input navigates"
        );
    }

    /// An explicit falsehood is honoured rather than read as "present,
    /// therefore yes" — a form that binds the field but leaves it off
    /// must not navigate.
    #[test]
    fn it_honours_an_explicit_refusal_to_open() {
        for text in ["false", "0", "", "  "] {
            let of: Entity = "did:key:zCreate".parse().expect("entity");
            let mut changes = Changes::new();
            name_fact(&mut changes, &of);
            the!("xyz.tonk.command.create-space/open")
                .of(of)
                .is(text.to_string())
                .assert(&mut changes);
            assert!(
                !super::open_from_facts(&artifacts(changes)),
                "`open={text:?}` must not navigate"
            );
        }
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

    /// The attribute the app posts now. The two cases above use the DOM
    /// read path a branch seeded before the migration still asserts, so
    /// between them both spellings are covered.
    #[test]
    fn it_reads_the_commands_own_remote_attribute() {
        let of: Entity = "did:key:zCreate".parse().expect("entity");
        let mut changes = Changes::new();
        name_fact(&mut changes, &of);
        the!("xyz.tonk.command.create-space/remote")
            .of(of)
            .is(URL.to_string())
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
mod notebook_creation_tests {
    use super::*;

    const NOTEBOOK_LIBRARY: &str = include_str!("../../../tonk-core/assets/library/notebook.yaml");

    /// Notebook creation with NO worker provider: the page-built notation
    /// (the inspector element's `create_notation`, evaluated through the
    /// same pipeline its host consumer submits on) writes `notebook/named`
    /// and the draft's blocks in one commit, and the library's own rules
    /// persist the inserted blocks. This is the end-to-end guard for the
    /// retired `CreateNotebook` provider — its failure mode is the old
    /// one: creation silently doing nothing.
    #[dialog_common::test]
    async fn it_creates_a_notebook_from_the_page_built_notation_alone() {
        use futures_util::StreamExt as _;

        let state = crate::router::command::tests::native::test_state().await;
        let key = create_space_inner(&state, "Notebook Host")
            .await
            .expect("the space creates");
        let tonk = state.read().await;
        // The notebook library installs onto the space, rules and all —
        // what a notebook-using space carries.
        super::super::evaluate::evaluate_body(
            &tonk,
            &key,
            CONTENT_BRANCH,
            NOTEBOOK_LIBRARY.to_owned(),
            true,
        )
        .await
        .expect("the notebook library installs");

        let notation = tonk_inspector::notation::create_notation(
            "notebook:probe",
            "Groceries: a \"list\"",
            "# Groceries\n\nmilk\n\neggs",
        )
        .expect("the document builds");
        let response =
            super::super::evaluate::evaluate_body(&tonk, &key, CONTENT_BRANCH, notation, true)
                .await
                .expect("the creation evaluates");
        assert!(
            response.commits.claims > 0,
            "the creation must commit, not silently no-op"
        );

        // Read back what landed on the notebook entity and its blocks.
        let session = tonk
            .reactor
            .repository(&key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("the content branch opens");
        let entity: dialog_artifacts::Entity = "notebook:probe".parse().unwrap();
        let stream = session
            .handle()
            .claims()
            .select(dialog_artifacts::ArtifactSelector::new().of(entity))
            .perform(&tonk.operator)
            .await
            .expect("the notebook claims select");
        tokio::pin!(stream);
        let mut named = None;
        let mut sequence_entries = 0usize;
        while let Some(artifact) = stream.next().await {
            let artifact = artifact
                .expect("a claim reads")
                .to_owned()
                .expect("a claim decodes");
            let attribute = artifact.the.to_string();
            if attribute == "xyz.tonk.notebook/title"
                && let dialog_artifacts::Value::String(title) = &artifact.is
            {
                named = Some(title.clone());
            }
            if attribute.starts_with("xyz.tonk.notebook/") {
                sequence_entries += 1;
            }
        }
        let named = named.expect("the notebook is named");
        assert_eq!(
            named, "Groceries: a \"list\"",
            "the YAML-hostile title survives as data"
        );
        assert!(
            sequence_entries >= 3,
            "the three draft blocks land in the sequence (got {sequence_entries} notebook facts)"
        );
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod rename_repository_tests {
    use super::*;
    use crate::router::command::{CommandOrigin, dispatch};
    use dialog_artifacts::Statement;
    use dialog_query::the;

    /// Every name-bearing record for `key`, read back the way its
    /// consumers read them: the space's own [`RepositoryName`] on its
    /// content branch (what the space renders), and the profile
    /// branch's [`tonk_schema::SpaceName`] directory mirror (what a
    /// device that never replicated the space labels it by).
    async fn names(state: &crate::router::AppState, key: &str) -> (Vec<String>, Vec<String>) {
        let tonk = state.read().await;
        let content = tonk
            .reactor
            .repository(key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("content branch opens");
        let space: Vec<RepositoryName> = content
            .handle()
            .query()
            .select(Query::<RepositoryName> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("repository-name query");
        let profile_branch = tonk
            .reactor
            .profile_repository()
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("profile branch opens");
        let mirror: Vec<tonk_schema::SpaceName> = profile_branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceName> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("space-name mirror query");
        (
            space.into_iter().map(|row| row.name.0).collect(),
            mirror.into_iter().map(|row| row.name.0).collect(),
        )
    }

    /// A space renaming ITSELF — the one space-side command with a
    /// write outside its own branch. Dispatched with the space as
    /// origin (the vocabulary split keeps `RenameRepository` in the
    /// space registry for exactly this), the provider must land the
    /// name in BOTH records: the space's own `RepositoryName` and the
    /// profile's `SpaceName` directory mirror. A regression that
    /// updates only one desynchronizes what the space shows from what
    /// the Hub of a non-replicated device shows.
    #[dialog_common::test]
    async fn it_updates_both_records_when_a_space_renames_itself() {
        let state = crate::router::command::tests::native::test_state().await;
        let key = create_space_inner(&state, "Before Rename")
            .await
            .expect("the space creates");

        let mut changes = dialog_artifacts::Changes::new();
        let command: dialog_artifacts::Entity = "cmd:rename".parse().unwrap();
        the!("xyz.tonk.command.rename-repository/name")
            .of(command.clone())
            .is("After Rename".to_string())
            .assert(&mut changes);
        the!("xyz.tonk.rename-repository/space")
            .of(command)
            .is(key.parse::<dialog_artifacts::Entity>().unwrap())
            .assert(&mut changes);
        dispatch(
            &state,
            CommandOrigin {
                repo: key.clone(),
                branch: CONTENT_BRANCH.to_string(),
                client: None,
            },
            changes,
        )
        .await;

        let (space, mirror) = names(&state, &key).await;
        assert_eq!(
            space,
            vec!["After Rename".to_string()],
            "the space's own RepositoryName record carries the new name"
        );
        assert_eq!(
            mirror,
            vec!["After Rename".to_string()],
            "the profile's SpaceName directory mirror carries the new name"
        );
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod invite_chain_tests {
    use super::*;

    /// The `Authorization` rows a mint records on the content branch —
    /// the durable half of an invite, carrying the base58 proof chain.
    async fn authorizations(
        state: &crate::router::AppState,
        repo: &str,
    ) -> Vec<tonk_schema::command::Authorization> {
        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .repository(repo)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("content branch opens");
        branch
            .handle()
            .query()
            .select(Query::<tonk_schema::command::Authorization> {
                this: Term::var("this"),
                proof: Term::var("proof"),
                remote: Term::var("remote"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("authorization query")
    }

    /// The chain a real mint produces, via the real path end to end: a
    /// linked, activated account against a live access service,
    /// `create_space_inner` (which delegates the fresh space to the
    /// ACCOUNT), `enable_sync_inner` (which provisions and attaches the
    /// remote), then `run_invite`.
    ///
    /// The shape pinned is `space → account → profile → membership`, and
    /// its durability: an invite is claimed hours or days after minting,
    /// so the presign path's session-bounded operator claim must never
    /// appear in it — the two claims live one seam apart
    /// (`Delegate::perform` binds the profile signer; the presign's
    /// `Authorize` binds the operator's), and nothing else fails if a
    /// refactor swaps them. The links would just start dying within
    /// `SESSION_TTL_SECONDS` (12h) of minting.
    #[dialog_common::test]
    async fn it_mints_the_durable_space_account_profile_membership_chain() {
        let (tonk, service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let profile_did = tonk.profile.did().to_string();
        let operator_did = tonk.operator.did().to_string();
        let session_expiry = tonk.session_expires_at;
        let account_did = root.did().to_string();

        // A creation custodies the space seed to the account's sealed
        // recipient, which a real ceremony hands back with the root; the
        // base fixture deliberately leaves it unpublished, so record one
        // the way `persist_test_root` does.
        let recipient =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([7u8; 32]))
                .secret()
                .did();
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "invite-chain-test".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .expect("the root persists with a recipient");
        let state: crate::router::AppState = std::sync::Arc::new(tokio::sync::RwLock::new(tonk));

        let key = create_space_inner(&state, "Invite Chain")
            .await
            .expect("the space creates");
        enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");

        let env =
            crate::router::CommandEnv::new(state.clone(), crate::router::CommandOrigin::default());
        run_invite(&env, &key, 1.0).await.expect("the mint settles");
        drop(env);

        let rows = authorizations(&state, &key).await;
        assert_eq!(rows.len(), 1, "the mint records one authorization");
        assert_eq!(
            rows[0].this.to_string(),
            key,
            "the Authorization row is keyed on the space subject (cardinality-one per space)"
        );
        let bytes = bs58::decode(&rows[0].proof.0)
            .into_vec()
            .expect("the proof is base58");
        let chain =
            DelegationChain::try_from(bytes.as_slice()).expect("the proof parses as a chain");
        // The membership principal is freshly minted per invite; the chain's
        // audience is the only place it appears.
        let membership = chain.audience().to_string();
        for held in [&key, &account_did, &profile_did, &operator_did] {
            assert_ne!(
                &membership, held,
                "the membership keypair is fresh, not a principal this device holds"
            );
        }
        let hops: Vec<_> = chain.proofs().collect();

        assert_eq!(
            hops.len(),
            3,
            "the chain is space → account → profile → membership"
        );
        assert_eq!(
            hops[0].issuer().to_string(),
            key,
            "the space signs its consent"
        );
        assert_eq!(
            hops[0].audience().to_string(),
            account_did,
            "creation delegates the space to the ACCOUNT, not the device"
        );
        assert_eq!(hops[1].issuer().to_string(), account_did);
        assert_eq!(
            hops[1].audience().to_string(),
            profile_did,
            "the link ceremony's account → profile grant bridges to this device"
        );
        assert_eq!(hops[2].issuer().to_string(), profile_did);
        assert_eq!(
            hops[2].audience().to_string(),
            membership,
            "the profile signs the membership leaf"
        );
        for hop in &hops {
            assert_ne!(
                hop.issuer().to_string(),
                operator_did,
                "no hop may be issued by the session-scoped operator"
            );
            if let Some(expiration) = hop.expiration() {
                assert!(
                    expiration.to_unix() > session_expiry,
                    "a hop expiring at {} dies with the operator session ({})",
                    expiration.to_unix(),
                    session_expiry,
                );
            }
        }

        // The minted link carries the space's display name as the
        // advisory `name` parameter, read back through the overlay
        // `Credential` the share view renders — so the recipient's Hub
        // row is labeled before the space's content syncs.
        {
            use tonk_schema::command::Credential;
            let tonk = state.read().await;
            let branch = tonk
                .reactor
                .repository(&key)
                .branch(CONTENT_BRANCH)
                .acquire(&tonk.operator)
                .await
                .expect("content branch opens");
            let credentials: Vec<Credential> = branch
                .handle()
                .query()
                .select(Query::<Credential> {
                    this: Term::var("this"),
                    seed: Term::var("seed"),
                    link: Term::var("link"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("credential query");
            assert_eq!(credentials.len(), 1, "the mint records one credential");
            // The link lives on the host actually serving the space —
            // the origin derived from its access endpoint — never on the
            // minting surface or the hardcoded production base.
            let link = credentials[0].link.0.clone();
            let serving = url::Url::parse(&remote).expect("the fixture remote is a URL");
            assert_eq!(
                url::Url::parse(&link).expect("the link is a URL").origin(),
                serving.origin(),
                "the minted link is rooted on the space's serving host"
            );
            // The fixture host provides conforming shortening (content
            // hash + redirect), so the mint shortened; resolve the way a
            // claimer does before parsing. A long link (a host without
            // shortening) parses as-is — both arms are live behavior.
            let resolved = if tonk_invite::shortcut::is_shortcut(&link) {
                let client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("probe client builds");
                let response = client
                    .get(&link)
                    .send()
                    .await
                    .expect("the short link answers");
                assert!(
                    response.status().is_redirection(),
                    "a short link redirects (got HTTP {})",
                    response.status()
                );
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .expect("the redirect carries a Location");
                tonk_invite::shortcut::resolve_location(&link, location)
                    .expect("the redirect resolves")
            } else {
                link
            };
            let minted = tonk_invite::Invite::parse_url(&resolved)
                .await
                .unwrap_or_else(|e| panic!("link {resolved} did not parse: {e}"));
            assert_eq!(
                minted.space_name.as_deref(),
                Some("Invite Chain"),
                "the minted link names the space it invites into"
            );
            assert!(
                matches!(minted.audience, tonk_invite::InviteAudience::Open { .. }),
                "the seed fragment survives shortening and resolution"
            );
        }

        // Fixture cleanup, the way account_state's own tests do it.
        let account_key = {
            let tonk = state.read().await;
            crate::router::account_state::require_ready_account_state(&tonk)
                .await
                .expect("the linked account is ready")
                .key
                .clone()
        };
        let tonk = std::sync::Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("the state has no other holders"))
            .into_inner();
        crate::router::account_state::tests::discard(tonk, &account_key);
        drop(service);
    }
}

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
        // Profile origin: `profile/rename` is a profile-vocabulary
        // command (the FAB dispatches it routeless on the profile
        // branch); a space-branch dispatch is contained by design.
        let changes = profile_rename_transient("did:key:zRenameCmd", "brave-lynx");
        crate::router::dispatch(&state, crate::router::CommandOrigin::default(), changes).await;

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

        // Rename from the profile branch, where the FAB dispatches it
        // routeless; a space-branch dispatch is contained by design.
        let changes = profile_rename_transient("did:key:zRenameAll", "brave-lynx");
        crate::router::dispatch(&state, crate::router::CommandOrigin::default(), changes).await;

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

        // Profile origin, like the FAB's routeless dispatch; a
        // space-branch `profile/rename` is contained by design.
        let changes = profile_rename_transient("did:key:zRenameEmpty", "   ");
        crate::router::dispatch(&state, crate::router::CommandOrigin::default(), changes).await;

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

    /// Entries the first result row folds together under `field` — an
    /// open collection query returns ONE row per entity, its entries
    /// keyed inside the field's dictionary.
    async fn entry_count(state: &AppState, repo: &str, query: &str, field: &str) -> usize {
        rows(state, repo, query)
            .await
            .first()
            .and_then(|row| row.get(field))
            .and_then(|value| value.as_object())
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// Run a query document and return its result rows, as JSON.
    async fn rows(
        state: &AppState,
        repo: &str,
        query: &str,
    ) -> Vec<std::collections::BTreeMap<String, serde_json::Value>> {
        let guard = state.read().await;
        let response = evaluate_body(&guard, repo, "main", query.to_owned(), false)
            .await
            .unwrap_or_else(|e| panic!("query failed: {e}"));
        response
            .matches_after
            .first()
            .map(|block| {
                block
                    .results
                    .iter()
                    .map(|result| {
                        // `this` is the match's entity, carried beside the
                        // bound fields rather than among them.
                        let mut fields = result.fields.clone();
                        fields.insert(
                            "this".to_owned(),
                            serde_json::Value::String(result.this.clone()),
                        );
                        fields
                    })
                    .collect()
            })
            .unwrap_or_default()
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
    const NOTEBOOK: &str = include_str!("../../../tonk-core/assets/library/notebook.yaml");

    /// The notebook library on a core-seeded branch: the `block`
    /// sequence seeds three entries, a `block/place` command adds a
    /// fourth under the position the element derived, and
    /// `block/remove` takes it back out.
    #[dialog_common::test]
    async fn it_places_and_removes_notebook_blocks() {
        let (_app, state, key) = fresh_repo("test-notebook-sequence").await;
        let repo = key.as_str();
        seed(&state, repo, CORE).await;
        seed(&state, repo, NOTEBOOK).await;

        let entries = "notebook:\n  this: id:notebook/scratch\n  block: {?key: ?block}\n";
        assert_eq!(
            entry_count(&state, repo, entries, "block").await,
            3,
            "the seed places three blocks"
        );

        seed(
            &state,
            repo,
            "block/place!:\n  subject: id:notebook/scratch/4\n  notebook: id:notebook/scratch\n  key: \"N7\"\n",
        )
        .await;
        assert_eq!(
            entry_count(&state, repo, entries, "block").await,
            4,
            "a placement adds an entry under its key"
        );
        assert_eq!(
            count(
                &state,
                repo,
                "notebook:\n  this: id:notebook/scratch\n  block: {N7: ?block}\n"
            )
            .await,
            1,
            "the entry is readable under the literal key"
        );

        seed(
            &state,
            repo,
            "block/remove!:\n  subject: id:notebook/scratch/4\n  notebook: id:notebook/scratch\n  key: \"N7\"\n",
        )
        .await;
        assert_eq!(
            entry_count(&state, repo, entries, "block").await,
            3,
            "a removal retracts the entry"
        );
    }

    /// A RUN of blocks inserted in ONE transaction lands in document order.
    ///
    /// This is the case a per-block command cannot express: block 2's
    /// position depends on block 3's, which does not exist until the same
    /// commit derives it. The chain (`next`) plus the recursive position
    /// rules resolve the whole run in a single round; a design that derived
    /// each block from its PREDECESSOR needed the predecessor's position to
    /// survive into a second round, which a transient command does not.
    #[dialog_common::test]
    async fn it_inserts_a_run_of_blocks_in_document_order() {
        let (_app, state, key) = fresh_repo("test-notebook-insert-run").await;
        let repo = key.as_str();
        seed(&state, repo, CORE).await;
        seed(&state, repo, NOTEBOOK).await;

        // Exactly what `insert_notation` emits for three blocks appended
        // after the seed's last block: written back to front, chained
        // forward by variable, every one anchored on the block the run
        // follows.
        seed(
            &state,
            repo,
            r#"block/insert!:
  this: ?b2
  notebook: id:notebook/scratch
  source: |-
    three
  next: case:none
  prev: id:notebook/scratch/3

block/insert!:
  this: ?b1
  notebook: id:notebook/scratch
  source: |-
    two
  next: ?b2
  prev: id:notebook/scratch/3

block/insert!:
  this: ?b0
  notebook: id:notebook/scratch
  source: |-
    one
  next: ?b1
  prev: id:notebook/scratch/3
"#,
        )
        .await;

        let blocks = count(
            &state,
            repo,
            "notebook/block:\n  this: ?this\n  notebook: id:notebook/scratch\n  source: ?source\n",
        )
        .await;
        let chains = count(&state, repo, "block/chain:\n  this: ?this\n  next: ?next\n").await;
        let positions = count(&state, repo, "block/position:\n  this: ?this\n  at: ?at\n").await;

        // An open collection query folds per entity: ONE row for the
        // notebook, its entries under `block` keyed by position. Order
        // lives in the KEYS: positions are fractional indices, ordered
        // lexicographically.
        let placed = rows(
            &state,
            repo,
            "notebook:\n  this: id:notebook/scratch\n  block: {?block/key: ?block}\n",
        )
        .await;
        let entry_map = placed
            .first()
            .and_then(|row| row.get("block"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            entry_map.len(),
            6,
            "three seeded blocks plus the three inserted ones \
             (blocks={blocks} chains={chains} positions={positions} placed={placed:#?})"
        );
        let sources = rows(
            &state,
            repo,
            "notebook/block:\n  this: ?this\n  notebook: id:notebook/scratch\n  source: ?source\n",
        )
        .await;
        let source_of: std::collections::BTreeMap<String, String> = sources
            .iter()
            .filter_map(|row| {
                Some((
                    row.get("this")?.as_str()?.to_owned(),
                    row.get("source")?.as_str()?.to_owned(),
                ))
            })
            .collect();

        // Position keys are fractional indices: their LEXICOGRAPHIC
        // order is the document order, which `serde_json`'s BTreeMap
        // iteration yields directly.
        let document: Vec<&str> = entry_map
            .values()
            .filter_map(|block| source_of.get(block.as_str()?))
            .map(String::as_str)
            .collect();

        assert_eq!(
            document.len(),
            6,
            "every placed block resolves to a source: placed={placed:#?}"
        );
        assert_eq!(
            &document[3..],
            &["one", "two", "three"],
            "the run reads in document order, after the seeded blocks: {document:#?}"
        );
    }

    /// A rename must not also CREATE. Both commands are transient and both
    /// carry a title, so they decode from the same event unless their
    /// attributes differ.
    #[dialog_common::test]
    async fn it_does_not_create_a_notebook_when_retitling() {
        let (_app, state, key) = fresh_repo("test-notebook-retitle-only").await;
        let repo = key.as_str();
        seed(&state, repo, CORE).await;
        seed(&state, repo, NOTEBOOK).await;

        let before = count(
            &state,
            repo,
            "notebook/named:\n  this: ?this\n  title: ?title\n",
        )
        .await;

        seed(
            &state,
            repo,
            "notebook/retitle!:\n  subject: id:notebook/scratch\n  title: \"Renamed\"\n",
        )
        .await;

        let named = rows(
            &state,
            repo,
            "notebook/named:\n  this: ?this\n  title: ?title\n",
        )
        .await;
        assert_eq!(
            named.len(),
            before,
            "a rename renames in place, it does not add one: {named:#?}"
        );
        let titles: Vec<&str> = named
            .iter()
            .filter_map(|row| row.get("title")?.as_str())
            .collect();
        assert!(titles.contains(&"Renamed"), "and it took: {named:#?}");
    }

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

    /// The empty-state canvas keeps the pending label only while the handoff
    /// request is unanswered. A refusal resolves the nested model and renders
    /// the explicit local-only notice instead of spinning forever.
    #[dialog_common::test]
    fn it_routes_refused_agent_links_to_the_local_only_notice() {
        assert!(
            CORE.contains("slot=\"no-entity\"") && CORE.contains("model=tonk:agent-handoff-state"),
            "agent-link fallback should query its independent handoff status",
        );
        assert!(
            !CORE.contains("agent link &middot; paste into your agent"),
            "the rendered state should provide its own single label",
        );
        assert!(
            CORE.contains("tonk-display > [slot][hidden]"),
            "inactive pending and refusal slots should not survive a ready result",
        );
        assert!(CORE.contains("<p data-agent-handoff-status>{status}</p>"));
        assert!(
            !CORE.contains("Use connect in the condition banner"),
            "the refusal must not prescribe a repair that is absent or inappropriate"
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

    /// After attach, a minted invite names the endpoint — the whole
    /// point of the opt-in remote, so `tonk join` has something to
    /// pull from. It rides inside the signed chain's `home.address`
    /// meta, not a `remote=` URL parameter.
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

        assert!(
            !invite.url().query_pairs().any(|(key, _)| key == "remote"),
            "the endpoint rides inside the signed chain, not the URL; url was {}",
            invite.url(),
        );
        let parsed = tonk_invite::Invite::parse_url(invite.url().as_str())
            .await
            .expect("minted invite URL parses");
        assert_eq!(
            parsed.remote_url.map(String::from).as_deref(),
            Some("https://example.test/ucan/"),
            "the chain's home.address must name the attached endpoint",
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

    /// Reconciling the cached handle refreshes it IN PLACE, so everything
    /// hanging off the `BranchState` — live subscriptions AND the session
    /// overlay — survives. An earlier version opened a fresh handle and
    /// swapped it in: the swap adopted subscriptions but a fresh open
    /// mints an empty overlay, so a tab's `tonk:site` stamp vanished the
    /// moment the share flow wired a remote and the space view sat on its
    /// placeholder dot indefinitely.
    #[dialog_common::test]
    async fn it_keeps_subscriptions_and_overlay_when_refreshing_a_branch() {
        use std::sync::Arc;

        use dialog_query::{ConceptQuery, Query};
        use tonk_schema::meta::{Name, name::Referent};

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
        // arrives next can only be a later write's doing.
        while subscriber.receiver.try_recv().is_ok() {}

        // An ephemeral session fact — the kind a tab's `tonk:site` stamp
        // or the sync status is made of. It must survive the refresh.
        {
            let guard = state.read().await;
            guard
                .reactor
                .repository(repo)
                .branch("main")
                .overlay()
                .assert(Name {
                    this: "id:overlay-survivor".parse().expect("entity"),
                    entity: Referent("id:overlay-target".parse().expect("entity")),
                })
                .write()
                .perform(&guard.operator)
                .await
                .expect("overlay write");
            guard.reactor.run_scheduled_polls(&guard.operator).await;
        }
        assert!(
            subscriber.receiver.try_recv().is_ok(),
            "the overlay write must reach the live subscriber",
        );
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
            std::ptr::eq(before_ptr, Arc::as_ptr(&session.state)),
            "refresh must keep the cached BranchState, not swap it",
        );
        assert_eq!(
            session.state.subscriptions().lock().len(),
            1,
            "the live subscription must survive the refresh",
        );
        // The refreshed handle must still track the wired upstream…
        assert!(
            session.state.branch.upstream().is_some(),
            "the in-place refresh must pick up the wired upstream",
        );
        // …and still fold the session overlay: a fresh subscriber's
        // snapshot carries the ephemeral fact. Before the in-place
        // refresh this was the share-flow regression — the swapped-in
        // handle's empty overlay silently dropped every session fact.
        let mut fresh = session
            .subscribe(ConceptQuery::from(Query::<Name>::default()), None)
            .expect("subscribe after refresh");
        // A new subscriber is Pending until a poll serves its snapshot;
        // drive one the way the request dispatcher would.
        guard.reactor.schedule_poll(Arc::clone(&session.state));
        guard.reactor.run_scheduled_polls(&guard.operator).await;
        let mut snapshot = Vec::new();
        while let Ok(bytes) = fresh.receiver.try_recv() {
            snapshot.extend_from_slice(&bytes);
        }
        let snapshot = String::from_utf8_lossy(&snapshot);
        assert!(
            snapshot.contains("overlay-survivor"),
            "the session overlay must survive the refresh; snapshot: {snapshot}",
        );
        drop(fresh);
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

    /// Build the routeless invite command emitted by `<tonk-share>`.
    ///
    /// Unlike a space-authored invite form, the FABB dispatches from the
    /// profile branch and names the target space explicitly. Joined members
    /// exercise this path too, so a regression here otherwise presents only
    /// as the share control waiting until its clipboard timeout.
    fn fabb_invite_transient(of: &str, subject: &dialog_varsig::Did) -> dialog_artifacts::Changes {
        use dialog_artifacts::Statement;
        use dialog_query::the;
        use tonk_schema::prelude::DidExt as _;

        let mut changes = invite_transient(of);
        let entity = of.parse().expect("entity URI");
        the!("xyz.tonk.invite/space")
            .of(entity)
            .is(subject.this())
            .assert(&mut changes);
        changes
    }

    /// A member who arrived through an invite can mint the next invite from
    /// the FABB. The visible failure is a timeout, but the boundary that must
    /// answer is the routeless `tonk:invite` dispatch: it has to resolve the
    /// joined repository and delegate from the authority accepted at join.
    #[dialog_common::test]
    async fn it_mints_from_the_fabb_after_joining_a_space() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = crate::router::join::tests::handcrafted_invite_url(121, 122).await;
        assert_eq!(
            crate::router::join::tests::post_join(&app, &url).await,
            StatusCode::CREATED,
            "the member first joins the space",
        );

        let _ = post_remote(&app, &key, "https://sync.example.test/ucan/", None).await;
        let subject: dialog_varsig::Did = key.parse().expect("joined subject DID");
        {
            let tonk = state.read().await;
            assert!(
                !crate::router::customer::space_provider_recorded(&tonk, &subject).await,
                "joining does not make this account the provider",
            );
            super::provision_space_consumer(&tonk, &subject)
                .await
                .expect("joined authority must leave provisioning with the existing provider");
        }
        let before = content_invitations(&state, &key).await.len();

        crate::router::dispatch(
            &state,
            crate::router::CommandOrigin::default(),
            fabb_invite_transient("did:key:zJoinedMemberFabbInvite", &subject),
        )
        .await;

        assert_eq!(
            content_invitations(&state, &key).await.len(),
            before + 1,
            "the joined member's FABB share must mint a fresh invitation",
        );
        assert!(
            share_blocked_rows(&state, &key).await.is_empty(),
            "a joined member's valid authority must not be reported as a refused share",
        );
    }

    /// Direct owned authority must still reach provisioning. This harness
    /// has no worker origin, so reaching the service boundary returns an
    /// error rather than silently treating the owned space as already served.
    #[dialog_common::test]
    async fn it_requires_provisioning_for_owned_space_authority() {
        let (_app, state, key) = fresh_repo("owned-space-provisioning").await;
        let tonk = state.read().await;
        let subject = key.parse().unwrap();
        let prefix = super::space_root_prefix(&tonk, &subject).await.unwrap();
        assert_eq!(prefix.proofs().count(), 1);
        let error = super::provision_space_consumer(&tonk, &subject)
            .await
            .expect_err("owned authority must still attempt provisioning");
        assert!(
            matches!(error, crate::TonkWorkerError::Internal(ref detail) if detail == "the worker origin is unavailable"),
            "expected the service boundary, got {error}",
        );
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

    /// The account sweep's eligibility boundary is "no remote at all", not
    /// merely "main has no upstream". Any existing remote may represent a
    /// non-default or partially configured deployment and must be preserved.
    #[dialog_common::test]
    async fn it_does_not_auto_attach_over_any_existing_remote() {
        use dialog_repository::RepositoryExt as _;

        let (app, state, repo) = fresh_repo("test-account-reconcile-preserves-remote").await;
        let _ = post_remote(&app, &repo, "https://existing.example.test/ucan/", None).await;

        let tonk = state.read().await;
        assert!(
            !super::attach_account_remote_if_local(
                &tonk,
                &repo,
                "https://account.example.test/ucan/",
            )
            .await
            .unwrap(),
            "a repository with any remote is ineligible for automatic attachment",
        );
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(&repo)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let info = super::build_repository_info(&tonk, &repo, &repository).await;
        let address = serde_json::to_string(&info.remote["origin"].address).unwrap();
        assert!(
            address.contains("https://existing.example.test/ucan/"),
            "the existing remote must survive account reconciliation: {address}",
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
    /// the query, on the resolved base — the host serving the space.
    ///
    /// Driven through [`long_invite_url`] directly rather than through the
    /// mint, which exercises the base resolution separately (the native
    /// chain test pins the link's origin to the space's remote).
    ///
    /// The fragment split is the load-bearing part. The seed must never
    /// reach a server, and shortening PUTs only the path + query — so a
    /// seed that slipped into the query would be uploaded to the shortcut
    /// service in plaintext.
    #[dialog_common::test]
    async fn it_builds_the_invite_url_on_the_resolved_base() {
        let url = super::long_invite_url(
            "https://tonk.example/join",
            "PROOF",
            "&remote=https%3A%2F%2Fhub%2Fucan%2F",
            "SEED",
            "did:key:zSpace",
        );

        let parsed = url::Url::parse(&url).expect("invite URL parses");
        assert_eq!(
            parsed.origin().ascii_serialization(),
            "https://tonk.example"
        );
        assert_eq!(parsed.path(), "/join");
        assert_eq!(parsed.fragment(), Some("SEED"));
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| { key == "access" && value == "PROOF" })
        );
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| { key == "remote" && value == "https://hub/ucan/" })
        );
        assert!(parsed.query_pairs().any(|(key, value)| {
            key == tonk_analytics::launch::CHANNEL_PARAMETER && value == "reshare"
        }));
        assert!(parsed.query_pairs().any(|(key, value)| {
            key == tonk_analytics::launch::SPACE_PARAMETER
                && value == tonk_analytics::anonymize("did:key:zSpace")
        }));

        // The secret is the fragment, never the query — everything before
        // `#` is what a shortcut PUT would upload.
        let (sent, fragment) = url.split_once('#').expect("the seed must be a fragment");
        assert_eq!(fragment, "SEED");
        assert!(
            !sent.contains("SEED"),
            "the seed must not appear in the path or query: {sent}",
        );
    }

    /// A modern delegation carries its endpoint in signed meta, so the
    /// `remote` suffix is empty — and empty must append *nothing*:
    /// `Invite::parse_url` rejects an empty `remote=`. (A repo with no
    /// endpoint at all never reaches the URL builder — the share
    /// pipeline refuses it first.)
    #[dialog_common::test]
    async fn it_omits_the_remote_when_the_chain_carries_the_endpoint() {
        let url = super::long_invite_url(
            "https://tonk.example/join",
            "PROOF",
            "",
            "SEED",
            "did:key:zSpace",
        );
        assert!(url.starts_with("https://tonk.example/join?access=PROOF&"));
        assert!(url.ends_with("#SEED"));
        assert!(url.contains("tonk_channel=reshare"));
        assert!(url.contains("tonk_space="));
        assert!(!url.contains("remote="));
        assert!(
            !url.contains("name="),
            "the name rides in the chain meta, never as a loose parameter: {url}"
        );
    }
}

#[cfg(test)]
mod seed_tests {
    /// The attribute names a set of instructions writes — what a record
    /// test asserts over, since `Instruction` is not `Debug`.
    #[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
    fn attributes_of(facts: &[dialog_artifacts::Instruction]) -> Vec<String> {
        facts
            .iter()
            .map(|instruction| match instruction {
                dialog_artifacts::Instruction::Assert(artifact)
                | dialog_artifacts::Instruction::Replace(artifact)
                | dialog_artifacts::Instruction::Retract(artifact) => artifact.the.to_string(),
            })
            .collect()
    }

    /// A seed's identity is the hash of its bytes, so two devices
    /// installing the same seed derive the same entity and converge.
    #[test]
    fn it_identifies_a_seed_by_its_content() {
        let one = super::seed_version("concept!: &a\n");
        let same = super::seed_version("concept!: &a\n");
        let other = super::seed_version("concept!: &b\n");

        assert_eq!(one, same, "the same bytes give the same identity");
        assert_ne!(one, other, "different bytes give a different identity");
        assert!(
            one.parse::<dialog_artifacts::Entity>().is_ok(),
            "the identity must be a usable entity: {one}"
        );
    }

    /// The record is the seed's identity, where it came from, what it
    /// replaced, and the commit it landed in — nothing about what it
    /// installed, which that commit's history already carries.
    #[test]
    fn it_records_where_a_seed_came_from_and_where_it_landed() {
        let facts = super::seed_record_facts(
            "seed:v",
            "/library/core.yaml",
            super::SEED_NONE,
            super::SEED_NONE,
            "version-bytes",
        );
        let rendered = attributes_of(&facts).join(" ");

        assert!(rendered.contains("seed/version"), "{rendered}");
        assert!(rendered.contains("seed/prior"), "{rendered}");
        assert!(rendered.contains("seed/source"), "{rendered}");
        assert!(
            !rendered.contains("route"),
            "routes are read from the commit's history, not recorded: {rendered}"
        );
    }

    /// The record is TWO concepts on one entity: what the seed is
    /// (`seed/available`) and that this space runs it (`seed/installed`).
    ///
    /// A seed a check merely found asserts only the first, so a waiting
    /// update can never be mistaken for an installed one.
    #[test]
    fn it_splits_a_seed_record_into_identity_and_install() {
        let facts = super::seed_record_facts(
            "seed:v",
            "/library/core.yaml",
            super::SEED_NONE,
            super::SEED_NONE,
            "version-bytes",
        );

        let attributes: Vec<String> = facts
            .iter()
            .map(|instruction| match instruction {
                dialog_artifacts::Instruction::Assert(artifact)
                | dialog_artifacts::Instruction::Replace(artifact)
                | dialog_artifacts::Instruction::Retract(artifact) => artifact.the.to_string(),
            })
            .collect();

        // Identity half: true of any seed, installed or merely fetched.
        assert!(
            attributes.iter().any(|the| the.contains("seed/source")),
            "{attributes:?}"
        );
        // Install half: only ever true of a seed a space is running.
        assert!(
            attributes.iter().any(|the| the.contains("seed/version")),
            "{attributes:?}"
        );
        assert!(
            attributes.iter().any(|the| the.contains("seed/prior")),
            "{attributes:?}"
        );
    }

    /// The round-trip a seed record depends on: a version encodes and
    /// decodes exactly, so an upgrade can find the revision it must
    /// withdraw. The entity cannot do this — it is a one-way hash.
    #[test]
    fn it_round_trips_a_seed_revision() {
        use dialog_artifacts::history::{Edition, Origin, Version};

        let version = Version::new(Origin::from([7u8; 32]), Edition::new(3));

        let encoded = super::encode_seed_version(&version);
        let decoded = super::decode_seed_version(&encoded).expect("the encoding round-trips");

        assert_eq!(decoded, version);
        assert!(
            super::decode_seed_version("not-a-version").is_none(),
            "a value that is not a version decodes to nothing rather than a wrong one"
        );
    }

    /// An upgrade follows the space's OWN seed source, not the shipped
    /// one.
    ///
    /// A space on a custom seed must not be dragged onto `core.yaml` the
    /// next time it is opened — comparing against the shipped library
    /// unconditionally did exactly that.
    #[test]
    fn it_records_the_source_it_upgrades_from() {
        let facts = super::seed_record_facts(
            "seed:v",
            "/library/custom.yaml",
            "seed:prior",
            "seed:prior",
            "revision-bytes",
        );
        let rendered = attributes_of(&facts).join(" ");

        assert!(
            !facts.is_empty(),
            "the record carries the source it came from, so the next upgrade \
             re-fetches THAT: {rendered}"
        );
        assert!(
            rendered.contains("seed/prior"),
            "and the seed it replaced, so the chain is walkable: {rendered}"
        );
    }

    /// A check reports whether an update is waiting, without installing
    /// one.
    ///
    /// The three answers a view has to tell apart: a space already on the
    /// shipped seed, one with a newer seed waiting, and one whose seed
    /// predates the record — which cannot be upgraded at all, since
    /// nothing names its definitions to withdraw.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_reports_whether_an_update_is_waiting() {
        const LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "seed-check").await;
        let tonk = state.read().await;

        let subject: dialog_varsig::Did = {
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

        let replica = tonk_schema::Replica::new(tonk.profile.did(), subject.clone())
            .this()
            .clone();

        /// Why the last check on this device failed, if it did.
        async fn failure_of(
            tonk: &crate::worker::TonkState,
            replica: &dialog_artifacts::Entity,
        ) -> Option<String> {
            use dialog_query::{Output as _, Query, Term};

            let main = tonk
                .reactor
                .profile_repository()
                .branch(super::PROFILE_BRANCH)
                .acquire(&tonk.operator)
                .await
                .expect("profile main acquires");
            let rows: Vec<tonk_schema::ReplicaCheckFailure> = main
                .handle()
                .query()
                .select(Query::<tonk_schema::ReplicaCheckFailure> {
                    this: Term::from(replica.clone()),
                    failure: Term::var("failure"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("check failure query");
            rows.into_iter().next().map(|row| row.failure.0)
        }

        /// Whether a check is still marked in flight on this device.
        async fn checking(
            tonk: &crate::worker::TonkState,
            replica: &dialog_artifacts::Entity,
        ) -> bool {
            use dialog_query::{Output as _, Query, Term};

            let main = tonk
                .reactor
                .profile_repository()
                .branch(super::PROFILE_BRANCH)
                .acquire(&tonk.operator)
                .await
                .expect("profile main acquires");
            let rows: Vec<tonk_schema::ReplicaChecking> = main
                .handle()
                .query()
                .select(Query::<tonk_schema::ReplicaChecking> {
                    this: Term::from(replica.clone()),
                    checking: Term::var("checking"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .expect("checking query");
            !rows.is_empty()
        }

        let check: dialog_artifacts::Entity = "check:one".parse().expect("an entity");

        // No record: the space predates one. That is not a failure —
        // nothing names its definitions, so there is simply nothing to
        // offer, and no failure is recorded for it.
        super::check_seed_update(&tonk, &subject, check.clone())
            .await
            .expect("the check runs");
        assert_eq!(
            failure_of(&tonk, &replica).await,
            None,
            "a space with no seed record is not a failed check"
        );
        assert!(
            !checking(&tonk, &replica).await,
            "the in-flight marker is retracted once the check settles"
        );

        // Install the shipped seed, recording it the way creation does.
        crate::router::evaluate::evaluate_body_recording(
            &tonk,
            &key,
            "main",
            LIBRARY.to_string(),
            &|minted| {
                super::seed_record_facts(
                    &super::seed_version(LIBRARY),
                    super::STANDARD_LIBRARY_URL,
                    super::SEED_NONE,
                    super::SEED_NONE,
                    &super::encode_seed_version(minted),
                )
            },
        )
        .await
        .expect("the seed installs");

        // With a record but no served library, the check could not look.
        // That IS a failure, and it is recorded as one — distinct from
        // being up to date, so a view never claims a space is current
        // when it simply could not fetch. (The harness serves no assets.)
        super::check_seed_update(&tonk, &subject, check)
            .await
            .expect("the check runs");
        let failure = failure_of(&tonk, &replica)
            .await
            .expect("an unfetchable source records why");
        assert!(
            failure.contains("could not fetch"),
            "the failure says what went wrong rather than a bare case: {failure}"
        );
        assert!(
            !checking(&tonk, &replica).await,
            "a failed check still clears the in-flight marker"
        );
    }

    /// A seed record names the very commit that carries it.
    ///
    /// This is what makes an upgrade work: the record names the commit
    /// that installed the library, and an upgrade reads THAT commit's
    /// history to know what to withdraw. If the two ever diverged, the
    /// record would point at a revision that never existed and the next
    /// upgrade would withdraw nothing.
    ///
    /// The version is not predicted. The library's commit stages — minted
    /// but not published — so the version handed to the record is a fact
    /// about a commit that has already happened, and a single publish
    /// makes the library and its record visible together.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_records_the_commit_that_carries_it() {
        use dialog_query::{Output as _, Query, Term};

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "seed-self-named").await;
        let tonk = state.read().await;

        const LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");
        let seed = super::seed_version(LIBRARY);

        // Install the library with its record chained onto the same batch.
        crate::router::evaluate::evaluate_body_recording(
            &tonk,
            &key,
            "main",
            LIBRARY.to_owned(),
            &|minted| {
                super::seed_record_facts(
                    &seed,
                    super::STANDARD_LIBRARY_URL,
                    super::SEED_NONE,
                    super::SEED_NONE,
                    &super::encode_seed_version(minted),
                )
            },
        )
        .await
        .expect("the library seeds");

        // The recorded version must name a commit that really exists, and
        // whose history carries the library's own claims — that history is
        // what an upgrade inverts.
        let session = tonk
            .reactor
            .repository(&key)
            .branch(super::CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("the branch acquires");
        let installed: Vec<tonk_schema::SeedInstalled> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::SeedInstalled> {
                this: Term::var("this"),
                prior: Term::var("prior"),
                version: Term::var("version"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("the install record reads");
        let record = installed.into_iter().next().expect("the seed is recorded");
        assert_eq!(
            record.this.to_string(),
            seed,
            "the record is keyed on the hash of the bytes installed"
        );

        let version =
            super::decode_seed_version(&record.version.0).expect("the recorded version decodes");
        let routes = super::seed_routes(&tonk, &session, &record.version.0)
            .await
            .expect("the recorded commit has a history");
        assert!(
            !routes.is_empty(),
            "the recorded version names the commit that installed the \
             library, so its history lists the routes it wrote: {version:?}"
        );
    }

    /// An upgrade withdraws what the previous seed asserted and installs
    /// the new one, in a single commit.
    ///
    /// The hazard this pins: the two seeds overlap, and a retract and an
    /// assert of the SAME fact in one batch must keep it. If the erase won
    /// instead, every shared definition would vanish. And a fact only the
    /// old seed had must actually go, or a space accretes definitions
    /// forever.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_replaces_the_previous_seed_without_stranding_facts() {
        use dialog_query::{Output as _, Query, Term};

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "seed-upgrade").await;
        let tonk = state.read().await;

        // An "old seed": two routes, one of which the new seed keeps.
        let old = r#"route!: &probe/kept
  this: id:probe/kept
  path: "/kept"
  concept: tonk:blank

route!: &probe/dropped
  this: id:probe/dropped
  path: "/dropped"
  concept: tonk:blank
"#;
        let library = include_str!("../../../tonk-core/assets/library/core.yaml");
        let seeded = crate::router::evaluate::evaluate_body(
            &tonk,
            &key,
            "main",
            format!("{library}\n{old}"),
            true,
        )
        .await
        .expect("the old seed evaluates");
        let old_revision = super::encode_seed_version(
            &seeded
                .revision_after
                .expect("a committing seed has a revision")
                .version(),
        );

        // The "new seed": keeps one route, drops the other.
        let new = r#"route!: &probe/kept
  this: id:probe/kept
  path: "/kept"
  concept: tonk:blank
"#;
        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("main acquires");
        let retract = super::prior_seed_retractions(&tonk, &session, &old_revision)
            .await
            .expect("the prior seed's assertions are readable");
        assert!(
            !retract.is_empty(),
            "the old seed asserted something to withdraw"
        );

        crate::router::evaluate::evaluate_with_retractions(
            &tonk,
            &key,
            "main",
            format!("{library}\n{new}"),
            retract,
            &|_minted| Vec::new(),
        )
        .await
        .expect("the upgrade commits");

        let routes: Vec<tonk_schema::Route> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::Route> {
                this: Term::var("this"),
                path: Term::var("path"),
                concept: Term::var("concept"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("route query");
        let paths: Vec<String> = routes.into_iter().map(|route| route.path.0).collect();

        assert!(
            paths.iter().any(|path| path == "/kept"),
            "a definition both seeds carry survives the retract-then-assert: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path == "/dropped"),
            "a definition only the old seed had is withdrawn: {paths:?}"
        );
    }

    /// Both shipped libraries must EVALUATE against a real branch, not
    /// merely analyze.
    ///
    /// The shipped-libraries test in tonk-analyzer only analyzes them, and
    /// analysis passes documents that evaluation rejects — an unbound
    /// variable in a rule, a field the concept does not declare. Both have
    /// now shipped broken: a seed that fails takes space creation with it,
    /// and the creator is left on the Hub with no redirect and no space.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_evaluates_both_shipped_libraries() {
        const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");
        const PROFILE: &str = include_str!("../../../tonk-core/assets/library/profile.yaml");

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);

        for (name, library, url) in [
            ("core", CORE, super::STANDARD_LIBRARY_URL),
            ("profile", PROFILE, super::PROFILE_LIBRARY_URL),
        ] {
            let key = crate::router::tests::put_repo(&app, &format!("eval-{name}")).await;
            let tonk = state.read().await;

            let outcome = crate::router::evaluate::evaluate_body(
                &tonk,
                &key,
                "main",
                library.to_owned(),
                true,
            )
            .await;
            assert!(outcome.is_ok(), "{name}.yaml must evaluate: {outcome:?}");

            let outcome = outcome.expect("evaluated");
            let version = outcome
                .revision_after
                .expect("a committing seed has a revision")
                .version();
            let facts = super::seed_record_facts(
                &super::seed_version(library),
                url,
                super::SEED_NONE,
                super::SEED_NONE,
                &super::encode_seed_version(&version),
            );
            assert!(
                !facts.is_empty(),
                "{name}.yaml's seed record must produce facts"
            );
        }
    }

    /// Seed a branch the way creation does and read the components back.
    ///
    /// The end-to-end check the unit tests around it kept missing: the
    /// body can parse, the concept can analyze, the declaration can say
    /// `cardinality: many` — and a space can still hold ONE component,
    /// or none of a whole kind. Rules were invisible for exactly that
    /// reason: they carry no anchor, and a text scan had nothing to name
    /// them by.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_records_every_seeded_component_on_the_branch() {
        use dialog_query::{Output as _, Query, Term};

        const LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "seed-components").await;

        let tonk = state.read().await;

        // The record names the commit that installed the library, which
        // is what makes the seed's claims findable afterwards. The
        // library's commit stages, so that version is minted before the
        // record is written, and one publish makes both visible.
        crate::router::evaluate::evaluate_body_recording(
            &tonk,
            &key,
            "main",
            LIBRARY.to_owned(),
            &|minted| {
                super::seed_record_facts(
                    &super::seed_version(LIBRARY),
                    super::STANDARD_LIBRARY_URL,
                    super::SEED_NONE,
                    super::SEED_NONE,
                    &super::encode_seed_version(minted),
                )
            },
        )
        .await
        .expect("the library and its record commit together");

        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("main acquires");

        let seeds: Vec<tonk_schema::SeedInstalled> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::SeedInstalled> {
                this: Term::var("this"),
                prior: Term::var("prior"),
                version: Term::var("version"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("seed query");
        let seed = seeds.first().expect("the install is recorded");
        assert!(
            super::decode_seed_version(&seed.version.0).is_some(),
            "the record names a real commit: {}",
            seed.version.0
        );

        // The router reads which routes the seed installed from the same
        // revision, so nothing about them is recorded separately.
        let seeded = super::seed_routes(&tonk, &session, &seed.version.0.to_string())
            .await
            .expect("the seed's routes are readable");
        assert!(
            seeded.len() > 1,
            "the library must install several routes for this to mean anything: {seeded:?}"
        );
        let declared: Vec<tonk_schema::Route> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::Route> {
                this: Term::var("this"),
                path: Term::var("path"),
                concept: Term::var("concept"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("route query");
        for route in &declared {
            assert!(
                seeded.contains(&route.this.to_string()),
                "every route the library installed is attributed to the seed: {route:?}"
            );
        }
    }
}

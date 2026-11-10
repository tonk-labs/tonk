//! A new agent and its first session, for the OAuth approval page.
//!
//! The page asks for an agent named after the app and the spaces it may
//! reach, read or read and write. The worker checks the ask, then has the
//! page ask for the passkey: granting the account's spaces needs the
//! account. With the account lent for it (see [`authorized`]) the worker
//! creates the agent's space as any space is created, with a key its host
//! makes and holds, which is the agent's key. Each chosen space is
//! delegated to the agent by the account (`space → account → agent`), and
//! the chains and a directory entry per space are kept in the agent space.
//! The agent's key, through its host, signs a fresh session: the token. The
//! page's top document hands the token to the authorization server, and
//! the page only ever sees where to go next.
//!
//! An app may instead connect an agent it made before, when the agent is
//! for the same provider's apps. The agent's key is kept for this device's
//! session and for the account (see [`open_agent`]), and the agent signs
//! the new session as it did the first.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use dialog_credentials::Signer;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use dialog_varsig::Principal as _;
use serde::Deserialize;
use tonk_invite::connection::AgentAccess;
#[cfg(any(test, target_arch = "wasm32"))]
use tonk_invite::connection::DEFAULT_GRANT_TTL_SECONDS;
use tonk_invite::connection::agent_grant_scopes;
#[cfg(target_arch = "wasm32")]
use tonk_kernel_world::world::sapling::kernel::credential::PersonaAuthority;
use tonk_macros::handler;
use tonk_worker_api::{AgentAuthorization, AgentSpaceChoice};
#[cfg(any(test, target_arch = "wasm32"))]
use zeroize::Zeroizing;

use super::agent_connections::issuer_ancestors;
#[cfg(any(test, target_arch = "wasm32"))]
use super::agent_space::Mark;
use super::agent_space::{self, AgentSpaceChange, DirectoryEntry};
use super::{
    AppState,
    agent_connections::failure,
    create_invite::{RemoteRequirement, resolve_remote_url},
};
use crate::{TonkWorkerError, axum::RequestOrigin};

/// Longest agent name the page may ask for.
const MAX_NAME: usize = 80;

/// What the approval page asks for.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionRequest {
    /// The agent's name: the agent space's label. Unused when `agent`
    /// names one already made.
    #[serde(default)]
    name: String,
    /// The routing key of an agent of this account to connect again,
    /// rather than making a new one.
    #[serde(default)]
    agent: Option<String>,
    /// The spaces the agent may reach, by routing key.
    #[serde(default)]
    spaces: Vec<AgentSpaceChoice>,
    /// The pending OAuth authorization this answers.
    request: String,
}

/// The origin each pending approval's authorization server is on, kept
/// here between the ask and the passkey rather than trusted from the
/// round trip through the page.
fn pending() -> &'static Mutex<HashMap<String, url::Url>> {
    static PENDING: OnceLock<Mutex<HashMap<String, url::Url>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Elapsed times of named steps since a start, for one log line:
/// `mounted 3 ms, loaded 12 ms (+9)`.
struct Laps {
    started: f64,
    last: f64,
    said: String,
}

impl Laps {
    fn start() -> Self {
        let now = tonk_common::diagnostics::now_ms();
        Self {
            started: now,
            last: now,
            said: String::new(),
        }
    }

    /// Record `step` as done now.
    fn lap(&mut self, step: &str) {
        let now = tonk_common::diagnostics::now_ms();
        if !self.said.is_empty() {
            self.said.push_str(", ");
        }
        self.said.push_str(&format!(
            "{step} {:.0} ms (+{:.0})",
            now - self.started,
            now - self.last
        ));
        self.last = now;
    }
}

impl std::fmt::Display for Laps {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.said)
    }
}

fn access(choice: &AgentSpaceChoice) -> Result<AgentAccess, TonkWorkerError> {
    match choice.access.as_str() {
        "read" => Ok(AgentAccess::Read),
        "write" => Ok(AgentAccess::ReadWrite),
        other => Err(TonkWorkerError::Router(format!(
            "unknown access `{other}`; choose read or write"
        ))),
    }
}

/// Which LLM provider an app belongs to, so an agent is never offered to
/// another provider's app: by its callback's domain, or, for a callback
/// on this computer (Claude Code), by the name it registered.
pub(crate) fn provider(redirect_uri: &str, client_name: &str) -> Option<&'static str> {
    let host = url::Url::parse(redirect_uri)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if under("claude.ai") || under("claude.com") || under("anthropic.com") {
        return Some("claude");
    }
    if under("chatgpt.com") || under("openai.com") {
        return Some("chatgpt");
    }
    let loopback = host == "localhost" || host == "[::1]" || host.starts_with("127.");
    let name = client_name.to_ascii_lowercase();
    if loopback && name.contains("claude") {
        return Some("claude");
    }
    if loopback && (name.contains("chatgpt") || name.contains("codex") || name.contains("openai")) {
        return Some("chatgpt");
    }
    None
}

/// A chosen space, checked: it exists here and syncs somewhere a hosted
/// agent can reach.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Target {
    subject: Did,
    access: AgentAccess,
    remote: url::Url,
    label: String,
}

/// Check the ask before anything is created, so a refusal leaves no agent
/// space behind.
async fn targets(
    state: &AppState,
    name: Option<&str>,
    spaces: &[AgentSpaceChoice],
) -> Result<Vec<Target>, TonkWorkerError> {
    if name.is_some_and(|name| name.trim().is_empty() || name.trim().chars().count() > MAX_NAME) {
        return Err(TonkWorkerError::Router(format!(
            "an agent needs a name of 1 to {MAX_NAME} characters"
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !spaces.iter().all(|space| seen.insert(&space.key)) {
        return Err(TonkWorkerError::Router(
            "each space may be chosen once".into(),
        ));
    }
    // A clone, not a guard: a first-use mount below can pull over the
    // network.
    let tonk = state.read().await.clone();
    if super::account::provider(&tonk).await.is_none() {
        return Err(TonkWorkerError::Forbidden(
            "sign in before connecting an agent".into(),
        ));
    }
    let mut targets = Vec::new();
    for space in spaces {
        let access = access(space)?;
        // How long each check of this space took, in one log line, to see
        // which one an approval waits on.
        let mut clock = Laps::start();
        // A space listed in the account directory but not yet on this
        // browser is mounted now, as opening it would.
        super::adopt::mount_on_first_use(&tonk, &space.key).await;
        clock.lap("mounted");
        let repository = super::repository::space_named(&tonk, &space.key)
            .await
            .map_err(|_| TonkWorkerError::NotFound(format!("no space {}", space.key)))?;
        clock.lap("loaded");
        let remote = match resolve_remote_url(&tonk, &repository).await? {
            RemoteRequirement::Ready(remote) => remote.access_url,
            RemoteRequirement::Refused(_) => {
                return Err(TonkWorkerError::Conflict(format!(
                    "space {} is not synced, so a hosted agent cannot reach it",
                    space.key
                )));
            }
        };
        clock.lap("remote");
        // An agent reaches other agents only as contacts, never as spaces.
        if super::agents::is_agent(&tonk, &repository.did()).await {
            return Err(TonkWorkerError::Forbidden(format!(
                "{} is an agent's own space, not one to grant",
                space.key
            )));
        }
        let label = super::repository::repository_display_name(&tonk, &repository, &space.key)
            .await
            .unwrap_or_else(|| space.key.clone());
        clock.lap("named");
        tonk_common::log!("approval space {}: {}", space.key, clock);
        targets.push(Target {
            subject: repository.did(),
            access,
            remote,
            label,
        });
    }
    Ok(targets)
}

/// An agent of this account that an app may connect again.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Reusable {
    did: Did,
    key: String,
    name: Option<String>,
}

/// The agents an app of `provider` may connect again: those made for the
/// same provider's apps, so Claude and ChatGPT never share one, whose
/// kept key this profile records.
async fn reusable(
    tonk: &crate::TonkState,
    provider: &str,
) -> Result<Vec<Reusable>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};
    let mut agents = Vec::new();
    for did in super::agents::spaces(tonk).await? {
        if !super::agents::is_agent(tonk, &did).await {
            continue;
        }
        let key = tonk_schema::prelude::DidExt::repo_key(&did).to_owned();
        // Where the agent space has a worker of its own, the profile keeps
        // the provider beside its mark of the agent (see [`mark`]).
        let held = if tonk.spaces_elsewhere() {
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await
        } else {
            tonk.reactor
                .repository(&key)
                .branch(super::repository::CONTENT_BRANCH)
                .acquire(&tonk.operator)
                .await
        };
        let Ok(session) = held else {
            continue;
        };
        let this: dialog_artifacts::Entity = did.to_string().parse().map_err(failure)?;
        let providers = session
            .handle()
            .query()
            .select(Query::<tonk_schema::agent::AgentProvider> {
                this: Term::from(this.clone()),
                provider: Term::var("provider"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(failure)?;
        if !providers.iter().any(|row| row.provider.0 == provider) {
            continue;
        }
        let held =
            dialog_repository::secrets::held_principal(tonk.profile.state(), &did, &tonk.profile)
                .await
                .map_err(failure)?;
        if held.is_none() {
            continue;
        }
        agents.push(Reusable {
            name: super::agents::name(tonk, &did).await,
            did,
            key,
        });
    }
    Ok(agents)
}

/// `POST /api/agent/session`: check the ask and have the page that made
/// it ask for the passkey; the work happens in [`authorized`]. Answers
/// `{"pending": true}`; how it goes is the `authorize-agent` ceremony
/// status, and the page is sent on to the app when it is done.
#[handler]
pub async fn create(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    client: Option<Extension<super::ClientId>>,
    Json(request): Json<AgentSessionRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    // Each step's time since the click, in the log, to see what the
    // passkey prompt is kept waiting on.
    let mut clock = Laps::start();
    // A lapsed request fails before the passkey is asked for.
    let described = describe_request(&origin, &request.request).await?;
    clock.lap("request described");
    let text = |key: &str| described.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let provider = provider(text("redirect_uri"), text("client_name"));
    if let Some(agent) = &request.agent {
        let tonk = state.read().await;
        let offered = match provider {
            Some(provider) => reusable(&tonk, provider).await?,
            None => Vec::new(),
        };
        if !offered.iter().any(|offered| &offered.key == agent) {
            return Err(TonkWorkerError::Forbidden(
                "that agent cannot be connected to this app; make a new one".into(),
            ));
        }
        clock.lap("agent checked");
    }
    let name = request.agent.is_none().then_some(request.name.as_str());
    targets(&state, name, &request.spaces).await?;
    clock.lap("targets checked");
    let Some(Extension(client)) = client else {
        return Err(TonkWorkerError::Conflict(
            "no page asked for this, so no passkey can be asked for".into(),
        ));
    };
    let intent = AgentAuthorization {
        request: request.request.clone(),
        name: request.name.trim().to_owned(),
        agent: request.agent,
        provider: provider.map(str::to_owned),
        spaces: request.spaces,
    };
    pending()
        .lock()
        .map_err(failure)?
        .insert(request.request, origin.url().clone());
    {
        let tonk = state.read().await;
        super::ceremony::report(
            &tonk,
            tonk_schema::ceremony::AUTHORIZE_AGENT,
            tonk_schema::ceremony_state::PENDING_CEREMONY,
            "",
        )
        .await;
    }
    clock.lap("pending reported");
    // Any passkey of this account will do: the account it opens is checked
    // against the signed-in one before anything is signed.
    ask_for_passkey(
        &client,
        tonk_schema::ceremony::AUTHORIZE_AGENT,
        tonk_worker_api::CustodyIntent::AuthorizeAgent(intent),
    )
    .await?;
    clock.lap("passkey asked");
    tonk_common::log!("approval ask: {clock}");
    Ok(Json(serde_json::json!({ "pending": true })))
}

/// Have the page `client` ask for the passkey `intent` needs, reporting as
/// `ceremony`. On a kernel the permanent persona is asked to authorize it,
/// and the embedder asks the page; a host with no page refuses.
pub(super) async fn ask_for_passkey(
    client: &super::ClientId,
    ceremony: &str,
    intent: tonk_worker_api::CustodyIntent,
) -> Result<(), TonkWorkerError> {
    #[cfg(target_arch = "wasm32")]
    {
        super::authorize::ask(Some(client), ceremony, intent).map_err(TonkWorkerError::Conflict)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ceremony;
        super::navigate::request_webauthn_with(
            client,
            tonk_worker_api::WebAuthnKind::Custody,
            Some(intent),
            None,
        )
        .await
    }
}

/// Make the agent the page asked for, as the account lent as `persona`,
/// answer the app's request, and send the page that asked to the app's
/// callback, reporting how it went.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn authorized(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: AgentAuthorization,
    client: Option<&super::ClientId>,
) -> Result<(), String> {
    let outcome = authorize_inner(state, persona, &authorization).await;
    let tonk = state.read().await.clone();
    let (status, detail) = match &outcome {
        // Done names the request it answered, so the approval page tells
        // its own approval's end from an earlier one's.
        Ok(_) => (
            tonk_schema::ceremony_state::DONE,
            authorization.request.clone(),
        ),
        // A refusal's code is for the handoff reply; the page shows the
        // words after it.
        Err(error) => (
            tonk_schema::ceremony_state::FAILED,
            error
                .split_once('\u{1f}')
                .map_or(error.as_str(), |(_, message)| message)
                .to_owned(),
        ),
    };
    super::ceremony::report(
        &tonk,
        tonk_schema::ceremony::AUTHORIZE_AGENT,
        status,
        &detail,
    )
    .await;
    if let Ok(answer) = &outcome {
        super::navigate::notify_replace(client, answer);
    }
    outcome.map(|_| ())
}

/// Report the phase an approval is in, so the page that asked shows how
/// far it got: `agent` (the account is open; the agent is being made or
/// reopened), `access` (its grants are being signed) and `saving`
/// (`spaces` grants are being pushed to its space). The ceremony row is the
/// account's, so the detail names the request it is about.
#[cfg(target_arch = "wasm32")]
async fn progress(
    state: &AppState,
    authorization: &AgentAuthorization,
    phase: &str,
    spaces: usize,
) {
    let detail = serde_json::json!({
        "request": authorization.request,
        "phase": phase,
        "spaces": spaces,
    });
    let tonk = state.read().await.clone();
    super::ceremony::report(
        &tonk,
        tonk_schema::ceremony::AUTHORIZE_AGENT,
        tonk_schema::ceremony_state::WORKING,
        &detail.to_string(),
    )
    .await;
}

/// Make or reopen the agent, sign its grants as the account, keep them in
/// its space and push it, then sign the agent's session: answers where the
/// page that asked goes with the app's answer.
#[cfg(target_arch = "wasm32")]
async fn authorize_inner(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: &AgentAuthorization,
) -> Result<String, String> {
    // Each step's time since the passkey, in the log, to see what the
    // app is kept waiting on.
    let mut clock = Laps::start();
    let said = |error: TonkWorkerError| error.to_string();
    let origin = pending()
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&authorization.request)
        .ok_or("this approval was not started here; start connecting again from your app")?;
    let origin = RequestOrigin::parse(origin.as_str())
        .map_err(|_| "the approval's origin is unreadable".to_string())?;
    let root: Did = persona
        .did()
        .parse()
        .map_err(|_| "the account names no DID".to_string())?;
    let linked = {
        let tonk = state.read().await.clone();
        super::identity::local_root(&tonk)
            .await
            .map_err(|error| format!("no account is signed in on this profile: {error}"))?
            .root_did
    };
    if root != linked {
        return Err("this passkey belongs to a different account".into());
    }
    progress(state, authorization, "agent", 0).await;
    clock.lap("account opened");
    let name = authorization
        .agent
        .is_none()
        .then_some(authorization.name.as_str());
    let targets = targets(state, name, &authorization.spaces)
        .await
        .map_err(said)?;
    clock.lap("targets checked");

    let (agent, key) = match &authorization.agent {
        Some(key) => reopen(state, persona, authorization, key).await?,
        None => create_agent(state, persona, authorization).await?,
    };
    clock.lap("agent space made");
    let agent_did = agent.did();
    progress(state, authorization, "access", 0).await;

    let tonk = state.read().await.clone();
    let repository = super::repository::space_named(&tonk, &key)
        .await
        .map_err(said)?;
    let home = match resolve_remote_url(&tonk, &repository).await.map_err(said)? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => return Err("the agent space could not be synced".into()),
    };

    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(|error| error.to_string())?;
    let mut granted = Vec::new();
    for target in targets {
        let chains = grant_as(
            &tonk,
            persona,
            &root,
            &agent_did,
            &target.subject,
            target.access,
            &target.remote,
            now,
            expires,
        )
        .await
        .map_err(said)?;
        granted.push(Granted {
            subject: target.subject,
            name: target.label,
            home: target.remote,
            chains,
        });
    }
    record_grants(&tonk, &key, &granted).await.map_err(said)?;
    clock.lap("grants recorded");
    progress(state, authorization, "saving", granted.len()).await;
    // The host reads the grants from the agent space's remote, and the page
    // goes to the app's callback as soon as this answers: push now.
    agent_space::push(state, &key)
        .await
        .map_err(|error| format!("the agent's grants could not be saved: {error}"))?;
    clock.lap("grants pushed");

    // The token is the session alone: what the agent may reach is in its
    // own space, where the host reads it like any other space's content.
    let issued = tonk_identity::agent::issue_agent_session(agent, &home)
        .await
        .map_err(|error| error.to_string())?;
    tonk_common::log!(
        "agent {agent_did} {} with {} new space grant(s); approval: {clock}",
        if authorization.agent.is_some() {
            "reconnected"
        } else {
            "created"
        },
        granted.len()
    );
    answered(
        &origin,
        &authorization.request,
        serde_json::json!({ "token": issued.token }),
    )
    .map_err(said)
}

/// Make a new agent space for `authorization`: created as any space is,
/// with a key its host makes and holds, and marked as an agent's. Answers
/// the agent, as a signer its host signs for, and the space's routing key.
#[cfg(target_arch = "wasm32")]
async fn create_agent(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: &AgentAuthorization,
) -> Result<(Signer, String), String> {
    let said = |error: TonkWorkerError| error.to_string();
    // The host makes the space's key, so the seed drawn here goes unused.
    let key = super::repository::create_space_from_seed(
        state,
        authorization.name.trim(),
        None,
        Zeroizing::new([0u8; 32]),
    )
    .await
    .map_err(|error| error.to_string())?;
    let agent: Did = key
        .parse()
        .map_err(|_| format!("the agent space {key} names no DID"))?;
    // It syncs where the account's spaces do, which is where the hosted
    // agent reads it.
    let remote = {
        let tonk = state.read().await.clone();
        super::repository::account_sync_remote(&tonk).await
    }
    .ok_or("this account has no sync endpoint")?;
    super::repository::attach_provisioned_remote(state, &key, &remote)
        .await
        .map_err(|error| error.to_string())?;
    let tonk = state.read().await.clone();
    let signer = open_agent(&tonk, persona, &agent).await?;
    mark(&tonk, &key, &agent, authorization.provider.as_deref())
        .await
        .map_err(said)?;
    Ok((signer, key))
}

/// Open the key of the agent at `key`, which the page chose to connect
/// again, as the account lent as `persona`. Checked again here: the agent
/// is this account's, for the requesting app's provider.
#[cfg(target_arch = "wasm32")]
async fn reopen(
    state: &AppState,
    persona: &PersonaAuthority,
    authorization: &AgentAuthorization,
    key: &str,
) -> Result<(Signer, String), String> {
    let provider = authorization
        .provider
        .as_deref()
        .ok_or("this app's agents cannot be told apart, so none is connected again")?;
    let tonk = state.read().await.clone();
    let agent = reusable(&tonk, provider)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|agent| agent.key == key)
        .ok_or("that agent cannot be connected to this app")?;
    let signer = open_agent(&tonk, persona, &agent.did).await?;
    Ok((signer, agent.key))
}

/// The key of `agent`, restored where its host holds it: from the copy
/// this device's session keeps, or else from the account's, which the
/// account lent as `persona` restores. The key never leaves its host; the
/// signer answered signs through it.
#[cfg(target_arch = "wasm32")]
pub(super) async fn open_agent(
    tonk: &crate::TonkState,
    persona: &PersonaAuthority,
    agent: &Did,
) -> Result<Signer, String> {
    use std::rc::Rc;
    use tonk_kernel_world::world::sapling::kernel::credential::CredentialBackup;
    use tonk_kernel_world::{credentials, session};

    let state = tonk.profile.state();
    let session_copy =
        dialog_repository::secrets::keys_of(state, agent, &tonk.profile.did(), &tonk.profile)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .next();
    let restored = match session_copy {
        Some(secret) => session::credential()
            .restore(session::backup_to_credential(secret))
            .await
            .map_err(|error| format!("the agent's key was not restored: {error:?}"))?,
        None => {
            let held = dialog_repository::secrets::held_principal(state, agent, &tonk.profile)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("this profile keeps no record of that agent's key")?;
            if held.to.as_str() != persona.did() {
                return Err("that agent's key is held for another account".into());
            }
            persona
                .restore(CredentialBackup {
                    custodian: persona.did(),
                    secret: held.sealed,
                })
                .await
                .map_err(|_| "the agent's key did not open with this account".to_string())?
        }
    };
    let signer = credentials::signer(Rc::new(restored))
        .map_err(|error| format!("the agent's key is not a signer: {error:?}"))?
        .signer()
        .clone();
    if &signer.did() != agent {
        return Err("the agent's kept key is not its own".into());
    }
    Ok(signer)
}

/// Mark the agent space as one, and note which provider's apps it is for:
/// in its own space, and on the profile, where the palette names it and an
/// app is offered it again.
#[cfg(any(test, target_arch = "wasm32"))]
pub(super) async fn mark(
    tonk: &crate::TonkState,
    key: &str,
    agent: &Did,
    provider: Option<&str>,
) -> Result<(), TonkWorkerError> {
    agent_space::change(
        tonk,
        key,
        &AgentSpaceChange {
            mark: Some(Mark {
                provider: provider.map(str::to_owned),
            }),
            ..AgentSpaceChange::default()
        },
    )
    .await?;
    super::agents::mark_on_profile(tonk, std::slice::from_ref(agent), provider).await
}

/// One space granted to an agent, as its space records it.
pub(super) struct Granted {
    pub(super) subject: Did,
    pub(super) name: String,
    pub(super) home: url::Url,
    pub(super) chains: Vec<dialog_ucan_core::DelegationChain>,
}

/// Record in the agent space what it may reach, the way the account
/// records its own spaces: each grant's chains retained into the agent
/// space's `main`, where pulling that branch brings them along, and a
/// directory entry per space (`xyz.tonk.space/name` and
/// `xyz.tonk.space/home-address` on the space's entity). What the agent
/// may do there is read off the chains themselves.
pub(super) async fn record_grants(
    tonk: &crate::TonkState,
    key: &str,
    granted: &[Granted],
) -> Result<(), TonkWorkerError> {
    if granted.is_empty() {
        return Ok(());
    }
    let mut change = AgentSpaceChange::default();
    for space in granted {
        for chain in &space.chains {
            change.retain.push(agent_space::encode(chain)?);
        }
        change.directory.push(DirectoryEntry {
            space: space.subject.clone(),
            name: space.name.clone(),
            home: space.home.to_string(),
        });
    }
    agent_space::change(tonk, key, &change).await
}

/// Where the authorization server keeps a pending request.
fn authorize_url(origin: &RequestOrigin, handle: &str) -> Result<url::Url, TonkWorkerError> {
    if handle.is_empty()
        || !handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(TonkWorkerError::Router(
            "malformed authorization request".into(),
        ));
    }
    origin
        .url()
        .join(&format!("authorize/{handle}"))
        .map_err(failure)
}

/// An authorization server refusal, worded for the page. A request that
/// is no longer pending has most likely expired while signing in.
fn refused(error: super::http::HttpError) -> TonkWorkerError {
    use super::http::HttpError;
    match error {
        HttpError::Upstream(failure) if failure.status == 404 || failure.status == 410 => {
            TonkWorkerError::NotFound(
                "this connection request has expired. Start connecting again from your app."
                    .into(),
            )
        }
        HttpError::Upstream(failure) if failure.status == 403 => TonkWorkerError::Forbidden(
            "this connection was started in another browser. Start connecting again from your app here."
                .into(),
        ),
        error => TonkWorkerError::Internal(format!("the authorization server: {error}")),
    }
}

async fn describe_request(
    origin: &RequestOrigin,
    handle: &str,
) -> Result<serde_json::Value, TonkWorkerError> {
    let response = super::http::get(&authorize_url(origin, handle)?)
        .await
        .map_err(refused)?;
    serde_json::from_slice(&response.body).map_err(failure)
}

/// Where the page goes to answer the pending request `handle` with
/// `decision`: the authorization server's own address for it, on the app's
/// origin, with the decision in its fragment, which never reaches a server.
/// The top page posts it from that origin, with the cookie that binds the
/// request to this browser, and goes where the server says (see
/// `tonk_host::answer_authorization`). A worker serving an origin of its own
/// learns the app's from its host; elsewhere the request came to the app's.
fn answered(
    origin: &RequestOrigin,
    handle: &str,
    decision: serde_json::Value,
) -> Result<String, TonkWorkerError> {
    let app = super::repository::app_origin()
        .and_then(|app| RequestOrigin::parse(&app).ok())
        .unwrap_or_else(|| origin.clone());
    let mut address = authorize_url(&app, handle)?;
    let decision = serde_json::to_string(&decision).map_err(failure)?;
    address.set_fragment(Some(&format!("answer={decision}")));
    Ok(address.into())
}

/// `GET /api/agent/request/{handle}`: who is asking, for the approval
/// page, which cannot reach the authorization server from its frame,
/// with `agents`, the agents of this account that app may connect again
/// (`{key, name}` each).
#[handler]
pub async fn request(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    Path(handle): Path<String>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let mut described = describe_request(&origin, &handle).await?;
    let text = |key: &str| described.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let tonk = state.read().await;
    let agents = match provider(text("redirect_uri"), text("client_name")) {
        Some(provider) => reusable(&tonk, provider).await?,
        None => Vec::new(),
    };
    let spaces = grantable(&tonk).await?;
    drop(tonk);
    if let Some(object) = described.as_object_mut() {
        object.insert(
            "agents".into(),
            agents
                .into_iter()
                .map(|agent| serde_json::json!({ "key": agent.key, "name": agent.name }))
                .collect(),
        );
        object.insert("spaces".into(), spaces.into());
    }
    Ok(Json(described))
}

/// The spaces the page may offer, from the account directory rather than
/// what this browser has mounted: a browser that just signed in has the
/// account's spaces listed but none mounted yet, and a chosen one is
/// mounted when the approval is checked. Each is `{key, name, synced}`;
/// `synced` is false for one recorded local-only, which a hosted agent
/// cannot reach. This browser's agents are left out; an agent made in
/// another browser is only known once its space is mounted, so the
/// approval refuses it then.
async fn grantable(tonk: &crate::TonkState) -> Result<Vec<serde_json::Value>, TonkWorkerError> {
    let account = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows = tonk_schema::directory::spaces(account.handle(), &tonk.operator)
        .await
        .map_err(failure)?;
    let mut spaces = Vec::new();
    for row in rows {
        if super::agents::is_agent(tonk, &row.subject).await {
            continue;
        }
        let key = tonk_schema::prelude::DidExt::repo_key(&row.subject).to_owned();
        spaces.push(serde_json::json!({
            "key": key,
            "name": row.name,
            "synced": row.mountable,
        }));
    }
    Ok(spaces)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenyRequest {
    request: String,
}

/// `POST /api/agent/deny`: refuse a pending request; answers
/// `{"redirect": ...}`, where the page goes to send the refusal.
#[handler]
pub async fn deny(
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<DenyRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let redirect = answered(
        &origin,
        &request.request,
        serde_json::json!({ "deny": true }),
    )?;
    Ok(Json(serde_json::json!({ "redirect": redirect })))
}

/// The chains delegating `subject` to `agent` at `access`, one per
/// scope, signed by `issuer`: the account's own proof for the space, cut
/// where it reaches `issuer`, with a leaf from `issuer` to the agent
/// space. Signed by the account, a grant runs `space → account → agent`;
/// signed by this device, as `space/invite-agent` signs it, it runs
/// `space → … → device → agent`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn grant(
    tonk: &crate::TonkState,
    issuer: &Signer,
    agent: &Did,
    subject: &Did,
    access: AgentAccess,
    remote: &url::Url,
    now: Timestamp,
    expires: Timestamp,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    let scopes = agent_grant_scopes(subject, access);
    let ancestors = ancestors(tonk, &issuer.did(), subject, &scopes, now, expires).await?;
    let bundle = super::agent_connections::issue_to_recipient(
        agent,
        issuer.clone(),
        ancestors,
        &scopes,
        remote,
        now,
        expires,
    )
    .await?;
    Ok(bundle.chains().to_vec())
}

/// [`grant`], signed by the account lent as `persona`, whose DID is `root`:
/// each leaf is the account's delegation to the agent, which its host
/// signs.
#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
async fn grant_as(
    tonk: &crate::TonkState,
    persona: &PersonaAuthority,
    root: &Did,
    agent: &Did,
    subject: &Did,
    access: AgentAccess,
    remote: &url::Url,
    now: Timestamp,
    expires: Timestamp,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    use tonk_kernel_world::world::sapling::kernel::credential::{Access, Argument};

    let encode = |key: &str, value: &ipld_core::ipld::Ipld| {
        serde_ipld_dagcbor::to_vec(value)
            .map(|value| Argument {
                key: key.to_owned(),
                value,
            })
            .map_err(failure)
    };
    let scopes = agent_grant_scopes(subject, access);
    let ancestors = ancestors(tonk, root, subject, &scopes, now, expires).await?;
    let meta = tonk_invite::home_address_meta(remote)
        .iter()
        .map(|(key, value)| encode(key, value))
        .collect::<Result<Vec<_>, _>>()?;
    let mut chains = Vec::new();
    for (ancestor, scope) in ancestors.into_iter().zip(&scopes) {
        let leaf = persona
            .delegate(
                agent.to_string(),
                Access {
                    subject: Some(subject.to_string()),
                    command: format!("/{}", scope.command.0.join("/")),
                    arguments: scope
                        .parameters
                        .0
                        .iter()
                        .map(|(key, value)| encode(key, value))
                        .collect::<Result<Vec<_>, _>>()?,
                    meta: meta.clone(),
                    not_before: None,
                    expiration: Some(expires.to_unix()),
                    reason: "to let an agent reach a space".to_owned(),
                },
            )
            .await
            .map_err(|error| failure(format!("the account did not delegate: {error:?}")))?;
        let leaf = dialog_ucan_core::DelegationChain::try_from(leaf.as_slice()).map_err(failure)?;
        let delegation = leaf
            .proofs()
            .last()
            .ok_or_else(|| failure("the account's delegation is empty"))?
            .clone();
        chains.push(ancestor.push(delegation).map_err(failure)?);
    }
    tonk_invite::connection::SpaceGrantBundle::validate(chains, agent, &scopes, remote, now)
        .await
        .map(|bundle| bundle.chains().to_vec())
        .map_err(failure)
}

/// The proof this profile holds for each of `scopes` of `subject`, cut
/// where it reaches `issuer`: what a grant `issuer` signs extends.
async fn ancestors(
    tonk: &crate::TonkState,
    issuer: &Did,
    subject: &Did,
    scopes: &[dialog_ucan::Scope],
    now: Timestamp,
    expires: Timestamp,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    let mut ancestors = Vec::new();
    for chain in issuer_ancestors(tonk, scopes, now, expires).await? {
        let proofs: Vec<_> = chain.proofs().cloned().collect();
        let reach = proofs
            .iter()
            .position(|link| link.audience() == issuer)
            .ok_or_else(|| {
                TonkWorkerError::Forbidden(format!("{issuer} holds no proof for {subject}"))
            })?;
        let mut kept = dialog_ucan_core::DelegationChain::new(proofs[0].clone());
        for link in &proofs[1..=reach] {
            kept = kept.push(link.clone()).map_err(failure)?;
        }
        ancestors.push(kept);
    }
    Ok(ancestors)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use dialog_repository::RepositoryExt as _;
    use std::sync::Arc;

    /// A token minted the way the approval page's request mints one is
    /// one the hosted front accepts, with the access that was chosen.
    #[dialog_common::test]
    async fn it_mints_grants_the_hosted_front_accepts() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("tonk-agent-session-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) =
            crate::device::open_profile_at("agent-session", location.clone()).await?;
        let registry = crate::device::Registry::new("agent-session".into(), location);
        let tonk =
            crate::worker::boot_state(storage, "agent-session".into(), profile, registry).await?;
        let state: AppState = Arc::new(crate::StateLock::new(tonk));

        let mut keys = Vec::new();
        for name in ["Notes", "Plans"] {
            keys.push(super::super::repository::create_space_inner(&state, name, None).await?);
        }
        let agent = Ed25519Signer::import(&[61; 32]).await?;
        let remote: url::Url = "https://sync.example.test/ucan/".parse()?;
        let now = Timestamp::now();
        let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;

        let tonk = state.read().await;
        let mut subjects = Vec::new();
        let mut grants = Vec::new();
        for (key, access) in keys.iter().zip([AgentAccess::Read, AgentAccess::ReadWrite]) {
            let subject = tonk
                .profile
                .space(key)
                .load()
                .perform(&tonk.operator)
                .await?
                .did();
            grants.extend(
                grant(
                    &tonk,
                    &tonk.profile.credential().signer().clone(),
                    &agent.did(),
                    &subject,
                    access,
                    &remote,
                    now,
                    expires,
                )
                .await?,
            );
            subjects.push(subject);
        }
        assert_eq!(grants.len(), 3 + 6);

        let issued = tonk_identity::agent::issue_agent_session_with_grants(
            Signer::from(agent.clone()),
            &remote,
            &grants,
        )
        .await?;
        let verified = super::super::hosted::mcp::verify(&issued.token)
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(verified.agent(), &agent.did());
        // What the agent space's writer classifies each space by: the
        // commands the chains to the agent space end in.
        let reaches = |subject: &Did, verb: &str| {
            grants.iter().any(|chain| {
                chain.subject() == Some(subject)
                    && chain.audience() == &agent.did()
                    && chain.proofs().last().is_some_and(|leaf| {
                        super::super::hosted::access::reaches(&leaf.command().0, verb)
                    })
            })
        };
        assert!(
            reaches(&subjects[0], "get") && !reaches(&subjects[0], "put"),
            "read only"
        );
        assert!(
            reaches(&subjects[1], "get") && reaches(&subjects[1], "put"),
            "read and write"
        );
        Ok(())
    }

    /// Approving two spaces leaves, in the agent space, both spaces'
    /// chains retained and a directory entry for each.
    #[dialog_common::test]
    async fn it_records_the_grants_in_the_agent_space() -> anyhow::Result<()> {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::prelude::DidExt as _;

        let directory =
            std::env::temp_dir().join(format!("tonk-agent-grants-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) =
            crate::device::open_profile_at("agent-grants", location.clone()).await?;
        let registry = crate::device::Registry::new("agent-grants".into(), location);
        let tonk =
            crate::worker::boot_state(storage, "agent-grants".into(), profile, registry).await?;
        let state: AppState = Arc::new(crate::StateLock::new(tonk));

        let mut keys = Vec::new();
        for name in ["Notes", "Plans"] {
            keys.push(super::super::repository::create_space_inner(&state, name, None).await?);
        }
        let seed = Zeroizing::new([62u8; 32]);
        let agent = Ed25519Signer::import(&*seed).await?.did();
        let agent_key =
            super::super::repository::create_space_from_seed(&state, "Claude", None, seed).await?;
        assert_eq!(agent.repo_key(), agent_key);

        let remote: url::Url = "https://sync.example.test/ucan/".parse()?;
        let now = Timestamp::now();
        let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
        let tonk = state.read().await;
        let mut granted = Vec::new();
        for ((key, name), access) in keys
            .iter()
            .zip(["Notes", "Plans"])
            .zip([AgentAccess::Read, AgentAccess::ReadWrite])
        {
            let subject = tonk
                .profile
                .space(key)
                .load()
                .perform(&tonk.operator)
                .await?
                .did();
            let chains = grant(
                &tonk,
                &tonk.profile.credential().signer().clone(),
                &agent,
                &subject,
                access,
                &remote,
                now,
                expires,
            )
            .await?;
            granted.push(Granted {
                subject,
                name: name.into(),
                home: remote.clone(),
                chains,
            });
        }
        record_grants(&tonk, &agent_key, &granted).await?;

        let branch = tonk
            .reactor
            .repository(&agent_key)
            .branch(super::super::repository::CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await?;
        let retained = branch
            .handle()
            .delegations()
            .issued_to(agent.clone())
            .perform(&tonk.operator)
            .await?;
        for space in &granted {
            let mine = retained
                .iter()
                .filter(|chain| chain.0.subject() == Some(&space.subject))
                .count();
            assert_eq!(mine, space.chains.len(), "chains for {}", space.name);
        }
        assert_eq!(retained.len(), 3 + 6);

        let names: Vec<tonk_schema::SpaceName> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceName> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        for space in &granted {
            assert!(
                names
                    .iter()
                    .any(|row| row.this == space.subject.this() && row.name.0 == space.name),
                "a directory entry for {} in {names:?}",
                space.name
            );
        }
        let homes: Vec<tonk_schema::SpaceHomeAddress> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceHomeAddress> {
                this: Term::var("this"),
                home_address: Term::var("home_address"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        assert_eq!(homes.len(), 2);

        // What the hosted front reads back from it: the access each grant
        // was given, the name, the home address and the grant's expiry.
        let access = super::super::hosted::access::read_agent_access(&tonk, &agent).await?;
        for (space, (read, write)) in granted.iter().zip([(true, false), (true, true)]) {
            let entry = access
                .get(&space.subject)
                .unwrap_or_else(|| panic!("{} is listed: {access:?}", space.name));
            assert_eq!((entry.read, entry.write), (read, write), "{}", space.name);
            assert_eq!(entry.name.as_deref(), Some(space.name.as_str()));
            assert_eq!(entry.address.as_deref(), Some(remote.as_str()));
            assert_eq!(entry.expires, Some(expires.to_unix()), "{}", space.name);
            assert_eq!(entry.lapsed, 0);
        }
        assert!(access.get(&agent).is_some_and(|own| own.agent));
        assert_eq!(access.spaces.len(), 3, "{access:?}");

        // What a view's delegation of a space extends: each grant whole,
        // from the space to the agent space, as the approval signed it.
        for space in &granted {
            let chains =
                super::super::hosted::access::space_chains(&tonk, &agent, &space.subject).await?;
            assert_eq!(chains.len(), space.chains.len(), "{}", space.name);
            for chain in &chains {
                assert_eq!(chain.issuer(), &space.subject, "rooted at {}", space.name);
                assert_eq!(chain.audience(), &agent);
                assert!(
                    space.chains.iter().any(|signed| signed.proofs().count()
                        == chain.proofs().count()
                        && signed
                            .proofs()
                            .zip(chain.proofs())
                            .all(|(a, b)| a.to_cid() == b.to_cid())),
                    "{} answers a chain the approval signed",
                    space.name
                );
            }
        }
        Ok(())
    }

    /// Claude's surfaces share Claude agents and ChatGPT gets its own;
    /// anything else is told apart by nothing, so it gets a new agent.
    #[test]
    fn it_tells_the_provider_of_an_app() {
        for (uri, name, expected) in [
            (
                "https://claude.ai/api/mcp/auth_callback",
                "Claude",
                Some("claude"),
            ),
            (
                "https://claude.com/oauth/callback",
                "Claude",
                Some("claude"),
            ),
            (
                "http://localhost:33418/callback",
                "Claude Code",
                Some("claude"),
            ),
            (
                "https://chatgpt.com/connector_platform_oauth_redirect",
                "ChatGPT",
                Some("chatgpt"),
            ),
            ("https://platform.openai.com/callback", "x", Some("chatgpt")),
            ("http://127.0.0.1:5000/cb", "Codex", Some("chatgpt")),
            ("http://localhost:5000/cb", "Inspector", None),
            ("https://evilclaude.ai/cb", "Claude", None),
            ("https://example.test/cb", "Claude", None),
        ] {
            assert_eq!(provider(uri, name), expected, "{uri} {name}");
        }
    }

    /// An app is offered again only the agents made for its provider whose
    /// key this profile keeps.
    #[dialog_common::test]
    async fn it_offers_an_app_only_the_agents_of_its_provider() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("tonk-agent-reuse-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) =
            crate::device::open_profile_at("agent-reuse", location.clone()).await?;
        let registry = crate::device::Registry::new("agent-reuse".into(), location);
        let tonk =
            crate::worker::boot_state(storage, "agent-reuse".into(), profile, registry).await?;
        // Signed in: the peer acts for an account whose key is in hand,
        // as it is once the passkey has opened it.
        let root = Ed25519Signer::generate().await?;
        let union = tonk_account::delegations::mint_account_union(
            &Signer::from(root.clone()),
            &tonk.profile.did(),
        )
        .await?;
        tonk_account::peer::hand_over(&tonk.profile, &union).await?;
        let state: AppState = Arc::new(crate::StateLock::new(tonk));

        let mut agents = Vec::new();
        for (name, byte, provider) in [
            ("Claude", 71u8, Some("claude")),
            ("ChatGPT", 72, Some("chatgpt")),
            ("Unknown", 73, None),
        ] {
            let seed = Zeroizing::new([byte; 32]);
            let did = Ed25519Signer::import(&*seed).await?.did();
            let key =
                super::super::repository::create_space_from_seed(&state, name, None, seed).await?;
            let tonk = state.read().await;
            mark(&tonk, &key, &did, provider).await?;
            agents.push((did, key));
        }

        let tonk = state.read().await;
        let offered = reusable(&tonk, "claude").await?;
        assert_eq!(
            offered.iter().map(|agent| &agent.key).collect::<Vec<_>>(),
            vec![&agents[0].1]
        );
        assert_eq!(offered[0].name.as_deref(), Some("Claude"));
        let offered = reusable(&tonk, "chatgpt").await?;
        assert_eq!(
            offered.iter().map(|agent| &agent.key).collect::<Vec<_>>(),
            vec![&agents[1].1]
        );
        assert!(reusable(&tonk, "other").await?.is_empty());
        Ok(())
    }
}

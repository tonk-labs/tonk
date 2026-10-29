//! A new agent and its first session, for the OAuth approval page.
//!
//! The page asks for an agent named after the app and the spaces it may
//! reach, read or read and write. The worker checks the ask, then has the
//! page ask for the passkey: granting the account's spaces needs the
//! account's own key. With it (see [`authorize`]) the worker draws the
//! agent space's key, has the agent space delegate to a fresh session
//! with it, and only then creates the space with that seed, which seals it
//! to the account like any other. Each chosen space is delegated to the
//! agent space by the account (`space → account → agent`), and the chains
//! and a directory entry per space are kept in the agent space. The token
//! is the session alone; the worker hands it to the authorization server
//! and the page only ever sees where to go next.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_repository::RepositoryExt as _;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::{Did, Principal as _};
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_handler_macros::wasm_compat;
use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS, agent_grant_scopes};
use tonk_worker_api::{AgentAuthorization, AgentSpaceChoice};
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
use zeroize::Zeroizing;

use super::{
    AppState,
    agent_connections::{failure, issuer_ancestors},
    create_invite::{RemoteRequirement, resolve_remote_url},
};
use crate::{TonkWorkerError, axum::RequestOrigin};

/// Longest agent name the page may ask for.
const MAX_NAME: usize = 80;

/// What the approval page asks for.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionRequest {
    /// The agent's name: the agent space's label.
    name: String,
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
    name: &str,
    spaces: &[AgentSpaceChoice],
) -> Result<Vec<Target>, TonkWorkerError> {
    if name.trim().is_empty() || name.trim().chars().count() > MAX_NAME {
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
    let tonk = state.read().await;
    if super::account::provider(&tonk).await.is_none() {
        return Err(TonkWorkerError::Forbidden(
            "sign in before connecting an agent".into(),
        ));
    }
    let mut targets = Vec::new();
    for space in spaces {
        let access = access(space)?;
        let repository = tonk
            .profile
            .space(&space.key)
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|_| TonkWorkerError::NotFound(format!("no space {}", space.key)))?;
        let remote = match resolve_remote_url(&tonk, &repository).await? {
            RemoteRequirement::Ready(remote) => remote.access_url,
            RemoteRequirement::Refused(_) => {
                return Err(TonkWorkerError::Conflict(format!(
                    "space {} is not synced, so a hosted agent cannot reach it",
                    space.key
                )));
            }
        };
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
        targets.push(Target {
            subject: repository.did(),
            access,
            remote,
            label,
        });
    }
    Ok(targets)
}

/// `POST /api/agent/session`: check the ask and have the page that made
/// it ask for the passkey; the work happens in [`authorize`]. Answers
/// `{"pending": true}`; how it goes is the `authorize-agent` ceremony
/// status, and the page is sent on to the app when it is done.
#[wasm_compat]
pub async fn create(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    client: Option<Extension<super::ClientId>>,
    Json(request): Json<AgentSessionRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    // A lapsed request fails before the passkey is asked for.
    let described = describe_request(&origin, &request.request).await?;
    targets(&state, &request.name, &request.spaces).await?;
    let Some(Extension(client)) = client else {
        return Err(TonkWorkerError::Conflict(
            "no page asked for this, so no passkey can be asked for".into(),
        ));
    };
    let text = |key: &str| described.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let intent = AgentAuthorization {
        request: request.request.clone(),
        name: request.name.trim().to_owned(),
        provider: provider(text("redirect_uri"), text("client_name")).map(str::to_owned),
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
    // Any passkey of this account will do: the recovered root is checked
    // against the signed-in account before anything is signed.
    super::navigate::request_webauthn_with(
        &client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(tonk_worker_api::CustodyIntent::AuthorizeAgent(intent)),
        None,
    )
    .await?;
    Ok(Json(serde_json::json!({ "pending": true })))
}

/// Make the agent the page asked for, with the account the passkey just
/// opened, and answer the app's request. Returns the app's callback.
pub(crate) async fn authorize(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    authorization: AgentAuthorization,
) -> Result<String, String> {
    let outcome = authorize_inner(state, custodian, &authorization).await;
    let tonk = state.read().await;
    let (status, detail) = match &outcome {
        Ok(_) => (tonk_schema::ceremony_state::DONE, String::new()),
        Err(error) => (tonk_schema::ceremony_state::FAILED, error.clone()),
    };
    super::ceremony::report(
        &tonk,
        tonk_schema::ceremony::AUTHORIZE_AGENT,
        status,
        &detail,
    )
    .await;
    outcome
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn authorize_inner(
    _state: &AppState,
    _custodian: &tonk_identity::custodian::Custodian,
    _authorization: &AgentAuthorization,
) -> Result<String, String> {
    Err("an agent is approved in the browser".into())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn authorize_inner(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    authorization: &AgentAuthorization,
) -> Result<String, String> {
    let said = |error: TonkWorkerError| error.to_string();
    let origin = pending()
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&authorization.request)
        .ok_or("this approval was not started here; start connecting again from your app")?;
    let origin = RequestOrigin::parse(origin.as_str())
        .map_err(|_| "the approval's origin is unreadable".to_string())?;
    {
        let tonk = state.read().await;
        super::ceremony::report(
            &tonk,
            tonk_schema::ceremony::AUTHORIZE_AGENT,
            tonk_schema::ceremony_state::WORKING,
            "",
        )
        .await;
    }
    let account = super::custody::held_account(custodian).await?;
    let Signer::Ed25519(root) = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    let linked = {
        let tonk = state.read().await;
        super::identity::local_root(&tonk)
            .await
            .map_err(|error| format!("no account is signed in on this profile: {error}"))?
            .root_did
    };
    if root.did() != linked {
        return Err("this passkey belongs to a different account".into());
    }
    let targets = targets(state, &authorization.name, &authorization.spaces)
        .await
        .map_err(said)?;

    // The agent space's key: signs the session's powerline, then is
    // sealed to the account by creating the space with it.
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|error| error.to_string())?;
    let agent = Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let agent_did = agent.did();
    let key = super::repository::create_space_from_seed(
        state,
        authorization.name.trim(),
        Some("An agent's own space."),
        seed,
    )
    .await
    .map_err(|error| error.to_string())?;
    let remote = {
        let tonk = state.read().await;
        super::repository::account_sync_remote(&tonk).await
    }
    .ok_or("this account has no sync endpoint")?;
    super::repository::enable_sync_inner(state, &key, &remote)
        .await
        .map_err(|error| error.to_string())?;

    let tonk = state.read().await;
    let repository = tonk
        .profile
        .space(&key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| error.to_string())?;
    let home = match resolve_remote_url(&tonk, &repository).await.map_err(said)? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => return Err("the agent space could not be synced".into()),
    };
    mark(&tonk, &key, &agent_did, authorization.provider.as_deref())
        .await
        .map_err(said)?;

    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(|error| error.to_string())?;
    let signer = Signer::from(root.clone());
    let mut granted = Vec::new();
    for target in targets {
        let chains = grant(
            &tonk,
            &signer,
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

    // The token is the session alone: what the agent may reach is in its
    // own space, where the host reads it like any other space's content.
    let issued = tonk_identity::agent::issue_agent_session(Signer::from(agent), &home)
        .await
        .map_err(|error| error.to_string())?;
    drop(tonk);
    tonk_common::log!(
        "agent {agent_did} created with {} space grant(s)",
        granted.len()
    );
    answer(
        &origin,
        &authorization.request,
        serde_json::json!({ "token": issued.token }),
    )
    .await
    .map_err(said)
}

/// Mark the agent space as one, and note which provider's apps it is for.
async fn mark(
    tonk: &crate::TonkState,
    key: &str,
    agent: &Did,
    provider: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let this: dialog_artifacts::Entity = agent.to_string().parse().map_err(failure)?;
    let mut transaction = tonk
        .reactor
        .repository(key)
        .branch(super::repository::CONTENT_BRANCH)
        .transaction()
        .assert(tonk_schema::agent::AgentSpace::new(this.clone()));
    if let Some(provider) = provider {
        transaction = transaction.assert(tonk_schema::agent::AgentProvider::new(this, provider));
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(())
}

/// One space granted to an agent, as its space records it.
struct Granted {
    subject: Did,
    name: String,
    home: url::Url,
    chains: Vec<dialog_ucan_core::DelegationChain>,
}

/// Record in the agent space what it may reach, the way the account
/// records its own spaces: each grant's chains retained into the agent
/// space's `main`, where pulling that branch brings them along, and a
/// directory entry per space (`xyz.tonk.space/name` and
/// `xyz.tonk.space/home-address` on the space's entity). What the agent
/// may do there is read off the chains themselves.
async fn record_grants(
    tonk: &crate::TonkState,
    key: &str,
    granted: &[Granted],
) -> Result<(), TonkWorkerError> {
    if granted.is_empty() {
        return Ok(());
    }
    let branch = tonk
        .reactor
        .repository(key)
        .branch(super::repository::CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    for space in granted {
        for chain in &space.chains {
            tonk_account::delegations::retain_space_delegation(
                branch.handle(),
                chain,
                &tonk.operator,
            )
            .await
            .map_err(failure)?;
        }
    }
    let mut transaction = tonk
        .reactor
        .repository(key)
        .branch(super::repository::CONTENT_BRANCH)
        .transaction();
    for space in granted {
        transaction = transaction
            .assert(tonk_schema::SpaceName::new(&space.subject, &space.name))
            .assert(tonk_schema::SpaceHomeAddress::new(
                &space.subject,
                space.home.as_str(),
            ));
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(key, js_sys::Date::now());
    Ok(())
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

/// Answer a pending request and return the callback to send the browser
/// to. Sent from this worker, so it carries this origin and its cookie.
async fn answer(
    origin: &RequestOrigin,
    handle: &str,
    decision: serde_json::Value,
) -> Result<String, TonkWorkerError> {
    let body = serde_json::to_vec(&decision).map_err(failure)?;
    let response = super::http::post_json(&authorize_url(origin, handle)?, &body)
        .await
        .map_err(refused)?;
    #[derive(Deserialize)]
    struct Answer {
        redirect: String,
    }
    let answer: Answer = serde_json::from_slice(&response.body).map_err(failure)?;
    Ok(answer.redirect)
}

/// `GET /api/agent/request/{handle}`: who is asking, for the approval
/// page, which cannot reach the authorization server from its frame.
#[wasm_compat]
pub async fn request(
    Extension(origin): Extension<RequestOrigin>,
    Path(handle): Path<String>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    Ok(Json(describe_request(&origin, &handle).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenyRequest {
    request: String,
}

/// `POST /api/agent/deny`: refuse a pending request; answers
/// `{"redirect": ...}`.
#[wasm_compat]
pub async fn deny(
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<DenyRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let redirect = answer(
        &origin,
        &request.request,
        serde_json::json!({ "deny": true }),
    )
    .await?;
    Ok(Json(serde_json::json!({ "redirect": redirect })))
}

/// The chains delegating `subject` to `agent` at `access`, one per
/// scope, signed by `issuer`: the account's own proof for the space, cut
/// where it reaches `issuer`, with a leaf from `issuer` to the agent
/// space. Signed by the account, a grant runs `space → account → agent`.
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
    let mut ancestors = Vec::new();
    for chain in issuer_ancestors(tonk, &scopes, now, expires).await? {
        let proofs: Vec<_> = chain.proofs().cloned().collect();
        let reach = proofs
            .iter()
            .position(|link| link.audience() == &issuer.did())
            .ok_or_else(|| {
                TonkWorkerError::Forbidden(format!("{} holds no proof for {subject}", issuer.did()))
            })?;
        let mut kept = dialog_ucan_core::DelegationChain::new(proofs[0].clone());
        for link in &proofs[1..=reach] {
            kept = kept.push(link.clone()).map_err(failure)?;
        }
        ancestors.push(kept);
    }
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_effects::storage::Directory;
    use std::sync::Arc;
    use tokio::sync::RwLock;

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
        let state: AppState = Arc::new(RwLock::new(tonk));

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
        let state: AppState = Arc::new(RwLock::new(tonk));

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
}

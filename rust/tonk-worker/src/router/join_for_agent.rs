//! One of the account's agents joins a space from an invite link the
//! person pasted into a chat with it.
//!
//! The person is in the agent's chain: the account claims the link
//! first, as any join does, then signs the agent's grant with its own
//! key, behind the passkey. The grant runs
//!
//! ```text
//! space -> ... -> link key -> account -> agent space
//! ```
//!
//! so "your agents" revokes it at the space's access service like any
//! grant the approval page made, and an admin who removes the person
//! from the space removes the person's agents with them.
//!
//! Two steps, because the link is a secret and the passkey request is
//! not the place for it:
//!
//! - `POST /api/agents/{key}/join` claims the link with this device's
//!   key, checks the account may grant what was asked, and has the page
//!   ask for the passkey with a [`CustodyIntent::GrantAgent`] that names
//!   the space by its routing key only.
//! - [`authorize`] runs when the passkey opened the account: the account
//!   signs the grant, and the agent space keeps it and is pushed, where
//!   the hosted agent reads it.
//!
//! [`CustodyIntent::GrantAgent`]: tonk_worker_api::CustodyIntent::GrantAgent

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use dialog_capability::{
    Subject,
    access::{Access, Prove},
};
use dialog_ucan::Ucan;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_handler_macros::wasm_compat;
use tonk_invite::connection::{DEFAULT_GRANT_TTL_SECONDS, agent_grant_scopes};
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::{AgentGrant, AgentSpaceChoice};

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
use super::agent_session::Target;
use super::{
    AppState,
    agent_connections::{failure, issuer_ancestors},
};
use crate::{TonkState, TonkWorkerError};

/// What the page asks for: the link and the access chosen for the agent.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentJoinRequest {
    /// The whole invite link, fragment and all.
    link: String,
    /// `read`, or `write` for read and write.
    access: String,
}

/// The answer: the space the account now holds, and that the passkey is
/// being asked for.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJoinAnswer {
    pending: bool,
    /// The space's routing key.
    space: String,
    /// The space's DID.
    subject: Did,
}

/// `POST /api/agents/{key}/join`: claim the link for the account, then
/// have the page that asked use the passkey; the grant is signed in
/// [`authorize`]. How it goes is the `grant-agent` ceremony status.
#[wasm_compat]
pub async fn join(
    State(state): State<AppState>,
    client: Option<Extension<super::ClientId>>,
    Path(key): Path<String>,
    Json(request): Json<AgentJoinRequest>,
) -> Result<Json<AgentJoinAnswer>, TonkWorkerError> {
    let Some(Extension(client)) = client else {
        return Err(TonkWorkerError::Conflict(
            "no page asked for this, so no passkey can be asked for".into(),
        ));
    };
    let (grant, subject) = claim(&state, &key, &request).await?;
    {
        let tonk = state.read().await;
        super::ceremony::report(
            &tonk,
            tonk_schema::ceremony::GRANT_AGENT,
            tonk_schema::ceremony_state::PENDING_CEREMONY,
            "",
        )
        .await;
    }
    super::navigate::request_webauthn_with(
        &client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(tonk_worker_api::CustodyIntent::GrantAgent(grant.clone())),
        None,
    )
    .await?;
    Ok(Json(AgentJoinAnswer {
        pending: true,
        space: grant.space.key,
        subject,
    }))
}

/// Check the agent and the access asked for, claim the link unless the
/// account holds the space already, and check the account can grant the
/// agent what was asked. Returns the grant for the passkey to sign.
pub(super) async fn claim(
    state: &AppState,
    agent_key: &str,
    request: &AgentJoinRequest,
) -> Result<(AgentGrant, Did), TonkWorkerError> {
    let agent = {
        let tonk = state.read().await;
        super::agents::find(&tonk, agent_key).await?
    };
    let access = super::agent_session::access_of(&request.access)?;
    let link = expand(request.link.trim()).await?;
    let invite = super::join::parse_invite(&link)
        .await
        .map_err(TonkWorkerError::from)?;
    let subject = invite.subject().clone();
    if subject == agent {
        return Err(TonkWorkerError::Forbidden(
            "that link opens the agent's own space".into(),
        ));
    }
    let held = {
        let tonk = state.read().await;
        super::join::find_replica_for_subject(&tonk, &subject).await?
    };
    // A member keeps the authority they hold: claiming again would renew
    // the membership, which retracts the hop it came in through.
    if !held {
        let tonk = state.write().await;
        let outcome = super::join::join_invite(&tonk, &link).await?;
        tonk_common::log!(
            "agent {agent}: the account joined {} to let it in",
            outcome.subject
        );
    }
    let choice = AgentSpaceChoice {
        key: subject.repo_key().to_owned(),
        access: request.access.clone(),
    };
    let target = super::agent_session::targets(state, None, std::slice::from_ref(&choice))
        .await?
        .pop()
        .ok_or_else(|| failure("no space to grant"))?;
    {
        let tonk = state.read().await;
        let now = Timestamp::now();
        let expires = deadline(&tonk, &target.subject, access, now).await?;
        issuer_ancestors(
            &tonk,
            &agent_grant_scopes(&target.subject, access),
            now,
            expires,
        )
        .await?;
    }
    Ok((
        AgentGrant {
            agent: agent_key.to_owned(),
            space: choice,
        },
        subject,
    ))
}

/// The long form of `link`: a shortened `/@/` link is followed to where
/// it lands, its fragment (the key) put back; any other is as given.
async fn expand(link: &str) -> Result<String, TonkWorkerError> {
    if !is_shortcut(link) {
        return Ok(link.to_owned());
    }
    super::join::resolve_shortcut(link).await.map_err(|error| {
        tonk_common::log!("agent join: the short link did not resolve: {error}");
        TonkWorkerError::Router("Could not open the shortened link. Ask for the full one.".into())
    })
}

/// Whether `link` is a shortened `/@/` invite link.
fn is_shortcut(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|parsed| parsed.path().starts_with("/@/"))
}

/// When the agent's grant lapses: in 90 days, or sooner when the
/// account's own authority for the space does, as a share link's does.
async fn deadline(
    tonk: &TonkState,
    subject: &Did,
    access: tonk_invite::connection::AgentAccess,
    now: Timestamp,
) -> Result<Timestamp, TonkWorkerError> {
    let mut until = now.to_unix() + DEFAULT_GRANT_TTL_SECONDS;
    for scope in agent_grant_scopes(subject, access) {
        let proof = Subject::from(tonk.profile.did())
            .attenuate(Access)
            .invoke(Prove::<Ucan>::new(tonk.profile.did(), scope))
            .perform(&tonk.operator)
            .await
            .map_err(|_| {
                TonkWorkerError::Forbidden(
                    "this account cannot grant the agent that access to the space".into(),
                )
            })?;
        for certificate in &proof.proofs {
            if let Some(at) = certificate.0.expiration() {
                until = until.min(at.to_unix());
            }
        }
    }
    if until <= now.to_unix() {
        return Err(TonkWorkerError::Forbidden(
            "the account's access to that space has run out".into(),
        ));
    }
    Timestamp::try_from(i128::from(until)).map_err(failure)
}

/// Sign the agent's grant to `target` with `issuer`, the account, and
/// keep it with a directory entry in the agent space. Returns the chains.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
pub(super) async fn issue(
    tonk: &TonkState,
    issuer: &dialog_credentials::Signer,
    agent: &Did,
    target: &Target,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    let now = Timestamp::now();
    let expires = deadline(tonk, &target.subject, target.access, now).await?;
    let chains = super::agent_session::grant(
        tonk,
        issuer,
        agent,
        &target.subject,
        target.access,
        &target.remote,
        now,
        expires,
    )
    .await?;
    super::agent_session::record_grants(
        tonk,
        agent.repo_key(),
        &[super::agent_session::Granted {
            subject: target.subject.clone(),
            name: target.label.clone(),
            home: target.remote.clone(),
            chains: chains.clone(),
        }],
    )
    .await?;
    Ok(chains)
}

/// Sign the grant the page asked for with the account the passkey just
/// opened, keep it in the agent space and push it there.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn authorize(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    grant: AgentGrant,
) -> Result<wasm_bindgen::JsValue, String> {
    {
        let tonk = state.read().await;
        super::ceremony::report(
            &tonk,
            tonk_schema::ceremony::GRANT_AGENT,
            tonk_schema::ceremony_state::WORKING,
            &grant.space.key,
        )
        .await;
    }
    let outcome = authorize_inner(state, custodian, &grant).await;
    let tonk = state.read().await;
    let (status, detail) = match &outcome {
        // Done names the space, so the page tells its own join from an
        // earlier one's.
        Ok(()) => (tonk_schema::ceremony_state::DONE, grant.space.key.clone()),
        Err(error) => (tonk_schema::ceremony_state::FAILED, error.clone()),
    };
    super::ceremony::report(&tonk, tonk_schema::ceremony::GRANT_AGENT, status, &detail).await;
    outcome.map(|()| wasm_bindgen::JsValue::UNDEFINED)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn authorize_inner(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    grant: &AgentGrant,
) -> Result<(), String> {
    use dialog_credentials::Signer;
    use dialog_varsig::Principal as _;

    let said = |error: TonkWorkerError| error.to_string();
    let account = super::custody::held_account(custodian).await?;
    let Signer::Ed25519(root) = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    let agent = {
        let tonk = state.read().await;
        let linked = super::identity::local_root(&tonk)
            .await
            .map_err(|error| format!("no account is signed in on this profile: {error}"))?
            .root_did;
        if root.did() != linked {
            return Err("this passkey belongs to a different account".into());
        }
        super::agents::find(&tonk, &grant.agent)
            .await
            .map_err(said)?
    };
    let target = super::agent_session::targets(state, None, std::slice::from_ref(&grant.space))
        .await
        .map_err(said)?
        .pop()
        .ok_or("no space to grant")?;
    {
        let tonk = state.read().await;
        issue(&tonk, &Signer::from(root), &agent, &target)
            .await
            .map_err(said)?;
    }
    // The hosted agent reads its grants from the agent space's remote.
    if let Err(error) = super::sync::sync_repository(state, &grant.agent).await {
        let tonk = state.read().await;
        tonk.sync_queue
            .mark_dirty(&grant.agent, js_sys::Date::now());
        return Err(format!("the agent's grant could not be saved: {error}"));
    }
    tonk_common::log!("agent {agent} granted {} by the account", target.subject);
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

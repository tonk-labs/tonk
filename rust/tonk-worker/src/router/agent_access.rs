//! An agent's request for access, approved from `/agent/connect`.
//!
//! The agent signs a request under its own key (see
//! [`tonk_invite::agent_request`]); the person opens its link here while
//! signed in. Opening verifies it and shows who is asking, with the code
//! the agent shows too. Approving a space verifies it again and issues that
//! space's grants to the agent's key through the same flow as any agent
//! invitation, so the account, sync and permission checks are those.
//!
//! Everything this module writes is profile overlay state: the request
//! expires within the hour, and a keyed link is useful only to its agent.

use crate::{TonkWorkerError, worker::TonkState};
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use std::sync::Arc;
use tonk_common::log;
use tonk_invite::agent_request::AgentRequest;
use tonk_schema::command::{
    AgentAccessFailure, AgentAccessGrant, AgentAccessRequest, ApproveAgentAccess, OpenAgentRequest,
};
use tonk_schema::domain::agent_access::{Agent, Code, Link, Mode, Name, Reason, Request, Status};
use tonk_schema::prelude::DidExt as _;

/// The fixed entity the open request and its failure live on.
const REQUEST_STATE: &str = "state:agent-access";

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<OpenAgentRequest> for crate::router::CommandEnv {
    async fn execute(&self, command: OpenAgentRequest) {
        if let Err(error) = open(self.state(), &command.url.0).await {
            log!("agent access request not shown: {error}");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<ApproveAgentAccess> for crate::router::CommandEnv {
    async fn execute(&self, command: ApproveAgentAccess) {
        if let Err(error) = approve(self, command).await {
            log!("agent access approval failed: {error}");
        }
    }
}

/// Verify the request at `url` and show it, replacing whatever request and
/// per-space outcomes the page showed before.
async fn open(state: &super::AppState, url: &str) -> Result<(), TonkWorkerError> {
    let tonk = state.read().await;
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let this = request_entity()?;
    let retained = this.clone();
    session
        .state
        .retain_overlay_entities(move |entity| *entity != retained);
    clear_grants(&tonk, &session).await?;
    match AgentRequest::from_url(url, Timestamp::now()).await {
        Ok(request) => session.state.assert_overlay(AgentAccessRequest {
            this,
            name: Name(request.name().to_owned()),
            agent: Agent(request.agent().to_string()),
            code: Code(request.verification_code()),
            request: Request(url.to_owned()),
        }),
        Err(error) => {
            log!("agent access request refused: {error:#}");
            session.state.assert_overlay(AgentAccessFailure {
                this,
                reason: Reason(refusal(&error).to_owned()),
            });
        }
    }
    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

/// The fixed copy for a request that cannot be opened. The verifier's
/// error is logged, never shown: it can echo the link's own bytes.
fn refusal(error: &impl std::fmt::Display) -> &'static str {
    if error.to_string().contains("agent_request_expired") {
        "this request has expired; ask the agent for a new one"
    } else {
        "this request link is not valid; ask the agent for a new one"
    }
}

/// Drop the outcomes an earlier request left on spaces' entities, so a
/// new request never shows another agent's links.
async fn clear_grants(
    tonk: &TonkState,
    session: &dialog_reactor::BranchSession,
) -> Result<(), TonkWorkerError> {
    let stale: Vec<_> = shown_grants(tonk, session, None)
        .await?
        .into_iter()
        .map(|grant| grant.this)
        .collect();
    session
        .state
        .retain_overlay_entities(move |entity| !stale.contains(entity));
    Ok(())
}

/// The outcomes the page shows, for one space or for all of them.
async fn shown_grants(
    tonk: &TonkState,
    session: &dialog_reactor::BranchSession,
    space: Option<&Did>,
) -> Result<Vec<AgentAccessGrant>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};
    session
        .handle()
        .query()
        .select(Query::<AgentAccessGrant> {
            this: space.map_or_else(|| Term::var("this"), |space| Term::from(space.this())),
            agent: Term::var("agent"),
            status: Term::var("status"),
            mode: Term::var("mode"),
            link: Term::var("link"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))
}

async fn approve(
    env: &crate::router::CommandEnv,
    command: ApproveAgentAccess,
) -> Result<(), TonkWorkerError> {
    let space: Did = command
        .space
        .0
        .to_string()
        .parse()
        .map_err(|_| TonkWorkerError::Router("space is not a DID".into()))?;
    let request = match AgentRequest::from_url(&command.request.0, Timestamp::now()).await {
        Ok(request) => request,
        Err(error) => {
            log!("agent access approval refused: {error:#}");
            return open(env.state(), &command.request.0).await;
        }
    };
    let agent = request.agent().clone();
    let repo = space.repo_key().to_owned();
    if !env.may_target_space(&repo) {
        log!("agent access approval ignored: origin may not target '{repo}'");
        return Ok(());
    }
    {
        let tonk = env.state().read().await;
        publish_grant(
            &tonk,
            &space,
            &agent,
            "busy",
            "granting access…".into(),
            String::new(),
        )
        .await?;
    }
    if let Err(error) = issue(env, &repo, &agent).await {
        log!("agent access grant failed: {error}");
        let tonk = env.state().read().await;
        publish_grant(
            &tonk,
            &space,
            &agent,
            "retry",
            "could not grant access to this space; try again".into(),
            String::new(),
        )
        .await?;
    }
    Ok(())
}

#[cfg(feature = "connection-invites")]
async fn issue(
    env: &crate::router::CommandEnv,
    repo: &str,
    agent: &Did,
) -> Result<(), TonkWorkerError> {
    super::repository::run_connection_invite_for(env, true, repo, Some(agent)).await
}

#[cfg(not(feature = "connection-invites"))]
async fn issue(
    env: &crate::router::CommandEnv,
    repo: &str,
    agent: &Did,
) -> Result<(), TonkWorkerError> {
    use dialog_repository::RepositoryExt as _;
    let tonk = env.state().read().await;
    let space = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?
        .did();
    publish_grant(
        &tonk,
        &space,
        agent,
        "unavailable",
        "tool connections are not enabled on this deployment yet".into(),
        String::new(),
    )
    .await
}

/// Record what approving `space` for `agent` produced, on the space's
/// entity in the profile overlay.
pub(crate) async fn publish_grant(
    tonk: &TonkState,
    space: &Did,
    agent: &Did,
    mode: &str,
    status: String,
    link: String,
) -> Result<(), TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    for previous in shown_grants(tonk, &session, Some(space)).await? {
        session.state.retract_overlay(previous);
    }
    session.state.assert_overlay(AgentAccessGrant {
        this: space.this(),
        agent: Agent(agent.to_string()),
        status: Status(status),
        mode: Mode(mode.to_owned()),
        link: Link(link),
    });
    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

fn request_entity() -> Result<dialog_artifacts::Entity, TonkWorkerError> {
    REQUEST_STATE
        .parse()
        .map_err(|_| TonkWorkerError::Internal("invalid agent access state entity".into()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::router::{CommandEnv, CommandOrigin};
    use dialog_capability::Provider;
    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::RepositoryExt as _;
    use dialog_varsig::Principal as _;
    use tonk_schema::domain::command::current::{approve_agent_access, open_agent_request};

    fn from_profile(state: &crate::AppState) -> CommandEnv {
        CommandEnv::new(
            state.clone(),
            CommandOrigin {
                repo: String::new(),
                branch: "main".into(),
                client: None,
            },
        )
    }

    async fn request_link(seed: u8, name: &str) -> (Did, String) {
        let agent = Signer::from(Ed25519Signer::import(&[seed; 32]).await.unwrap());
        let url = AgentRequest::new(&agent, name, Timestamp::now())
            .await
            .unwrap()
            .to_url("https://tonk.example/agent/connect")
            .unwrap();
        (agent.did(), url)
    }

    async fn open_link(env: &CommandEnv, url: &str) {
        <CommandEnv as Provider<OpenAgentRequest>>::execute(
            env,
            OpenAgentRequest {
                this: "id:test:open".parse().unwrap(),
                url: open_agent_request::Url(url.to_owned()),
            },
        )
        .await;
    }

    async fn profile_session(state: &crate::AppState) -> dialog_reactor::BranchSession {
        let tonk = state.read().await;
        tonk.reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .unwrap()
    }

    async fn shown_request(state: &crate::AppState) -> Vec<AgentAccessRequest> {
        let session = profile_session(state).await;
        let tonk = state.read().await;
        session
            .handle()
            .query()
            .select(Query::<AgentAccessRequest> {
                this: Term::from(request_entity().unwrap()),
                name: Term::var("name"),
                agent: Term::var("agent"),
                code: Term::var("code"),
                request: Term::var("request"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap()
    }

    async fn shown_failure(state: &crate::AppState) -> Vec<AgentAccessFailure> {
        let session = profile_session(state).await;
        let tonk = state.read().await;
        session
            .handle()
            .query()
            .select(Query::<AgentAccessFailure> {
                this: Term::from(request_entity().unwrap()),
                reason: Term::var("reason"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap()
    }

    async fn grants(state: &crate::AppState, space: &Did) -> Vec<AgentAccessGrant> {
        let session = profile_session(state).await;
        let tonk = state.read().await;
        shown_grants(&tonk, &session, Some(space)).await.unwrap()
    }

    #[dialog_common::test]
    async fn it_shows_a_verified_request_and_refuses_a_forged_one() {
        let state = crate::router::command::tests::native::test_state().await;
        let env = from_profile(&state);
        let (agent, url) = request_link(80, "Claude Code on the laptop").await;

        open_link(&env, &url).await;
        let shown = shown_request(&state).await;
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].name.0, "Claude Code on the laptop");
        assert_eq!(shown[0].agent.0, agent.to_string());
        assert_eq!(
            shown[0].code.0,
            tonk_invite::agent_request::verification_code(&agent)
        );
        assert!(shown_failure(&state).await.is_empty());

        let (_, other) = request_link(81, "someone else").await;
        // Swap one character in the middle of the signed bytes.
        let (base, encoded) = other.split_once("tonk-access-request=").unwrap();
        let mut chars: Vec<char> = encoded.chars().collect();
        let middle = chars.len() / 2;
        chars[middle] = if chars[middle] == '2' { '3' } else { '2' };
        let forged = format!(
            "{base}tonk-access-request={}",
            chars.into_iter().collect::<String>()
        );
        open_link(&env, &forged).await;
        assert!(shown_request(&state).await.is_empty());
        let failure = shown_failure(&state).await;
        assert_eq!(failure.len(), 1);
        assert!(
            failure[0].reason.0.contains("not valid"),
            "{}",
            failure[0].reason.0
        );
    }

    #[dialog_common::test]
    async fn it_answers_an_approval_on_the_space_and_clears_it_for_the_next_request() {
        let state = crate::router::command::tests::native::test_state().await;
        let repo = super::super::repository::create_space_inner(&state, "Agent access", None)
            .await
            .unwrap();
        let space = {
            let tonk = state.read().await;
            tonk.profile
                .space(&repo)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap()
                .did()
        };
        let env = from_profile(&state);
        let (agent, url) = request_link(82, "agent").await;
        open_link(&env, &url).await;

        <CommandEnv as Provider<ApproveAgentAccess>>::execute(
            &env,
            ApproveAgentAccess {
                this: "id:test:approve".parse().unwrap(),
                request: approve_agent_access::Request(url),
                space: approve_agent_access::Space(space.this()),
            },
        )
        .await;
        let answered = grants(&state, &space).await;
        assert_eq!(answered.len(), 1, "one outcome per space: {answered:?}");
        assert_eq!(answered[0].agent.0, agent.to_string());
        // No account is signed in here, so the flow asks for one rather
        // than issuing anything.
        assert_ne!(answered[0].mode.0, "scoped");
        assert!(answered[0].link.0.is_empty());

        let (_, next) = request_link(83, "another agent").await;
        open_link(&env, &next).await;
        assert!(grants(&state, &space).await.is_empty());
    }
}

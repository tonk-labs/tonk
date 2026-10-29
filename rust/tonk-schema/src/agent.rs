//! Agent spaces: [`AgentSpace`], the mark that makes a space an agent's
//! identity, and [`AgentSession`], one session key's powerline from it
//! as a session list presents it.
//!
//! Both live on the agent space's own content branch. The account that
//! provisioned the agent keeps its key; nothing here records it.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::{Entity, Preload, Speculation};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::{Concept, EvaluationError, Output as _, Query, Term};
use dialog_repository::{Branch, Hydrate, RemoteSite};

use crate::device_link::delegation_audience;
use crate::domain::agent_session::{CreatedAt, Reason, Title};
use crate::domain::repo::Kind;

/// The kind recorded on an agent space.
pub const AGENT_SPACE: &str = "case:agent";

/// The reason recorded on a session minted for an agent.
pub const AGENT_SESSION: &str = "case:agent-session";

/// The stored [`Kind`] of an agent space. The URI is a constant, so the
/// parse cannot fail.
pub fn agent_space_kind() -> Kind {
    Kind(AGENT_SPACE.parse().expect("a constant kind URI parses"))
}

/// The stored [`Reason`] for an agent session. The URI is a constant,
/// so the parse cannot fail.
pub fn agent_session_reason() -> Reason {
    Reason(AGENT_SESSION.parse().expect("a constant reason URI parses"))
}

/// The mark that makes a space an agent's identity, keyed by the
/// space's subject DID like [`RepositoryName`](crate::RepositoryName).
///
/// An agent space is otherwise an ordinary space: provisioned through an
/// account, its key sealed under it. Its DID is the agent wherever an
/// account DID could stand — a member, an audience, a contact.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentSpace {
    /// The space's subject DID.
    pub this: Entity,
    /// What kind of space this is — [`AGENT_SPACE`].
    pub kind: Kind,
}

impl AgentSpace {
    /// Mark the space whose subject is `space` as an agent space.
    pub fn new(space: Entity) -> Self {
        Self {
            this: space,
            kind: agent_space_kind(),
        }
    }
}

/// One session of an agent: the label and creation time of an
/// `agent space -> session` powerline.
///
/// Shaped like [`DeviceLink`](crate::DeviceLink) and for the same
/// reason: the entity is the delegation itself, as dialog keyed it when
/// the chain was retained, so the row cannot disagree with the proof and
/// revoking the powerline takes the row with it. The session's DID is
/// the delegation's own audience.
/// Which LLM provider's apps an agent space is for, on the agent
/// space's subject DID like [`AgentSpace`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentProvider {
    /// The agent space's subject DID.
    pub this: Entity,
    /// `claude` or `chatgpt`.
    pub provider: crate::domain::agent_space::Provider,
}

impl AgentProvider {
    /// Note that the agent space `space` is for `provider`'s apps.
    pub fn new(space: Entity, provider: impl Into<String>) -> Self {
        Self {
            this: space,
            provider: crate::domain::agent_space::Provider(provider.into()),
        }
    }
}

#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentSession {
    /// The delegation's entity — its blob hash, as dialog keyed it.
    pub this: Entity,
    /// When the session was minted, unix seconds.
    pub created_at: CreatedAt,
    /// Human label for the session, e.g. "Claude".
    pub title: Title,
    /// Why the delegation exists — [`AGENT_SESSION`].
    pub reason: Reason,
}

impl AgentSession {
    /// Describe the powerline stored at `entity` as an agent session.
    ///
    /// `entity` comes from retaining the chain — dialog returns the
    /// entities it wrote — so this never derives a hash of its own and
    /// cannot describe a delegation that was never stored.
    pub fn new(entity: Entity, title: impl Into<String>, created_at: u64) -> Self {
        Self {
            this: entity,
            created_at: CreatedAt(created_at),
            title: Title(title.into()),
            reason: agent_session_reason(),
        }
    }
}

/// This agent's sessions and their session DIDs, from the agent space
/// branch's own facts.
///
/// A row whose audience fact is missing is skipped, as
/// [`device_links`](crate::device_link::device_links) skips one: without
/// an audience there is no session to name.
pub async fn agent_sessions<Env>(
    agent: &Branch,
    env: &Env,
) -> Result<Vec<(AgentSession, String)>, EvaluationError>
where
    Env: dialog_common::Holds
        + Provider<Get>
        + Provider<dialog_effects::blob::Read>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Hydrate>
        + Provider<Preload>
        + Provider<Speculation>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let rows: Vec<AgentSession> = agent
        .query()
        .select(Query::<AgentSession> {
            this: Term::var("this"),
            created_at: Term::var("created_at"),
            title: Term::var("title"),
            reason: Term::var("reason"),
        })
        .perform(env)
        .try_vec()
        .await?;

    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(did) = delegation_audience(agent, &row.this, env).await else {
            continue;
        };
        sessions.push((row, did));
    }
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use anyhow::Result;
    use dialog_credentials::Ed25519Signer;
    use dialog_peer::helpers;
    use dialog_ucan::UcanDelegation;
    use dialog_varsig::{Principal as _, did};
    use tonk_identity::agent::mint_agent_session;
    use url::Url;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A retained powerline described as a session lists with its
    /// audience — the session DID — read from the delegation itself.
    #[dialog_common::test]
    async fn it_lists_a_session_by_the_powerline_it_describes() -> Result<()> {
        let (operator, profile) = helpers::test_session_with_peer().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;

        let agent = Ed25519Signer::import(&[5u8; 32]).await?;
        let session = Ed25519Signer::import(&[6u8; 32]).await?;
        let home = Url::parse("https://tonk.example/")?;
        let powerline = mint_agent_session(agent, &session.did(), &home).await?;

        let entities = branch
            .delegations()
            .retain(UcanDelegation(powerline))
            .perform(&operator)
            .await?;
        assert_eq!(entities.len(), 1, "a powerline is one certificate");

        let branch = repository.branch("main").open().perform(&operator).await?;
        branch
            .transaction()
            .assert(AgentSession::new(
                entities[0].clone(),
                "Claude",
                1_700_000_000,
            ))
            .commit()
            .publish()
            .perform(&operator)
            .await?;

        let branch = repository.branch("main").open().perform(&operator).await?;
        let sessions = agent_sessions(&branch, &operator).await?;
        assert_eq!(sessions.len(), 1);
        let (row, did) = &sessions[0];
        assert_eq!(row.this, entities[0]);
        assert_eq!(row.title.0, "Claude");
        assert_eq!(row.reason, agent_session_reason());
        assert_eq!(did, &session.did().to_string());
        Ok(())
    }

    /// The mark is keyed by the space's own subject, so asserting it
    /// twice is one row.
    #[dialog_common::test]
    async fn it_marks_a_space_by_its_subject() -> Result<()> {
        let (operator, profile) = helpers::test_session_with_peer().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let subject = did!("test:agent").this();

        for _ in 0..2 {
            branch
                .transaction()
                .assert(AgentSpace::new(subject.clone()))
                .commit()
                .publish()
                .perform(&operator)
                .await?;
        }

        let branch = repository.branch("main").open().perform(&operator).await?;
        let marks: Vec<AgentSpace> = branch
            .query()
            .select(Query::<AgentSpace> {
                this: Term::var("this"),
                kind: Term::var("kind"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(marks, vec![AgentSpace::new(subject)]);
        Ok(())
    }
}

//! What an agent space reaches, read from the agent space itself.
//!
//! The approval page retains each grant (a chain `space -> ... -> agent
//! space`) in the agent space's `main` with
//! [`tonk_account::delegations::retain_space_delegation`], the same call
//! an account uses for `space -> account`, and records a directory entry
//! on the granted space's entity: its [`tonk_schema::SpaceName`] and its
//! [`tonk_schema::SpaceHomeAddress`]. There is no access-level fact:
//! access is what the retained chains prove.
//!
//! The agent space's own kernel answers the hosted agent's writer with it:
//!
//! - `GET /api/repository/{agent}/agent/access`: [`read_agent_access`],
//!   each space classified by the grants in force;
//! - `GET /api/repository/{agent}/agent/chains?space=`: [`space_chains`],
//!   the grants in force for one space, which a writer of that space boots
//!   its kernel with;
//! - `POST /api/repository/{agent}/agent/grant`: [`retain_agent_grants`],
//!   what joining a space from the chat keeps.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query as RouteQuery, State};

use dialog_capability::access::TimeRange;
use dialog_query::{Output as _, Query, Term};
use dialog_ucan::{Parameters, Scope};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{Container, DelegationChain, Tag};
use dialog_varsig::Did;
use tonk_schema::prelude::DidExt as _;
use tonk_worker_api::agent::{AgentAccess, SpaceAccess, reaches};

use serde::Deserialize;
use serde_json::{Value, json};
use tonk_macros::handler;

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// The branch of the agent space that holds its grants and directory.
const GRANTS_BRANCH: &str = "main";

/// Classify the grants the agent space `agent` holds in its `main`, as
/// `tonk` (a peer with `agent` mounted) reads it now.
///
/// A grant is a delegation retained there whose audience is `agent` and
/// whose subject is another space; it counts when a chain from that
/// space through it verifies. A chain in force for `/use/get/...` makes
/// the space readable, one for `/use/put/...` writable too. A grant
/// outside its time bounds counts as lapsed, so a tool can name the space
/// whose access ran out. One whose chain does not verify counts as
/// nothing.
pub async fn read_agent_access(
    tonk: &TonkState,
    agent: &Did,
) -> Result<AgentAccess, TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let session = tonk
        .reactor
        .repository(agent.as_ref())
        .branch(GRANTS_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let branch = session.handle();
    let leaves = branch
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let now = Timestamp::now().to_unix();
    let at_now = TimeRange {
        not_before: Some(now),
        expiration: Some(now + 1),
    };
    let mut spaces: BTreeMap<Did, SpaceAccess> = BTreeMap::new();
    for leaf in leaves {
        let chain = &leaf.0;
        let Some(subject) = chain.subject().cloned() else {
            // A subject-open delegation to the agent space names no space
            // to list.
            continue;
        };
        if &subject == agent {
            continue;
        }
        let Some(delegation) = chain.proofs().next() else {
            continue;
        };
        let command = delegation.command().clone();
        let in_window = chain.expiration().is_none_or(|at| at.to_unix() > now)
            && chain.not_before().is_none_or(|at| at.to_unix() <= now);
        let scope = Scope {
            subject: UcanSubject::Specific(subject.clone()),
            command: command.clone(),
            parameters: Parameters::default(),
        };
        let proof = branch
            .delegations()
            .prove(agent.clone(), scope)
            .during(at_now)
            .perform(&tonk.operator)
            .await;
        match proof {
            Ok(proof) if in_window => {
                let access = spaces.entry(subject).or_default();
                access.read |= reaches(&command.0, "get");
                access.write |= reaches(&command.0, "put");
                if let Some(at) = proof.duration.expiration {
                    access.expires = Some(access.expires.map_or(at, |held| held.min(at)));
                }
                access.grants.push(encode(chain)?);
                // Signed by whoever granted the space, so the agent, which
                // can write its own space's directory, cannot redirect it.
                if access.address.is_none() {
                    access.address = tonk_invite::home_address(chain)
                        .ok()
                        .flatten()
                        .map(|address| address.to_string());
                }
            }
            _ if !in_window => spaces.entry(subject).or_default().lapsed += 1,
            // A chain that does not verify grants nothing.
            _ => {}
        }
    }
    let mut access = AgentAccess::default();
    for (space, mut entry) in spaces {
        // A space is writable only if it is readable too: a write pulls
        // first.
        entry.write &= entry.read;
        if !entry.read && entry.lapsed == 0 {
            continue;
        }
        entry.grants.sort_unstable();
        entry.grants.dedup();
        entry.name = directory_name(tonk, branch, &space).await;
        access.spaces.insert(space.to_string(), entry);
    }
    access.spaces.insert(
        agent.to_string(),
        SpaceAccess {
            agent: true,
            read: true,
            write: true,
            // The agent space names itself, as any space does; a directory
            // entry for it, should one exist, wins.
            name: match directory_name(tonk, branch, agent).await {
                Some(name) => Some(name),
                None => own_name(tonk, branch, agent).await,
            },
            ..SpaceAccess::default()
        },
    );
    Ok(access)
}

/// The grants in force that the agent space `agent` holds for `space`,
/// each a chain `space -> ... -> agent`, as `tonk` reads them now: what
/// [`read_agent_access`] counts for that space, for a view to extend
/// with the powerline and a hop to its own key.
pub async fn space_chains(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let session = tonk
        .reactor
        .repository(agent.as_ref())
        .branch(GRANTS_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let branch = session.handle();
    let leaves = branch
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    let now = Timestamp::now().to_unix();
    let at_now = TimeRange {
        not_before: Some(now),
        expiration: Some(now + 1),
    };
    let mut chains = Vec::new();
    for leaf in leaves {
        let chain = &leaf.0;
        if chain.subject() != Some(space) || space == agent {
            continue;
        }
        let Some(delegation) = chain.proofs().next() else {
            continue;
        };
        let in_window = chain.expiration().is_none_or(|at| at.to_unix() > now)
            && chain.not_before().is_none_or(|at| at.to_unix() <= now);
        if !in_window {
            continue;
        }
        let scope = Scope {
            subject: UcanSubject::Specific(space.clone()),
            command: delegation.command().clone(),
            parameters: Parameters::default(),
        };
        // The store keeps each delegation apart, so the leaf names only its
        // last hop. What reaches its issuer is proved from there, rather
        // than from the agent space, so the walk cannot set off through
        // another of the agent space's leaves and miss this one.
        let issuer = delegation.issuer().clone();
        let mut links = if &issuer == space {
            Vec::new()
        } else {
            let proved = branch
                .delegations()
                .prove(issuer.clone(), scope)
                .during(at_now)
                .perform(&tonk.operator)
                .await;
            let Ok(proof) = proved else {
                continue;
            };
            let mut links: Vec<_> = proof.proofs.into_iter().map(|link| link.0).collect();
            if links.first().is_some_and(|root| root.issuer() != space) {
                links.reverse();
            }
            links
        };
        links.push(delegation.clone());
        if let Ok(proved) = dialog_ucan_core::DelegationChain::try_from(links) {
            chains.push(proved);
        }
    }
    Ok(chains)
}

/// The name the agent space's directory gives `space`.
async fn directory_name(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    space: &Did,
) -> Option<String> {
    let rows: Vec<tonk_schema::SpaceName> = branch
        .query()
        .select(Query::<tonk_schema::SpaceName> {
            this: Term::from(space.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.name.0)
}

/// The name `space` gives itself (`tonk/repository`).
async fn own_name(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    space: &Did,
) -> Option<String> {
    let rows: Vec<tonk_schema::RepositoryName> = branch
        .query()
        .select(Query::<tonk_schema::RepositoryName> {
            this: Term::from(space.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter().next().map(|row| row.name.0)
}

/// One granted space's directory entry in the agent space.
#[derive(Debug, Deserialize)]
pub(crate) struct DirectoryEntry {
    pub space: Did,
    pub name: Option<String>,
    pub address: Option<String>,
}

/// What the approval does to the agent space `agent` as `tonk` holds it:
/// retain each of `chains` in its `main` with
/// [`tonk_account::delegations::retain_space_delegation`], as an account
/// retains `space -> account`, and record each entry of `directory` as
/// [`tonk_schema::SpaceName`] and [`tonk_schema::SpaceHomeAddress`] on
/// the space's entity. The caller pushes.
pub(crate) async fn retain_agent_grants(
    tonk: &TonkState,
    agent: &Did,
    chains: &[dialog_ucan_core::DelegationChain],
    directory: &[DirectoryEntry],
) -> Result<(), TonkWorkerError> {
    let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
    let key = agent.to_string();
    let session = tonk
        .reactor
        .repository(&key)
        .branch(GRANTS_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    for chain in chains {
        tonk_account::delegations::retain_space_delegation(session.handle(), chain, &tonk.operator)
            .await
            .map_err(|error| internal(&error))?;
    }
    if directory.is_empty() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .repository(&key)
        .branch(GRANTS_BRANCH)
        .transaction();
    for entry in directory {
        if let Some(name) = &entry.name {
            transaction = transaction.assert(tonk_schema::SpaceName::new(&entry.space, name));
        }
        if let Some(address) = &entry.address {
            transaction =
                transaction.assert(tonk_schema::SpaceHomeAddress::new(&entry.space, address));
        }
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| internal(&error))?;
    Ok(())
}

/// `chain` as a base64url container.
fn encode(chain: &DelegationChain) -> Result<String, TonkWorkerError> {
    let bytes = Container::from(chain)
        .encode(Tag::Base64Url)
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| TonkWorkerError::Internal(error.to_string()))
}

/// The agent space a route names by its DID.
fn agent(repo: &str) -> Result<Did, TonkWorkerError> {
    repo.parse()
        .map_err(|_| TonkWorkerError::Router(format!("'{repo}' names no agent space")))
}

/// `GET /api/repository/{repo}/agent/access`: what the agent space `repo`
/// reaches.
#[handler]
pub async fn access(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<AgentAccess>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    Ok(Json(read_agent_access(&tonk, &agent(&repo)?).await?))
}

/// The space `GET .../agent/chains` asks for.
#[derive(Debug, Deserialize)]
pub struct ChainsQuery {
    space: String,
}

/// `GET /api/repository/{repo}/agent/chains?space=`: the grants in force
/// for `space`, `{"chains": [...]}`, each a base64url container.
#[handler]
pub async fn chains(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    RouteQuery(query): RouteQuery<ChainsQuery>,
) -> Result<Json<Value>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    let space: Did = query
        .space
        .parse()
        .map_err(|_| TonkWorkerError::Router(format!("'{}' names no space", query.space)))?;
    let chains = space_chains(&tonk, &agent(&repo)?, &space).await?;
    let encoded = chains.iter().map(encode).collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({ "chains": encoded })))
}

/// What `POST .../agent/grant` keeps: chains, each a base64url container
/// the writer checked reaches the agent space, and the directory entries of
/// the spaces they grant.
#[derive(Debug, Deserialize)]
pub struct GrantRequest {
    chains: Vec<String>,
    #[serde(default)]
    directory: Vec<DirectoryEntry>,
}

/// `POST /api/repository/{repo}/agent/grant`: retain the grants and the
/// directory entries a join from the chat gives the agent space `repo`.
/// The caller pushes.
#[handler]
pub async fn grant(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Json(request): Json<GrantRequest>,
) -> Result<Json<Value>, TonkWorkerError> {
    let chains = request
        .chains
        .iter()
        .map(|encoded| {
            Container::decode(encoded.as_bytes())
                .ok()
                .and_then(|container| DelegationChain::try_from(container).ok())
                .ok_or_else(|| TonkWorkerError::Router("a grant is not a delegation chain".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tonk = state.read().await.clone();
    retain_agent_grants(&tonk, &agent(&repo)?, &chains, &request.directory).await?;
    Ok(Json(json!({})))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    use super::super::repository::profile_library_tests::test_state;

    /// A space made on `app`, by its DID.
    async fn space(app: &Router) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/repository/agent")
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
        let info: super::super::RepositoryInfo = serde_json::from_slice(&body).unwrap();
        info.subject.to_string()
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap_or_default())
    }

    #[dialog_common::test]
    async fn it_answers_that_an_agent_space_reaches_itself() {
        let (app, _state, _lsp) = super::super::api_router_with_state(test_state().await);
        let agent = space(&app).await;
        let (status, access) = get(&app, &format!("/api/repository/{agent}/agent/access")).await;
        assert_eq!(status, StatusCode::OK, "{access}");
        assert_eq!(access["spaces"][&agent]["agent"], true, "{access}");
    }

    #[dialog_common::test]
    async fn it_answers_no_chains_for_a_space_it_holds_no_grant_for() {
        let (app, _state, _lsp) = super::super::api_router_with_state(test_state().await);
        let agent = space(&app).await;
        let other = "did:key:z6MkvRXNYcE7MMduynWTgeKbDaT1iijDSC8pZqXZc8rHPrf2";
        let (status, chains) = get(
            &app,
            &format!("/api/repository/{agent}/agent/chains?space={other}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{chains}");
        assert_eq!(chains["chains"], serde_json::json!([]));
    }

    #[dialog_common::test]
    async fn it_refuses_a_route_that_names_no_agent_space() {
        let (app, _state, _lsp) = super::super::api_router_with_state(test_state().await);
        let (status, _) = get(&app, "/api/repository/not-a-did/agent/access").await;
        assert!(status.is_client_error(), "{status}");
    }
}

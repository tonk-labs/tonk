//! An agent's own space, as the account that made the agent acts on it.
//!
//! An agent is a space whose key is the agent's. It keeps the grants that let
//! the agent reach other spaces, each retained as the chain it is, and a
//! directory naming those spaces and where each is served. The hosted agent
//! reads both from the space's remote.
//!
//! The person's profile approves an agent, lists what it can reach, and
//! removes that access. Where each space has a worker of its own, the profile
//! holds none of the agent space's content, so it asks that worker: the
//! routes here are what it asks, and on a host with one database the profile
//! answers them itself.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
};
use dialog_query::{Output as _, Query, Term};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use tonk_macros::handler;
use tonk_schema::agent::{AgentProvider, AgentSpace};
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{SpaceHomeAddress, SpaceName};

use super::AppState;
use super::repository::CONTENT_BRANCH;
use crate::{TonkState, TonkWorkerError};

/// The most hops a grant's chain is walked up before giving up.
const MAX_HOPS: usize = 16;

/// What an agent's space holds that its account acts on.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSpaceRecord {
    /// Each delegation to the agent the space retains, live or not.
    pub(crate) grants: Vec<HeldGrant>,
    /// Where each space the agent may reach is served, by the space's DID.
    pub(crate) homes: BTreeMap<String, String>,
    /// What the agent's directory names each space, by the space's DID.
    pub(crate) names: BTreeMap<String, String>,
}

/// One delegation to the agent, as its space retains it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeldGrant {
    /// The chain, as retained: base58 of its bytes.
    chain: String,
    /// The public chain from the space it is of down to this one, rebuilt
    /// from what the space retains, when it can be: what a revocation of
    /// it is witnessed by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence: Option<String>,
}

impl HeldGrant {
    /// The delegation.
    #[cfg(feature = "persona")]
    pub(crate) fn chain(&self) -> Result<DelegationChain, TonkWorkerError> {
        decode(&self.chain)
    }

    /// The chain a revocation of this delegation is witnessed by.
    #[cfg(all(
        feature = "persona",
        any(all(test, feature = "integration-tests"), target_arch = "wasm32")
    ))]
    pub(crate) fn evidence(&self) -> Result<DelegationChain, TonkWorkerError> {
        let evidence = self.evidence.as_deref().ok_or_else(|| {
            TonkWorkerError::Conflict(
                "the agent's space does not hold the chain that grant was issued through".into(),
            )
        })?;
        decode(evidence)
    }
}

/// What to change in an agent's space, in one commit.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSpaceChange {
    /// Grants to keep, base58 of each chain.
    #[serde(default)]
    pub(crate) retain: Vec<String>,
    /// Grants to drop, base58 of each chain.
    #[serde(default)]
    pub(crate) forget: Vec<String>,
    /// Spaces the agent may reach, for its directory.
    #[serde(default)]
    pub(crate) directory: Vec<DirectoryEntry>,
    /// Mark the space as the agent's, for the apps of `provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mark: Option<Mark>,
}

/// A space in an agent's directory.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryEntry {
    pub(crate) space: Did,
    pub(crate) name: String,
    pub(crate) home: String,
}

/// What marks a space as an agent's.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Mark {
    /// The provider whose apps the agent is offered to again, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<String>,
}

/// Base58 of `chain`'s bytes, as [`AgentSpaceChange`] carries it.
pub(crate) fn encode(chain: &DelegationChain) -> Result<String, TonkWorkerError> {
    chain
        .to_bytes()
        .map(|bytes| bs58::encode(bytes).into_string())
        .map_err(|error| TonkWorkerError::Internal(format!("a chain did not serialize: {error}")))
}

fn decode(text: &str) -> Result<DelegationChain, TonkWorkerError> {
    let bytes = bs58::decode(text)
        .into_vec()
        .map_err(|error| TonkWorkerError::Router(format!("a chain is not base58: {error}")))?;
    DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("a chain does not decode: {error}")))
}

fn failure(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Internal(error.to_string())
}

#[cfg(feature = "persona")]
fn path(key: &str) -> String {
    format!("/api/repository/{key}/agent")
}

/// What the agent space at routing key `key` holds: read here, or asked of
/// the space's own worker where it has one.
#[cfg(feature = "persona")]
pub(crate) async fn read(tonk: &TonkState, key: &str) -> Result<AgentSpaceRecord, TonkWorkerError> {
    if tonk.spaces_elsewhere() {
        let record = super::space_reach::ask(key, "GET", &path(key), None).await?;
        return serde_json::from_value(record).map_err(failure);
    }
    held(tonk, key).await
}

/// Make `change` to the agent space at routing key `key`: here, or by the
/// space's own worker where it has one.
#[cfg(feature = "persona")]
pub(crate) async fn change(
    tonk: &TonkState,
    key: &str,
    change: &AgentSpaceChange,
) -> Result<(), TonkWorkerError> {
    if tonk.spaces_elsewhere() {
        let body = serde_json::to_value(change).map_err(failure)?;
        super::space_reach::ask(key, "POST", &path(key), Some(&body)).await?;
        return Ok(());
    }
    make(tonk, key, change).await
}

/// Push the agent space at routing key `key` to its remote now, where the
/// hosted agent reads it: here, or by the space's own worker.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub(crate) async fn push(state: &AppState, key: &str) -> Result<(), TonkWorkerError> {
    if state.read().await.spaces_elsewhere() {
        super::space_reach::ask(
            key,
            "POST",
            &format!("/api/repository/{key}/branch/{CONTENT_BRANCH}/sync"),
            Some(&serde_json::json!({})),
        )
        .await?;
        return Ok(());
    }
    super::sync::sync_repository(state, key)
        .await
        .map_err(TonkWorkerError::Internal)
}

/// `GET /api/repository/{repo}/agent`: what the agent space holds.
#[handler]
pub async fn get(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<AgentSpaceRecord>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    held(&tonk, &repo).await.map(Json)
}

/// `POST /api/repository/{repo}/agent`: change what the agent space holds.
#[handler]
pub async fn post(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Json(change): Json<AgentSpaceChange>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    make(&tonk, &repo, &change).await?;
    Ok(Json(serde_json::json!({})))
}

async fn held(tonk: &TonkState, key: &str) -> Result<AgentSpaceRecord, TonkWorkerError> {
    let agent: Did = key
        .parse()
        .map_err(|_| TonkWorkerError::NotFound(format!("no agent {key}")))?;
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let handle = session.handle();
    let mut grants = Vec::new();
    for UcanDelegation(chain) in handle
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(failure)?
    {
        let evidence = match chain.subject() {
            Some(space) if space != &agent => evidence(tonk, handle, space, &chain)
                .await
                .and_then(|path| encode(&path))
                .ok(),
            _ => None,
        };
        grants.push(HeldGrant {
            chain: encode(&chain)?,
            evidence,
        });
    }
    let homes = handle
        .query()
        .select(Query::<SpaceHomeAddress> {
            this: Term::var("this"),
            home_address: Term::var("home_address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?
        .into_iter()
        .map(|row| (row.this.to_string(), row.home_address.0))
        .collect();
    let names = handle
        .query()
        .select(Query::<SpaceName> {
            this: Term::var("this"),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?
        .into_iter()
        .filter(|row| !row.name.0.trim().is_empty())
        .map(|row| (row.this.to_string(), row.name.0))
        .collect();
    Ok(AgentSpaceRecord {
        grants,
        homes,
        names,
    })
}

async fn make(
    tonk: &TonkState,
    key: &str,
    change: &AgentSpaceChange,
) -> Result<(), TonkWorkerError> {
    let agent: Did = key
        .parse()
        .map_err(|_| TonkWorkerError::NotFound(format!("no agent {key}")))?;
    let branch = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    for chain in &change.retain {
        tonk_account::delegations::retain_space_delegation(
            branch.handle(),
            &decode(chain)?,
            &tonk.operator,
        )
        .await
        .map_err(failure)?;
    }
    for chain in &change.forget {
        branch
            .handle()
            .delegations()
            .retract(UcanDelegation(decode(chain)?))
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
    }
    if change.directory.is_empty() && change.mark.is_none() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction();
    for entry in &change.directory {
        transaction = transaction
            .assert(SpaceName::new(&entry.space, &entry.name))
            .assert(SpaceHomeAddress::new(&entry.space, &entry.home));
    }
    if let Some(mark) = &change.mark {
        transaction = transaction.assert(AgentSpace::new(agent.this()));
        if let Some(provider) = &mark.provider {
            transaction = transaction.assert(AgentProvider::new(agent.this(), provider));
        }
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(())
}

/// The public chain `space -> ... -> agent` that ends in `leaf`, rebuilt
/// from the certificates the agent space retains. Each step up takes a
/// certificate to the current issuer, for `space` or a powerline, whose
/// command covers the one below it.
async fn evidence(
    tonk: &TonkState,
    handle: &dialog_repository::Branch,
    space: &Did,
    leaf: &DelegationChain,
) -> Result<DelegationChain, TonkWorkerError> {
    let certificate = leaf
        .proofs()
        .last()
        .ok_or_else(|| failure("an empty grant"))?
        .clone();
    let now = Timestamp::now().to_unix();
    let subject = Subject::Specific(space.clone());
    let mut path = vec![certificate];
    while path[0].issuer() != space {
        if path.len() > MAX_HOPS {
            return Err(failure("the grant's chain is too long"));
        }
        let below = &path[0];
        let above = handle
            .delegations()
            .issued_to(below.issuer().clone())
            .perform(&tonk.operator)
            .await
            .map_err(failure)?
            .into_iter()
            .filter_map(|UcanDelegation(chain)| chain.proofs().last().cloned())
            .find(|parent| {
                // A device or account hop on a link's path is often a
                // powerline, which names no subject of its own.
                (*parent.subject() == subject || *parent.subject() == Subject::Any)
                    && parent.audience() == below.issuer()
                    && parent.expiration().is_none_or(|at| at.to_unix() > now)
                    && covers(parent.command().segments(), below.command().segments())
            })
            .ok_or_else(|| {
                failure(format!(
                    "nothing the agent's space holds to {} covers {} for {space}",
                    below.issuer(),
                    below.command(),
                ))
            })?;
        path.insert(0, above);
    }
    let mut certificates = path.into_iter();
    let mut chain = DelegationChain::new(
        certificates
            .next()
            .ok_or_else(|| failure("an empty grant"))?,
    );
    for certificate in certificates {
        chain = chain.push(certificate).map_err(failure)?;
    }
    Ok(chain)
}

fn covers(above: &[String], below: &[String]) -> bool {
    above.len() <= below.len() && above.iter().zip(below).all(|(a, b)| a == b)
}

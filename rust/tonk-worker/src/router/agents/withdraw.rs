//! Removing access a share link gave one of the account's agents.
//!
//! An agent that joins from a chat link holds
//! `space -> ... -> link key -> agent space`. The person is not in that
//! chain, so neither the profile nor the account root can revoke it. The
//! agent can: dialog lets a delegation's audience disclaim it, and the
//! access service then refuses every chain that presents it, whoever
//! presents it. Only the account opens the agent's key, so this runs after
//! the passkey, with the account it opened.
//!
//! This revokes the delegations the link issued to the agent. It does not
//! touch the link: whoever holds the link and its secret can still redeem
//! it for a fresh grant.

// The removal runs behind the passkey, in the browser; natively only
// tests reach it.
#![cfg_attr(
    not(any(test, all(target_arch = "wasm32", target_os = "unknown"))),
    allow(dead_code)
)]

use dialog_credentials::Signer;
use dialog_query::{Output as _, Query, Term};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use tonk_schema::prelude::DidExt as _;

use super::super::{agent_connections::failure, repository::CONTENT_BRANCH};
use super::{Issued, grants};
use crate::{TonkState, TonkWorkerError};

/// The most hops a link's chain is walked up before giving up.
const MAX_HOPS: usize = 16;

/// How a removal went: every grant the access service acknowledged is
/// gone from the agent space; any other is still there to try again.
#[derive(Debug, Default)]
pub(crate) struct Withdrawal {
    /// The grants the access service acknowledged revoking.
    pub(crate) revoked: Vec<Cid>,
    /// The grants it did not, and why.
    pub(crate) unresolved: Vec<(Cid, String)>,
}

impl Withdrawal {
    /// `Ok` when every grant was revoked, else a sentence for the person.
    pub(crate) fn finished(self) -> Result<usize, String> {
        match self.unresolved.first() {
            None => Ok(self.revoked.len()),
            Some((_, why)) => Err(format!(
                "{} of {} grants were not removed, so the agent may still have access: {why}",
                self.unresolved.len(),
                self.unresolved.len() + self.revoked.len(),
            )),
        }
    }
}

/// The link-issued grants to `agent` for `space` still in force: every
/// scoped leaf, read and write, including renewals.
pub(crate) async fn link_grants(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
) -> Result<Vec<DelegationChain>, TonkWorkerError> {
    Ok(grants(tonk, agent)
        .await?
        .into_iter()
        .filter(|(chain, _, issued)| *issued == Issued::Link && chain.subject() == Some(space))
        .map(|(chain, _, _)| chain)
        .collect())
}

/// Revoke, as the agent at `key`, every link-issued grant it holds to
/// `space`, with `root`, the account the passkey opened.
///
/// Checks that `root` is the signed-in account and that the agent is one
/// of its agents, opens the agent's sealed key with it, and publishes one
/// revocation per grant to the space's access service. A grant is dropped
/// from the agent space only once the service has acknowledged its
/// revocation; the agent space is then queued to sync, so the hosted agent
/// stops listing it.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn withdraw(
    tonk: &TonkState,
    root: &dialog_credentials::Ed25519Signer,
    key: &str,
    space: &Did,
) -> Result<Withdrawal, String> {
    let said = |error: TonkWorkerError| error.to_string();
    let linked = super::super::identity::local_root(tonk)
        .await
        .map_err(|error| format!("no account is signed in on this profile: {error}"))?
        .root_did;
    use dialog_varsig::Principal as _;
    if root.did() != linked {
        return Err("this passkey belongs to a different account".into());
    }
    let agent = super::find(tonk, key).await.map_err(said)?;
    let signer = Signer::from(super::super::agent_session::open_held(tonk, root, &agent).await?);
    let leaves = link_grants(tonk, &agent, space).await.map_err(said)?;
    if leaves.is_empty() {
        return Ok(Withdrawal::default());
    }
    let remote = home(tonk, &agent, space).await.map_err(said)?;

    let session = tonk
        .reactor
        .repository(agent.repo_key())
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| error.to_string())?;
    let mut outcome = Withdrawal::default();
    for leaf in leaves {
        let target = *leaf.proof_cids().last().ok_or("an empty grant")?;
        let revoked = async {
            let path = evidence(tonk, &agent, space, &leaf).await?;
            publish(&signer, &remote, &path, &target).await
        }
        .await;
        match revoked {
            Ok(()) => {
                session
                    .handle()
                    .delegations()
                    .retract(UcanDelegation(leaf))
                    .perform(&tonk.operator)
                    .await
                    .map_err(|error| error.to_string())?;
                outcome.revoked.push(target);
            }
            Err(error) => outcome.unresolved.push((target, error.to_string())),
        }
    }
    if !outcome.revoked.is_empty() {
        tonk.sync_queue
            .mark_dirty(agent.repo_key(), super::super::sync::now_millis());
    }
    tonk_common::log!(
        "agent {agent}: {} link grant(s) to {space} revoked, {} not",
        outcome.revoked.len(),
        outcome.unresolved.len()
    );
    Ok(outcome)
}

/// Sign the agent's revocation of `target`, witnessed by `path`, check it
/// as the access service does, send it, and check the service
/// acknowledged this revocation by this agent.
pub(crate) async fn publish(
    agent: &Signer,
    remote: &url::Url,
    path: &DelegationChain,
    target: &Cid,
) -> Result<(), TonkWorkerError> {
    let artifact = tonk_identity::revocation::mint_root_revocation(agent.clone(), path, target)
        .await
        .map_err(failure)?;
    let expected = tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(failure)?;
    let response = super::super::http::post_cbor(remote, &artifact).await?;
    let receipt: tonk_account::customer::RevokeReceipt =
        serde_json::from_slice(&response.body).map_err(failure)?;
    if receipt.revoked != *target || receipt.subject != expected.subject {
        return Err(failure(
            "the access service acknowledged a different revocation",
        ));
    }
    Ok(())
}

/// The public chain `space -> ... -> link key -> agent` that ends in
/// `leaf`, rebuilt from the certificates the agent space retains. Each
/// step up takes a certificate to the current issuer, for `space` or a
/// powerline, whose command covers the one below it.
pub(crate) async fn evidence(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
    leaf: &DelegationChain,
) -> Result<DelegationChain, TonkWorkerError> {
    let certificate = leaf
        .proofs()
        .last()
        .ok_or_else(|| failure("an empty grant"))?
        .clone();
    let session = tonk
        .reactor
        .repository(agent.repo_key())
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let now = Timestamp::now().to_unix();
    let subject = Subject::Specific(space.clone());
    let mut path = vec![certificate];
    while path[0].issuer() != space {
        if path.len() > MAX_HOPS {
            return Err(failure("the grant's chain is too long"));
        }
        let below = &path[0];
        let held = session
            .handle()
            .delegations()
            .issued_to(below.issuer().clone())
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        let count = held.len();
        let above = held
            .into_iter()
            .filter_map(|UcanDelegation(chain)| chain.proofs().last().cloned())
            .find(|parent| {
                // A device or account hop on the link's path is often a
                // powerline, which names no subject of its own.
                (*parent.subject() == subject || *parent.subject() == Subject::Any)
                    && parent.audience() == below.issuer()
                    && parent.expiration().is_none_or(|at| at.to_unix() > now)
                    && covers(parent.command().segments(), below.command().segments())
            })
            .ok_or_else(|| {
                failure(format!(
                    "the agent space does not hold the link's chain: nothing of its {count} \
                     grant(s) to {} covers {} for {space}",
                    below.issuer(),
                    below.command(),
                ))
            })?;
        path.insert(0, above);
    }
    let mut certificates = path.into_iter();
    let mut chain = DelegationChain::new(certificates.next().unwrap());
    for certificate in certificates {
        chain = chain.push(certificate).map_err(failure)?;
    }
    Ok(chain)
}

fn covers(above: &[String], below: &[String]) -> bool {
    above.len() <= below.len() && above.iter().zip(below).all(|(a, b)| a == b)
}

/// Where `space` is served, as the agent space's directory records it.
async fn home(tonk: &TonkState, agent: &Did, space: &Did) -> Result<url::Url, TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(agent.repo_key())
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<tonk_schema::SpaceHomeAddress> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::SpaceHomeAddress> {
            this: Term::from(space.this()),
            home_address: Term::var("home_address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    rows.into_iter()
        .find_map(|row| url::Url::parse(&row.home_address.0).ok())
        .ok_or_else(|| {
            TonkWorkerError::Conflict(
                "the agent's space does not say where that space is served".into(),
            )
        })
}

/// How long a removal waits for its passkey before the list stops
/// showing it as under way: the custody deadline, and some.
const PENDING_FOR_MS: f64 = 180_000.0;

/// Where a removal the person started stands, for the list to show.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "state")]
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
pub(crate) enum Removal {
    /// Waiting for the passkey.
    Pending,
    /// The passkey opened the account; revocations are being published.
    Working,
    /// It stopped, and why. What was not acknowledged is still listed.
    Failed { reason: String },
}

type Removals = std::sync::Mutex<std::collections::HashMap<(String, String), (Removal, f64)>>;

fn removals() -> &'static Removals {
    static REMOVALS: std::sync::OnceLock<Removals> = std::sync::OnceLock::new();
    REMOVALS.get_or_init(Default::default)
}

/// Record where the removal of `agent`'s access to `space` stands, or
/// clear it with `None` once it is done.
pub(crate) fn note(agent: &str, space: &str, removal: Option<Removal>) {
    let Ok(mut removals) = removals().lock() else {
        return;
    };
    let key = (agent.to_owned(), space.to_owned());
    match removal {
        Some(removal) => {
            removals.insert(key, (removal, super::super::sync::now_millis()));
        }
        None => {
            removals.remove(&key);
        }
    }
}

/// Where the removal of `agent`'s access to `space` stands. A removal
/// still waiting for its passkey after the deadline was dismissed or
/// never answered, so it is no longer reported.
pub(crate) fn status(agent: &str, space: &str) -> Option<Removal> {
    let removals = removals().lock().ok()?;
    let (removal, at) = removals.get(&(agent.to_owned(), space.to_owned()))?;
    let stale = super::super::sync::now_millis() - at > PENDING_FOR_MS;
    match removal {
        Removal::Pending | Removal::Working if stale => None,
        removal => Some(removal.clone()),
    }
}

/// The passkey continuation of `POST /api/agents/{key}/revoke` for
/// link-issued grants: open the account the passkey holds, then
/// [`withdraw`] as the agent and push the agent space.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn authorize(
    state: &super::super::AppState,
    custodian: &tonk_identity::custodian::Custodian,
    request: tonk_worker_api::AgentRevocation,
) -> Result<wasm_bindgen::JsValue, String> {
    note(&request.agent, &request.space, Some(Removal::Working));
    report(state, tonk_schema::ceremony_state::WORKING, &request.space).await;
    let outcome = authorize_inner(state, custodian, &request).await;
    match &outcome {
        Ok(()) => {
            note(&request.agent, &request.space, None);
            report(state, tonk_schema::ceremony_state::DONE, &request.space).await;
        }
        Err(reason) => {
            let reason = reason
                .split_once('\u{1f}')
                .map_or(reason.as_str(), |(_, message)| message)
                .to_owned();
            note(
                &request.agent,
                &request.space,
                Some(Removal::Failed {
                    reason: reason.clone(),
                }),
            );
            report(state, tonk_schema::ceremony_state::FAILED, &reason).await;
        }
    }
    outcome.map(|()| wasm_bindgen::JsValue::UNDEFINED)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn report(state: &super::super::AppState, status: &str, detail: &str) {
    let tonk = state.read().await;
    super::super::ceremony::report(&tonk, tonk_schema::ceremony::REVOKE_AGENT, status, detail)
        .await;
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn authorize_inner(
    state: &super::super::AppState,
    custodian: &tonk_identity::custodian::Custodian,
    request: &tonk_worker_api::AgentRevocation,
) -> Result<(), String> {
    let space: Did = request
        .space
        .parse()
        .map_err(|_| "that space's name is unreadable".to_string())?;
    let account = super::super::custody::held_account(custodian).await?;
    let Signer::Ed25519(root) = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    // Bring in grants a join from the chat kept since this browser last
    // synced the agent space. If that fails, what is here is withdrawn;
    // anything newer shows once it syncs, to remove again.
    if let Err(error) = super::super::sync::sync_repository(state, &request.agent).await {
        tonk_common::log!(
            "agent {}: not synced before removal: {error}",
            request.agent
        );
    }
    let withdrawn = {
        let tonk = state.read().await;
        withdraw(&tonk, &root, &request.agent, &space).await?
    };
    let finished = withdrawn.finished();
    // The hosted agent lists its spaces from the agent space. The access
    // service already refuses what was revoked; a failed push is left to
    // the background sync, which the removal queued.
    if let Err(error) = super::super::sync::sync_repository(state, &request.agent).await {
        tonk_common::log!("agent {}: removal not pushed yet: {error}", request.agent);
    }
    finished.map(|_| ())
}

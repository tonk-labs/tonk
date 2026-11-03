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

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_query::{Output as _, Query, Term};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::{Did, Principal as _};
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
pub(crate) async fn withdraw(
    tonk: &TonkState,
    root: &Ed25519Signer,
    key: &str,
    space: &Did,
) -> Result<Withdrawal, String> {
    let said = |error: TonkWorkerError| error.to_string();
    let linked = super::super::identity::local_root(tonk)
        .await
        .map_err(|error| format!("no account is signed in on this profile: {error}"))?
        .root_did;
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
/// step up takes a certificate for `space` to the current issuer whose
/// command covers the one below it.
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
        let above = held
            .into_iter()
            .filter_map(|UcanDelegation(chain)| chain.proofs().last().cloned())
            .find(|parent| {
                *parent.subject() == subject
                    && parent.audience() == below.issuer()
                    && parent.expiration().is_none_or(|at| at.to_unix() > now)
                    && covers(parent.command().segments(), below.command().segments())
            })
            .ok_or_else(|| failure("the agent space does not hold the link's chain"))?;
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

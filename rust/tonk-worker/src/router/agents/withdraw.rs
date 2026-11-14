//! Removing access a share link gave one of the account's agents.
//!
//! An agent that joins from a chat link holds
//! `space -> ... -> link key -> agent space`. The person is not in that
//! chain, so neither the profile nor the account root can revoke it. The
//! agent can: dialog lets a delegation's audience disclaim it, and the
//! access service then refuses every chain that presents it, whoever
//! presents it. The agent's key is the account's to open, so this runs
//! after the passkey, with the account it lent.
//!
//! This revokes the delegations the link issued to the agent. It does not
//! touch the link: whoever holds the link and its secret can still redeem
//! it for a fresh grant.

// What revokes runs behind the passkey, on a kernel; natively only the
// tests against an access service reach it.
#[cfg(target_arch = "wasm32")]
use tonk_kernel_world::world::sapling::kernel::credential::PersonaAuthority;
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
use {
    super::super::agent_connections::failure,
    super::super::agent_space::{self, AgentSpaceChange, AgentSpaceRecord},
    super::{Issued, grants},
    crate::{TonkState, TonkWorkerError},
    dialog_credentials::Signer,
    dialog_ucan_core::DelegationChain,
    dialog_varsig::Did,
    ipld_core::cid::Cid,
    tonk_schema::prelude::DidExt as _,
};

/// How a removal went: every grant the access service acknowledged is
/// gone from the agent space; any other is still there to try again.
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub(crate) struct Withdrawal {
    /// The grants the access service acknowledged revoking.
    pub(crate) revoked: Vec<Cid>,
    /// The grants it did not, and why.
    pub(crate) unresolved: Vec<(Cid, String)>,
}

#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
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

/// The link-issued grants to `agent` for `space` that `record`, what the
/// agent space holds, keeps in force: every scoped leaf, read and write,
/// including renewals.
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
async fn issued_by_links(
    tonk: &TonkState,
    agent: &Did,
    record: &AgentSpaceRecord,
    space: &Did,
) -> Result<Vec<DelegationChain>, TonkWorkerError> {
    Ok(grants(tonk, agent, record)
        .await?
        .into_iter()
        .filter(|(chain, _, issued)| *issued == Issued::Link && chain.subject() == Some(space))
        .map(|(chain, _, _)| chain)
        .collect())
}

/// [`issued_by_links`], of what the agent space holds now.
#[cfg(all(test, feature = "integration-tests"))]
pub(crate) async fn link_grants(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
) -> Result<Vec<DelegationChain>, TonkWorkerError> {
    let record = agent_space::read(tonk, agent.repo_key()).await?;
    issued_by_links(tonk, agent, &record, space).await
}

/// Revoke, as `agent` signing with `signer`, every link-issued grant it
/// holds to `space`.
///
/// Publishes one revocation per grant to the space's access service, each
/// witnessed by the chain the agent space holds it through. A grant is
/// dropped from the agent space only once the service has acknowledged its
/// revocation; the agent space is then queued to sync, so the hosted agent
/// stops listing it.
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
pub(crate) async fn withdraw(
    tonk: &TonkState,
    agent: &Did,
    signer: &Signer,
    space: &Did,
) -> Result<Withdrawal, String> {
    let said = |error: TonkWorkerError| error.to_string();
    let record = agent_space::read(tonk, agent.repo_key())
        .await
        .map_err(said)?;
    let leaves = issued_by_links(tonk, agent, &record, space)
        .await
        .map_err(said)?;
    if leaves.is_empty() {
        return Ok(Withdrawal::default());
    }
    let remote = record
        .homes
        .get(space.as_str())
        .and_then(|home| url::Url::parse(home).ok())
        .ok_or("the agent's space does not say where that space is served")?;

    let mut outcome = Withdrawal::default();
    let mut forget = AgentSpaceChange::default();
    for leaf in leaves {
        let target = *leaf.proof_cids().last().ok_or("an empty grant")?;
        let revoked = async {
            let path = evidence(&record, &target)?;
            publish(signer, &remote, &path, &target).await
        }
        .await;
        match revoked {
            Ok(()) => {
                forget
                    .forget
                    .push(agent_space::encode(&leaf).map_err(said)?);
                outcome.revoked.push(target);
            }
            Err(error) => outcome.unresolved.push((target, error.to_string())),
        }
    }
    if !forget.forget.is_empty() {
        agent_space::change(tonk, agent.repo_key(), &forget)
            .await
            .map_err(said)?;
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

/// The chain a revocation of the grant `target` is witnessed by, as the
/// agent space rebuilt it.
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
fn evidence(record: &AgentSpaceRecord, target: &Cid) -> Result<DelegationChain, TonkWorkerError> {
    for grant in &record.grants {
        if grant.chain()?.proof_cids().last() == Some(target) {
            return grant.evidence();
        }
    }
    Err(failure("the agent's space no longer holds that grant"))
}

/// Sign the agent's revocation of `target`, witnessed by `path`, check it
/// as the access service does, send it, and check the service
/// acknowledged this revocation by this agent.
#[cfg(any(all(test, feature = "integration-tests"), target_arch = "wasm32"))]
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

/// How long a removal waits for its passkey before the list stops
/// showing it as under way: the custody deadline, and some.
const PENDING_FOR_MS: f64 = 180_000.0;

/// Where a removal the person started stands, for the list to show.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "state")]
pub(crate) enum Removal {
    /// Waiting for the passkey.
    Pending,
    /// The passkey opened the account; revocations are being published.
    #[cfg(target_arch = "wasm32")]
    Working,
    /// It stopped, and why. What was not acknowledged is still listed.
    #[cfg(target_arch = "wasm32")]
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
        Removal::Pending if stale => None,
        #[cfg(target_arch = "wasm32")]
        Removal::Working if stale => None,
        removal => Some(removal.clone()),
    }
}

/// The passkey continuation of `POST /api/agents/{key}/revoke` for
/// link-issued grants, with the account lent as `persona`: open the agent's
/// key, then [`withdraw`] as the agent and push the agent space.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn authorized(
    state: &super::super::AppState,
    persona: &PersonaAuthority,
    request: tonk_worker_api::AgentRevocation,
) -> Result<(), String> {
    note(&request.agent, &request.space, Some(Removal::Working));
    report(state, tonk_schema::ceremony_state::WORKING, &request.space).await;
    let outcome = authorize_inner(state, persona, &request).await;
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
    outcome
}

#[cfg(target_arch = "wasm32")]
async fn report(state: &super::super::AppState, status: &str, detail: &str) {
    let tonk = state.read().await.clone();
    super::super::ceremony::report(&tonk, tonk_schema::ceremony::REVOKE_AGENT, status, detail)
        .await;
}

#[cfg(target_arch = "wasm32")]
async fn authorize_inner(
    state: &super::super::AppState,
    persona: &PersonaAuthority,
    request: &tonk_worker_api::AgentRevocation,
) -> Result<(), String> {
    let space: Did = request
        .space
        .parse()
        .map_err(|_| "that space's name is unreadable".to_string())?;
    let linked = {
        let tonk = state.read().await.clone();
        super::super::identity::local_root(&tonk)
            .await
            .map_err(|error| format!("no account is signed in on this profile: {error}"))?
            .root_did
    };
    if persona.did() != linked.as_str() {
        return Err("this passkey belongs to a different account".into());
    }
    // Bring in grants a join from the chat kept since the agent space last
    // synced. If that fails, what is here is withdrawn; anything newer
    // shows once it syncs, to remove again.
    if let Err(error) = agent_space::push(state, &request.agent).await {
        tonk_common::log!(
            "agent {}: not synced before removal: {error}",
            request.agent
        );
    }
    let withdrawn = {
        let tonk = state.read().await.clone();
        let agent = super::find(&tonk, &request.agent)
            .await
            .map_err(|error| error.to_string())?;
        let signer = super::super::agent_session::open_agent(&tonk, persona, &agent).await?;
        withdraw(&tonk, &agent, &signer, &space).await?
    };
    let finished = withdrawn.finished();
    // The hosted agent lists its spaces from the agent space. The access
    // service already refuses what was revoked; a failed push is left to
    // the background sync, which the removal queued.
    if let Err(error) = agent_space::push(state, &request.agent).await {
        tonk_common::log!("agent {}: removal not pushed yet: {error}", request.agent);
    }
    finished.map(|_| ())
}

//! Letting someone into a space from the palette: `invite <agent>` and
//! `invite <email>`. Both make the invitee a writer, and neither hands
//! anyone a link: each leaves the grant where the invitee already looks.
//!
//! - An agent's grant is kept in the agent's own space, beside the grants
//!   its approval made, which is where the hosted agent lists the spaces
//!   it can reach. The agent finds the space the next time it lists them.
//! - An address is resolved to the account registered under it, the way
//!   signing in resolves one, and the space is delegated to that account
//!   as a scoped invite. Its chain is left in the space as the account's
//!   ticket, the way publishing leaves the public principal's: a device of
//!   that account opening the space's address claims it through the
//!   account's powerline (see `join::open_space`).
//!
//! An agent's grant is signed by this device, as an invite or a
//! publication is, so it runs `space → … → device → agent`. Signing this
//! device out takes it with it; the approval page signs with the account
//! under the passkey instead, which a palette command cannot ask for.

use dialog_remote_ucan::UcanAddress;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use tonk_common::log;
use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS};
use tonk_schema::prelude::DidExt as _;
use url::Url;

use super::agent_session::{Granted, grant, record_grants};
use super::create_invite::{RemoteRequirement, explain_refusal, resolve_remote_url};
use super::http::{HttpError, get};
use crate::{TonkWorkerError, router::AppState};

/// Let the agent `agent` read and write the space at `repo`.
///
/// # Errors
///
/// [`TonkWorkerError::NotFound`] when `agent` is not one of this
/// account's agents or `repo` is not one of its spaces;
/// [`TonkWorkerError::Forbidden`] when `repo` is an agent's own space or
/// this device holds no authority to share it;
/// [`TonkWorkerError::Conflict`] when the space does not sync anywhere a
/// hosted agent could reach it.
pub(crate) async fn invite_agent(
    state: &AppState,
    agent: &Did,
    repo: &str,
) -> Result<(), TonkWorkerError> {
    // A clone, not a guard: inviting pulls, proves and pushes over the network.
    let tonk = state.read().await.clone();
    // Either may be listed in the account's directory and not be on this
    // device yet; opening it would mount it, so inviting does too.
    super::adopt::mount_on_first_use(&tonk, state, agent.repo_key()).await;
    super::adopt::mount_on_first_use(&tonk, state, repo).await;
    let agent = super::agents::find(&tonk, agent.repo_key()).await?;
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    let subject = repository.did();
    // An agent reaches other agents only as contacts, never as spaces.
    if subject == agent || super::agents::is_agent(&tonk, &subject).await {
        return Err(TonkWorkerError::Forbidden(format!(
            "'{repo}' is an agent's own space, not one to invite an agent into"
        )));
    }
    let remote = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(reason) => {
            let reason = explain_refusal(&tonk, reason).await;
            return Err(TonkWorkerError::Conflict(format!(
                "a hosted agent cannot reach '{repo}': {} ({})",
                reason.detail(),
                reason.code()
            )));
        }
    };
    let name = super::repository::repository_display_name(&tonk, &repository, repo)
        .await
        .unwrap_or_else(|| repo.to_owned());

    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let signer = tonk.profile.credential().signer().clone();
    let chains = grant(
        &tonk,
        &signer,
        &agent,
        &subject,
        AgentAccess::ReadWrite,
        &remote,
        now,
        expires,
    )
    .await?;
    record_grants(
        &tonk,
        agent.repo_key(),
        &[Granted {
            subject: subject.clone(),
            name,
            home: remote,
            chains,
        }],
    )
    .await?;
    // The hosted agent reads its grants from the agent space's remote.
    tonk.sync_queue
        .mark_dirty(agent.repo_key(), super::sync::now_millis());
    log!("invited agent {agent} into '{repo}' to read and write");
    Ok(())
}

/// Let the account registered under `email` read and write the space at
/// `repo`, and answer that account's DID.
///
/// A ticket the space already keeps for the account, from an earlier
/// invite, is replaced: the service never overwrites one, so it is taken
/// back first. The earlier grant is not revoked by that; it stays
/// retained on the space, revocable like any invite.
///
/// # Errors
///
/// [`TonkWorkerError::Router`] when `email` is not an address;
/// [`TonkWorkerError::NotFound`] when no confirmed account is registered
/// under it; [`TonkWorkerError::Conflict`] when the lookup cannot be
/// answered or the space does not keep the ticket; and whatever minting
/// the invite refuses with.
pub(crate) async fn invite_by_email(
    state: &AppState,
    email: &str,
    repo: &str,
) -> Result<Did, TonkWorkerError> {
    let origin = {
        let tonk = state.read().await;
        super::email_invitation::origin(&tonk).await?
    };
    let account = account_for_address(&origin, email).await?;

    // Minting takes the state itself, so no guard is held across it.
    let request = tonk_worker_api::CreateInviteRequest {
        recipient_root: Some(account.clone()),
        ..Default::default()
    };
    let request_origin = crate::axum::RequestOrigin::parse(origin.as_str()).map_err(|_| {
        TonkWorkerError::Internal(format!("the lookup origin {origin} is unreadable"))
    })?;
    let axum::Json(minted) = super::create_invite::mint_invite(
        state.clone(),
        repo.to_owned(),
        request_origin,
        request,
        None,
        None,
    )
    .await?;
    let invite = tonk_invite::Invite::parse_url(minted.url().as_str())
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("the minted invite: {error}")))?;

    // A clone, not a guard: the ticket is issued over the network.
    let tonk = state.read().await.clone();
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    // The mint resolved the same remote a moment ago, so a refusal here
    // is the space losing its upstream in between.
    let RemoteRequirement::Ready(remote) = resolve_remote_url(&tonk, &repository).await? else {
        return Err(TonkWorkerError::Conflict(format!(
            "'{repo}' stopped syncing while the invite was made"
        )));
    };
    let site = || SiteAddress::from(UcanAddress::new(remote.access_url.as_str()));
    if let Err(error) =
        tonk_account::ticket::withdraw(site(), &repository.did(), &account, &tonk.operator).await
    {
        log!("invite: the earlier ticket for {account} was not taken back: {error}");
    }
    tonk_account::ticket::issue(site(), &invite.chain, &tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Conflict(format!(
                "the space did not keep the invited account's ticket: {error}"
            ))
        })?;
    log!("invited {account} into '{repo}' to read and write");
    Ok(account)
}

/// The DID of the confirmed account registered under `email`, as the
/// access service at `origin` answers its address lookup: the same
/// `/customer/{domain}/{local}/did.json` registration and sign-in ask.
///
/// The service is the one this account uses, never one a document names.
/// Only a confirmed, active account answers: an address awaiting its
/// confirmation could still be taken by someone else, and an invite to a
/// suspended one could not be used.
pub(crate) async fn account_for_address(origin: &Url, email: &str) -> Result<Did, TonkWorkerError> {
    let (domain, local) = super::email_status::split_address(email)
        .ok_or_else(|| TonkWorkerError::Router(format!("'{email}' is not an email address")))?;
    let endpoint = origin
        .join(&format!("customer/{domain}/{local}/did.json"))
        .map_err(|error| TonkWorkerError::Internal(format!("lookup address: {error}")))?;
    let refused = |status: u16| match status {
        404 => TonkWorkerError::NotFound(format!("no Tonk account is registered under {email}")),
        202 => TonkWorkerError::Conflict(format!(
            "the account registered under {email} has not confirmed its address yet"
        )),
        410 => {
            TonkWorkerError::Conflict(format!("the account registered under {email} is suspended"))
        }
        status => TonkWorkerError::Conflict(format!(
            "the account lookup for {email} could not be answered ({status})"
        )),
    };
    let response = match get(&endpoint).await {
        Ok(response) => response,
        Err(HttpError::Upstream(failure)) => return Err(refused(failure.status)),
        Err(error) => {
            return Err(TonkWorkerError::Conflict(format!(
                "the account lookup for {email} did not complete: {error}"
            )));
        }
    };
    if response.status != 200 {
        return Err(refused(response.status));
    }
    account_in_document(&response.body).ok_or_else(|| {
        TonkWorkerError::Conflict(format!(
            "the account lookup for {email} answered a document naming no active account"
        ))
    })
}

/// The account a customer document names, when it names an active one:
/// its `alsoKnownAs` `did:key`, which must be the key the document
/// verifies with.
fn account_in_document(body: &[u8]) -> Option<Did> {
    let document: serde_json::Value = serde_json::from_slice(body).ok()?;
    if document["status"] != "Active" || document["deactivated"] == true {
        return None;
    }
    let account: Did = document["alsoKnownAs"][0].as_str()?.parse().ok()?;
    let key = document["verificationMethod"][0]["publicKeyMultibase"].as_str()?;
    (account.as_str() == format!("did:key:{key}")).then_some(account)
}

/// Run the [`InviteAgent`] command for the space it names.
///
/// [`InviteAgent`]: tonk_schema::command::InviteAgent
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::InviteAgent> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::InviteAgent) {
        let (Ok(agent), Ok(space)) = (
            command.agent.0.to_string().parse::<Did>(),
            command.space.0.to_string().parse::<Did>(),
        ) else {
            log!("space/invite-agent: the command names no agent or no space; skipping");
            return;
        };
        let key = space.repo_key().to_owned();
        if !self.may_target_space(&key) {
            log!("space/invite-agent: refused to target '{key}' from another space");
            return;
        }
        if let Err(error) = invite_agent(self.state(), &agent, &key).await {
            log!("space/invite-agent '{key}' failed: {error}");
        }
    }
}

/// Run the [`InviteByEmail`] command for the space it names.
///
/// [`InviteByEmail`]: tonk_schema::command::InviteByEmail
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::InviteByEmail>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::InviteByEmail) {
        let Ok(space) = command.space.0.to_string().parse::<Did>() else {
            log!("space/invite-email: the command names no space; skipping");
            return;
        };
        let key = space.repo_key().to_owned();
        if !self.may_target_space(&key) {
            log!("space/invite-email: refused to target '{key}' from another space");
            return;
        }
        if let Err(error) = invite_by_email(self.state(), command.email.0.trim(), &key).await {
            log!("space/invite-email '{key}' failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::account_in_document;
    use serde_json::json;

    const KEY: &str = "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    fn document(status: &str, also: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "id": "did:web:tonk.test:customer:example.com:bob",
            "alsoKnownAs": [also],
            "status": status,
            "verificationMethod": [{ "publicKeyMultibase": KEY }],
        }))
        .unwrap()
    }

    /// An active customer's document names its account by the key it
    /// verifies with.
    #[dialog_common::test]
    fn it_reads_the_account_an_active_document_names() {
        let account = account_in_document(&document("Active", &format!("did:key:{KEY}")));
        assert_eq!(
            account.map(|did| did.to_string()),
            Some(format!("did:key:{KEY}"))
        );
    }

    /// A pending or suspended customer, a deactivated document, or one
    /// whose `alsoKnownAs` is not its own key names no account to invite.
    #[dialog_common::test]
    fn it_names_no_account_unless_the_document_is_active_and_consistent() {
        let own = format!("did:key:{KEY}");
        assert!(account_in_document(&document("Pending", &own)).is_none());
        assert!(account_in_document(&document("Suspended", &own)).is_none());
        let other = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";
        assert!(account_in_document(&document("Active", other)).is_none());
        let mut deactivated: serde_json::Value =
            serde_json::from_slice(&document("Active", &own)).unwrap();
        deactivated["deactivated"] = json!(true);
        assert!(account_in_document(&serde_json::to_vec(&deactivated).unwrap()).is_none());
        assert!(account_in_document(b"not json").is_none());
    }
}

#[cfg(all(test, feature = "integration-tests", not(target_arch = "wasm32")))]
mod integration {
    use std::sync::Arc;

    use axum::extract::State;
    use dialog_capability::Subject;
    use dialog_credentials::Ed25519Signer;
    use dialog_effects::Use;
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::RepositoryExt as _;
    use dialog_varsig::{Did, Principal as _};
    use tonk_schema::Invitation;
    use tonk_worker_api::JoinFailureKind;

    use super::{invite_agent, invite_by_email};
    use crate::TonkWorkerError;
    use crate::router::account_state::tests::{device_on, ready_account_state};
    use crate::router::{AppState, repository};

    /// Whether `state`'s device can prove `/use` on `space`: write it.
    async fn writes(state: &AppState, space: &Did) -> bool {
        let tonk = state.read().await;
        tonk.profile
            .access()
            .prove(Subject::from(space.clone()).attenuate(Use))
            .audience(&tonk.operator)
            .perform(&tonk.operator)
            .await
            .is_ok()
    }

    /// The invitations recorded on the space at `key`.
    async fn invitations(state: &AppState, key: &str) -> usize {
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(key)
            .branch(repository::CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        session
            .handle()
            .query()
            .select(Query::<Invitation> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                inviter: Term::var("inviter"),
                audience: Term::var("audience"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap()
            .len()
    }

    /// The account's agents as `GET /api/agents` answers them.
    async fn listed(state: &AppState) -> Vec<serde_json::Value> {
        let axum::Json(agents) = crate::router::agents::list(State(state.clone()))
            .await
            .unwrap();
        serde_json::from_value(serde_json::to_value(agents).unwrap()).unwrap()
    }

    /// A synced space of `state`'s, pushed so its service serves it, and
    /// its subject.
    async fn synced_space(state: &AppState, name: &str, remote: &str) -> (String, Did) {
        let key = repository::create_space_inner(state, name, None)
            .await
            .unwrap();
        repository::enable_sync_inner(state, &key, remote)
            .await
            .unwrap();
        crate::router::sync::sync_repository(state, &key)
            .await
            .unwrap();
        let tonk = state.read().await;
        let subject = tonk
            .profile
            .space(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap()
            .did();
        (key, subject)
    }

    /// Inviting an address resolves the confirmed account registered
    /// under it and leaves it a ticket: that account's device opening the
    /// space's bare address joins it as a writer, though the space is not
    /// published. Another account opening the same address is refused as
    /// a private space, and holds nothing.
    #[dialog_common::test]
    async fn an_invited_address_opens_the_space_as_a_writer() {
        let (alice, service, _alice_root, remote) = ready_account_state(None).await;
        let alice: AppState = Arc::new(crate::StateLock::new(alice));
        let bob_root = Ed25519Signer::generate().await.unwrap();
        let mallory_root = Ed25519Signer::generate().await.unwrap();
        let email = "bob+invite@example.com";
        service
            .address
            .activate_customer(&bob_root, email)
            .await
            .unwrap();
        service
            .address
            .activate_customer(&mallory_root, "mallory@example.com")
            .await
            .unwrap();
        let (key, subject) = synced_space(&alice, "Invited garden", &remote).await;

        let invited = invite_by_email(&alice, " BOB+invite@example.com", &key)
            .await
            .expect("the owner invites a registered address");
        assert_eq!(invited, bob_root.did());
        crate::router::sync::sync_repository(&alice, &key)
            .await
            .unwrap();

        let address = tonk_invite::Ticket::public(subject.clone(), &remote.parse().unwrap())
            .unwrap()
            .to_url()
            .unwrap();

        let mallory: AppState = Arc::new(crate::StateLock::new(
            device_on(&mallory_root, &remote, None).await,
        ));
        let refused = {
            let tonk = mallory.read().await;
            crate::router::join::join_invite(&tonk, &address).await
        };
        let Err(refused) = refused else {
            panic!("an account that was not invited opened the space");
        };
        assert!(
            matches!(refused.kind(), JoinFailureKind::Private),
            "{refused:?}"
        );
        assert!(!writes(&mallory, &subject).await);

        let bob: AppState = Arc::new(crate::StateLock::new(
            device_on(&bob_root, &remote, None).await,
        ));
        assert!(!writes(&bob, &subject).await);
        let outcome = {
            let tonk = bob.read().await;
            crate::router::join::join_invite(&tonk, &address)
                .await
                .expect("the invited account opens the space by its address")
        };
        assert_eq!(outcome.subject, subject);
        assert!(!outcome.renewed, "the space is new on this device");
        assert!(writes(&bob, &subject).await, "the invitee writes");
    }

    /// An address nobody registered is refused as not found, and nothing
    /// is minted for it.
    #[dialog_common::test]
    async fn an_unregistered_address_is_not_invited() {
        let (alice, _service, _alice_root, remote) = ready_account_state(None).await;
        let alice: AppState = Arc::new(crate::StateLock::new(alice));
        let (key, _) = synced_space(&alice, "Quiet garden", &remote).await;
        let before = invitations(&alice, &key).await;
        let refused = invite_by_email(&alice, "nobody@example.com", &key)
            .await
            .unwrap_err();
        assert!(matches!(refused, TonkWorkerError::NotFound(_)), "{refused}");
        assert_eq!(invitations(&alice, &key).await, before);
    }

    /// Inviting an agent keeps a read-and-write grant in its space, which
    /// the agents list then shows; an agent's own space is not one an
    /// agent is invited into, and the refusal grants nothing.
    #[dialog_common::test]
    async fn an_invited_agent_writes_the_space() {
        let (tonk, _service, _root, remote) = ready_account_state(None).await;
        let state: AppState = Arc::new(crate::StateLock::new(tonk));
        let (key, subject) = synced_space(&state, "Agent garden", &remote).await;
        let seed = zeroize::Zeroizing::new([83u8; 32]);
        let agent = Ed25519Signer::import(&*seed).await.unwrap().did();
        let agent_key = repository::create_space_from_seed(&state, "Claude", None, seed)
            .await
            .unwrap();
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&agent_key)
                .branch(repository::CONTENT_BRANCH)
                .transaction()
                .assert(tonk_schema::agent::AgentSpace::new(
                    agent.to_string().parse().unwrap(),
                ))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
        }

        let refused = invite_agent(&state, &agent, &agent_key).await.unwrap_err();
        assert!(
            matches!(refused, TonkWorkerError::Forbidden(_)),
            "{refused}"
        );
        assert_eq!(
            listed(&state).await[0]["spaces"],
            serde_json::json!([]),
            "the refusal granted nothing"
        );

        invite_agent(&state, &agent, &key)
            .await
            .expect("the owner invites its agent");
        let agents = listed(&state).await;
        let granted = agents[0]["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|space| space["subject"] == subject.as_str())
            .cloned()
            .expect("the space is listed among the agent's");
        assert_eq!(granted["access"], "write");
        assert_eq!(granted["name"], "Agent garden");
    }
}

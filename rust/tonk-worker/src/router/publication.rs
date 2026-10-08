//! Publishing a space: making it readable by anyone, and private again.
//!
//! Publishing is an invite to the public principal (see
//! [`tonk_invite::public`]) that grants `/use/get` instead of `/use`.
//! It is minted, recorded and retained the way an invite is, so the
//! revocation machinery that removes an invitee removes it too, and its
//! chain is left in the space as the public principal's ticket, which is
//! what a visitor to the space's bare address claims.
//!
//! Only someone holding `/` on the space publishes. A member holding
//! `/use` could sign the delegation, but could never revoke it, so the
//! space would stay public until an admin noticed.

use dialog_capability::Subject;
use dialog_effects::{Use, method};
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan::UcanAddress;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use dialog_ucan::UcanDelegation;
use dialog_varsig::Did;
use tonk_common::log;
use tonk_invite::{Ticket, home_address_meta};
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{Invitation, InvitationExecution};

use super::create_invite::{RemoteRequirement, explain_refusal, resolve_remote_url};
use super::revoke_invite::{account_authority, leaf_cid, publish_revocation, retract_leaf};
use crate::{TonkState, TonkWorkerError};

const CONTENT_BRANCH: &str = "main";

/// The invitation kind a publication is recorded under.
pub(crate) const PUBLIC_KIND: &str = "public";

/// The command a publication grants: reads, and nothing else.
const READ_COMMAND: &str = "/use/get";

/// Publish the space at `repo` and answer its public address,
/// `{origin}/space/{did}`. Publishing a space already published answers
/// the same address and mints nothing.
///
/// # Errors
///
/// [`TonkWorkerError::Forbidden`] when this account holds no `/` chain
/// on the space; [`TonkWorkerError::Conflict`] when the space has no sync
/// service to publish through, or the service does not keep the ticket.
pub(crate) async fn publish(tonk: &TonkState, repo: &str) -> Result<String, TonkWorkerError> {
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    let subject = repository.did();
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    account_authority(tonk, session.handle(), &subject)
        .await
        .map_err(|_| {
            TonkWorkerError::Forbidden("only an admin of the space can publish it".into())
        })?;

    let remote = match resolve_remote_url(tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote,
        RemoteRequirement::Refused(reason) => {
            let reason = explain_refusal(tonk, reason).await;
            return Err(TonkWorkerError::Conflict(format!(
                "cannot publish '{repo}': {} ({})",
                reason.detail(),
                reason.code()
            )));
        }
    };
    let address = Ticket::public(subject.clone(), &remote.access_url)
        .and_then(|ticket| ticket.to_url())
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let public = tonk_invite::public::did()
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    if !publications(tonk, session.handle(), &public)
        .await?
        .is_empty()
    {
        return Ok(address);
    }

    let delegation: UcanDelegation = tonk
        .profile
        .access()
        .claim(
            Subject::from(subject.clone())
                .attenuate(Use)
                .attenuate(method::Get),
        )
        .delegate(public.clone())
        .meta(home_address_meta(&remote.access_url))
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("failed to delegate: {error}")))?;
    let chain = delegation.into_chain();

    // The ticket first: until the space keeps it, nothing can claim the
    // grant, so a publication that stops here leaves no record claiming
    // the space is public when it is not.
    tonk_account::ticket::issue(
        SiteAddress::from(UcanAddress::new(remote.access_url.as_str())),
        &chain,
        &tonk.operator,
    )
    .await
    .map_err(|error| {
        TonkWorkerError::Conflict(format!("the space did not keep its public ticket: {error}"))
    })?;

    let invitation = Invitation::from_chain(&chain)
        .ok_or_else(|| TonkWorkerError::Internal("the publication names no space".into()))?;
    let execution = InvitationExecution::new(&invitation, PUBLIC_KIND);
    tonk.reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(invitation)
        .assert(execution)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to record the publication: {error}"))
        })?;
    super::create_invite::retain_invite_authority(tonk, repo, &chain).await?;
    log!("published '{repo}' at {address}");
    Ok(address)
}

/// Make the space at `repo` private again: take its public ticket back,
/// so the next visitor finds none, then revoke each publication, so a
/// reader who already claimed one stops reading.
///
/// # Errors
///
/// [`TonkWorkerError::Forbidden`] when this account holds no `/` chain
/// on the space, or the service refuses the revocation.
pub(crate) async fn unpublish(tonk: &TonkState, repo: &str) -> Result<(), TonkWorkerError> {
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    let subject = repository.did();
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("space '{repo}': {error}")))?;
    account_authority(tonk, session.handle(), &subject)
        .await
        .map_err(|_| {
            TonkWorkerError::Forbidden("only an admin of the space can unpublish it".into())
        })?;
    let public = tonk_invite::public::did()
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;

    if let RemoteRequirement::Ready(remote) = resolve_remote_url(tonk, &repository).await? {
        tonk_account::ticket::withdraw(
            SiteAddress::from(UcanAddress::new(remote.access_url.as_str())),
            &subject,
            &public,
            &tonk.operator,
        )
        .await
        .map_err(|error| {
            TonkWorkerError::Conflict(format!(
                "the space did not give up its public ticket: {error}"
            ))
        })?;
    }

    for (invitation, execution) in publications(tonk, session.handle(), &public).await? {
        match super::revoke_invite::prove_path_at(
            session.handle(),
            tonk,
            &subject,
            &public,
            READ_COMMAND,
        )
        .await
        {
            Ok(path) => {
                let target = leaf_cid(&path)?;
                publish_revocation(tonk, repo, &repository, session.handle(), &path, &target)
                    .await?;
                retract_leaf(tonk, session.handle(), &path).await;
            }
            // Nothing proves to the public principal: the publication was
            // revoked already, or its chain never retained. Either way
            // there is nothing left to revoke, only the record.
            Err(error) => log!("unpublish: no path to revoke: {error}"),
        }
        tonk.reactor
            .repository(repo)
            .branch(CONTENT_BRANCH)
            .transaction()
            .retract(invitation)
            .retract(execution)
            .commit()
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to retract the publication: {error}"))
            })?;
    }
    log!("unpublished '{repo}'");
    Ok(())
}

/// The publications recorded on the space's content branch: invitations
/// to the public principal, with their `public` kind.
async fn publications(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    public: &Did,
) -> Result<Vec<(Invitation, InvitationExecution)>, TonkWorkerError> {
    let invitations: Vec<Invitation> = branch
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            inviter: Term::var("inviter"),
            audience: Term::from(tonk_schema::domain::invitation::Audience(public.this())),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("publication query: {error:?}")))?;
    let mut publications = Vec::new();
    for invitation in invitations {
        let executions: Vec<InvitationExecution> = branch
            .query()
            .select(Query::<InvitationExecution> {
                this: Term::from(invitation.this.clone()),
                kind: Term::from(tonk_schema::domain::invitation_execution::Kind(
                    PUBLIC_KIND.to_owned(),
                )),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("publication kind query: {error:?}"))
            })?;
        if let Some(execution) = executions.into_iter().next() {
            publications.push((invitation, execution));
        }
    }
    Ok(publications)
}

/// Run the [`PublishSpace`] command for the space it names.
///
/// [`PublishSpace`]: tonk_schema::command::PublishSpace
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::PublishSpace> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::PublishSpace) {
        let Ok(space) = command.space.0.to_string().parse::<Did>() else {
            log!("space/publish: the command names no space; skipping");
            return;
        };
        let key = space.repo_key().to_owned();
        if !self.may_target_space(&key) {
            log!("space/publish: refused to target '{key}' from another space");
            return;
        }
        let tonk = self.state().read().await;
        if let Err(error) = publish(&tonk, &key).await {
            log!("space/publish '{key}' failed: {error}");
        }
    }
}

/// Run the [`UnpublishSpace`] command for the space it names.
///
/// [`UnpublishSpace`]: tonk_schema::command::UnpublishSpace
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::UnpublishSpace>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::UnpublishSpace) {
        let Ok(space) = command.space.0.to_string().parse::<Did>() else {
            log!("space/unpublish: the command names no space; skipping");
            return;
        };
        let key = space.repo_key().to_owned();
        if !self.may_target_space(&key) {
            log!("space/unpublish: refused to target '{key}' from another space");
            return;
        }
        let tonk = self.state().read().await;
        if let Err(error) = unpublish(&tonk, &key).await {
            log!("space/unpublish '{key}' failed: {error}");
        }
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use dialog_repository::RepositoryExt as _;
    use dialog_ucan_core::DelegationChain;
    use dialog_varsig::Principal as _;

    use super::{PUBLIC_KIND, publications, publish, unpublish};

    /// Claim the ticket `space` keeps for the public principal, signed as
    /// that principal, from the service at `address`.
    async fn claim_public(address: &str, space: &dialog_varsig::Did) -> Option<Vec<u8>> {
        let ticket = tonk_invite::Ticket::public_for_url(address)
            .expect("the address parses")
            .expect("a space's bare address is its public ticket");
        dialog_remote_ucan::claim(
            &dialog_remote_ucan::UcanAddress::new(ticket.remote().as_str()),
            ticket.holder().await.expect("the public key derives"),
            space,
        )
        .await
        .expect("the serving host answers the claim")
    }

    /// Publishing against a live access service leaves a `/use/get` ticket
    /// for the public principal that a visitor to the space's bare address
    /// claims, and unpublishing takes it back so the next visitor finds
    /// none, along with the record the bar reads.
    #[dialog_common::test]
    async fn it_publishes_a_space_for_anyone_to_read_and_takes_it_back() {
        let (tonk, service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let recipient =
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([7u8; 32]))
                .secret()
                .did();
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "publication-test".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .expect("the root persists");
        let state: crate::router::AppState = std::sync::Arc::new(tokio::sync::RwLock::new(tonk));
        let key = super::super::repository::create_space_inner(&state, "Published", None)
            .await
            .expect("the space creates");
        super::super::repository::enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");
        let space: dialog_varsig::Did = key.parse().expect("the key is the space's DID");
        let public = tonk_invite::public::did()
            .await
            .expect("the public key derives");

        {
            let tonk = state.read().await;
            let address = publish(&tonk, &key).await.expect("the founder publishes");
            let serving = url::Url::parse(&remote).expect("the fixture remote is a URL");
            let parsed = url::Url::parse(&address).expect("the address is a URL");
            assert_eq!(
                parsed.origin(),
                serving.origin(),
                "rooted on the serving host"
            );
            assert_eq!(parsed.path(), format!("/space/{key}"));
            assert_eq!(
                parsed.fragment(),
                None,
                "the public seed is everyone's: no fragment"
            );

            let bytes = claim_public(&address, &space)
                .await
                .expect("a published space keeps the public principal's ticket");
            let chain = DelegationChain::try_from(bytes.as_slice()).expect("the ticket is a chain");
            assert_eq!(chain.subject(), Some(&space));
            assert_eq!(chain.audience(), &public);
            let leaf = chain.proofs().last().expect("the chain has a leaf");
            assert_eq!(
                leaf.command().to_string(),
                "/use/get",
                "the public principal is granted reads and nothing else"
            );

            // Publishing again answers the same address and records nothing
            // new: the ticket cell takes one write.
            assert_eq!(publish(&tonk, &key).await.expect("republish"), address);
            let branch = tonk
                .reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("content branch opens");
            let recorded = publications(&tonk, branch.handle(), &public)
                .await
                .expect("publications read");
            assert_eq!(recorded.len(), 1, "one publication is recorded");
            assert_eq!(recorded[0].1.kind.0, PUBLIC_KIND);

            drop(branch);
            drop(tonk);

            // A visitor with no access opens it from the bare address as a
            // reader, and a second open finds the replica it already has.
            let reader = crate::router::command::tests::native::test_state().await;
            {
                let reader = reader.read().await;
                let opened = crate::router::join::join_invite(&reader, &address)
                    .await
                    .expect("a visitor opens a published space");
                assert_eq!(opened.subject, space);
                assert!(!opened.renewed, "the first open installs a replica");
                let again = crate::router::join::join_invite(&reader, &address)
                    .await
                    .expect("a second open succeeds");
                assert!(again.renewed, "the second open finds it here");
            }

            let tonk = state.read().await;
            unpublish(&tonk, &key)
                .await
                .expect("the founder unpublishes");
            assert!(
                claim_public(&address, &space).await.is_none(),
                "an unpublished space keeps no ticket for the next visitor"
            );

            // The next visitor is told it is private, and installs nothing.
            let newcomer = crate::router::command::tests::native::test_state().await;
            {
                let newcomer = newcomer.read().await;
                let refused = crate::router::join::join_invite(&newcomer, &address)
                    .await
                    .err()
                    .expect("an unpublished space does not open");
                assert_eq!(refused.kind(), tonk_worker_api::JoinFailureKind::Private);
                assert!(
                    !crate::router::join::find_replica_for_subject(&newcomer, &space)
                        .await
                        .expect("replica lookup"),
                    "a refused visit installs no replica"
                );
            }

            // The reader who claimed before stops reading: the revocation
            // reaches the delegation it already holds.
            {
                let reader = reader.read().await;
                let branch = reader
                    .profile
                    .space(key.as_str())
                    .load()
                    .perform(&reader.operator)
                    .await
                    .expect("the reader's replica loads")
                    .branch("main")
                    .open()
                    .perform(&reader.operator)
                    .await
                    .expect("the reader's branch opens");
                let refused = branch
                    .pull()
                    .perform(&reader.operator)
                    .await
                    .expect_err("a revoked reader's pull is refused");
                assert!(
                    matches!(
                        crate::router::sync::authorization_reason(&refused),
                        Some(dialog_capability::access::AuthorizeError::Revoked { .. })
                    ),
                    "refused as revoked, got {refused}"
                );
            }
            let branch = tonk
                .reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("content branch opens");
            assert!(
                publications(&tonk, branch.handle(), &public)
                    .await
                    .expect("publications read")
                    .is_empty(),
                "the record the bar reads goes with it"
            );
        }

        let account_key = {
            let tonk = state.read().await;
            crate::router::account_state::require_ready_account_state(&tonk)
                .await
                .expect("the linked account is ready")
                .key
                .clone()
        };
        let tonk = std::sync::Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("the state has no other holders"))
            .into_inner();
        crate::router::account_state::tests::discard(tonk, &account_key);
        drop(service);
    }
}

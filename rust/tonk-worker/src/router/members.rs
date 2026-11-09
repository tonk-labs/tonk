//! Space membership management: promoting a member to admin, and
//! removing a member. Both are commands (`member/promote`,
//! `member/expel`), asserted transiently on the space's branch and run
//! by the handlers here after the commit.
//!
//! Both are about chains, with the roster following. An admin is a
//! member who holds a `/` chain for the space, minted by someone who
//! holds one (the founder's account, or another admin) and retained in
//! the space db where every member can prove it; `MemberRole::admin` is
//! the roster's description of that. Removing a member is a delegated
//! revocation of the hop that admits them, minted under the remover's
//! own `/` chain and recorded at the space's access service, after which
//! the member's roster rows are retracted.
//!
//! Members hold `/use`, which does not cover `/ucan/revoke`, so the
//! service refuses a revocation minted under a member's chain; the
//! authority question is answered by what the chain covers, not by the
//! role fact. See `plan/space-admins.md`.

use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use tonk_account::customer::RevokeReceipt;
use tonk_common::log;
use tonk_schema::{InvitedVia, MemberName, MemberRole, Membership};

use super::revoke_invite::{leaf_cid, prove_path, publish_revocation, retract_leaf};
use crate::{TonkState, TonkWorkerError};

const CONTENT_BRANCH: &str = "main";

/// The role stamped on `member`'s membership of `subject`, if any.
async fn role_of(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    member: &Did,
    subject: &Did,
) -> Result<Option<MemberRole>, TonkWorkerError> {
    let membership = Membership::new(member.clone(), subject.clone());
    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(membership.this().clone()),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("member role query failed: {error:?}"))
        })?;
    Ok(roles.into_iter().next())
}

/// The hop that admits `member` to the space, and the path that reaches
/// it, proved from the space's own retained chains.
///
/// Proving as the member makes their own hop the leaf: for an open
/// invite that is the hop from the invite's ephemeral audience to them,
/// retained at claim time; for a scoped invite, the invite hop itself.
/// The founder is refused, since the only hop that reaches them is the
/// space's grant to their account, and a member with no retained hop
/// (a join from before claims were retained) is reported rather than
/// approximated by the invite everyone else also came in through.
pub(super) async fn resolve_member_target(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
    member: &Did,
) -> Result<(DelegationChain, Cid), TonkWorkerError> {
    if role_of(tonk, branch, member, subject)
        .await?
        .is_some_and(|role| role.role.0.to_string() == MemberRole::FOUNDER)
    {
        return Err(TonkWorkerError::Forbidden(
            "the founder cannot be removed from their own space".to_string(),
        ));
    }
    let path = prove_path(branch, tonk, subject, member)
        .await
        .map_err(|_| {
            TonkWorkerError::NotFound(format!(
                "no retained grant admits {member} to this space; a member who joined \
                 before claims were retained can only be removed with the invite they used"
            ))
        })?;
    let target = leaf_cid(&path)?;
    Ok((path, target))
}

/// Retract every roster row of `member`'s membership of `subject`.
///
/// Best-effort and after the revocation is durable: the service is what
/// denies the member, and a row left behind lists someone who cannot
/// sync, not someone who can.
pub(super) async fn retract_member_rows(
    tonk: &TonkState,
    repo: &str,
    branch: &dialog_repository::Branch,
    subject: &Did,
    member: &Did,
) {
    let membership = Membership::new(member.clone(), subject.clone());
    let entity = membership.this().clone();
    let mut transaction = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .transaction()
        .retract(membership.clone());

    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(entity.clone()),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    for role in roles {
        transaction = transaction.retract(role);
    }
    let names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::from(entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    for name in names {
        transaction = transaction.retract(name);
    }
    let stamps: Vec<InvitedVia> = branch
        .query()
        .select(Query::<InvitedVia> {
            this: Term::from(entity),
            invitation: Term::var("invitation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    for stamp in stamps {
        transaction = transaction.retract(stamp);
    }
    if let Err(error) = transaction.commit().perform(&tonk.operator).await {
        log!("removed member's roster rows were not retracted: {error}");
    }
}

/// Remove `member` from the space at `repo`: revoke the hop that admits
/// them, record it at the space's access service, and retract their
/// roster rows.
pub(crate) async fn expel_member(
    tonk: &TonkState,
    repo: &str,
    member: &Did,
) -> Result<RevokeReceipt, TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!("repository '{repo}' not found: {error}"))
        })?;
    let subject = repository.did();
    if super::account::member_did(tonk).await? == *member {
        return Err(TonkWorkerError::Forbidden(
            "removing yourself is leaving, not a revocation".to_string(),
        ));
    }

    let (path, target) = resolve_member_target(tonk, session.handle(), &subject, member).await?;
    let receipt =
        publish_revocation(tonk, repo, &repository, session.handle(), &path, &target).await?;
    retract_leaf(tonk, session.handle(), &path).await;
    retract_member_rows(tonk, repo, session.handle(), &subject, member).await;
    tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    Ok(receipt)
}

/// Admit `member` of the space at `repo` as admin, with the hop the page
/// minted under the passkey.
///
/// The promoter's own `/` chain for the space (an admin chain retained in
/// the space db, or the creation prefix for a founder) ends at their
/// account; `hop` must be that account's delegation of `/` on the space to
/// the member, and nothing else: issued by the account, to the member, over
/// this space, unattenuated. The composed chain is retained beside the
/// invites, where the member's devices prove from, and the role is
/// stamped. Returns the CID of the hop: what a demotion revokes.
pub(crate) async fn admit_member(
    tonk: &TonkState,
    repo: &str,
    member: &Did,
    hop: DelegationChain,
) -> Result<Cid, TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let repository = tonk
        .profile
        .space(repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!("repository '{repo}' not found: {error}"))
        })?;
    let subject = repository.did();
    let membership = Membership::new(member.clone(), subject.clone());
    match role_of(tonk, session.handle(), member, &subject).await? {
        None => {
            return Err(TonkWorkerError::NotFound(format!(
                "{member} is not a member of this space"
            )));
        }
        Some(role) if role.role.0.to_string() == MemberRole::FOUNDER => {
            return Err(TonkWorkerError::Conflict(
                "the founder already holds the space".to_string(),
            ));
        }
        Some(_) => {}
    }

    let authority =
        super::revoke_invite::account_authority(tonk, session.handle(), &subject).await?;
    let hops: Vec<_> = hop.proofs().collect();
    let [delegation] = hops.as_slice() else {
        return Err(TonkWorkerError::Forbidden(
            "the page must answer with exactly one hop".to_string(),
        ));
    };
    if delegation.issuer() != authority.audience() {
        return Err(TonkWorkerError::Forbidden(format!(
            "the hop is issued by {}, not by this profile's account {}",
            delegation.issuer(),
            authority.audience()
        )));
    }
    if delegation.audience() != member {
        return Err(TonkWorkerError::Forbidden(format!(
            "the hop admits {}, not {member}",
            delegation.audience()
        )));
    }
    if !delegation.subject().allows(&subject) {
        return Err(TonkWorkerError::Forbidden(
            "the hop is over another subject than this space".to_string(),
        ));
    }
    if !delegation.command().segments().is_empty() {
        return Err(TonkWorkerError::Forbidden(
            "an admin holds the whole space; the hop is attenuated".to_string(),
        ));
    }
    let chain = authority.push((*delegation).clone()).map_err(|error| {
        TonkWorkerError::Forbidden(format!("the hop does not chain onto the account: {error}"))
    })?;
    let target = leaf_cid(&chain)?;
    super::create_invite::retain_invite_authority(tonk, repo, &chain).await?;

    tonk.reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(MemberRole::admin(membership.this().clone()))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("stamp admin role: {error}")))?;
    tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    Ok(target)
}

/// Run `member/promote`: dispatched by the FAB's roster with the hop the
/// page minted, naming its space.
///
/// The real gate is cryptographic, not positional: [`admit_member`]
/// verifies the hop is issued by this profile's account authority, admits
/// exactly the named member, covers exactly this space, and is
/// unattenuated — a same-shaped fact from anywhere else cannot forge
/// that signature. Unparseable fields skip with a log.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg(feature = "persona")]
impl dialog_capability::Provider<tonk_schema::command::PromoteMember>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::PromoteMember) {
        use tonk_schema::prelude::DidExt as _;
        let decoded = (|| {
            let space: Did = command.space.0.to_string().parse().ok()?;
            let member: Did = command.member.0.to_string().parse().ok()?;
            let bytes = bs58::decode(&command.chain.0).into_vec().ok()?;
            let hop = DelegationChain::try_from(bytes.as_slice()).ok()?;
            Some((space.repo_key().to_owned(), member, hop))
        })();
        let Some((repo, member, hop)) = decoded else {
            log!("member/promote: no/unparseable member, space, or chain; skipping");
            return;
        };
        // The chains the promotion composes onto and the roster it stamps
        // are the space's. Where a worker of its own holds the space, that
        // worker admits the member.
        if self.from_profile() && self.state().read().await.spaces_elsewhere() {
            let admitted = async {
                let claim = super::space_reach::command(
                    &[
                        ("member", "xyz.tonk.command.admit-member/member", "Entity"),
                        ("hop", "xyz.tonk.command.admit-member/hop", "Text"),
                    ],
                    serde_json::json!({ "member": member.to_string(), "hop": command.chain.0 }),
                )?;
                let peer = super::space_reach::peer(&repo);
                super::space_reach::run(peer, peer.content(), claim).await
            };
            if let Err(error) = admitted.await {
                log!("member/promote for {member} on {repo} failed: {error}");
            }
            return;
        }
        let tonk = self.state().read().await;
        match admit_member(&tonk, &repo, &member, hop).await {
            Ok(target) => log!("member/promote: {member} is an admin of {repo} ({target})"),
            Err(error) => log!("member/promote for {member} on {repo} failed: {error}"),
        }
    }
}

/// Run [`AdmitMember`]: admit, in the space this worker holds, the member
/// the person's profile was asked to promote.
///
/// [`AdmitMember`]: tonk_schema::command::AdmitMember
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AdmitMember> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::AdmitMember) {
        let repo = self.origin().repo.clone();
        let decoded = (|| {
            let member: Did = command.member.0.to_string().parse().ok()?;
            let bytes = bs58::decode(&command.hop.0).into_vec().ok()?;
            Some((member, DelegationChain::try_from(bytes.as_slice()).ok()?))
        })();
        let Some((member, hop)) = decoded else {
            log!("admit member on {repo}: no/unparseable member or hop; skipping");
            return;
        };
        let tonk = self.state().read().await;
        match admit_member(&tonk, &repo, &member, hop).await {
            Ok(target) => log!("admit member: {member} is an admin of {repo} ({target})"),
            Err(error) => log!("admit member {member} on {repo} failed: {error}"),
        }
    }
}

/// Run `member/expel`: the roster row's remove form on the space the
/// command fires in. The target space is the ORIGIN — the branch the
/// transient committed on — so a space can only expel from itself; the
/// revocation is further gated by the access service refusing one not
/// minted under a `/` chain.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ExpelMember> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::ExpelMember) {
        let Ok(member) = command.member.0.to_string().parse::<Did>() else {
            log!("member/expel: no/unparseable member, skipping");
            return;
        };
        let repo = self.origin().repo.clone();
        let tonk = self.state().read().await.clone();
        match expel_member(&tonk, &repo, &member).await {
            Ok(receipt) => log!(
                "member/expel: {member} removed from {repo} (revoked {})",
                receipt.revoked
            ),
            Err(error) => log!("member/expel for {member} on {repo} failed: {error}"),
        }
    }
}

#[cfg(test)]
mod forwarding_tests {
    use dialog_capability::Provider;
    use tonk_schema::command::PromoteMember;
    use tonk_schema::domain::command::promote::{Chain, Member, Space};

    use crate::router::space_reach::stand_in;
    use crate::router::{CommandEnv, CommandOrigin};

    const SPACE: &str = "did:key:z6MkkAKBuUTy2r88au4Ehu6uUwdRRpDYnKd1euvreZi3YG7M";
    const MEMBER: &str = "did:key:z6Mki8Mf2Trp2qmXqNoSihfVi9sEg8Z4aSCSnyUfadj4jB1E";

    /// The bar promotes a member on the person's profile, naming the space.
    /// Where the space is held by a worker of its own, the profile passes
    /// the promotion on to that worker and touches nothing of the space.
    #[dialog_common::test]
    async fn it_passes_a_promotion_on_to_the_spaces_own_worker() {
        let state = crate::router::command::tests::native::test_state().await;
        state
            .read()
            .await
            .site_origins
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Any chain decodes: the hop is checked by the worker that admits.
        let hop = {
            use dialog_credentials::{Ed25519Signer, Signer};
            use dialog_ucan_core::subject::Subject as UcanSubject;
            use dialog_ucan_core::{DelegationBuilder, DelegationChain};

            let account = Ed25519Signer::generate().await.unwrap();
            let member: dialog_varsig::Did = MEMBER.parse().unwrap();
            let space: dialog_varsig::Did = SPACE.parse().unwrap();
            let grant = DelegationBuilder::new()
                .issuer(Signer::from(account))
                .audience(&member)
                .subject(UcanSubject::Specific(space))
                .command(vec![])
                .try_build()
                .await
                .unwrap();
            bs58::encode(DelegationChain::new(grant).to_bytes().unwrap()).into_string()
        };
        stand_in::answer_with(|_| Ok(serde_json::json!({})));

        let env = CommandEnv::new(state.clone(), CommandOrigin::default());
        <CommandEnv as Provider<PromoteMember>>::execute(
            &env,
            PromoteMember {
                this: "cmd:promote-one".parse().unwrap(),
                member: Member(MEMBER.parse().unwrap()),
                space: Space(SPACE.parse().unwrap()),
                chain: Chain(hop),
            },
        )
        .await;

        assert_eq!(
            stand_in::asked(),
            [(
                SPACE.to_owned(),
                "POST".to_owned(),
                format!("/api/repository/{SPACE}/branch/main/transact"),
            )]
        );
    }
}

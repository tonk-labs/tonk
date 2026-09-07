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
        .repository(repo)
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
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(repo, js_sys::Date::now());
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
        .repository(repo)
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
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(repo, js_sys::Date::now());
    Ok(target)
}

/// The DID a `member/*` command names, or `None` when it does not decode.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn member_of<C>(facts: &crate::reactor::EntityFacts, member: impl Fn(&C) -> String) -> Option<Did>
where
    C: crate::reactor::Decode,
{
    facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| C::decode(entity, facts))
        .and_then(|command| member(&command).parse::<Did>().ok())
}

/// Runs `member/promote`: dispatched by the FAB's roster with the hop the
/// page minted, naming its space.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct PromoteMemberHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl PromoteMemberHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::PromoteMember::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_promotion(facts: &crate::reactor::EntityFacts) -> Option<(String, Did, DelegationChain)> {
    use crate::reactor::Decode as _;
    use tonk_schema::prelude::DidExt as _;
    let command = facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::PromoteMember::decode(entity, facts))?;
    let space: Did = command.space.0.to_string().parse().ok()?;
    let member: Did = command.member.0.to_string().parse().ok()?;
    let bytes = bs58::decode(&command.chain.0).into_vec().ok()?;
    let hop = DelegationChain::try_from(bytes.as_slice()).ok()?;
    Some((space.repo_key().to_owned(), member, hop))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for PromoteMemberHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_promotion(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let decoded = decode_promotion(facts);
        let env = env.clone();
        Box::pin(async move {
            let Some((repo, member, hop)) = decoded else {
                log!("member/promote: no/unparseable member, space, or chain; skipping");
                return;
            };
            let tonk = env.state().read().await;
            match admit_member(&tonk, &repo, &member, hop).await {
                Ok(target) => log!("member/promote: {member} is an admin of {repo} ({target})"),
                Err(error) => log!("member/promote for {member} on {repo} failed: {error}"),
            }
        })
    }
}

/// Runs `member/expel`: the roster row's remove form on the space the
/// command fires in.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct ExpelMemberHandler {
    /// Decodes the current shape, and the deprecated one a
    /// branch seeded before the migration still asserts.
    command: crate::reactor::Migrated<
        tonk_schema::command::ExpelMember,
        tonk_schema::command::legacy::ExpelMember,
    >,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl ExpelMemberHandler {
    pub(crate) fn new() -> Self {
        Self {
            command: crate::reactor::Migrated::new(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for ExpelMemberHandler {
    fn trigger_attributes(&self) -> &[String] {
        self.command.trigger_attributes()
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        member_of::<tonk_schema::command::ExpelMember>(facts, |command| {
            command.member.0.to_string()
        })
        .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let member = member_of::<tonk_schema::command::ExpelMember>(facts, |command| {
            command.member.0.to_string()
        });
        let env = env.clone();
        Box::pin(async move {
            let Some(member) = member else {
                log!("member/expel: no/unparseable member, skipping");
                return;
            };
            let repo = env.origin().repo.clone();
            let tonk = env.state().read().await;
            match expel_member(&tonk, &repo, &member).await {
                Ok(receipt) => log!(
                    "member/expel: {member} removed from {repo} (revoked {})",
                    receipt.revoked
                ),
                Err(error) => log!("member/expel for {member} on {repo} failed: {error}"),
            }
        })
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan::{Parameters, Scope};
    use dialog_ucan_core::command::Command;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_varsig::Principal as _;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::api_router_with_state;
    use crate::router::tests::{content_member_roles, content_memberships, put_repo, test_state};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn open_content(
        state: &crate::router::AppState,
        key: &str,
    ) -> (dialog_repository::Branch, Did) {
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let repository = tonk
            .profile
            .repository(key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        (session.handle().clone(), repository.did())
    }

    /// The founder's only hop is the space's grant to their account, and
    /// revoking that is not removing a member.
    #[dialog_common::test]
    async fn it_refuses_to_resolve_the_founder_as_a_removal_target() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "members-founder").await;
        let (branch, subject) = open_content(&state, &key).await;
        let tonk = state.read().await;
        let founder = crate::router::account::member_did(&tonk).await.unwrap();

        let error = resolve_member_target(&tonk, &branch, &subject, &founder)
            .await
            .unwrap_err();
        assert!(
            matches!(error, TonkWorkerError::Forbidden(_)),
            "the founder must be refused, got {error:?}"
        );
    }

    /// A member who joined through an open invite is admitted by their own
    /// hop, retained at claim time, so removing them targets that hop and
    /// nobody else's.
    #[dialog_common::test]
    async fn it_resolves_a_joined_members_own_hop_as_the_removal_target() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = crate::router::join::tests::handcrafted_invite_url(90, 91).await;
        assert_eq!(
            crate::router::join::tests::post_join(&app, &url).await,
            StatusCode::CREATED
        );
        let (branch, subject) = open_content(&state, &key).await;
        let tonk = state.read().await;
        let member = crate::router::account::member_did(&tonk).await.unwrap();

        let (path, target) = resolve_member_target(&tonk, &branch, &subject, &member)
            .await
            .expect("the member's own hop is retained and provable");
        assert_eq!(path.audience(), &member, "the path ends at the member");
        assert_eq!(leaf_cid(&path).unwrap(), target);
        let proofs = path.proofs().collect::<Vec<_>>();
        let leaf = proofs.last().unwrap();
        assert_ne!(
            leaf.issuer(),
            path.proofs().next().unwrap().issuer(),
            "the target is the member's own hop, not the space's grant"
        );
    }

    /// Retracting a member takes every roster row keyed on their membership.
    #[dialog_common::test]
    async fn it_retracts_every_roster_row_of_a_removed_member() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let (url, key) = crate::router::join::tests::handcrafted_invite_url(92, 93).await;
        assert_eq!(
            crate::router::join::tests::post_join(&app, &url).await,
            StatusCode::CREATED
        );
        let (branch, subject) = open_content(&state, &key).await;
        let member = {
            let tonk = state.read().await;
            crate::router::account::member_did(&tonk).await.unwrap()
        };
        let entity = Membership::new(member.clone(), subject.clone())
            .this()
            .clone();
        assert!(
            content_memberships(&state, &key)
                .await
                .iter()
                .any(|row| *row.this() == entity),
            "the join recorded the membership"
        );

        {
            let tonk = state.read().await;
            retract_member_rows(&tonk, &key, &branch, &subject, &member).await;
        }

        assert!(
            content_memberships(&state, &key)
                .await
                .iter()
                .all(|row| *row.this() != entity),
            "the membership row is gone"
        );
        assert!(
            content_member_roles(&state, &key)
                .await
                .iter()
                .all(|row| row.this != entity),
            "the role row is gone"
        );
    }

    /// The hop the page mints under the passkey: this profile's account root
    /// delegating `/` on the space to the member.
    async fn root_hop(
        state: &crate::router::AppState,
        subject: &Did,
        member: &Did,
        issuer: Option<Ed25519Signer>,
    ) -> DelegationChain {
        use dialog_ucan_core::DelegationBuilder;
        let issuer = match issuer {
            Some(issuer) => issuer,
            None => {
                let seed = crate::router::tests::test_root_seed(&state.read().await.profile_name);
                Ed25519Signer::import(&seed).await.unwrap()
            }
        };
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(member)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    async fn seed_member(state: &crate::router::AppState, key: &str, member: &Did, subject: &Did) {
        let tonk = state.read().await;
        let membership = Membership::new(member.clone(), subject.clone());
        tonk.reactor
            .repository(key)
            .branch(CONTENT_BRANCH)
            .transaction()
            .assert(membership.clone())
            .assert(MemberRole::member(membership.this().clone()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }

    /// Admitting stamps the role and leaves a `/` chain to the member's
    /// account in the space db, where any member can prove it, with no
    /// device key in it.
    #[dialog_common::test]
    async fn it_admits_a_member_with_a_root_signed_chain_retained_in_the_space() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "members-promote").await;
        let (branch, subject) = open_content(&state, &key).await;
        let member = Ed25519Signer::generate().await.unwrap().did();
        seed_member(&state, &key, &member, &subject).await;
        let hop = root_hop(&state, &subject, &member, None).await;

        let target = {
            let tonk = state.read().await;
            admit_member(&tonk, &key, &member, hop).await.unwrap()
        };

        let roles = content_member_roles(&state, &key).await;
        let entity = Membership::new(member.clone(), subject.clone())
            .this()
            .clone();
        assert!(
            roles
                .iter()
                .any(|row| row.this == entity && row.role.0.to_string() == MemberRole::ADMIN),
            "the admin role is stamped"
        );

        let tonk = state.read().await;
        let device = tonk.profile.did();
        let full = Scope {
            subject: UcanSubject::Specific(subject.clone()),
            command: Command::parse("/").unwrap(),
            parameters: Parameters::default(),
        };
        let proof = branch
            .delegations()
            .prove(member.clone(), full)
            .perform(&tonk.operator)
            .await
            .expect("the admin chain is provable from the space db at /");
        assert_eq!(
            proof.proofs.last().unwrap().0.to_cid(),
            target,
            "the returned target is the chain's leaf"
        );
        assert!(
            proof
                .proofs
                .iter()
                .all(|certificate| certificate.0.issuer() != &device),
            "no device key issues a hop of the admin chain"
        );
    }

    /// A hop signed by anything but this profile's account is refused,
    /// whatever it says.
    #[dialog_common::test]
    async fn it_refuses_a_hop_the_account_did_not_sign() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "members-forged").await;
        let (_, subject) = open_content(&state, &key).await;
        let member = Ed25519Signer::generate().await.unwrap().did();
        seed_member(&state, &key, &member, &subject).await;
        let forger = Ed25519Signer::generate().await.unwrap();
        let hop = root_hop(&state, &subject, &member, Some(forger)).await;

        let tonk = state.read().await;
        let error = admit_member(&tonk, &key, &member, hop).await.unwrap_err();
        assert!(
            matches!(error, TonkWorkerError::Forbidden(_)),
            "got {error:?}"
        );
    }

    /// A hop the account signed for someone else does not admit this member.
    #[dialog_common::test]
    async fn it_refuses_a_hop_to_another_audience() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "members-audience").await;
        let (_, subject) = open_content(&state, &key).await;
        let member = Ed25519Signer::generate().await.unwrap().did();
        let other = Ed25519Signer::generate().await.unwrap().did();
        seed_member(&state, &key, &member, &subject).await;
        let hop = root_hop(&state, &subject, &other, None).await;

        let tonk = state.read().await;
        let error = admit_member(&tonk, &key, &member, hop).await.unwrap_err();
        assert!(
            matches!(error, TonkWorkerError::Forbidden(_)),
            "got {error:?}"
        );
    }

    /// A stranger cannot be promoted: promotion describes a member.
    #[dialog_common::test]
    async fn it_refuses_to_promote_a_non_member() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "members-stranger").await;
        let stranger = Ed25519Signer::generate().await.unwrap().did();
        let (_, subject) = open_content(&state, &key).await;
        let hop = root_hop(&state, &subject, &stranger, None).await;
        let tonk = state.read().await;
        let error = admit_member(&tonk, &key, &stranger, hop).await.unwrap_err();
        assert!(
            matches!(error, TonkWorkerError::NotFound(_)),
            "got {error:?}"
        );
    }

    /// The command decodes the member from its distinct attribute, so a
    /// promote and an expel never decode as each other.
    #[dialog_common::test]
    async fn it_decodes_each_member_command_by_its_own_attribute() {
        use crate::reactor::Decode as _;
        use tonk_schema::command::{ExpelMember, PromoteMember};
        let promote = PromoteMember::trigger_attributes();
        let expel = ExpelMember::trigger_attributes();
        assert!(
            promote.iter().all(|attribute| !expel.contains(attribute)),
            "promote {promote:?} and expel {expel:?} share no trigger attribute"
        );
    }
}

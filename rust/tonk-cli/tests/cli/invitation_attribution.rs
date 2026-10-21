//! Real CLI mint/claim/sync coverage for sender identity on a reshare.

use anyhow::Result;
use dialog_query::{Output as _, Query, Term};
use dialog_varsig::Did;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::{
    invite, remote,
    site::{self, SiteConfig, TonkSite},
    sync,
};
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::prelude::DidExt as _;
use tonk_schema::{Invitation, InvitedVia, MemberRole, Membership};

use crate::common;

async fn assert_targeted_sender(url: &str, sender: &Did, recipient: &Did) -> Result<Invitation> {
    let parsed = Invite::parse_url(url).await?;
    assert!(matches!(parsed.audience, InviteAudience::Scoped));
    assert!(url::Url::parse(url)?.fragment().is_none());
    assert_eq!(parsed.chain.audience(), recipient);
    assert_eq!(tonk_invite::inviter(&parsed.chain)?, Some(sender.clone()));
    let record = Invitation::from_chain(&parsed.chain).unwrap();
    assert_eq!(record.inviter.0, sender.this());
    Ok(record)
}

async fn reshare_flow(env: AccessServiceAddress, bob_config: SiteConfig, bob: Did) -> Result<()> {
    let alice = common::TestSite::new().await?;
    let carol = common::TestSite::new().await?;
    let mallory = common::TestSite::new().await?;
    let alice_did = site::member_did(&alice.site).await?;
    site::record_founder_membership(&alice.site).await?;
    let carol_did = site::member_did(&carol.site).await?;
    let mallory_did = site::member_did(&mallory.site).await?;
    assert_ne!(alice_did, alice.site.profile.did());
    assert_ne!(carol_did, carol.site.profile.did());
    assert_ne!(alice_did, bob);
    assert_ne!(carol_did, bob);
    assert_ne!(alice_did, carol_did);
    let space = alice.site.repository.did();
    env.provision_subject(space.as_str()).await?;
    let endpoint = env.access_service_url.as_str();
    remote::add(&alice.site, "origin", endpoint, None).await?;
    remote::set_upstream(&alice.site, "origin").await?;
    let first =
        invite::mint_targeted_with_relay(&alice.site, None, Some(endpoint), None, bob.as_str())
            .await?;
    let first_record = assert_targeted_sender(&first.url, &alice_did, &bob).await?;

    let rejected = mallory.parent.join("wrong-recipient");
    let error = invite::claim(&rejected, &first.url, mallory.config.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(error, invite::InviteError::InvalidInvite(_)),
        "{error}"
    );
    assert!(
        !rejected.exists(),
        "a refused invite must not publish a site"
    );

    let joined_tmp = tempfile::tempdir()?;
    let bob_path = joined_tmp.path().canonicalize()?.join("bob");
    let claimed = invite::claim(&bob_path, &first.url, bob_config.clone()).await?;
    assert!(
        matches!(claimed.completion, invite::ClaimCompletion::Complete),
        "{claimed:?}"
    );
    let joined_bob = TonkSite::open_with(&bob_path, bob_config).await?;
    assert_eq!(site::member_did(&joined_bob).await?, bob);
    assert_ne!(joined_bob.profile.did(), bob);
    let second = invite::mint_targeted_with_relay(
        &joined_bob,
        None,
        Some(endpoint),
        None,
        carol_did.as_str(),
    )
    .await?;
    let second_record = assert_targeted_sender(&second.url, &bob, &carol_did).await?;
    assert_ne!(first_record.this, second_record.this);

    let carol_path = carol.parent.join("alice-space");
    let claimed = invite::claim(&carol_path, &second.url, carol.config.clone()).await?;
    assert!(
        matches!(claimed.completion, invite::ClaimCompletion::Complete),
        "{claimed:?}"
    );
    let joined_carol = TonkSite::open_with(&carol_path, carol.config.clone()).await?;
    assert_eq!(joined_carol.repository.did(), space);
    let session = joined_carol.branch().await?;
    let content = session.handle();
    let memberships: Vec<Membership> = content
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::from(space.this()),
            member: Term::var("member"),
        })
        .perform(&joined_carol.operator)
        .try_vec()
        .await?;
    assert_eq!(memberships.len(), 3);
    assert!(
        !memberships
            .iter()
            .any(|member| member.member.0 == mallory_did.this())
    );
    let roles: Vec<MemberRole> = content
        .query()
        .select(Query::<MemberRole> {
            this: Term::var("this"),
            role: Term::var("role"),
        })
        .perform(&joined_carol.operator)
        .try_vec()
        .await?;
    let alice_membership = Membership::new(alice_did.clone(), space.clone());
    let bob_membership = Membership::new(bob.clone(), space.clone());
    let carol_membership = Membership::new(carol_did.clone(), space.clone());
    assert_eq!(
        roles
            .iter()
            .filter(|role| role.role.0.to_string() == MemberRole::FOUNDER)
            .collect::<Vec<_>>(),
        vec![&MemberRole::founder(alice_membership.this.clone())]
    );
    assert!(roles.contains(&MemberRole::member(bob_membership.this.clone())));
    assert!(roles.contains(&MemberRole::member(carol_membership.this.clone())));
    let invitations: Vec<Invitation> = content
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::from(space.this()),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&joined_carol.operator)
        .try_vec()
        .await?;
    assert!(invitations.contains(&first_record));
    assert!(invitations.contains(&second_record));
    let stamps: Vec<InvitedVia> = content
        .query()
        .select(Query::<InvitedVia> {
            this: Term::var("this"),
            invitation: Term::var("invitation"),
        })
        .perform(&joined_carol.operator)
        .try_vec()
        .await?;
    assert!(stamps.contains(&InvitedVia::new(bob_membership.this, first_record.this)));
    assert!(stamps.contains(&InvitedVia::new(carol_membership.this, second_record.this)));
    assert!(
        !stamps
            .iter()
            .any(|stamp| stamp.this == alice_membership.this)
    );
    drop(session);
    // Carol's claim already pushed her roster. Pull it into Alice's original
    // replica to prove the shared state survives the trip back to the owner.
    sync::pull(&alice.site).await?;
    let session = alice.site.branch().await?;
    let owners_copy: Vec<Invitation> = session
        .handle()
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::from(space.this()),
            inviter: Term::from(bob.this()),
            audience: Term::from(carol_did.this()),
        })
        .perform(&alice.site.operator)
        .try_vec()
        .await?;
    assert_eq!(owners_copy.len(), 1);
    eprintln!(
        "CLI attribution spike: Carol records Bob as inviter; Alice remains founder; wrong account refused"
    );
    Ok(())
}

#[dialog_common::test]
async fn onboarding_account_reshare_preserves_sender_and_founder(
    env: AccessServiceAddress,
) -> Result<()> {
    let bob = common::TestSite::new().await?;
    let did = site::member_did(&bob.site).await?;
    reshare_flow(env, bob.config.clone(), did).await
}

#[dialog_common::test]
async fn linked_account_reshare_preserves_sender_and_founder(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let bob = common::AccountFixture::with_account_remote(&remote).await?;
    bob.activate_with(&env).await?;
    reshare_flow(env, bob.config.clone(), bob.link.issuer().clone()).await
}

//! Actual browser Profile/Operator proof path, without browser network/UI.
use dialog_capability::{
    Subject,
    access::{Access, Authorization as _, Proof as _, Prove, TimeRange},
};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::Use;
use dialog_peer::{Peer, Profile};
use dialog_storage::provider::storage::Storage;
use dialog_ucan::{Ucan, UcanDelegation};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, time::Timestamp};
use dialog_varsig::Principal;
use tonk_invite::{
    connection::{AgentInvite, DEFAULT_GRANT_TTL_SECONDS, candidate_build_scopes},
    home_address_meta,
};
use url::Url;

#[dialog_common::test]
async fn connection_browser_profile_issues_long_grants_without_operator_suffix()
-> anyhow::Result<()> {
    let storage = Storage::volatile();
    let profile = Profile::open("browser-durable-connection")
        .perform(&storage)
        .await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
    let operator_deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let peer = Peer::new()
        .storage(storage)
        .attach(profile.signer().clone())
        .await?;
    let operator = peer
        .session(peer.derive(b"one-hour-browser-operator").await?)
        .allow(
            peer.access()
                .claim(Subject::any())
                .expires(operator_deadline),
        )
        .build()
        .await?;
    // The space owner is external: the profile retains a shared-space delegation,
    // and never installs the owner's secret in its credentials.
    let owner = Signer::from(Ed25519Signer::generate().await?);
    let ancestor = DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&profile.did())
        .subject(dialog_ucan_core::subject::Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .expiration(deadline)
        .try_build()
        .await?;
    profile
        .access()
        .save(UcanDelegation::new(DelegationChain::new(ancestor.clone())))
        .perform(&operator)
        .await?;
    let seed = [53; 32];
    let invitation = Ed25519Signer::import(&seed).await?;
    let remote = Url::parse("https://access.example/ucan/")?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        // This is Claim::perform's exact public effect for a dynamic scope. Its
        // principal is the profile, even though the operator executes the walk.
        let mut claim = Prove::<Ucan>::new(profile.did(), scope.clone());
        claim.duration = TimeRange {
            not_before: Some(now.to_unix()),
            expiration: Some(deadline.to_unix()),
        };
        let proof = Subject::from(profile.did())
            .attenuate(Access)
            .invoke(claim)
            .perform(&operator)
            .await?;
        let grant = proof
            .claim(profile.signer().signer().clone())?
            .expires(deadline.to_unix())?
            .meta(home_address_meta(&remote))
            .delegate(invitation.did())
            .await?;
        let chain = grant.into_chain();
        assert_eq!(chain.proof_cids().len(), 2);
        assert_eq!(chain.expiration(), Some(deadline));
        assert!(
            chain
                .proofs()
                .all(|hop| hop.issuer() != &operator.did() && hop.audience() != &operator.did())
        );
        chains.push(chain);
    }
    let invite = AgentInvite::new(seed, chains, &scopes, &remote, now).await?;
    assert_eq!(invite.grants().expires_at(), deadline);
    assert!(deadline > operator_deadline);
    // The same high-level API used by browser issuance refuses to promise a day
    // beyond upstream authority instead of extending it or silently shortening.
    let too_late =
        Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS + 86400) as i128)?;
    assert!(
        profile
            .access()
            .claim(Subject::from(owner.did()).attenuate(Use))
            .expires(too_late)
            .delegate(invitation.did())
            .perform(&operator)
            .await
            .is_err()
    );
    // A different shared space grants read only. The browser profile cannot
    // manufacture the write leaf even though its own operator session is broad.
    let read_only_owner = Signer::from(Ed25519Signer::generate().await?);
    let read_only = DelegationBuilder::new()
        .issuer(read_only_owner.clone())
        .audience(&profile.did())
        .subject(dialog_ucan_core::subject::Subject::Specific(
            read_only_owner.did(),
        ))
        .command(vec!["use".into(), "get".into()])
        .expiration(deadline)
        .try_build()
        .await?;
    profile
        .access()
        .save(UcanDelegation::new(DelegationChain::new(read_only)))
        .perform(&operator)
        .await?;
    let mut write = Prove::<Ucan>::new(
        profile.did(),
        candidate_build_scopes(&read_only_owner.did())[3].clone(),
    );
    write.duration = TimeRange {
        not_before: Some(now.to_unix()),
        expiration: Some(deadline.to_unix()),
    };
    assert!(
        Subject::from(profile.did())
            .attenuate(Access)
            .invoke(write)
            .perform(&operator)
            .await
            .is_err()
    );
    Ok(())
}

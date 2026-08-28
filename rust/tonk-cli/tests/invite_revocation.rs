//! The delegation-facts model behind invite revocation.
//!
//! Revocation used to read a hex-encoded chain off the invitation record.
//! It now retains the minted chain into the space's content branch and
//! rebuilds the path with `prove`. These tests pin the three properties
//! that substitution depends on, none of which are obvious:
//!
//! - proving as the invite's AUDIENCE returns a path whose leaf is the
//!   invite hop, which is the CID a revocation witness must name;
//! - retracting only that leaf leaves the shared prefix provable, so one
//!   revocation does not take every other invite down with it;
//! - the profile-to-account union makes the path reachable from the
//!   account, so a second device can revoke what this one minted.

mod common;

use anyhow::Result;
use dialog_credentials::Ed25519Signer;
use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::{Did, Principal as _};

/// The unattenuated scope an invite is minted from.
fn scope(subject: &Did) -> Scope {
    Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse("/use").expect("the use command parses"),
        parameters: Parameters::default(),
    }
}

/// Mint an invite the way the worker does: claim the space, delegate to a
/// fresh audience key.
async fn mint_invite(site: &common::TestSite, audience: &Did) -> Result<DelegationChain> {
    let delegation: UcanDelegation = site
        .site
        .profile
        .access()
        .claim(&site.site.repository)
        .delegate(audience.clone())
        .perform(site.site.operator.inner())
        .await?;
    Ok(delegation.into_chain())
}

/// The property the whole revocation path rests on: a retained invite is
/// re-derivable, leaf and all, by proving as the principal it was issued to.
///
/// Proving as the profile instead would return the profile's own access
/// path, which stops one hop short and never contains the invite. That is
/// the exact bug the old `path_hex` record papered over.
#[dialog_common::test]
async fn it_proves_a_retained_invite_back_to_its_leaf() -> Result<()> {
    let site = common::TestSite::new().await?;
    let operator = site.site.operator.inner();
    let subject = site.site.repository.did();
    let branch = site.site.branch().await?;

    let audience = Ed25519Signer::generate().await?.did();
    let chain = mint_invite(&site, &audience).await?;
    let minted_leaf = *chain
        .proof_cids()
        .last()
        .expect("a minted chain is non-empty");

    branch
        .handle()
        .delegations()
        .retain(UcanDelegation(chain))
        .perform(operator)
        .await?;

    let proof = branch
        .handle()
        .delegations()
        .prove(audience.clone(), scope(&subject))
        .perform(operator)
        .await?;
    let mut certificates = proof.proofs.into_iter();
    let mut proved = DelegationChain::new(certificates.next().expect("a proof is non-empty").0);
    for certificate in certificates {
        proved = proved.push(certificate.0)?;
    }

    assert_eq!(
        proved.proof_cids().last(),
        Some(&minted_leaf),
        "proving as the invite's audience must land on the invite hop, or a \
         revocation witness cannot name it"
    );
    assert_eq!(
        proved.audience(),
        &audience,
        "the proved path must end at the audience it was proved for"
    );
    Ok(())
}

/// Retracting the leaf revokes one invite and only that invite.
///
/// Chains share their proof prefix, so retracting the whole proved path
/// would tombstone the space-to-profile hop every other invite proves
/// through. This is the test that would catch that regression.
#[dialog_common::test]
async fn it_retracts_only_the_revoked_leaf() -> Result<()> {
    let site = common::TestSite::new().await?;
    let operator = site.site.operator.inner();
    let subject = site.site.repository.did();
    let branch = site.site.branch().await?;

    let revoked = Ed25519Signer::generate().await?.did();
    let spared = Ed25519Signer::generate().await?.did();
    for audience in [&revoked, &spared] {
        let chain = mint_invite(&site, audience).await?;
        branch
            .handle()
            .delegations()
            .retain(UcanDelegation(chain))
            .perform(operator)
            .await?;
    }

    // Prove, then retract exactly the leaf the proof landed on.
    let proof = branch
        .handle()
        .delegations()
        .prove(revoked.clone(), scope(&subject))
        .perform(operator)
        .await?;
    let leaf = proof.proofs.last().expect("a proof is non-empty").0.clone();
    branch
        .handle()
        .delegations()
        .retract(UcanDelegation(DelegationChain::new(leaf)))
        .perform(operator)
        .await?;

    assert!(
        branch
            .handle()
            .delegations()
            .prove(revoked, scope(&subject))
            .perform(operator)
            .await
            .is_err(),
        "the revoked invite must no longer prove"
    );
    assert!(
        branch
            .handle()
            .delegations()
            .prove(spared, scope(&subject))
            .perform(operator)
            .await
            .is_ok(),
        "revoking one invite must not tear the shared prefix out from under \
         the others"
    );
    Ok(())
}

/// The union edge is what lets a device other than the minter revoke.
///
/// Without it the account can walk only as far as its own grants, so an
/// invite this profile minted is unreachable from the account and the
/// minting device is the only one that could ever revoke it.
#[dialog_common::test]
async fn it_reaches_a_minted_invite_from_the_account() -> Result<()> {
    let site = common::TestSite::new().await?;
    let operator = site.site.operator.inner();
    let subject = site.site.repository.did();
    let branch = site.site.branch().await?;

    let account = Ed25519Signer::generate().await?.did();
    let audience = Ed25519Signer::generate().await?.did();
    let chain = mint_invite(&site, &audience).await?;

    branch
        .handle()
        .delegations()
        .retain(UcanDelegation(chain))
        .perform(operator)
        .await?;

    // The account can reach the space through the union edge: the invite's
    // path runs back to this profile, and the union says the account may act
    // as this profile.
    assert!(
        branch
            .handle()
            .delegations()
            .prove(account.clone(), scope(&subject))
            .perform(operator)
            .await
            .is_err(),
        "before the union is retained the account reaches nothing"
    );

    let union = tonk_account::delegations::mint_account_union(
        &site.site.profile.signer().signer().clone(),
        &account,
    )
    .await?;
    branch
        .handle()
        .delegations()
        .retain(UcanDelegation(union))
        .perform(operator)
        .await?;

    let proof = branch
        .handle()
        .delegations()
        .prove(account, scope(&subject))
        .perform(operator)
        .await;
    assert!(
        proof.is_ok(),
        "the union edge must carry the account to the space this profile \
         minted invites for: {:?}",
        proof.err()
    );
    Ok(())
}

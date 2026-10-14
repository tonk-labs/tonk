//! `/use/get/ticket/claim` against the access service: a ticket is
//! fetched by the key it is kept for, or by a principal that key
//! delegated the claim to, and by no one else.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test ticket_claim
//! ```

#![cfg(feature = "integration-tests")]

use dialog_capability::access::AuthorizeError;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_peer::helpers::{test_session_with_peer, unique_name};
use dialog_remote_ucan::UcanAddress;
use dialog_repository::RepositoryExt as _;
use dialog_repository::SiteAddress;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::{Did, Principal as _};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_account::ticket::{self, ClaimError};
use url::Url;

/// A provisioned space on the service that keeps a ticket for `holder`:
/// the space's own delegation to it. Answers the space and the ticket's
/// bytes.
async fn space_keeping_a_ticket_for(
    env: &AccessServiceAddress,
    holder: &Did,
) -> anyhow::Result<(Did, Vec<u8>)> {
    let (operator, profile) = test_session_with_peer().await;
    let space = profile
        .space(unique_name("ticket-claim"))
        .create()
        .perform(&operator)
        .await?;
    let ownership = space
        .access()
        .claim(&space)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    env.provision_subject(space.did().as_str()).await?;

    let grant = space
        .access()
        .claim(&space)
        .delegate(holder.clone())
        .perform(&operator)
        .await?;
    let chain: DelegationChain = grant.0;
    ticket::issue(
        SiteAddress::from(UcanAddress::new(&env.access_service_url)),
        &chain,
        &operator,
    )
    .await?;
    Ok((space.did(), chain.to_bytes()?))
}

/// `holder` delegates the claim, on itself, to `delegate`.
async fn delegate_claim(holder: &Ed25519Signer, delegate: &Did) -> DelegationChain {
    let delegation = DelegationBuilder::new()
        .issuer(Signer::from(holder.clone()))
        .audience(delegate)
        .subject(UcanSubject::Specific(holder.did()))
        .command(
            ticket::CLAIM
                .iter()
                .map(|segment| (*segment).to_string())
                .collect(),
        )
        .try_build()
        .await
        .expect("the delegation signs");
    DelegationChain::new(delegation)
}

fn endpoint(env: &AccessServiceAddress) -> Url {
    Url::parse(&env.access_service_url).expect("the service URL parses")
}

async fn key(seed: u8) -> Ed25519Signer {
    Ed25519Signer::import(&[seed; 32])
        .await
        .expect("the key imports")
}

/// The key a ticket is kept for claims it, signing as itself.
#[dialog_common::test]
async fn it_answers_the_holder_its_ticket(env: AccessServiceAddress) -> anyhow::Result<()> {
    let holder = key(11).await;
    let (space, kept) = space_keeping_a_ticket_for(&env, &holder.did()).await?;

    let claimed = ticket::claim(
        &endpoint(&env),
        Signer::from(holder.clone()),
        &holder.did(),
        None,
        &space,
    )
    .await?;
    assert_eq!(claimed, Some(kept), "the holder fetches its own ticket");
    Ok(())
}

/// A principal the holder delegated the claim to fetches the holder's
/// ticket, signing as itself and presenting the delegation.
#[dialog_common::test]
async fn it_answers_a_delegate_of_the_holder_its_ticket(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let holder = key(12).await;
    let delegate = key(13).await;
    let (space, kept) = space_keeping_a_ticket_for(&env, &holder.did()).await?;
    let proof = delegate_claim(&holder, &delegate.did()).await;

    let claimed = ticket::claim(
        &endpoint(&env),
        Signer::from(delegate.clone()),
        &holder.did(),
        Some(&proof),
        &space,
    )
    .await?;
    assert_eq!(
        claimed,
        Some(kept),
        "the delegate fetches the holder's ticket"
    );
    Ok(())
}

/// A principal the holder delegated nothing to cannot fetch the holder's
/// ticket, whether it claims bare or presents a delegation made out to
/// someone else; claiming as itself reaches only its own cell, which is
/// empty.
#[dialog_common::test]
async fn it_refuses_the_ticket_to_any_other_principal(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let holder = key(14).await;
    let delegate = key(15).await;
    let stranger = key(16).await;
    let (space, _) = space_keeping_a_ticket_for(&env, &holder.did()).await?;

    let bare = ticket::claim(
        &endpoint(&env),
        Signer::from(stranger.clone()),
        &holder.did(),
        None,
        &space,
    )
    .await;
    assert!(
        matches!(
            bare,
            Err(ClaimError::Refused(AuthorizeError::InvalidAudience { .. }))
        ),
        "a claim on the holder with no proof is refused: {bare:?}"
    );

    let borrowed = ticket::claim(
        &endpoint(&env),
        Signer::from(stranger.clone()),
        &holder.did(),
        Some(&delegate_claim(&holder, &delegate.did()).await),
        &space,
    )
    .await;
    assert!(
        matches!(
            borrowed,
            Err(ClaimError::Refused(AuthorizeError::InvalidAudience { .. }))
        ),
        "a delegation made out to another principal proves nothing: {borrowed:?}"
    );

    let own = ticket::claim(
        &endpoint(&env),
        Signer::from(stranger.clone()),
        &stranger.did(),
        None,
        &space,
    )
    .await?;
    assert_eq!(
        own, None,
        "claiming as itself reaches only its own, empty, cell"
    );
    Ok(())
}

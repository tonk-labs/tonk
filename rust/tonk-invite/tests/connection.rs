//! Characterization of the pinned delegation primitives for Plan 001.
//! Historical metadata and legacy-claim probes; markers do not authorize access.

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::time::timestamp::{Duration, UNIX_EPOCH};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::Principal;
use ipld_core::ipld::Ipld;
use std::collections::BTreeMap;
use tonk_invite::{Invite, InviteAudience};

const MARKER: &str = "tonk.connection.spike";

#[dialog_common::test]
async fn connection_marker_lookup_can_be_shadowed_but_ancestor_is_retained() {
    let owner = Signer::from(Ed25519Signer::generate().await.unwrap());
    let bootstrap = Signer::from(Ed25519Signer::generate().await.unwrap());
    let session = Signer::from(Ed25519Signer::generate().await.unwrap());
    let grant = DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&bootstrap.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .meta(BTreeMap::from([(
            MARKER.into(),
            Ipld::String("original".into()),
        )]))
        .try_build()
        .await
        .unwrap();
    let grant_cid = grant.to_cid();
    let child = DelegationBuilder::new()
        .issuer(bootstrap)
        .audience(&session.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .meta(BTreeMap::from([(
            MARKER.into(),
            Ipld::String("shadow".into()),
        )]))
        .try_build()
        .await
        .unwrap();
    let chain = DelegationChain::new(grant).push(child).unwrap();
    let decoded = DelegationChain::try_from(chain.to_bytes().unwrap().as_slice()).unwrap();

    // Generic display metadata lookup trusts the nearest descendant; a security
    // decision must use ordinary verified UCAN authority, never this metadata.
    assert_eq!(decoded.meta(MARKER), Some(&Ipld::String("shadow".into())));
    let ancestor = decoded
        .proofs()
        .find(|hop| hop.to_cid() == grant_cid)
        .unwrap();
    assert_eq!(
        ancestor.meta().get(MARKER),
        Some(&Ipld::String("original".into()))
    );
}

#[dialog_common::test]
async fn connection_legacy_claim_keeps_marker_and_parent_expiry_but_is_repeatable() {
    let owner = Signer::from(Ed25519Signer::generate().await.unwrap());
    let seed = [42; 32];
    let bootstrap = Signer::from(Ed25519Signer::import(&seed).await.unwrap());
    let first = Signer::from(Ed25519Signer::generate().await.unwrap());
    let second = Signer::from(Ed25519Signer::generate().await.unwrap());
    let deadline = Timestamp::new(UNIX_EPOCH + Duration::from_secs(1_000_000_000)).unwrap();
    let grant = DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&bootstrap.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .expiration(deadline)
        .meta(BTreeMap::from([(
            MARKER.into(),
            Ipld::String("original".into()),
        )]))
        .try_build()
        .await
        .unwrap();
    let grant_cid = grant.to_cid();
    let invite = Invite::new(
        DelegationChain::new(grant),
        InviteAudience::Open { seed },
        None,
    )
    .await
    .unwrap();

    // Claim assembles a chain, without checking current validity or binding a
    // session remotely. The service judges expiry and standard UCAN revocation.
    for audience in [first.did(), second.did()] {
        let claimed = invite.clone().claim(&audience).await.unwrap();
        assert_eq!(claimed.chain.audience(), &audience);
        assert_eq!(claimed.chain.expiration(), Some(deadline));
        assert!(claimed.chain.proof_cids().contains(&grant_cid));
        let ancestor = claimed
            .chain
            .proofs()
            .find(|hop| hop.to_cid() == grant_cid)
            .unwrap();
        assert_eq!(ancestor.command().0, vec!["use".to_owned()]);
        assert_eq!(
            claimed.chain.meta(MARKER),
            Some(&Ipld::String("original".into()))
        );
    }
}

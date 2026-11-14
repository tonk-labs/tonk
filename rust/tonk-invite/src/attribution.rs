//! Account attribution carried by an invitation's signed leaf grant.

use anyhow::{Context, Result, bail, ensure};
use dialog_ucan_core::{DelegationChain, subject::Subject};
use dialog_varsig::Did;
use ipld_core::ipld::Ipld;
use std::collections::BTreeMap;

/// Signed metadata identifying the account minting this invitation.
pub const INVITER: &str = "invitation.inviter";

/// Metadata to attach to the invitation's leaf delegation before signing.
pub fn inviter_meta(account: &Did) -> BTreeMap<String, Ipld> {
    BTreeMap::from([(INVITER.into(), Ipld::String(account.to_string()))])
}

/// Read and validate the leaf's declared inviter, if present.
///
/// The account must be the leaf issuer or reach it through the contiguous
/// subject-open identity grants immediately preceding the leaf. A scoped
/// space-sharing grant cannot confer the upstream account's identity. This
/// checks structural binding; cryptographic verification and capability
/// admission remain the responsibility of the usual UCAN verifier.
///
/// Only the leaf may declare this invitation's sender. Missing metadata
/// returns `None` so callers can preserve legacy attribution.
///
/// # Errors
///
/// Rejects malformed metadata and accounts not bound to the leaf issuer.
pub fn inviter(chain: &DelegationChain) -> Result<Option<Did>> {
    let proofs: Vec<_> = chain.proofs().collect();
    let (leaf, ancestors) = proofs
        .split_last()
        .expect("delegation chains are non-empty");
    let Some(value) = leaf.meta().get(INVITER) else {
        return Ok(None);
    };
    let Ipld::String(value) = value else {
        bail!("invitation inviter must be a DID string");
    };
    let account: Did = value.parse().context("invalid invitation inviter DID")?;
    let mut principal = leaf.issuer();
    if principal == &account {
        return Ok(Some(account));
    }
    for grant in ancestors.iter().rev() {
        ensure!(
            grant.audience() == principal,
            "invitation identity grants must align"
        );
        if !matches!(grant.subject(), Subject::Any) {
            break;
        }
        principal = grant.issuer();
        if principal == &account {
            return Ok(Some(account));
        }
    }
    bail!("invitation inviter is not bound to the signing identity")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Invite, InviteAudience};
    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_ucan_core::DelegationBuilder;
    use dialog_varsig::Principal;

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    // space -> Alice -> Alice device -> Bob -> Bob device -> session -> Carol.
    // Both identity paths have open subjects; the transfer to Bob is scoped.
    async fn reshare(meta: BTreeMap<String, Ipld>) -> DelegationChain {
        let mut chain: Option<DelegationChain> = None;
        let space = signer(1).await.did();
        for (issuer, audience, identity) in [
            (1, 2, false),
            (2, 3, true),
            (3, 4, false),
            (4, 5, true),
            (5, 6, true),
            (6, 7, false),
        ] {
            let metadata = match issuer {
                3 => inviter_meta(&signer(2).await.did()),
                6 => meta.clone(),
                _ => BTreeMap::new(),
            };
            let grant = DelegationBuilder::new()
                .issuer(Signer::from(signer(issuer).await))
                .audience(&signer(audience).await.did())
                .subject(if identity {
                    Subject::Any
                } else {
                    Subject::Specific(space.clone())
                })
                .command(vec![])
                .meta(metadata)
                .try_build()
                .await
                .unwrap();
            chain = Some(match chain {
                None => DelegationChain::new(grant),
                Some(chain) => chain.push(grant).unwrap(),
            });
        }
        chain.unwrap()
    }

    #[dialog_common::test]
    async fn reshare_binds_bob_through_device_and_session_and_roundtrips() {
        let bob = signer(4).await.did();
        let chain = reshare(inviter_meta(&bob)).await;
        assert_eq!(inviter(&chain).unwrap(), Some(bob.clone()));
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();
        let parsed = Invite::parse_url(&invite.to_url(crate::DEFAULT_BASE_URL).unwrap())
            .await
            .unwrap();
        assert_eq!(inviter(&parsed.chain).unwrap(), Some(bob));
        assert_eq!(parsed.chain.audience(), &signer(7).await.did());
    }

    #[dialog_common::test]
    async fn reshare_cannot_claim_alice_or_an_unrelated_account() {
        for seed in [2, 8] {
            let chain = reshare(inviter_meta(&signer(seed).await.did())).await;
            assert!(
                inviter(&chain)
                    .unwrap_err()
                    .to_string()
                    .contains("not bound")
            );
            assert!(
                Invite::new(chain, InviteAudience::Scoped, None)
                    .await
                    .is_err()
            );
        }
    }

    #[dialog_common::test]
    async fn malformed_inviter_is_rejected_at_construction_and_url_parse() {
        for value in [Ipld::Integer(42), Ipld::String("not-a-did".into())] {
            let chain = reshare(BTreeMap::from([(INVITER.into(), value)])).await;
            assert!(
                Invite::new(chain.clone(), InviteAudience::Scoped, None)
                    .await
                    .is_err()
            );
            // Encode directly to exercise parser validation without bypassing
            // construction through a mutable Invite.
            let mut url = url::Url::parse(crate::DEFAULT_BASE_URL).unwrap();
            url.query_pairs_mut().append_pair(
                "access",
                &bs58::encode(chain.to_bytes().unwrap()).into_string(),
            );
            assert!(Invite::parse_url(url.as_str()).await.is_err());
        }
    }

    #[dialog_common::test]
    async fn legacy_leaf_does_not_inherit_alices_sender_metadata() {
        let chain = reshare(BTreeMap::new()).await;
        assert!(
            chain.meta(INVITER).is_some(),
            "upstream grant declares Alice"
        );
        assert_eq!(inviter(&chain).unwrap(), None);
        assert!(
            Invite::new(chain, InviteAudience::Scoped, None)
                .await
                .is_ok()
        );
    }

    #[dialog_common::test]
    async fn account_signed_open_invite_preserves_sender_before_claim() {
        let alice = signer(2).await;
        let ephemeral = signer(7).await;
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(alice.clone()))
            .audience(&ephemeral.did())
            .subject(Subject::Specific(signer(1).await.did()))
            .command(vec![])
            .meta(inviter_meta(&alice.did()))
            .try_build()
            .await
            .unwrap();
        let invite = Invite::new(
            DelegationChain::new(grant),
            InviteAudience::Open { seed: [7; 32] },
            None,
        )
        .await
        .unwrap();
        let parsed = Invite::parse_url(&invite.to_url(crate::DEFAULT_BASE_URL).unwrap())
            .await
            .unwrap();
        assert_eq!(inviter(&parsed.chain).unwrap(), Some(alice.did()));
        assert!(matches!(parsed.audience, InviteAudience::Open { .. }));
    }
}

//! The chain a guest kernel opens a layer outside its stack with: delegated by
//! the layer's subject, down to the worker's session, signed at each hop and
//! in date.

use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::time::{TimeRange, Timestamp};
use dialog_varsig::Did;

use crate::bindings::sapling::kernel::types::Error;

/// Check that `chain` delegates `subject` to `session`.
pub async fn check(chain: &[u8], subject: &str, session: &str) -> Result<(), Error> {
    let denied = |why: String| Error::Denied(why);
    let chain = DelegationChain::try_from(chain)
        .map_err(|error| denied(format!("the chain does not decode: {error:?}")))?;
    let subject: Did = subject
        .parse()
        .map_err(|_| Error::Invalid(format!("{subject} is not a DID")))?;
    if chain.subject() != Some(&subject) {
        return Err(denied(format!("the chain is not for {subject}")));
    }
    if chain.audience().to_string() != session {
        return Err(denied(format!("the chain is not to the session {session}")));
    }
    if chain.proofs().next().map(|hop| hop.issuer()) != Some(&subject) {
        return Err(denied(format!("the chain does not start at {subject}")));
    }
    let now = Timestamp::now();
    for hop in chain.proofs() {
        hop.verify_signature(&DidKeyResolver)
            .await
            .map_err(|error| denied(format!("a hop of the chain is not signed: {error}")))?;
        TimeRange::new(hop.not_before(), hop.expiration())
            .check(&now)
            .map_err(|error| denied(format!("a hop of the chain is out of date: {error}")))?;
    }
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject;

    async fn signer(seed: u8) -> dialog_credentials::Ed25519Signer {
        dialog_credentials::Ed25519Signer::import(&[seed; 32])
            .await
            .unwrap()
    }

    async fn delegated(subject: u8, audience: u8) -> (Vec<u8>, String, String) {
        use dialog_varsig::Principal as _;
        let issuer = signer(subject).await;
        let audience = signer(audience).await.did();
        let subject = issuer.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(&audience)
            .subject(Subject::Specific(subject.clone()))
            .command(vec!["use".to_owned()])
            .try_build()
            .await
            .unwrap();
        (
            DelegationChain::new(delegation).to_bytes().unwrap(),
            subject.to_string(),
            audience.to_string(),
        )
    }

    #[tokio::test]
    async fn it_opens_with_a_chain_from_the_subject_to_the_session() {
        let (chain, subject, session) = delegated(3, 7).await;
        check(&chain, &subject, &session).await.unwrap();
    }

    #[tokio::test]
    async fn it_refuses_a_chain_to_another_session() {
        let (chain, subject, _) = delegated(3, 7).await;
        let (_, _, other) = delegated(3, 8).await;
        assert!(matches!(
            check(&chain, &subject, &other).await,
            Err(Error::Denied(_))
        ));
    }

    #[tokio::test]
    async fn it_refuses_no_chain() {
        let (_, subject, session) = delegated(3, 7).await;
        assert!(matches!(
            check(&[], &subject, &session).await,
            Err(Error::Denied(_))
        ));
    }
}

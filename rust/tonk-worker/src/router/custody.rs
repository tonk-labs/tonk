//! Getting the account's encryption key onto a device that needs it.
//!
//! A passkey account's secret only exists inside a WebAuthn assertion on
//! a page, so the ceremonies that need it ask the page that originated
//! the operation to run one. Taking a space or an invite principal into
//! custody needs none of this: dialog seals the key to the account's own
//! DID.

use crate::router::ClientId;
use tonk_common::log;

/// Ask the page to mediate a passkey so this worker can mint custody
/// material.
///
/// The worker has no `window`, so the assertion happens on the page
/// that asked for the enrollment. What comes back is two transient PRF
/// byte arrays, and this enrollment travels with the request so the
/// handoff knows what it is for. The receiver imports worker-owned
/// derivation handles before doing custody work.
pub(crate) async fn request_mediation(client: &ClientId, email: Option<String>) {
    let enrollment = tonk_worker_api::Enrollment { email };
    if let Err(error) = super::navigate::request_webauthn_with(
        client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(tonk_worker_api::CustodyIntent::Enroll(enrollment)),
        None,
    )
    .await
    {
        log!("custody: the page could not be asked to mediate: {error}");
    }
}

/// Sign `root -> audience` over `subject` at `command`: one hop, addressed
/// to exactly the audience asked for and granting exactly the command asked
/// for, as a base58 chain. Nothing of the request is taken on trust beyond
/// its being well formed; what the hop is worth is for whoever redeems it
/// to check against what it is composed with.
#[cfg(test)]
async fn mint_delegation(
    root: impl Into<dialog_credentials::Signer>,
    request: &tonk_worker_api::RootDelegation,
) -> Result<String, String> {
    use dialog_ucan_core::command::Command;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Did;

    let subject: Did = request
        .subject
        .parse()
        .map_err(|error| format!("the subject is not a DID: {error:?}"))?;
    let audience: Did = request
        .audience
        .parse()
        .map_err(|error| format!("the audience is not a DID: {error:?}"))?;
    let command = Command::parse(&request.command)
        .map_err(|error| format!("the command does not parse: {error}"))?;
    let delegation = DelegationBuilder::new()
        .issuer(root.into())
        .audience(&audience)
        .subject(Subject::Specific(subject))
        .command(command.segments().clone())
        .try_build()
        .await
        .map_err(|error| format!("failed to mint the delegation: {error}"))?;
    let bytes = DelegationChain::new(delegation)
        .to_bytes()
        .map_err(|error| format!("the delegation did not encode: {error}"))?;
    Ok(bs58::encode(bytes).into_string())
}

#[cfg(test)]
mod delegation_tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::DelegationChain;
    use dialog_varsig::Principal as _;
    use tonk_worker_api::RootDelegation;

    use super::mint_delegation;

    async fn request(command: &str) -> (Ed25519Signer, RootDelegation) {
        let root = Ed25519Signer::generate().await.unwrap();
        let subject = Ed25519Signer::generate().await.unwrap();
        let audience = Ed25519Signer::generate().await.unwrap();
        let request = RootDelegation {
            subject: subject.did().to_string(),
            command: command.to_owned(),
            audience: audience.did().to_string(),
        };
        (root, request)
    }

    #[dialog_common::test]
    async fn it_mints_one_hop_from_the_root_to_the_audience_asked_for() {
        let (root, request) = request("/").await;

        let minted = mint_delegation(root.clone(), &request).await.unwrap();

        let bytes = bs58::decode(minted).into_vec().expect("base58");
        let chain = DelegationChain::try_from(bytes.as_slice()).expect("a delegation chain");
        assert_eq!(chain.issuer().to_string(), root.did().to_string());
        assert_eq!(chain.audience().to_string(), request.audience);
        assert_eq!(
            chain.subject().map(|subject| subject.to_string()),
            Some(request.subject)
        );
        assert_eq!(chain.proof_cids().len(), 1, "a single hop");
    }

    #[dialog_common::test]
    async fn it_refuses_a_request_that_is_not_well_formed() {
        let (root, well_formed) = request("/").await;

        let no_subject = RootDelegation {
            subject: "not a did".into(),
            ..well_formed.clone()
        };
        let no_audience = RootDelegation {
            audience: String::new(),
            ..well_formed.clone()
        };

        assert!(
            mint_delegation(root.clone(), &no_subject)
                .await
                .is_err_and(|said| said.contains("subject"))
        );
        assert!(
            mint_delegation(root, &no_audience)
                .await
                .is_err_and(|said| said.contains("audience"))
        );
    }
}

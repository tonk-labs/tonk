//! Email-gated issuance of an anchor chain to a new credential.
//!
//! The service holds `root → recovery` for an account because that account's
//! root delegated to it at genesis. Exercising it is policy, not authority:
//! the policy is a one-time code to the account address, and a notice to the
//! same address whether or not the person asked for it.
//!
//! The gate stands even when the caller already holds a valid chain. An
//! anchor chain outlives revocation of whatever enrolled it, so "holds a
//! valid chain" would let a briefly compromised passkey mint itself a
//! successor that survives being revoked.

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal};
use tonk_identity::credential::extend_with_enrollment;

use crate::core::CeremonyError;
use crate::core::codes::verify_code;
use crate::email::EmailSender;
use crate::enrollments::{EnrollmentStore, verify};
use crate::revocations::PutOutcome;
use crate::store::Store;

/// An anchor chain minted for one credential.
pub struct IssuedAnchor {
    /// Account subject the chain runs from.
    pub account_root: Did,
    /// Credential the chain was issued to.
    pub credential: Did,
    /// The full `root → recovery → credential` chain.
    pub chain: DelegationChain,
    /// Whether publication created a new entry.
    pub stored: bool,
}

/// Load the account's anchor proof and check it is one this key can extend.
async fn anchor_proof<E: EnrollmentStore>(
    enrollments: &E,
    anchor_did: &Did,
    account_root: &Did,
) -> Result<DelegationChain, CeremonyError> {
    let bytes = enrollments
        .anchor(account_root)
        .await
        .map_err(|error| CeremonyError::Internal(error.to_string()))?
        .ok_or_else(|| {
            CeremonyError::Conflict(
                "this account has no recovery anchor; enrol from a device that holds a credential"
                    .to_string(),
            )
        })?;
    let chain = DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        CeremonyError::Internal(format!("stored anchor proof is unreadable: {error}"))
    })?;
    if chain.issuer() != account_root {
        return Err(CeremonyError::Internal(
            "stored anchor proof does not run from this account".to_string(),
        ));
    }
    if chain.audience() != anchor_did {
        return Err(CeremonyError::Conflict(
            "this account's recovery anchor is a different key; it may have been rotated or revoked"
                .to_string(),
        ));
    }
    Ok(chain)
}

/// Verify `code` for `email`, then mint and publish `recovery → credential`.
#[allow(clippy::too_many_arguments)]
pub async fn issue_anchor_chain<S: Store, E: EnrollmentStore, M: EmailSender>(
    store: &S,
    enrollments: &E,
    emails: &M,
    anchor: Ed25519Signer,
    email: &str,
    code: &str,
    credential: &Did,
    now: u64,
) -> Result<IssuedAnchor, CeremonyError> {
    verify_code(store, email, code, now).await?;

    let account = store
        .account_by_email(&email.to_lowercase())
        .await?
        .ok_or_else(|| CeremonyError::NotFound("no account for this address".to_string()))?;
    let account_root: Did = account.root_did.parse().map_err(|error| {
        CeremonyError::Internal(format!("stored account root DID is invalid: {error:?}"))
    })?;
    if credential == &account_root {
        return Err(CeremonyError::Invalid(
            "the account subject cannot be enrolled as its own credential".to_string(),
        ));
    }

    let anchor_did = anchor.did();
    let proof = anchor_proof(enrollments, &anchor_did, &account_root).await?;
    let chain = extend_with_enrollment(&proof, anchor, credential)
        .await
        .map_err(|error| CeremonyError::Internal(format!("failed to mint the chain: {error}")))?;

    let bytes = chain
        .to_bytes()
        .map_err(|error| CeremonyError::Internal(format!("failed to serialize: {error}")))?;
    let verified = verify(&bytes)
        .await
        .map_err(|error| CeremonyError::Internal(format!("minted an invalid chain: {error}")))?;
    let stored = enrollments
        .put(&verified, &bytes)
        .await
        .map_err(|error| CeremonyError::Internal(error.to_string()))?
        == PutOutcome::Stored;

    // Delivery is best-effort on purpose: the chain is already minted and
    // published, and failing the request now would leave the caller unable to
    // tell whether it happened. A missed notice is logged, not fatal.
    if let Err(error) = emails
        .send_enrollment_notice(&account.email, credential.as_ref())
        .await
    {
        crate::core::log_detail(&format!("enrollment notice failed to send: {error:?}"));
    }

    Ok(IssuedAnchor {
        account_root,
        credential: credential.clone(),
        chain,
        stored,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::core::codes::request_code;
    use crate::email::CapturedEmail;
    use crate::enrollments::MemoryEnrollmentStore;
    use crate::store::sqlite::SqliteStore;
    use tonk_identity::credential::mint_enrollment;

    const EMAIL: &str = "peer@example.com";
    const CODE: &str = "123456";

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    /// An account whose root delegated to the anchor at genesis, with the
    /// resulting proof filed the way the publish route files it.
    async fn account_with_anchor(
        store: &SqliteStore,
        enrollments: &MemoryEnrollmentStore,
        anchor: &Ed25519Signer,
    ) -> Ed25519Signer {
        let root = register(store).await;
        let proof = mint_enrollment(root.clone(), &anchor.did()).await.unwrap();
        enrollments
            .put_anchor(&root.did(), &proof.to_bytes().unwrap())
            .await
            .unwrap();
        root
    }

    async fn register(store: &SqliteStore) -> Ed25519Signer {
        let root = signer(1).await;
        store
            .create_account(EMAIL, root.did().as_ref(), "credential", 100)
            .await
            .unwrap();
        root
    }

    async fn pending_code(store: &SqliteStore, emails: &CapturedEmail) {
        request_code(store, emails, EMAIL, CODE, 100).await.unwrap();
    }

    #[dialog_common::test]
    async fn it_issues_a_depth_two_chain_and_announces_it() {
        let store = SqliteStore::in_memory().unwrap();
        let enrollments = MemoryEnrollmentStore::default();
        let emails = CapturedEmail::default();
        let anchor = signer(2).await;
        let root = account_with_anchor(&store, &enrollments, &anchor).await;
        let credential = signer(3).await;
        pending_code(&store, &emails).await;

        let issued = issue_anchor_chain(
            &store,
            &enrollments,
            &emails,
            anchor.clone(),
            EMAIL,
            CODE,
            &credential.did(),
            200,
        )
        .await
        .unwrap();

        assert_eq!(issued.account_root, root.did());
        assert_eq!(
            issued.chain.proof_cids().len(),
            2,
            "root → anchor → credential"
        );
        assert_eq!(*issued.chain.audience(), credential.did());
        assert!(issued.stored);
        assert_eq!(
            emails.notices.lock().unwrap().as_slice(),
            [(EMAIL.to_string(), credential.did().to_string())],
        );

        // The credential can now find its chain with nothing but its own DID.
        let claimed = enrollments.claim(&credential.did()).await.unwrap();
        assert_eq!(claimed, vec![issued.chain.to_bytes().unwrap()]);
    }

    #[dialog_common::test]
    async fn it_refuses_without_a_verified_code() {
        let store = SqliteStore::in_memory().unwrap();
        let enrollments = MemoryEnrollmentStore::default();
        let emails = CapturedEmail::default();
        let anchor = signer(2).await;
        account_with_anchor(&store, &enrollments, &anchor).await;
        let credential = signer(3).await;
        pending_code(&store, &emails).await;

        assert!(matches!(
            issue_anchor_chain(
                &store,
                &enrollments,
                &emails,
                anchor,
                EMAIL,
                "000000",
                &credential.did(),
                200,
            )
            .await,
            Err(CeremonyError::CodeInvalid)
        ));
        assert!(
            enrollments
                .claim(&credential.did())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(emails.notices.lock().unwrap().is_empty());
    }

    #[dialog_common::test]
    async fn it_consumes_the_code_so_one_gate_yields_one_credential() {
        let store = SqliteStore::in_memory().unwrap();
        let enrollments = MemoryEnrollmentStore::default();
        let emails = CapturedEmail::default();
        let anchor = signer(2).await;
        account_with_anchor(&store, &enrollments, &anchor).await;
        pending_code(&store, &emails).await;

        issue_anchor_chain(
            &store,
            &enrollments,
            &emails,
            anchor.clone(),
            EMAIL,
            CODE,
            &signer(3).await.did(),
            200,
        )
        .await
        .unwrap();

        let shadow = signer(4).await;
        assert!(matches!(
            issue_anchor_chain(
                &store,
                &enrollments,
                &emails,
                anchor,
                EMAIL,
                CODE,
                &shadow.did(),
                201,
            )
            .await,
            Err(CeremonyError::CodeInvalid)
        ));
        assert!(enrollments.claim(&shadow.did()).await.unwrap().is_empty());
    }

    #[dialog_common::test]
    async fn it_refuses_when_the_account_never_delegated_to_this_anchor() {
        let store = SqliteStore::in_memory().unwrap();
        let enrollments = MemoryEnrollmentStore::default();
        let emails = CapturedEmail::default();
        let anchor = signer(2).await;
        register(&store).await;
        pending_code(&store, &emails).await;

        assert!(matches!(
            issue_anchor_chain(
                &store,
                &enrollments,
                &emails,
                anchor,
                EMAIL,
                CODE,
                &signer(3).await.did(),
                200,
            )
            .await,
            Err(CeremonyError::Conflict(_))
        ));
    }

    /// A seed rotation must not let the service act for accounts that
    /// delegated to the key it used to hold.
    #[dialog_common::test]
    async fn it_refuses_when_the_anchor_key_is_not_the_one_delegated_to() {
        let store = SqliteStore::in_memory().unwrap();
        let enrollments = MemoryEnrollmentStore::default();
        let emails = CapturedEmail::default();
        let delegated = signer(2).await;
        account_with_anchor(&store, &enrollments, &delegated).await;
        pending_code(&store, &emails).await;

        assert!(matches!(
            issue_anchor_chain(
                &store,
                &enrollments,
                &emails,
                signer(9).await,
                EMAIL,
                CODE,
                &signer(3).await.did(),
                200,
            )
            .await,
            Err(CeremonyError::Conflict(_))
        ));
    }
}

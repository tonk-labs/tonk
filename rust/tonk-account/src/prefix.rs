//! Reusable `space → … → account-root` delegation prefixes.
//!
//! A prefix is what lets any device on the account regain authority
//! over a space: the account pull brings the retained certificates
//! down, and the prefix chain proves the space through them. This
//! module holds the credential-site naming for locally persisted
//! prefixes and the validation every consumer runs before trusting
//! one.

/// Credential site holding the exact reusable `space → … → account-root`
/// delegation prefix for a repository subject.
pub const SPACE_ROOT_SITE_PREFIX: &str = "tonk-space-root-v1/";
/// Credential prefix for account-root-specific reusable space authority.
pub const SPACE_ROOT_SITE_V2_PREFIX: &str = "tonk-space-root-v2/";

/// Credential key for one repository's prefix ending at one exact account root.
pub fn space_root_site(
    repository_did: &dialog_varsig::Did,
    account_root: &dialog_varsig::Did,
) -> String {
    format!("{SPACE_ROOT_SITE_V2_PREFIX}{repository_did}/{account_root}")
}

/// Credential prefix for exact hosted-space deletion grants.
pub const SPACE_DELETE_SITE_V1_PREFIX: &str = "tonk-space-delete-v1/";

/// Credential key for one space's deletion grant to one exact account root.
pub fn space_delete_site(
    repository_did: &dialog_varsig::Did,
    account_root: &dialog_varsig::Did,
) -> String {
    format!("{SPACE_DELETE_SITE_V1_PREFIX}{repository_did}/{account_root}")
}

/// A decoded and verified reusable prefix.
#[derive(Debug)]
pub struct ValidatedPrefix {
    /// Repository subject delegated by the chain.
    pub subject: dialog_varsig::Did,
    /// Exact verified root-ending delegation chain.
    pub chain: dialog_ucan_core::DelegationChain,
}

/// Stable validation failures for prefix chains.
#[derive(Debug, thiserror::Error)]
pub enum PrefixError {
    /// The bytes were not a delegation-chain container.
    #[error("prefix chain container is invalid: {0}")]
    InvalidChain(String),
    /// One of the delegation signatures did not verify.
    #[error("prefix chain signature is invalid: {0}")]
    InvalidSignature(String),
    /// The root delegation must be issued by the delegated repository.
    #[error("prefix chain subject does not match its issuer")]
    SubjectIssuerMismatch,
    /// Every proof must remain scoped to the root repository subject.
    #[error("prefix chain changes its repository subject")]
    SubjectChanged,
    /// The account root may occur only as the final audience.
    #[error("prefix chain continues after reaching the account root")]
    AccountRootIntermediate,
    /// Every proof must be valid at the time the prefix is consumed.
    #[error("prefix chain is not currently valid: {0}")]
    NotCurrentlyValid(String),
    /// The reusable prefix must terminate at the account root.
    #[error("prefix chain does not terminate at this account root")]
    WrongAccountRoot,
}

/// Decode and verify `bytes` as a reusable prefix ending at `account_root`.
pub async fn validate_prefix(
    bytes: &[u8],
    account_root: &dialog_varsig::Did,
) -> Result<ValidatedPrefix, PrefixError> {
    let chain = dialog_ucan_core::DelegationChain::try_from(bytes)
        .map_err(|error| PrefixError::InvalidChain(error.to_string()))?;

    let proof_count = chain.proofs().count();
    let first = chain
        .proofs()
        .next()
        .ok_or_else(|| PrefixError::InvalidChain("empty chain".to_string()))?;
    // Dialog powerline delegations carry `Subject::Any`; for a root
    // delegation their effective subject is the issuer. Repository invite
    // chains minted through an account use that shape, while an owned
    // space's direct `space -> root` prefix carries `Specific(space)`.
    // Both name the same stable authority as long as an explicit root
    // subject, when present, agrees with its issuer.
    let subject = match first.subject() {
        dialog_ucan_core::subject::Subject::Specific(subject) => {
            if subject != first.issuer() {
                return Err(PrefixError::SubjectIssuerMismatch);
            }
            subject.clone()
        }
        dialog_ucan_core::subject::Subject::Any => first.issuer().clone(),
    };

    let now = dialog_ucan_core::time::Timestamp::now();
    for (index, delegation) in chain.proofs().enumerate() {
        match delegation.subject() {
            dialog_ucan_core::subject::Subject::Specific(proof_subject)
                if proof_subject == &subject => {}
            dialog_ucan_core::subject::Subject::Specific(_) => {
                return Err(PrefixError::SubjectChanged);
            }
            // A powerline proof preserves the root delegation's
            // effective subject; it does not broaden this prefix to a
            // different repository.
            dialog_ucan_core::subject::Subject::Any => {}
        }
        if delegation.issuer() == account_root
            || delegation.audience() == account_root && index + 1 != proof_count
        {
            return Err(PrefixError::AccountRootIntermediate);
        }
        delegation
            .verify_signature(&dialog_credentials::DidKeyResolver)
            .await
            .map_err(|error| PrefixError::InvalidSignature(error.to_string()))?;
        dialog_ucan_core::time::TimeRange::new(delegation.not_before(), delegation.expiration())
            .check(&now)
            .map_err(|error| PrefixError::NotCurrentlyValid(error.to_string()))?;
    }

    if chain.audience() != account_root {
        return Err(PrefixError::WrongAccountRoot);
    }

    Ok(ValidatedPrefix { subject, chain })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn signer(seed: u8) -> dialog_credentials::Ed25519Signer {
        dialog_credentials::Ed25519Signer::import(&[seed; 32])
            .await
            .unwrap()
    }

    async fn space_chain(
        issuer: dialog_credentials::Ed25519Signer,
        audience: &dialog_varsig::Did,
        subject: &dialog_varsig::Did,
    ) -> dialog_ucan_core::DelegationChain {
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject;

        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        dialog_ucan_core::DelegationChain::new(delegation)
    }

    async fn validate(
        chain: &dialog_ucan_core::DelegationChain,
        account_root: &dialog_varsig::Did,
    ) -> Result<ValidatedPrefix, PrefixError> {
        validate_prefix(&chain.to_bytes().unwrap(), account_root).await
    }

    async fn delegation(
        issuer: dialog_credentials::Ed25519Signer,
        audience: &dialog_varsig::Did,
        subject: &dialog_varsig::Did,
        expiration: Option<dialog_ucan_core::time::Timestamp>,
    ) -> dialog_ucan_core::Delegation<dialog_varsig::AnySignature> {
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject;

        let builder = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![]);
        match expiration {
            Some(expiration) => builder.expiration(expiration).try_build().await.unwrap(),
            None => builder.try_build().await.unwrap(),
        }
    }

    #[dialog_common::test]
    async fn it_accepts_only_a_verified_space_to_account_root_prefix() {
        use dialog_varsig::Principal as _;

        let space = signer(1).await;
        let space_did = space.did();
        let account = signer(2).await.did();
        let other = signer(3).await.did();
        let valid_chain = space_chain(space.clone(), &account, &space_did).await;
        let validated = validate(&valid_chain, &account).await.unwrap();
        assert_eq!(validated.subject, space_did);
        assert_eq!(validated.chain, valid_chain);

        assert!(validate_prefix(b"not a chain", &account).await.is_err());

        let powerline = dialog_ucan_core::DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(&account)
            .subject(dialog_ucan_core::subject::Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let powerline = validate(&dialog_ucan_core::DelegationChain::new(powerline), &account)
            .await
            .unwrap();
        assert_eq!(powerline.subject, space_did);

        let wrong_subject_issuer = signer(5).await;
        let wrong_subject = space_chain(wrong_subject_issuer, &account, &other).await;
        assert!(validate(&wrong_subject, &account).await.is_err());

        assert!(validate(&validated.chain, &other).await.is_err());

        let mut corrupted = validated.chain.to_bytes().unwrap();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        assert!(validate_prefix(&corrupted, &account).await.is_err());
    }

    #[dialog_common::test]
    async fn it_rejects_aligned_suffixes_subject_changes_and_expired_proofs() {
        use dialog_ucan_core::DelegationChain;
        use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
        use dialog_varsig::Principal as _;

        let space = signer(10).await;
        let root = signer(11).await;
        let old_device = signer(12).await;
        let session = signer(13).await;
        let other_subject = signer(14).await.did();
        let subject = space.did();
        let root_did = root.did();

        let space_to_root = delegation(space.clone(), &root_did, &subject, None).await;
        let root_to_old_device = delegation(root.clone(), &old_device.did(), &subject, None).await;
        let old_device_to_root = delegation(old_device.clone(), &root_did, &subject, None).await;
        let root_suffix = DelegationChain::new(space_to_root.clone())
            .push(root_to_old_device.clone())
            .unwrap()
            .push(old_device_to_root)
            .unwrap();
        assert!(
            validate(&root_suffix, &root_did).await.is_err(),
            "a proof after reaching the account root must not be reusable"
        );

        let changed_subject = DelegationChain::new(
            delegation(space.clone(), &old_device.did(), &subject, None).await,
        )
        .push(delegation(old_device.clone(), &root_did, &other_subject, None).await)
        .unwrap();
        assert!(
            validate(&changed_subject, &root_did).await.is_err(),
            "every proof must retain the repository subject"
        );

        let session_suffix = DelegationChain::new(space_to_root)
            .push(root_to_old_device)
            .unwrap()
            .push(delegation(old_device, &session.did(), &subject, None).await)
            .unwrap()
            .push(delegation(session, &root_did, &subject, None).await)
            .unwrap();
        assert!(
            validate(&session_suffix, &root_did).await.is_err(),
            "device/session suffixes must not be reusable"
        );

        let expired_at = Timestamp::new(SystemTime::now() - Duration::from_secs(60)).unwrap();
        let expired =
            DelegationChain::new(delegation(space, &root_did, &subject, Some(expired_at)).await);
        assert!(
            validate(&expired, &root_did).await.is_err(),
            "an expired prefix is not currently valid authority"
        );
    }
}

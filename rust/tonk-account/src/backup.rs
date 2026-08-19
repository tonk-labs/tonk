//! Provider-neutral account spot backup artifacts.

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
/// Credential site holding the content key of the last account backup a
/// native client successfully uploaded for a repository subject.
pub const ACCOUNT_SPOT_BACKUP_MARKER_PREFIX: &str = "tonk-account-spot-backup-v1/";
/// Response header advertising semantic account-spot inventory support.
pub const ACCOUNT_SPOTS_CAPABILITY_HEADER: &str = "X-Tonk-Account-Spots";
/// Capability version understood by account-spot clients.
pub const ACCOUNT_SPOTS_CAPABILITY_V1: &str = "v1";

/// Reusable account-backed spot authority plus sync metadata.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountSpotBackup {
    /// Hex-encoded [`dialog_ucan_core::DelegationChain`] bytes.
    pub chain_hex: String,
    /// Access-service URL used to synchronize the spot.
    pub remote_url: Option<String>,
    /// Immutable invitation-revocation relay URL.
    #[serde(default)]
    pub revocation_url: Option<String>,
    /// Synced repository display name, absent on legacy artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One semantic spot in an account's backup inventory.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpotSummary {
    /// Repository subject DID.
    pub subject: String,
    /// Current immutable blob key, absent for an ambiguous legacy spot.
    pub key: Option<String>,
    /// Synced display name, when one was stored.
    pub name: Option<String>,
    /// Access-service URL, when one was stored.
    pub remote_url: Option<String>,
    /// Invitation-revocation relay URL, when one was stored.
    pub revocation_url: Option<String>,
    /// Whether conflicting unindexed artifacts prevent safe selection.
    pub ambiguous: bool,
}

/// A decoded and verified reusable account spot backup.
#[derive(Debug)]
pub struct ValidatedAccountSpot {
    /// Repository subject delegated by the chain.
    pub subject: dialog_varsig::Did,
    /// Exact verified root-ending delegation chain.
    pub chain: dialog_ucan_core::DelegationChain,
}

/// Stable validation failures for account spot artifacts.
#[derive(Debug, thiserror::Error)]
pub enum AccountSpotBackupError {
    /// `chain_hex` was not hexadecimal.
    #[error("backup chain is not valid hex: {0}")]
    InvalidHex(String),
    /// The decoded bytes were not a delegation-chain container.
    #[error("backup chain container is invalid: {0}")]
    InvalidChain(String),
    /// One of the delegation signatures did not verify.
    #[error("backup chain signature is invalid: {0}")]
    InvalidSignature(String),
    /// Reusable spot authority must be scoped to one subject.
    #[error("backup chain is not scoped to a repository subject")]
    MissingSubject,
    /// The root delegation must be issued by the delegated repository.
    #[error("backup chain subject does not match its issuer")]
    SubjectIssuerMismatch,
    /// Every proof must remain scoped to the root repository subject.
    #[error("backup chain changes its repository subject")]
    SubjectChanged,
    /// The account root may occur only as the final audience.
    #[error("backup chain continues after reaching the account root")]
    AccountRootIntermediate,
    /// Every proof must be valid at the time the backup is consumed.
    #[error("backup chain is not currently valid: {0}")]
    NotCurrentlyValid(String),
    /// The reusable prefix must terminate at the account root.
    #[error("backup chain does not terminate at this account root")]
    WrongAccountRoot,
    /// Empty display names carry no useful metadata.
    #[error("backup spot name must not be empty")]
    EmptyName,
    /// The sync endpoint metadata was malformed.
    #[error("backup remote URL is invalid: {0}")]
    InvalidRemoteUrl(String),
    /// The revocation relay metadata was malformed.
    #[error("backup revocation URL is invalid: {0}")]
    InvalidRevocationUrl(String),
}

impl AccountSpotBackup {
    /// Decode and verify this artifact as a reusable prefix ending at
    /// `account_root`.
    pub async fn validate_for(
        &self,
        account_root: &dialog_varsig::Did,
    ) -> Result<ValidatedAccountSpot, AccountSpotBackupError> {
        let bytes = hex::decode(&self.chain_hex)
            .map_err(|error| AccountSpotBackupError::InvalidHex(error.to_string()))?;
        let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
            .map_err(|error| AccountSpotBackupError::InvalidChain(error.to_string()))?;

        let proof_count = chain.proofs().count();
        let first = chain
            .proofs()
            .next()
            .ok_or_else(|| AccountSpotBackupError::InvalidChain("empty chain".to_string()))?;
        // Dialog powerline delegations carry `Subject::Any`; for a root
        // delegation their effective subject is the issuer. Repository invite
        // chains minted through an account use that shape, while an owned
        // space's direct `space -> root` prefix carries `Specific(space)`.
        // Both name the same stable authority as long as an explicit root
        // subject, when present, agrees with its issuer.
        let subject = match first.subject() {
            dialog_ucan_core::subject::Subject::Specific(subject) => {
                if subject != first.issuer() {
                    return Err(AccountSpotBackupError::SubjectIssuerMismatch);
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
                    return Err(AccountSpotBackupError::SubjectChanged);
                }
                // A powerline proof preserves the root delegation's
                // effective subject; it does not broaden this artifact to a
                // different repository.
                dialog_ucan_core::subject::Subject::Any => {}
            }
            if delegation.issuer() == account_root
                || delegation.audience() == account_root && index + 1 != proof_count
            {
                return Err(AccountSpotBackupError::AccountRootIntermediate);
            }
            delegation
                .verify_signature(&dialog_credentials::DidKeyResolver)
                .await
                .map_err(|error| AccountSpotBackupError::InvalidSignature(error.to_string()))?;
            dialog_ucan_core::time::TimeRange::new(
                delegation.not_before(),
                delegation.expiration(),
            )
            .check(&now)
            .map_err(|error| AccountSpotBackupError::NotCurrentlyValid(error.to_string()))?;
        }

        if chain.audience() != account_root {
            return Err(AccountSpotBackupError::WrongAccountRoot);
        }
        if self.name.as_deref() == Some("") {
            return Err(AccountSpotBackupError::EmptyName);
        }
        if let Some(remote_url) = &self.remote_url {
            url::Url::parse(remote_url)
                .map_err(|error| AccountSpotBackupError::InvalidRemoteUrl(error.to_string()))?;
        }
        if let Some(revocation_url) = &self.revocation_url {
            url::Url::parse(revocation_url)
                .map_err(|error| AccountSpotBackupError::InvalidRevocationUrl(error.to_string()))?;
        }

        Ok(ValidatedAccountSpot { subject, chain })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_legacy_unnamed_backups_and_round_trips_named_backups() {
        let legacy = r#"{
            "chain_hex":"00ff",
            "remote_url":"https://access.example/ucan/",
            "revocation_url":"https://artifacts.example/revocations/"
        }"#;
        let decoded: AccountSpotBackup = serde_json::from_str(legacy).unwrap();
        assert_eq!(decoded.name, None);

        let named = AccountSpotBackup {
            chain_hex: "00ff".to_string(),
            remote_url: Some("https://access.example/ucan/".to_string()),
            revocation_url: Some("https://artifacts.example/revocations/".to_string()),
            name: Some("garden".to_string()),
        };
        let value = serde_json::to_value(&named).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 4);
        for key in ["chain_hex", "remote_url", "revocation_url", "name"] {
            assert!(object.contains_key(key), "missing {key}: {object:?}");
        }

        let summary = AccountSpotSummary {
            subject: "did:key:zSpace".to_string(),
            key: Some("abc".to_string()),
            name: Some("garden".to_string()),
            remote_url: named.remote_url,
            revocation_url: named.revocation_url,
            ambiguous: false,
        };
        let summary = serde_json::to_value(summary).unwrap();
        assert!(summary.get("remoteUrl").is_some());
        assert!(summary.get("revocationUrl").is_some());
        assert!(summary.get("remote_url").is_none());
    }

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

    fn artifact(chain: &dialog_ucan_core::DelegationChain) -> AccountSpotBackup {
        AccountSpotBackup {
            chain_hex: hex::encode(chain.to_bytes().unwrap()),
            remote_url: Some("https://access.example/ucan/".to_string()),
            revocation_url: Some("https://artifacts.example/revocations/".to_string()),
            name: Some("garden".to_string()),
        }
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
        let validated = artifact(&valid_chain).validate_for(&account).await.unwrap();
        assert_eq!(validated.subject, space_did);
        assert_eq!(validated.chain, valid_chain);

        let malformed = AccountSpotBackup {
            chain_hex: "not-hex".to_string(),
            ..artifact(&validated.chain)
        };
        assert!(malformed.validate_for(&account).await.is_err());

        let powerline = dialog_ucan_core::DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(&account)
            .subject(dialog_ucan_core::subject::Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let powerline = artifact(&dialog_ucan_core::DelegationChain::new(powerline))
            .validate_for(&account)
            .await
            .unwrap();
        assert_eq!(powerline.subject, space_did);

        let wrong_subject_issuer = signer(5).await;
        let wrong_subject = space_chain(wrong_subject_issuer, &account, &other).await;
        assert!(
            artifact(&wrong_subject)
                .validate_for(&account)
                .await
                .is_err()
        );

        assert!(
            artifact(&validated.chain)
                .validate_for(&other)
                .await
                .is_err()
        );

        let mut corrupted = validated.chain.to_bytes().unwrap();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        let corrupted = AccountSpotBackup {
            chain_hex: hex::encode(corrupted),
            ..artifact(&validated.chain)
        };
        assert!(corrupted.validate_for(&account).await.is_err());

        for invalid in [
            AccountSpotBackup {
                name: Some(String::new()),
                ..artifact(&validated.chain)
            },
            AccountSpotBackup {
                remote_url: Some("not a URL".to_string()),
                ..artifact(&validated.chain)
            },
            AccountSpotBackup {
                revocation_url: Some("not a URL".to_string()),
                ..artifact(&validated.chain)
            },
        ] {
            assert!(invalid.validate_for(&account).await.is_err());
        }
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
            artifact(&root_suffix)
                .validate_for(&root_did)
                .await
                .is_err(),
            "a proof after reaching the account root must not be reusable"
        );

        let changed_subject = DelegationChain::new(
            delegation(space.clone(), &old_device.did(), &subject, None).await,
        )
        .push(delegation(old_device.clone(), &root_did, &other_subject, None).await)
        .unwrap();
        assert!(
            artifact(&changed_subject)
                .validate_for(&root_did)
                .await
                .is_err(),
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
            artifact(&session_suffix)
                .validate_for(&root_did)
                .await
                .is_err(),
            "device/session suffixes must not be reusable"
        );

        let expired_at = Timestamp::new(SystemTime::now() - Duration::from_secs(60)).unwrap();
        let expired =
            DelegationChain::new(delegation(space, &root_did, &subject, Some(expired_at)).await);
        assert!(
            artifact(&expired).validate_for(&root_did).await.is_err(),
            "an expired prefix is not currently valid authority"
        );
    }
}

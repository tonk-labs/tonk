//! Chain backup: immutable content-addressed blobs plus semantic account-spot
//! heads, all namespaced by an account's root DID.

use std::collections::{BTreeMap, HashSet};

use dialog_varsig::Did;
use tonk_account::backup::{AccountSpotBackup, AccountSpotSummary};

use crate::chains::{ChainStore, SpotHeadSlot};
use crate::core::CeremonyError;
use crate::store::Account;

/// The content-addressed key for `bytes`: the blake3 hash, hex-encoded.
pub fn chain_key(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn subject_key(subject: &Did) -> String {
    blake3::hash(subject.to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn account_root(account: &Account) -> Result<Did, CeremonyError> {
    account.root_did.parse().map_err(|error| {
        CeremonyError::Internal(format!("stored account root DID is invalid: {error:?}"))
    })
}

/// Store arbitrary bytes under `account.root_did`'s namespace, returning the
/// content-addressed key. Re-storing identical bytes is idempotent.
///
/// Kept as the unchanged generic core path for compatibility and for tests
/// that model blobs written before semantic spot indexing existed.
pub async fn put_chain<C: ChainStore>(
    chains: &C,
    account: &Account,
    bytes: &[u8],
) -> Result<String, CeremonyError> {
    let key = chain_key(bytes);
    chains.put(&account.root_did, &key, bytes).await?;
    Ok(key)
}

/// Store a generic chain blob and, when it is an account-spot artifact,
/// validate it and advance that repository subject's current head.
pub async fn put_chain_and_index_spot<C: ChainStore>(
    chains: &C,
    account: &Account,
    bytes: &[u8],
) -> Result<String, CeremonyError> {
    let parsed = serde_json::from_slice::<AccountSpotBackup>(bytes).ok();
    let validated = match &parsed {
        Some(backup) => Some(
            backup
                .validate_for(&account_root(account)?)
                .await
                .map_err(|error| CeremonyError::Invalid(error.to_string()))?,
        ),
        None => None,
    };

    let key = put_chain(chains, account, bytes).await?;
    let (Some(backup), Some(validated)) = (parsed, validated) else {
        return Ok(key);
    };
    let subject_key = subject_key(&validated.subject);
    let slot = if backup.name.is_some() {
        SpotHeadSlot::Named
    } else {
        SpotHeadSlot::Unnamed
    };
    chains
        .put_spot_head(&account.root_did, &subject_key, slot, &key)
        .await?;
    Ok(key)
}

/// List the chain keys backed up under `account.root_did`'s namespace.
pub async fn list_chains<C: ChainStore>(
    chains: &C,
    account: &Account,
) -> Result<Vec<String>, CeremonyError> {
    Ok(chains.list(&account.root_did).await?)
}

/// Look up the bytes backed up under `key` in `account.root_did`'s namespace.
///
/// Returns `CeremonyError::Invalid` if no such chain is backed up.
pub async fn get_chain<C: ChainStore>(
    chains: &C,
    account: &Account,
    key: &str,
) -> Result<Vec<u8>, CeremonyError> {
    chains
        .get(&account.root_did, key)
        .await?
        .ok_or_else(|| CeremonyError::Invalid("unknown chain".to_string()))
}

fn summary(
    backup: &AccountSpotBackup,
    subject: &Did,
    key: Option<String>,
    ambiguous: bool,
) -> AccountSpotSummary {
    AccountSpotSummary {
        subject: subject.to_string(),
        key,
        name: backup.name.clone(),
        remote_url: backup.remote_url.clone(),
        revocation_url: backup.revocation_url.clone(),
        ambiguous,
        deletion_ready: backup.deletion_grant_hex.is_some(),
    }
}

/// Produce one deterministic semantic row per backed-up repository subject.
/// Current heads win over all unindexed legacy candidates.
pub async fn list_account_spots<C: ChainStore>(
    chains: &C,
    account: &Account,
) -> Result<Vec<AccountSpotSummary>, CeremonyError> {
    let root = account_root(account)?;
    let mut rows = Vec::new();
    let mut claimed_subject_keys = HashSet::new();
    let mut selected_keys = HashSet::new();

    for slot in [SpotHeadSlot::Named, SpotHeadSlot::Unnamed] {
        for (stored_subject_key, blob_key) in
            chains.list_spot_heads(&account.root_did, slot).await?
        {
            selected_keys.insert(blob_key.clone());
            if !claimed_subject_keys.insert(stored_subject_key.clone()) {
                continue;
            }

            let loaded = async {
                let bytes = chains
                    .get(&account.root_did, &blob_key)
                    .await
                    .map_err(|error| format!("blob fetch failed: {error:?}"))?
                    .ok_or_else(|| "blob is missing".to_string())?;
                let backup: AccountSpotBackup = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("artifact JSON is invalid: {error}"))?;
                let validated = backup
                    .validate_for(&root)
                    .await
                    .map_err(|error| format!("artifact validation failed: {error}"))?;
                let validated_subject_key = subject_key(&validated.subject);
                if validated_subject_key != stored_subject_key {
                    return Err(format!(
                        "subject-key mismatch: validated subject hashes to {validated_subject_key}"
                    ));
                }
                Ok::<_, String>((backup, validated))
            }
            .await;

            match loaded {
                Ok((backup, validated)) => {
                    rows.push(summary(&backup, &validated.subject, Some(blob_key), false));
                }
                Err(reason) => crate::core::log_detail(&format!(
                    "omitting unusable account spot head: account_root={root} slot={slot:?} stored_subject_key={stored_subject_key} blob_key={blob_key}: {reason}"
                )),
            }
        }
    }

    let mut legacy: BTreeMap<String, Vec<(String, AccountSpotBackup, Did)>> = BTreeMap::new();
    for blob_key in chains.list(&account.root_did).await? {
        if selected_keys.contains(&blob_key) {
            continue;
        }
        let Some(bytes) = chains.get(&account.root_did, &blob_key).await? else {
            continue;
        };
        let Ok(backup) = serde_json::from_slice::<AccountSpotBackup>(&bytes) else {
            continue;
        };
        let Ok(validated) = backup.validate_for(&root).await else {
            continue;
        };
        if claimed_subject_keys.contains(&subject_key(&validated.subject)) {
            continue;
        }
        legacy
            .entry(validated.subject.to_string())
            .or_default()
            .push((blob_key, backup, validated.subject));
    }

    for candidates in legacy.into_values() {
        let (first_key, first, subject) = &candidates[0];
        let materially_different = candidates
            .iter()
            .skip(1)
            .any(|(_, candidate, _)| candidate != first);
        if materially_different {
            rows.push(AccountSpotSummary {
                subject: subject.to_string(),
                key: None,
                name: None,
                remote_url: None,
                revocation_url: None,
                ambiguous: true,
                deletion_ready: false,
            });
        } else {
            rows.push(summary(first, subject, Some(first_key.clone()), false));
        }
    }

    rows.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(rows)
}

#[cfg(all(test, feature = "helpers"))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;

    use crate::chains::{ChainError, MemoryChainStore};

    #[derive(Default)]
    struct InterleavingChainStore {
        inner: MemoryChainStore,
        unnamed_read: tokio::sync::Notify,
        named_written: tokio::sync::Notify,
    }

    impl ChainStore for InterleavingChainStore {
        async fn put(&self, root_did: &str, key: &str, bytes: &[u8]) -> Result<(), ChainError> {
            self.inner.put(root_did, key, bytes).await
        }

        async fn list(&self, root_did: &str) -> Result<Vec<String>, ChainError> {
            self.inner.list(root_did).await
        }

        async fn get(&self, root_did: &str, key: &str) -> Result<Option<Vec<u8>>, ChainError> {
            self.inner.get(root_did, key).await
        }

        async fn put_spot_head(
            &self,
            root_did: &str,
            subject_key: &str,
            slot: SpotHeadSlot,
            blob_key: &str,
        ) -> Result<(), ChainError> {
            match slot {
                SpotHeadSlot::Named => {
                    self.unnamed_read.notified().await;
                    self.inner
                        .put_spot_head(root_did, subject_key, slot, blob_key)
                        .await?;
                    self.named_written.notify_one();
                }
                SpotHeadSlot::Unnamed => {
                    self.unnamed_read.notify_one();
                    self.named_written.notified().await;
                    self.inner
                        .put_spot_head(root_did, subject_key, slot, blob_key)
                        .await?;
                }
            }
            Ok(())
        }

        async fn spot_head(
            &self,
            root_did: &str,
            subject_key: &str,
            slot: SpotHeadSlot,
        ) -> Result<Option<String>, ChainError> {
            self.inner.spot_head(root_did, subject_key, slot).await
        }

        async fn list_spot_heads(
            &self,
            root_did: &str,
            slot: SpotHeadSlot,
        ) -> Result<Vec<(String, String)>, ChainError> {
            self.inner.list_spot_heads(root_did, slot).await
        }

        async fn delete_namespace(&self, root_did: &str) -> Result<(), ChainError> {
            self.inner.delete_namespace(root_did).await
        }
    }

    fn account(id: i64, root_did: &str) -> Account {
        Account {
            id,
            email: format!("account-{id}@x.com"),
            root_did: root_did.to_string(),
            credential_id: "cred".to_string(),
            repository_descriptor: None,
            passkey_created_at: None,
            passkey_created_on: None,
            created_at: 100,
        }
    }

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    async fn backup(root: &Did, seed: u8, name: Option<&str>, remote: &str) -> (Did, Vec<u8>) {
        let space = signer(seed).await;
        let subject = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(space)
            .audience(root)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(delegation);
        let artifact = AccountSpotBackup {
            chain_hex: hex::encode(chain.to_bytes().unwrap()),
            deletion_grant_hex: None,
            remote_url: Some(remote.to_string()),
            revocation_url: None,
            name: name.map(str::to_string),
        };
        (subject, serde_json::to_vec(&artifact).unwrap())
    }

    fn artifact(chain: &DelegationChain) -> Vec<u8> {
        serde_json::to_vec(&AccountSpotBackup {
            chain_hex: hex::encode(chain.to_bytes().unwrap()),
            deletion_grant_hex: None,
            remote_url: Some("https://access.example/".to_string()),
            revocation_url: None,
            name: Some("garden".to_string()),
        })
        .unwrap()
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_spot_authority_without_storing_or_indexing_it() {
        use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};

        let chains = MemoryChainStore::default();
        let root = signer(40).await;
        let root_did = root.did();
        let account = account(1, root_did.as_ref());
        let space = signer(41).await;
        let subject = space.did();
        let device = signer(42).await;
        let other_subject = signer(43).await.did();
        let wrong_root = signer(44).await.did();

        let first = DelegationBuilder::new()
            .issuer(space.clone())
            .audience(&root_did)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let root_suffix = DelegationChain::new(first.clone())
            .push(
                DelegationBuilder::new()
                    .issuer(root)
                    .audience(&device.did())
                    .subject(Subject::Specific(subject.clone()))
                    .command(vec![])
                    .try_build()
                    .await
                    .unwrap(),
            )
            .unwrap()
            .push(
                DelegationBuilder::new()
                    .issuer(device.clone())
                    .audience(&root_did)
                    .subject(Subject::Specific(subject.clone()))
                    .command(vec![])
                    .try_build()
                    .await
                    .unwrap(),
            )
            .unwrap();
        let changed_subject = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(space.clone())
                .audience(&device.did())
                .subject(Subject::Specific(subject.clone()))
                .command(vec![])
                .try_build()
                .await
                .unwrap(),
        )
        .push(
            DelegationBuilder::new()
                .issuer(device)
                .audience(&root_did)
                .subject(Subject::Specific(other_subject))
                .command(vec![])
                .try_build()
                .await
                .unwrap(),
        )
        .unwrap();
        let expired = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(space.clone())
                .audience(&root_did)
                .subject(Subject::Specific(subject.clone()))
                .command(vec![])
                .expiration(Timestamp::new(SystemTime::now() - Duration::from_secs(60)).unwrap())
                .try_build()
                .await
                .unwrap(),
        );
        let wrong_root = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(space)
                .audience(&wrong_root)
                .subject(Subject::Specific(subject))
                .command(vec![])
                .try_build()
                .await
                .unwrap(),
        );
        let mut corrupted_chain = first.encoded().to_vec();
        let last = corrupted_chain.len() - 1;
        corrupted_chain[last] ^= 1;
        let corrupted = AccountSpotBackup {
            chain_hex: hex::encode(
                DelegationChain::from_delegation_bytes(vec![corrupted_chain])
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
            ),
            deletion_grant_hex: None,
            remote_url: Some("https://access.example/".to_string()),
            revocation_url: None,
            name: Some("garden".to_string()),
        };

        let mut invalid = vec![
            artifact(&root_suffix),
            artifact(&changed_subject),
            artifact(&expired),
            artifact(&wrong_root),
        ];
        invalid.push(serde_json::to_vec(&corrupted).unwrap());
        for bytes in invalid {
            assert!(matches!(
                put_chain_and_index_spot(&chains, &account, &bytes).await,
                Err(CeremonyError::Invalid(_))
            ));
        }
        assert!(chains.list(&account.root_did).await.unwrap().is_empty());
        assert!(
            chains
                .list_spot_heads(&account.root_did, SpotHeadSlot::Named)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            chains
                .list_spot_heads(&account.root_did, SpotHeadSlot::Unnamed)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[dialog_common::test]
    async fn it_indexes_one_current_head_per_spot_subject() {
        let chains = MemoryChainStore::default();
        let root = signer(50).await.did();
        let account = account(1, root.as_ref());
        let (subject, first) = backup(&root, 51, Some("garden"), "https://one.example/").await;
        let (_, second) = backup(&root, 51, Some("renamed"), "https://two.example/").await;
        let (_, unnamed) = backup(&root, 51, None, "https://three.example/").await;

        let first_key = put_chain_and_index_spot(&chains, &account, &first)
            .await
            .unwrap();
        let second_key = put_chain_and_index_spot(&chains, &account, &second)
            .await
            .unwrap();
        put_chain_and_index_spot(&chains, &account, &second)
            .await
            .unwrap();
        put_chain_and_index_spot(&chains, &account, &unnamed)
            .await
            .unwrap();

        assert_ne!(first_key, second_key);
        let rows = list_account_spots(&chains, &account).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject, subject.to_string());
        assert_eq!(rows[0].key.as_deref(), Some(second_key.as_str()));
        assert_eq!(rows[0].name.as_deref(), Some("renamed"));
        assert_eq!(
            chains
                .list_spot_heads(&account.root_did, SpotHeadSlot::Named)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(chains.list(&account.root_did).await.unwrap().len(), 3);
    }

    #[dialog_common::test]
    async fn a_late_unnamed_writer_never_erases_a_named_head() {
        let chains = InterleavingChainStore::default();
        let root = signer(55).await.did();
        let account = account(1, root.as_ref());
        let (_, named) = backup(&root, 56, Some("garden"), "https://named.example/").await;
        let (_, unnamed) = backup(&root, 56, None, "https://unnamed.example/").await;

        let (unnamed_result, named_result) = tokio::join!(
            put_chain_and_index_spot(&chains, &account, &unnamed),
            put_chain_and_index_spot(&chains, &account, &named),
        );
        unnamed_result.unwrap();
        let named_key = named_result.unwrap();

        let rows = list_account_spots(&chains, &account).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name.as_deref(), Some("garden"));
        assert_eq!(rows[0].key.as_deref(), Some(named_key.as_str()));
    }

    #[dialog_common::test]
    async fn it_omits_unusable_heads_without_poisoning_healthy_rows_or_reviving_legacy_rows() {
        let chains = MemoryChainStore::default();
        let root = signer(57).await.did();
        let account = account(1, root.as_ref());

        let (healthy_subject, healthy) =
            backup(&root, 58, Some("healthy"), "https://healthy.example/").await;
        put_chain_and_index_spot(&chains, &account, &healthy)
            .await
            .unwrap();

        let (broken_subject, older_valid) =
            backup(&root, 59, Some("older"), "https://older.example/").await;
        let older_valid_key = put_chain(&chains, &account, &older_valid).await.unwrap();
        let malformed_key = put_chain(&chains, &account, b"not json").await.unwrap();
        let broken_subject_key = subject_key(&broken_subject);
        chains
            .put_spot_head(
                &account.root_did,
                &broken_subject_key,
                SpotHeadSlot::Unnamed,
                &older_valid_key,
            )
            .await
            .unwrap();
        chains
            .put_spot_head(
                &account.root_did,
                &broken_subject_key,
                SpotHeadSlot::Named,
                &malformed_key,
            )
            .await
            .unwrap();

        let (missing_subject, _) =
            backup(&root, 63, Some("missing"), "https://missing.example/").await;
        chains
            .put_spot_head(
                &account.root_did,
                &subject_key(&missing_subject),
                SpotHeadSlot::Named,
                "missing-blob",
            )
            .await
            .unwrap();

        let (mismatched_subject, mismatched) =
            backup(&root, 64, Some("mismatched"), "https://mismatch.example/").await;
        let mismatched_key = put_chain(&chains, &account, &mismatched).await.unwrap();
        let (stored_subject, _) =
            backup(&root, 65, Some("stored"), "https://stored.example/").await;
        chains
            .put_spot_head(
                &account.root_did,
                &subject_key(&stored_subject),
                SpotHeadSlot::Named,
                &mismatched_key,
            )
            .await
            .unwrap();

        let rows = list_account_spots(&chains, &account).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject, healthy_subject.to_string());
        assert!(!rows.iter().any(|row| {
            row.subject == broken_subject.to_string()
                || row.subject == missing_subject.to_string()
                || row.subject == mismatched_subject.to_string()
                || row.subject == stored_subject.to_string()
        }));
    }

    #[dialog_common::test]
    async fn it_reports_legacy_and_ambiguous_spots_without_poisoning_valid_rows() {
        let chains = MemoryChainStore::default();
        let root = signer(60).await.did();
        let account = account(1, root.as_ref());
        put_chain(&chains, &account, b"not json").await.unwrap();

        let (single_subject, single) = backup(&root, 61, None, "https://one.example/").await;
        put_chain(&chains, &account, &single).await.unwrap();
        let (conflict_subject, conflict_a) = backup(&root, 62, None, "https://a.example/").await;
        let (_, conflict_b) = backup(&root, 62, None, "https://b.example/").await;
        put_chain(&chains, &account, &conflict_a).await.unwrap();
        put_chain(&chains, &account, &conflict_b).await.unwrap();

        let rows = list_account_spots(&chains, &account).await.unwrap();
        assert_eq!(rows.len(), 2);
        let single = rows
            .iter()
            .find(|row| row.subject == single_subject.to_string())
            .unwrap();
        assert!(single.key.is_some());
        assert!(!single.ambiguous);
        let conflict = rows
            .iter()
            .find(|row| row.subject == conflict_subject.to_string())
            .unwrap();
        assert!(conflict.key.is_none());
        assert!(conflict.ambiguous);
    }

    #[dialog_common::test]
    async fn it_stores_chains_content_addressed_and_idempotent() {
        let chains = MemoryChainStore::default();
        let account = account(1, "did:key:root1");
        let bytes = b"chain-bytes";

        let key1 = put_chain(&chains, &account, bytes).await.unwrap();
        let key2 = put_chain(&chains, &account, bytes).await.unwrap();

        assert_eq!(key1, key2);
        let listed = list_chains(&chains, &account).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], key1);
    }

    #[dialog_common::test]
    async fn it_scopes_backups_to_the_root_did() {
        let chains = MemoryChainStore::default();
        let account_a = account(1, "did:key:root-a");
        let account_b = account(2, "did:key:root-b");

        put_chain(&chains, &account_a, b"a-bytes").await.unwrap();
        put_chain(&chains, &account_b, b"b-bytes").await.unwrap();

        let listed_a = list_chains(&chains, &account_a).await.unwrap();
        let listed_b = list_chains(&chains, &account_b).await.unwrap();
        assert_eq!(listed_a.len(), 1);
        assert_eq!(listed_b.len(), 1);
        assert_ne!(listed_a[0], listed_b[0]);
    }

    #[dialog_common::test]
    async fn it_round_trips_chain_bytes() {
        let chains = MemoryChainStore::default();
        let account = account(1, "did:key:root1");
        let bytes = b"chain-bytes".to_vec();

        let key = put_chain(&chains, &account, &bytes).await.unwrap();
        let round_tripped = get_chain(&chains, &account, &key).await.unwrap();

        assert_eq!(round_tripped, bytes);
    }

    #[dialog_common::test]
    async fn it_refuses_a_chain_get_across_accounts() {
        let chains = MemoryChainStore::default();
        let account_a = account(1, "did:key:root-a");
        let account_b = account(2, "did:key:root-b");

        let key = put_chain(&chains, &account_a, b"a-bytes").await.unwrap();

        assert!(matches!(
            get_chain(&chains, &account_b, &key).await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}

//! Native account-system repository lifecycle.
//!
//! Account bytes live under the spot store's dedicated `account/` directory,
//! never in a named spot or in `spots.json`. The trusted marker remains a
//! profile credential so account status can be read without opening any spot.

use std::path::Path;

use anyhow::{Context, Result, bail};
use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::credential::CredentialError;
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_effects::storage::Directory;
use dialog_operator::{Operator, Profile};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Repository, RepositoryExt as _, SiteAddress, Upstream};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use tonk_account::{
    AccountRepositoryDescriptorV1, AccountStateStatus, CreateGenesis, RemotePresence,
    probe_remote_main, publish_genesis_if_absent,
};
use tonk_schema::{Replica, prelude::DidExt as _};

/// Stable derivation context for the account-system operator.
///
/// This must never be replaced with the historical spot context (`slide`):
/// changing either context re-derives an operator DID and invalidates existing
/// authority chains.
const ACCOUNT_OPERATOR_CONTEXT: &[u8] = b"tonk/account-state/v1";
const META_BRANCH: &str = "meta";

/// Result of an ensure attempt. Remote failures are a durable Unhydrated
/// status plus diagnostics, not a failure of the already-persisted account
/// link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureOutcome {
    /// Durable lifecycle status after the attempt.
    pub status: AccountStateStatus,
    /// Remote/local diagnostic when readiness was not acquired.
    pub warning: Option<String>,
}

pub(crate) fn credential_is_missing(error: &CredentialError) -> bool {
    match error {
        CredentialError::NotFound(_) => true,
        // The native filesystem provider currently converts io::Error into a
        // string-only CredentialError::Storage. Compare the concrete OS
        // NotFound message; no other local storage failure is absence.
        CredentialError::Storage(message) => {
            message.contains(&std::io::Error::from_raw_os_error(2).to_string())
        }
        CredentialError::Corrupted(_) => false,
    }
}

async fn marker(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<Option<Vec<u8>>> {
    match profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .load::<Vec<u8>>()
        .perform(operator)
        .await
    {
        Ok(marker) => Ok(Some(marker)),
        Err(error) if credential_is_missing(&error) => Ok(None),
        Err(error) => Err(error).context("failed to load account trusted-base marker"),
    }
}

async fn save_marker(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    descriptor: &AccountRepositoryDescriptorV1,
) -> Result<()> {
    profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .save(descriptor.content_hash().to_vec())
        .perform(operator)
        .await
        .context("failed to save account trusted-base marker")
}

fn marker_matches(marker: Option<&[u8]>, descriptor: &AccountRepositoryDescriptorV1) -> bool {
    marker == Some(descriptor.content_hash().as_slice())
}

/// The descriptor this profile is configured with, absent when the local link
/// is missing or is still a legacy raw delegation.
async fn descriptor(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<AccountRepositoryDescriptorV1>> {
    Ok(
        crate::account::stored_provider_with_operator(profile, operator)
            .await?
            .and_then(|provider| provider.descriptor().cloned()),
    )
}

/// Read durable native account-state status without contacting the remote.
pub async fn status(profile: &Profile) -> Result<AccountStateStatus> {
    let operator = credential_operator(profile).await?;
    let Some(descriptor) = descriptor(profile, &operator).await? else {
        return Ok(AccountStateStatus::Unconfigured);
    };
    if marker_matches(marker(profile, &operator).await?.as_deref(), &descriptor) {
        Ok(AccountStateStatus::Ready)
    } else {
        Ok(AccountStateStatus::Unhydrated)
    }
}

async fn operator_with_profile(
    profile: &Profile,
    root: &Path,
    profile_name: &str,
    profile_directory: Directory,
) -> Result<Operator<NativeSpace>> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create account state at {}", root.display()))?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))?;
    let root = root
        .to_str()
        .with_context(|| format!("non-UTF-8 account state path: {}", root.display()))?;
    let storage = Storage::<NativeSpace>::default();
    let mounted = Profile::load(profile_name)
        .at(profile_directory)
        .perform(&storage)
        .await
        .with_context(|| format!("failed to mount profile '{profile_name}' for account state"))?;
    if mounted.did() != profile.did() {
        bail!("account-state profile does not match the active CLI profile");
    }
    mounted
        .derive(ACCOUNT_OPERATOR_CONTEXT)
        .allow(Subject::any())
        .base(Directory::At(root.to_owned()))
        .build(storage)
        .await
        .context("failed to build account-state operator")
}

/// Build the stable account operator used by both credentials and repository
/// storage.
pub(crate) async fn credential_operator_for_store(
    profile: &Profile,
    store: &crate::spot::SpotStore,
) -> Result<Operator<NativeSpace>> {
    let root = store.account_dir();
    std::fs::create_dir_all(&root)
        .with_context(|| format!("failed to create account state at {}", root.display()))?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))?;
    let root = root
        .to_str()
        .with_context(|| format!("non-UTF-8 account state path: {}", root.display()))?;
    profile
        .derive(ACCOUNT_OPERATOR_CONTEXT)
        .allow(Subject::any())
        .base(Directory::At(root.to_owned()))
        .build(Storage::<NativeSpace>::default())
        .await
        .context("failed to build account-state operator")
}

pub(crate) async fn credential_operator(profile: &Profile) -> Result<Operator<NativeSpace>> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    operator_with_profile(
        profile,
        &store.account_dir(),
        crate::site::PROFILE_NAME,
        Directory::Profile,
    )
    .await
}

async fn mount(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    descriptor: &AccountRepositoryDescriptorV1,
) -> Result<Repository> {
    let subject = descriptor.account_subject().clone();
    let key = subject.repo_key();
    let repository = match profile.repository(key).load().perform(operator).await {
        Ok(repository) => repository,
        Err(_) => {
            let verifier: Ed25519Verifier = subject
                .to_string()
                .parse()
                .context("account subject is not an Ed25519 did:key")?;
            let local = Subject::from(profile.did()).attenuate(Space::new(key));
            Repository::from(
                local
                    .create(Credential::from(verifier))
                    .perform(operator)
                    .await
                    .context("failed to mount local account repository")?,
            )
        }
    };

    let address = SiteAddress::from(UcanAddress::new(descriptor.remote().as_str()));
    let remote = match repository
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(operator)
        .await
    {
        Ok(remote) => {
            if remote.address().site() != &address || remote.did() != subject {
                bail!("mounted account repository has different immutable remote configuration");
            }
            remote
        }
        Err(_) => repository
            .remote(tonk_account::ORIGIN_REMOTE)
            .create(address.clone())
            .subject(subject.clone())
            .perform(operator)
            .await
            .context("failed to configure account remote")?,
    };

    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account main branch")?;
    let remote_branch = remote
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account remote main")?;
    match branch.upstream() {
        Some(Upstream::Remote { remote, branch, .. })
            if remote == tonk_account::ORIGIN_REMOTE && branch == tonk_account::MAIN_BRANCH => {}
        Some(_) => bail!("mounted account main tracks a different upstream"),
        None => branch
            .set_upstream(&remote_branch)
            .perform(operator)
            .await
            .context("failed to set account upstream")?,
    }

    let replica = Replica::account(profile.did(), subject);
    repository
        .branch(META_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account meta")?
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(META_BRANCH))
        .assert(replica.branch(tonk_account::MAIN_BRANCH))
        .commit()
        .perform(operator)
        .await
        .context("failed to stamp account replica kind")?;

    Ok(repository)
}

async fn hydrate(
    repository: &Repository,
    operator: &crate::account_authority::AccountBoundOperator,
) -> Result<()> {
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await?;
    let remote = repository
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(operator)
        .await?
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await?;

    match probe_remote_main(&remote, operator).await {
        Ok(RemotePresence::Present(_)) => {
            branch.pull().perform(operator).await?;
            let winner = match probe_remote_main(&remote, operator).await? {
                RemotePresence::Present(winner) => winner,
                RemotePresence::Absent => bail!("account remote disappeared after pull"),
            };
            if branch.revision().as_ref() != Some(&winner) {
                branch.reset(winner).perform(operator).await?;
            }
        }
        Ok(RemotePresence::Absent) => {
            let genesis = branch.transaction().commit().perform(operator).await?;
            if let CreateGenesis::Loser(winner) =
                publish_genesis_if_absent(&remote, genesis, operator).await?
            {
                // Adopt the winner directly, with no fetch ahead of it: fact
                // reads resolve missing blocks through the configured remote,
                // so this is readable even when the winner already wrote past
                // genesis. See `account_remote`'s losing-adoption test.
                branch.reset(winner).perform(operator).await?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Ensure the native account repository after linking or on demand.
pub async fn ensure(profile: &Profile) -> Result<EnsureOutcome> {
    ensure_with_operator(profile, credential_operator(profile).await?).await
}

async fn ensure_with_operator(
    profile: &Profile,
    operator: Operator<NativeSpace>,
) -> Result<EnsureOutcome> {
    let Some(descriptor) = descriptor(profile, &operator).await? else {
        return Ok(EnsureOutcome {
            status: AccountStateStatus::Unconfigured,
            warning: None,
        });
    };
    let repository = mount(profile, &operator, &descriptor).await?;
    let operator = crate::account_authority::wrap(
        operator,
        profile.clone(),
        crate::spot::SpotStore::open().context("failed to locate account state")?,
    )
    .await?;

    if marker_matches(
        marker(profile, operator.local()).await?.as_deref(),
        &descriptor,
    ) {
        // Ready remains ready offline. A best-effort normal sync catches up
        // without clearing the durable trust marker on failure.
        let branch = repository
            .branch(tonk_account::MAIN_BRANCH)
            .open()
            .perform(&operator)
            .await?;
        let warning = match branch.pull().perform(&operator).await {
            Ok(_) => branch
                .push()
                .perform(&operator)
                .await
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(error.to_string()),
        };
        return Ok(EnsureOutcome {
            status: AccountStateStatus::Ready,
            warning,
        });
    }

    match hydrate(&repository, &operator).await {
        Ok(()) => {
            save_marker(profile, operator.local(), &descriptor).await?;
            Ok(EnsureOutcome {
                status: AccountStateStatus::Ready,
                warning: None,
            })
        }
        Err(error) => Ok(EnsureOutcome {
            status: AccountStateStatus::Unhydrated,
            warning: Some(error.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;

    use super::*;

    #[dialog_common::test]
    async fn it_requires_the_exact_descriptor_hash() {
        let root = Ed25519Signer::import(&[7; 32]).await.unwrap();
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        assert!(marker_matches(
            Some(&descriptor.content_hash()),
            &descriptor
        ));
        assert!(!marker_matches(Some(&[8; 32]), &descriptor));
        assert!(!marker_matches(None, &descriptor));
    }

    #[dialog_common::test]
    async fn it_ensures_account_state_outside_the_spot_registry() {
        use dialog_common::helpers::Provisionable as _;
        use dialog_operator::Profile;
        use dialog_ucan::UcanDelegation;
        use tonk_access_service::helpers::AccessServiceAddress;

        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::spot::SpotStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let profile_name = format!("cli-account-test-{}", rand::random::<u64>());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(&profile_name)
            .at(profile_dir.clone())
            .perform(&storage)
            .await
            .unwrap();
        let root = Ed25519Signer::generate().await.unwrap();
        let remote = format!(
            "{}/",
            service.address.access_service_url.trim_end_matches('/')
        );
        let descriptor = AccountRepositoryDescriptorV1::sign(&root, &remote)
            .await
            .unwrap();
        let root_did = {
            use dialog_varsig::Principal as _;
            root.did()
        };
        let delegation = tonk_identity::delegation::mint_device_delegation(root, &profile.did())
            .await
            .unwrap();
        let account_operator = operator_with_profile(
            &profile,
            &store.account_dir(),
            &profile_name,
            profile_dir.clone(),
        )
        .await
        .unwrap();
        profile
            .save(UcanDelegation(delegation.clone()))
            .perform(&account_operator)
            .await
            .unwrap();
        // Lay the two records down exactly where `account::persist` puts them:
        // the profile's own credential store, through the same encoding. The
        // descriptor is read back by a different code path than the one that
        // writes it, and an earlier revision of this merge wrote it here and
        // read it through the spot's account operator — which found nothing
        // and reported an established account as unconfigured.
        // Lay down the same two records `account::link` persists, through the
        // shared type, so the descriptor this reads back is one that was
        // actually bound to the local root rather than hand-stitched bytes.
        let local_root = crate::identity::LocalRoot {
            credential_id: "cli-account-test-credential".to_string(),
            root_did: root_did.to_string(),
            delegation_cid: delegation.proof_cids()[0].to_string(),
            delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        };
        profile
            .credential()
            .site(crate::identity::LOCAL_ROOT_SITE)
            .save(serde_json::to_vec(&local_root).unwrap())
            .perform(&account_operator)
            .await
            .unwrap();
        let attachment = tonk_account::AccountProviderRecord::attach(
            "https://accounts.example",
            descriptor.bytes(),
            &root_did,
            1,
        )
        .await
        .unwrap();
        profile
            .credential()
            .site(crate::account::ACCOUNT_LINK_SITE)
            .save(attachment.encode().unwrap())
            .perform(&account_operator)
            .await
            .unwrap();
        assert_eq!(
            super::descriptor(&profile, &account_operator)
                .await
                .unwrap()
                .expect("a linked profile reads back the descriptor it stored")
                .bytes(),
            descriptor.bytes(),
        );

        let ensure_operator = operator_with_profile(
            &profile,
            &store.account_dir(),
            &profile_name,
            profile_dir.clone(),
        )
        .await
        .unwrap();
        let outcome = ensure_with_operator(&profile, ensure_operator)
            .await
            .unwrap();
        assert_eq!(
            outcome.status,
            AccountStateStatus::Ready,
            "ensure warning: {:?}",
            outcome.warning
        );
        assert!(store.account_dir().is_dir());
        assert!(!store.registry_path().exists());
        assert!(!store.canonical_site("account").exists());

        service.stop().await.unwrap();
        let offline_operator =
            operator_with_profile(&profile, &store.account_dir(), &profile_name, profile_dir)
                .await
                .unwrap();
        let offline = ensure_with_operator(&profile, offline_operator)
            .await
            .unwrap();
        assert_eq!(offline.status, AccountStateStatus::Ready);
    }
}

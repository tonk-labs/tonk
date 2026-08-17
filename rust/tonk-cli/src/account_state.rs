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
use dialog_operator::{DeriveOperator, Operator, Profile};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Repository, RepositoryExt as _, SiteAddress, Upstream};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan_core::DelegationChain;
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
/// Remote name for the account's access branch in the profile repository.
const ACCOUNT_ACCESS_REMOTE: &str = "account-access";

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
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    descriptor_in(profile, operator, &store).await
}

/// [`descriptor`] against a caller-supplied store.
async fn descriptor_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<Option<AccountRepositoryDescriptorV1>> {
    Ok(crate::account::stored_provider_in(profile, operator, store)
        .await?
        .and_then(|provider| provider.descriptor().cloned()))
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

/// Point this profile's access branch at the account and pull it.
///
/// The account repository syncing with its own remote is not enough: the
/// operator resolves proofs from the PROFILE's access branch, so authority
/// living in the account is present but unusable until the access branch
/// adopts it. This is what makes a recovered delegation authorize anything.
///
/// `Ok(false)` means there was nothing to adopt — no account, or one with no
/// trusted base — which is ordinary for a profile that has not signed in.
pub async fn adopt_account_access(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<bool> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    adopt_account_access_in(profile, operator, &store).await
}

/// [`adopt_account_access`] against a caller-supplied store.
pub async fn adopt_account_access_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<bool> {
    let Some(descriptor) = descriptor_in(profile, operator, store).await? else {
        return Ok(false);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &descriptor) {
        return Ok(false);
    }
    let subject = descriptor.account_subject().clone();
    let repository = Repository::from(profile);
    let access = repository
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open the profile access branch")?;

    // A remote resolved against the ACCOUNT's DID. A local upstream would
    // resolve against this profile's own subject and could only name a
    // sibling branch, never the account's.
    let address = SiteAddress::from(UcanAddress::new(descriptor.remote().as_str()));
    let remote = match repository
        .remote(ACCOUNT_ACCESS_REMOTE)
        .load()
        .perform(operator)
        .await
    {
        Ok(remote) => remote,
        Err(_) => repository
            .remote(ACCOUNT_ACCESS_REMOTE)
            .create(address)
            .subject(subject)
            .perform(operator)
            .await
            .context("failed to configure the account access remote")?,
    };
    let upstream = remote
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open the account access branch")?;

    tonk_account::delegations::adopt_account_upstream(&access, upstream, operator)
        .await
        .context("failed to adopt the account as the access upstream")?;
    Ok(true)
}

/// Mount this profile's account repository and open its branch.
///
/// Mounting is idempotent — an already-mounted repository loads, and its
/// immutable remote configuration is verified rather than rewritten — so a
/// caller may open the account branch without knowing whether this device has
/// touched it before.
///
/// `Ok(None)` means there is nothing to mount: no account is configured, or
/// the one that is has no trusted remote base yet. That is an ordinary state
/// for a profile that has not signed in or hydrated.
pub async fn open_account_branch(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<dialog_repository::Branch>> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    open_account_branch_in(profile, operator, &store).await
}

/// [`open_account_branch`] against a caller-supplied store.
pub async fn open_account_branch_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<Option<dialog_repository::Branch>> {
    let Some(descriptor) = descriptor_in(profile, operator, store).await? else {
        return Ok(None);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &descriptor) {
        return Ok(None);
    }
    let repository = mount(profile, operator, &descriptor).await?;
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account main branch")?;
    Ok(Some(branch))
}

/// Retain a `space → account-root` delegation into this profile's account
/// space, resolving the branch on the way.
///
/// The retain itself is [`tonk_account::delegations::retain_space_delegation`],
/// shared with the worker so both adapters retain the same thing. What is
/// local here is resolving the account repository against the caller's own
/// operator, so the retain runs in whatever storage the caller already opened
/// rather than reaching for the global install store.
///
/// `Ok(false)` means there was nowhere to retain — no account, or one whose
/// repository is not mounted — which is an ordinary state for a profile that
/// has not signed in or hydrated, rather than a failure. A real failure is
/// returned rather than swallowed, so the caller can decide: space creation
/// treats it as non-fatal, because a space is fully usable the moment its
/// delegation reaches the profile's own access branch and failing creation
/// over an unreachable account repository would trade a working space for a
/// recoverable one.
pub async fn retain_space_delegation(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    chain: &DelegationChain,
) -> Result<bool> {
    let Some(branch) = open_account_branch(profile, operator).await? else {
        return Ok(false);
    };
    tonk_account::delegations::retain_space_delegation(&branch, chain, operator)
        .await
        .context("failed to retain space delegation")
}

/// What a delegation migration did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// Legacy certificate-store entries drained into access-branch facts.
    pub certificates: usize,
    /// Spots whose authority was retained into the account space.
    pub spots: usize,
    /// Spots already retained, so nothing was written for them.
    pub already: usize,
    /// The account repository was legacy and could not receive retained spots.
    pub account_legacy: bool,
}

fn account_repository_readability(
    store: &crate::spot::SpotStore,
    subject: &dialog_varsig::Did,
) -> tonk_account::Readability {
    let revision = store.account_dir().join(tonk_account::revision_path(
        subject.repo_key(),
        tonk_account::MAIN_BRANCH,
    ));
    let bytes = std::fs::read(revision).ok();
    tonk_account::readability(bytes.as_deref())
}

/// Move this profile's delegations into their durable homes.
///
/// Two things move, and both are idempotent:
///
/// 1. **The legacy certificate store into the access branch.** Dialog now
///    keeps delegations as `dialog.ucan/*` facts; older profiles hold them in
///    a certificate store the proof walk is migrating away from. Dialog's own
///    `migrate` drains it, skipping self-issued certificates — an operator's
///    session grants are re-minted in memory on every build, so persisting
///    them only accumulated one per build forever.
///
/// 2. **Each spot's authority into the account space.** The account
///    repository is the durable home of delegations: retaining a spot's
///    `space → account-root` prefix there is what lets the next device regain
///    access by pulling, instead of fetching a backup artifact.
///
/// Spots that fail individually are counted and skipped rather than aborting
/// the run, so one unreadable spot cannot block migrating the rest.
///
/// This form resolves the operator and spot registry from the install; the
/// [`migrate_delegations`] form takes them, for callers that already hold one.
pub async fn migrate_delegations_here() -> Result<MigrationOutcome> {
    let storage = Storage::<NativeSpace>::default();
    let profile = Profile::load(crate::site::PROFILE_NAME)
        .at(Directory::Profile)
        .perform(&storage)
        .await
        .context("failed to mount the profile for delegation migration")?;
    let operator = credential_operator(&profile).await?;
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    migrate_delegations(&profile, &operator, &storage, &store).await
}

/// [`migrate_delegations_here`] against a caller-supplied profile, operator,
/// storage, and spot store.
///
/// `profile` must be mounted in `storage`: the certificate migration commits
/// as the profile, so a storage without it errors rather than silently
/// migrating nothing.
pub async fn migrate_delegations(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    storage: &Storage<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<MigrationOutcome> {
    use dialog_repository::MigrateAccess as _;

    // Migration commits as the profile itself, so it runs against the storage
    // the profile is mounted in. Passing a fresh `Storage::default()` would
    // fail with "no provider found for subject" — the profile has to be
    // mounted there first, which is what `Profile::load` does.
    let certificates = profile
        .access()
        .migrate()
        .perform(storage)
        .await
        .map_err(|error| anyhow::anyhow!("failed to migrate the certificate store: {error}"))?
        .len();
    let mut outcome = MigrationOutcome {
        certificates,
        ..MigrationOutcome::default()
    };

    // Without a hydrated account there is nowhere to retain, which is an
    // ordinary state rather than a failure: the certificate migration above
    // still stands on its own.
    let Some(descriptor) = descriptor(profile, operator).await? else {
        return Ok(outcome);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &descriptor) {
        return Ok(outcome);
    }
    let account_root = match crate::identity::local_root_with_operator(profile, operator).await? {
        Some(root) => root
            .root_did
            .parse::<dialog_varsig::Did>()
            .context("stored root DID is invalid")?,
        None => return Ok(outcome),
    };
    if account_repository_readability(store, descriptor.account_subject())
        == tonk_account::Readability::Legacy
    {
        outcome.account_legacy = true;
        return Ok(outcome);
    }
    let repository = mount(profile, operator, &descriptor).await?;
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account main branch")?;

    for entry in store.load()?.spots.values() {
        let Ok(site) = crate::site::TonkSite::open(&entry.site).await else {
            continue;
        };
        let subject = site.repository.did();
        let Ok(chain) =
            crate::site::account_root_prefix_for(profile, operator, &subject, &account_root).await
        else {
            continue;
        };
        match tonk_account::delegations::retain_space_delegation(&branch, &chain, operator).await {
            Ok(true) => outcome.spots += 1,
            Ok(false) => outcome.already += 1,
            Err(_) => continue,
        }
    }

    Ok(outcome)
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
    let default_store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    // Persisted profiles must be remounted before deriving another operator;
    // isolated test stores use the caller's in-memory profile directly.
    if root == default_store.account_dir() {
        return operator_with_profile(
            profile,
            &root,
            crate::site::PROFILE_NAME,
            Directory::Profile,
        )
        .await;
    }
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
        }
        Ok(RemotePresence::Absent) => {
            branch.transaction().commit().perform(operator).await?;
            if let CreateGenesis::Loser(_) =
                publish_genesis_if_absent(&branch, &remote, operator).await?
            {
                // Adopt the winner by pulling: the pull integrates the
                // established head AND records it as this branch's sync
                // base, so the next push fast-forwards instead of CASing
                // against an empty upstream. Reads resolve missing blocks
                // through the configured remote. See `account_remote`'s
                // losing-adoption test.
                branch.pull().perform(operator).await?;
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

/// [`ensure`] against a caller-supplied operator.
///
/// [`ensure`] resolves the operator from the global install, mounting the
/// profile by name — which a caller that already holds a profile cannot
/// satisfy. This form takes the operator, so the same path is reachable
/// from a test or an embedder without reaching for install state.
pub async fn ensure_with_operator(
    profile: &Profile,
    operator: Operator<NativeSpace>,
) -> Result<EnsureOutcome> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    ensure_with_operator_and_store(profile, operator, store).await
}

/// [`ensure_with_operator`] against a caller-supplied spot store.
///
/// The store locates account state on disk. A caller running outside an
/// install — a test, or an embedder with its own layout — supplies one rather
/// than having the real install directory resolved behind its back.
pub async fn ensure_with_operator_and_store(
    profile: &Profile,
    operator: Operator<NativeSpace>,
    store: crate::spot::SpotStore,
) -> Result<EnsureOutcome> {
    let Some(descriptor) = descriptor_in(profile, &operator, &store).await? else {
        return Ok(EnsureOutcome {
            status: AccountStateStatus::Unconfigured,
            warning: None,
        });
    };
    let repository = mount(profile, &operator, &descriptor).await?;
    let operator =
        crate::account_authority::wrap(operator, profile.clone(), store.clone(), true).await?;

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
        // Adopting the account as the access branch's upstream is what makes
        // recovered authority usable: the operator proves from the access
        // branch, not the account's. Best-effort, like the sync above — a
        // device that cannot reach the account is still ready with whatever
        // it already holds.
        let warning = match adopt_account_access_in(profile, operator.local(), &store).await {
            Ok(_) => warning,
            Err(error) => warning.or_else(|| Some(error.to_string())),
        };
        return Ok(EnsureOutcome {
            status: AccountStateStatus::Ready,
            warning,
        });
    }

    match hydrate(&repository, &operator).await {
        Ok(()) => {
            save_marker(profile, operator.local(), &descriptor).await?;
            // A device reaching Ready for the FIRST time needs the access
            // upstream just as much as one that was already ready — more so,
            // since this is the path a fresh link takes. Adopting only in the
            // marker-matched branch above left exactly that device unable to
            // use the authority it had just been granted.
            let warning = adopt_account_access_in(profile, operator.local(), &store)
                .await
                .err()
                .map(|error| error.to_string());
            Ok(EnsureOutcome {
                status: AccountStateStatus::Ready,
                warning,
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
    use tonk_schema::prelude::DidExt as _;

    use super::*;

    #[test]
    fn it_detects_a_legacy_account_repository_before_opening_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::spot::SpotStore::at(temp.path().join("state"));
        let subject: dialog_varsig::Did =
            "did:key:z6MkhFDyBYNT1Y1jNj8RJKVc7CWurCVPmrnGEGmbYxvwHJkX"
                .parse()
                .unwrap();
        let revision = store.account_dir().join(tonk_account::revision_path(
            subject.repo_key(),
            tonk_account::MAIN_BRANCH,
        ));
        std::fs::create_dir_all(revision.parent().unwrap()).unwrap();
        std::fs::write(
            &revision,
            include_bytes!("../../tonk-account/tests/fixtures/revision-legacy.cbor"),
        )
        .unwrap();

        assert_eq!(
            account_repository_readability(&store, &subject),
            tonk_account::Readability::Legacy
        );
    }

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

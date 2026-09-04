//! Native account-system repository lifecycle.
//!
//! Account bytes live under the space store's dedicated `account/` directory,
//! never in a named space or in `spaces.json`. The trusted marker remains a
//! profile credential so account status can be read without opening any space.

use std::path::Path;

use anyhow::{Context, Result, bail};
use dialog_capability::Subject;
use dialog_effects::credential::CredentialError;
use dialog_effects::storage::Directory;
use dialog_operator::{DeriveOperator, Operator, Profile};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{RemoteAddress, RemoteRepository, Repository, SiteAddress, Upstream};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan_core::DelegationChain;
use tonk_account::{
    AccountStateStatus, CreateGenesis, RemotePresence, probe_remote_main, publish_genesis_if_absent,
};
use tonk_schema::{Replica, prelude::DidExt as _};

/// Stable derivation context for the account-system operator.
///
/// This must never be replaced with the historical space context (`slide`):
/// changing either context re-derives an operator DID and invalidates existing
/// authority chains.
const ACCOUNT_OPERATOR_CONTEXT: &[u8] = b"tonk/account-state/v1";
/// Remote name for the account's access branch in the profile repository.
const ACCOUNT_ACCESS_REMOTE: &str = "account-access";

/// Result of an ensure attempt. Remote failures preserve the durable local
/// lifecycle status and return diagnostics rather than changing account-link
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureOutcome {
    /// Durable lifecycle status after the attempt.
    pub status: AccountStateStatus,
    /// Ordered remote/local diagnostics from the latest ensure attempt.
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
    subject: &dialog_varsig::Did,
) -> Result<()> {
    profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .save(subject.as_str().as_bytes().to_vec())
        .perform(operator)
        .await
        .context("failed to save account trusted-base marker")
}

/// Whether the trusted base already recorded names this account.
fn marker_matches(marker: Option<&[u8]>, subject: &dialog_varsig::Did) -> bool {
    marker == Some(subject.as_str().as_bytes())
}

/// The account this profile is linked to, absent when unlinked.
async fn linked_account(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<dialog_varsig::Did>> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    linked_account_in(profile, operator, &store).await
}

/// [`linked_account`] against a caller-supplied store.
async fn linked_account_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<Option<dialog_varsig::Did>> {
    if crate::account::stored_provider_in(profile, operator, store)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let Some(root) = crate::identity::local_root_with_operator(profile, operator).await? else {
        return Ok(None);
    };
    Ok(Some(root.root_did.parse()?))
}

/// Where the linked account syncs, when the link named it.
async fn account_remote_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<Option<String>> {
    Ok(crate::account::stored_provider_in(profile, operator, store)
        .await?
        .map(|provider| provider.address().to_owned()))
}

/// Read durable native account-state status without contacting the remote.
pub async fn status(profile: &Profile) -> Result<AccountStateStatus> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    status_in(profile, &store).await
}

/// Read account repository status from one explicit profile store.
pub async fn status_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<AccountStateStatus> {
    let operator = credential_operator_for_store(profile, store).await?;
    status_with_operator_in(profile, &operator, store).await
}

/// Read durable account state through an already-mounted local operator.
pub(crate) async fn status_with_operator_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<AccountStateStatus> {
    let Some(subject) = linked_account_in(profile, operator, store).await? else {
        return Ok(AccountStateStatus::Unconfigured);
    };
    if marker_matches(marker(profile, operator).await?.as_deref(), &subject) {
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
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    adopt_account_access_in(profile, operator, &store).await
}

/// [`adopt_account_access`] against a caller-supplied store.
pub async fn adopt_account_access_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<bool> {
    let Some(subject) = linked_account_in(profile, operator, store).await? else {
        return Ok(false);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &subject) {
        return Ok(false);
    }
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
    let Some(remote) = account_remote_in(profile, operator, store).await? else {
        return Ok(false);
    };
    let address = SiteAddress::from(UcanAddress::new(remote.as_str()));
    let remote = match repository
        .remote(ACCOUNT_ACCESS_REMOTE)
        .load()
        .perform(operator)
        .await
    {
        Ok(remote) if remote.address().site() == &address && remote.did() == subject => remote,
        // Stale cell from an earlier link; see `repoint_remote`.
        Ok(_) => {
            repoint_remote(
                &repository,
                ACCOUNT_ACCESS_REMOTE,
                &address,
                &subject,
                operator,
            )
            .await?
        }
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
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    open_account_branch_in(profile, operator, &store).await
}

/// [`open_account_branch`] against a caller-supplied store.
pub async fn open_account_branch_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<Option<dialog_repository::Branch>> {
    trace("open: start");
    let Some(subject) = linked_account_in(profile, operator, store).await? else {
        return Ok(None);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &subject) {
        return Ok(None);
    }
    trace("open: marker matches, mounting");
    let remote = account_remote_in(profile, operator, store)
        .await?
        .context("the linked account names no remote")?;
    let repository = mount(profile, operator, &subject, &remote).await?;
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account main branch")?;
    trace("open: done");
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
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    retain_space_delegation_in(profile, operator, &store, chain).await
}

/// Retain one space delegation in the account repository owned by `store`.
pub async fn retain_space_delegation_in(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    chain: &DelegationChain,
) -> Result<bool> {
    let Some(branch) = open_account_branch_in(profile, operator, store).await? else {
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
    /// Spaces whose authority was retained into the account space.
    pub spaces: usize,
    /// Spaces already retained, so nothing was written for them.
    pub already: usize,
    /// The account repository was legacy and could not receive retained spaces.
    pub account_legacy: bool,
}

fn account_repository_readability(
    store: &crate::space::SpaceStore,
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
/// 2. **Each space's authority into the account space.** The account
///    repository is the durable home of delegations: retaining a space's
///    `space → account-root` prefix there is what lets the next device regain
///    access by pulling, instead of fetching a backup artifact.
///
/// Spaces that fail individually are counted and skipped rather than aborting
/// the run, so one unreadable space cannot block migrating the rest.
///
/// This form resolves the operator and space registry from the install; the
/// [`migrate_delegations`] form takes them, for callers that already hold one.
pub async fn migrate_delegations_here() -> Result<MigrationOutcome> {
    let storage = Storage::<NativeSpace>::default();
    let profile = Profile::load(crate::site::PROFILE_NAME)
        .at(Directory::Profile)
        .perform(&storage)
        .await
        .context("failed to mount the profile for delegation migration")?;
    let operator = credential_operator(&profile).await?;
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    migrate_delegations(&profile, &operator, &storage, &store).await
}

/// [`migrate_delegations_here`] against a caller-supplied profile, operator,
/// storage, and space store.
///
/// `profile` must be mounted in `storage`: the certificate migration commits
/// as the profile, so a storage without it errors rather than silently
/// migrating nothing.
pub async fn migrate_delegations(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    storage: &Storage<NativeSpace>,
    store: &crate::space::SpaceStore,
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
    let Some(subject) = linked_account(profile, operator).await? else {
        return Ok(outcome);
    };
    if !marker_matches(marker(profile, operator).await?.as_deref(), &subject) {
        return Ok(outcome);
    }
    let account_root = match crate::identity::local_root_with_operator(profile, operator).await? {
        Some(root) => root
            .root_did
            .parse::<dialog_varsig::Did>()
            .context("stored root DID is invalid")?,
        None => return Ok(outcome),
    };
    if account_repository_readability(store, &subject) == tonk_account::Readability::Legacy {
        outcome.account_legacy = true;
        return Ok(outcome);
    }
    let remote = account_remote_in(profile, operator, store)
        .await?
        .context("the linked account names no remote")?;
    let repository = mount(profile, operator, &subject, &remote).await?;
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open account main branch")?;

    for entry in store.load()?.spaces.values() {
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
            Ok(true) => outcome.spaces += 1,
            Ok(false) => outcome.already += 1,
            Err(_) => continue,
        }
    }

    Ok(outcome)
}

/// The account-state operator, remounting the profile by explicit name
/// and directory. Buildable before any account attachment — which the
/// onboarding path needs, since an unlinked device writes its custody
/// rows into the account branch that does not have an account yet.
pub(crate) async fn store_operator_with_config(
    profile: &Profile,
    store: &crate::space::SpaceStore,
    profile_name: &str,
    profile_directory: Directory,
) -> Result<Operator<NativeSpace>> {
    operator_with_profile(
        profile,
        &store.account_dir(),
        profile_name,
        profile_directory,
    )
    .await
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
pub async fn credential_operator_for_store(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<Operator<NativeSpace>> {
    let root = store.account_dir();
    let default_store =
        crate::space::SpaceStore::open().context("failed to locate account state")?;
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
    // An attached integration profile knows its own name and directory,
    // so it can be remounted like the persisted install profile —
    // deriving from an unmounted profile handle fails with "no provider
    // found for subject".
    #[cfg(feature = "integration-tests")]
    if let Some(config) = crate::account::integration_site_config(profile) {
        return operator_with_profile(
            profile,
            &root,
            &config.profile_name,
            config.profile_directory,
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
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    operator_with_profile(
        profile,
        &store.account_dir(),
        crate::site::PROFILE_NAME,
        Directory::Profile,
    )
    .await
}

/// Build the account operator for one explicit native profile store.
pub async fn operator_for_store(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<Operator<NativeSpace>> {
    credential_operator_for_store(profile, store).await
}

/// Republish a stored remote's address cell so it matches the current
/// descriptor, returning the repointed remote.
///
/// A remote's address is a memory cell, not an immutable record: when
/// the link's provider address changes (local dev services bind a fresh
/// port every restart) or the profile links to a different account, the
/// stored cell goes stale and every strict-equality check after it
/// would refuse to mount forever.
async fn repoint_remote(
    repository: &Repository<dialog_credentials::SignerCredential>,
    name: &str,
    address: &SiteAddress,
    subject: &dialog_varsig::Did,
    operator: &Operator<NativeSpace>,
) -> Result<RemoteRepository> {
    let reference = repository.remote(name);
    let target = RemoteAddress::new(address.clone(), subject.clone());
    let cell = reference.address();
    // Resolve first: publish is a compare-and-swap against the cell's
    // current version, and a fresh handle has not seen one yet.
    cell.resolve()
        .perform(operator)
        .await
        .with_context(|| format!("failed to resolve the '{name}' remote"))?;
    cell.publish(target.clone())
        .perform(operator)
        .await
        .with_context(|| format!("failed to repoint the '{name}' remote"))?;
    Ok(RemoteRepository::new(cell.retain(target), reference))
}

/// Point profile main at the account: the account is the upstream
/// remote of the profile repository's main branch, exactly as the
/// worker configures it — no separate repository, no extra storage.
async fn mount(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &dialog_varsig::Did,
    remote: &str,
) -> Result<Repository<dialog_credentials::SignerCredential>> {
    let subject = subject.clone();
    let repository = Repository::from(profile);

    trace("mount: loading remote record");
    let address = SiteAddress::from(UcanAddress::new(remote));
    let remote = match repository
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(operator)
        .await
    {
        Ok(remote) if remote.address().site() == &address && remote.did() == subject => remote,
        // A stored remote that disagrees with the descriptor follows an
        // older link — a previous provider address (dev services change
        // ports every restart) or an account this profile has since
        // left. The descriptor is the current link, so repoint the
        // address cell to it rather than refusing to mount forever.
        Ok(_) => {
            repoint_remote(
                &repository,
                tonk_account::ORIGIN_REMOTE,
                &address,
                &subject,
                operator,
            )
            .await?
        }
        Err(_) => repository
            .remote(tonk_account::ORIGIN_REMOTE)
            .create(address.clone())
            .subject(subject.clone())
            .perform(operator)
            .await
            .context("failed to configure account remote")?,
    };

    trace("mount: remote record loaded, opening profile main");
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open profile main branch")?;
    trace("mount: profile main open");
    match branch.upstream() {
        Some(Upstream::Remote { remote, branch, .. })
            if remote == tonk_account::ORIGIN_REMOTE && branch == tonk_account::MAIN_BRANCH => {}
        // A pointer left by an earlier account scheme (or an older link)
        // is repointed, like the remote cell above: with a linked
        // account, the account IS profile main's upstream by
        // definition, and set_upstream promotes over an existing
        // tracking target.
        //
        // The remote branch is opened only on this arm: opening it
        // resolves the remote head, and a mount on the steady path —
        // every local read runs one — must not wait on the network for
        // a value only repointing consumes.
        _ => {
            let remote_branch = remote
                .branch(tonk_account::MAIN_BRANCH)
                .open()
                .perform(operator)
                .await
                .context("failed to open account remote main")?;
            branch
                .set_upstream(&remote_branch)
                .perform(operator)
                .await
                .context("failed to set account upstream")?
        }
    }

    trace("mount: upstream settled, stamping replica");
    let replica = Replica::account(profile.did(), subject);
    branch
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(tonk_account::MAIN_BRANCH))
        .commit()
        .perform(operator)
        .await
        .context("failed to stamp account replica kind")?;
    trace("mount: done");

    Ok(repository)
}

async fn hydrate(
    repository: &Repository<dialog_credentials::SignerCredential>,
    branch: &dialog_repository::Branch,
    operator: &crate::account_authority::AccountBoundOperator,
) -> Result<()> {
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
            // Download, not a bare pull: `main` is the access branch the
            // operator proves from, and a head that runs ahead of the local
            // archive would have its own hydration authorized by a walk
            // over nodes that are not here yet.
            branch.pull().download().perform(operator).await?;
        }
        Ok(RemotePresence::Absent) => {
            branch.transaction().commit().perform(operator).await?;
            if let CreateGenesis::Loser(_) =
                publish_genesis_if_absent(branch, &remote, operator).await?
            {
                // Adopt the winner by pulling: the pull integrates the
                // established head AND records it as this branch's sync
                // base, so the next push fast-forwards instead of CASing
                // against an empty upstream. Reads resolve missing blocks
                // through the configured remote. See `account_remote`'s
                // losing-adoption test.
                branch.pull().download().perform(operator).await?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Retain both directions of the durable account/profile authority union.
///
/// The browser grant is stable and can be retained on every ensure. The
/// return edge is not: Dialog gives every newly minted delegation a random
/// nonce, so prove the semantic capability before minting to keep retries
/// idempotent.
async fn converge_account_union(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &dialog_varsig::Did,
    branch: &dialog_repository::Branch,
) -> Result<bool> {
    let local_root = crate::identity::local_root_with_operator(profile, operator)
        .await?
        .context("account link has no local root")?;
    let root_did: dialog_varsig::Did = local_root
        .root_did
        .parse()
        .context("stored account root DID is invalid")?;
    if &root_did != subject {
        bail!(
            "stored account root {} does not match repository {}",
            root_did,
            subject
        );
    }
    let grant_bytes = hex::decode(&local_root.delegation_hex)
        .context("stored account-to-profile grant is not hex")?;
    let inbound = crate::account::validate_account_grant(profile, &grant_bytes)
        .await
        .context("stored account-to-profile grant is invalid")?;
    if inbound.issuer() != subject
        || inbound.proof_cids()[0].to_string() != local_root.delegation_cid
    {
        bail!("stored account-to-profile grant does not match the local root");
    }

    trace("ensure: retaining account-to-profile edge");
    let mut changed =
        tonk_account::delegations::retain_space_delegation(branch, &inbound, operator)
            .await
            .context("failed to retain account-to-profile grant")?;
    trace("ensure: retained account-to-profile edge");

    let return_scope = dialog_ucan::Scope {
        subject: dialog_ucan_core::subject::Subject::Specific(profile.did()),
        command: dialog_ucan_core::command::Command::parse("/")
            .expect("the root command always parses"),
        parameters: dialog_ucan::Parameters::default(),
    };
    if branch
        .delegations()
        .prove(root_did.clone(), return_scope)
        .perform(operator)
        .await
        .is_err()
    {
        let signer = profile.signer().signer().clone();
        let returning = tonk_account::delegations::mint_account_union(&signer, &root_did)
            .await
            .context("failed to mint profile-to-account return edge")?;
        trace("ensure: retaining profile-to-account edge");
        changed |= tonk_account::delegations::retain_space_delegation(branch, &returning, operator)
            .await
            .context("failed to retain profile-to-account return edge")?;
        trace("ensure: retained profile-to-account edge");
    } else {
        trace("ensure: profile-to-account edge already proves");
    }
    Ok(changed)
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
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    ensure_with_operator_and_store(profile, operator, store).await
}

/// Human-readable progress on stderr for `tonk account sync --verbose`:
/// what the sync is doing right now, naming the endpoint it talks to, so
/// a slow or wedged sync says where it is instead of sitting silent.
static PROGRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turn on step-by-step sync progress on stderr.
pub fn enable_progress() {
    PROGRESS.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn progress(step: std::fmt::Arguments<'_>) {
    let line = step.to_string();
    if PROGRESS.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("{line}");
    }
    *LAST_STEP
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(line);
}

/// The most recent sync step, recorded whether or not progress printing
/// is on, so a timeout can say where the sync actually hung instead of
/// only that it did.
static LAST_STEP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The most recent sync step, for a deadline that gave up on it.
pub fn last_progress() -> Option<String> {
    LAST_STEP
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Breadcrumb for `TONK_TRACE=1`: one stderr line per step of the sync
/// and mount paths, so a command that stalls names the await it stalled
/// in rather than the timeout that eventually gave up on it.
pub(crate) fn trace(step: &str) {
    if std::env::var_os("TONK_TRACE").is_some_and(|value| !value.is_empty() && value != "0") {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        eprintln!(
            "[tonk-trace {}.{:03}] {step}",
            now.as_secs() % 1000,
            now.subsec_millis()
        );
    }
}

/// [`ensure_with_operator`] against a caller-supplied space store.
///
/// The store locates account state on disk. A caller running outside an
/// install — a test, or an embedder with its own layout — supplies one rather
/// than having the real install directory resolved behind its back.
pub async fn ensure_with_operator_and_store(
    profile: &Profile,
    operator: Operator<NativeSpace>,
    store: crate::space::SpaceStore,
) -> Result<EnsureOutcome> {
    trace("ensure: start");
    progress(format_args!("Reading the linked account…"));
    let Some(subject) = linked_account_in(profile, &operator, &store).await? else {
        progress(format_args!("No account is linked; nothing to sync."));
        return Ok(EnsureOutcome {
            status: AccountStateStatus::Unconfigured,
            warning: None,
        });
    };
    trace("ensure: descriptor read");
    let remote = account_remote_in(profile, &operator, &store)
        .await?
        .context("the linked account names no remote")?;
    progress(format_args!("Account {subject} syncs with {remote}"));
    progress(format_args!("Connecting to the remote…"));
    let repository = mount(profile, &operator, &subject, &remote).await?;
    trace("ensure: mounted");
    let operator =
        crate::account_authority::wrap(operator, profile.clone(), store.clone(), true).await?;
    trace("ensure: operator wrapped");
    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&operator)
        .await
        .context("failed to open account main branch")?;
    let already_ready = marker_matches(
        marker(profile, operator.local()).await?.as_deref(),
        &subject,
    );
    let mut warnings = Vec::new();
    let (status, may_push) = if already_ready {
        trace("ensure: marker matches, pulling");
        progress(format_args!("Pulling what other devices changed…"));
        // Ready remains ready offline. A best-effort normal sync catches up
        // without clearing the durable trust marker on failure.
        let may_push = match branch.pull().download().perform(&operator).await {
            Ok(_) => {
                trace("ensure: pulled");
                progress(format_args!("Pulled."));
                true
            }
            Err(error) => {
                warnings.push(format!("account pull failed: {error}"));
                false
            }
        };
        (AccountStateStatus::Ready, may_push)
    } else {
        trace("ensure: first hydration starting");
        progress(format_args!(
            "First sync on this device: downloading the account…"
        ));
        match hydrate(&repository, &branch, &operator).await {
            Ok(()) => {
                trace("ensure: first hydration finished");
                save_marker(profile, operator.local(), &subject).await?;
                trace("ensure: trusted-base marker saved");
                (AccountStateStatus::Ready, true)
            }
            Err(error) => {
                warnings.push(format!("account hydration failed: {error}"));
                trace("ensure: first hydration failed");
                (AccountStateStatus::Unhydrated, false)
            }
        }
    };

    // Adopting the account as the access branch's upstream is what makes
    // recovered authority usable: the operator proves from the access
    // branch, not the account's. It remains best-effort so durable Ready
    // state survives an offline retry.
    trace("ensure: account-access adoption starting");
    progress(format_args!("Adopting the account's access authority…"));
    if let Err(error) = adopt_account_access_in(profile, operator.local(), &store).await {
        warnings.push(format!("account access adoption failed: {error}"));
    }
    trace("ensure: account-access adoption finished");

    if let Err(error) = converge_account_union(profile, operator.local(), &subject, &branch).await {
        warnings.push(format!("account authority convergence failed: {error:#}"));
    }

    if may_push {
        trace("ensure: final push starting");
        progress(format_args!("Pushing local changes…"));
        if let Err(error) = branch.push().perform(&operator).await {
            warnings.push(format!("account push failed: {error}"));
        }
        trace("ensure: final push finished");
    } else {
        trace("ensure: final push skipped after remote failure");
        progress(format_args!(
            "Skipping the push: the remote did not answer cleanly."
        ));
    }

    Ok(EnsureOutcome {
        status,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    })
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use tonk_schema::prelude::DidExt as _;

    use super::*;

    #[test]
    fn it_detects_a_legacy_account_repository_before_opening_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::space::SpaceStore::at(temp.path().join("state"));
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
        use dialog_varsig::Principal as _;

        let root = Ed25519Signer::import(&[7; 32]).await.unwrap();
        let subject = root.did();
        let other = Ed25519Signer::import(&[8; 32]).await.unwrap().did();
        assert!(marker_matches(Some(subject.as_str().as_bytes()), &subject));
        assert!(!marker_matches(Some(other.as_str().as_bytes()), &subject));
        assert!(!marker_matches(None, &subject));
    }

    /// Relinking must survive the provider address changing (local dev
    /// services bind a fresh port every restart): the stored `origin`
    /// remote cell is repointed to the current descriptor instead of
    /// refusing forever with "profile main already follows a different
    /// account remote" — the failure that left `tonk account space`
    /// reporting no account while `tonk account status` said signed in.
    #[dialog_common::test]
    async fn it_repoints_the_account_remote_when_the_link_moves() {
        use dialog_common::helpers::Provisionable as _;
        use dialog_operator::Profile;
        use dialog_ucan::UcanDelegation;
        use dialog_varsig::Principal as _;
        use tonk_access_service::helpers::AccessServiceAddress;

        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::space::SpaceStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let profile_name = format!("cli-account-repoint-{}", rand::random::<u64>());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(&profile_name)
            .at(profile_dir.clone())
            .perform(&storage)
            .await
            .unwrap();
        let root = Ed25519Signer::generate().await.unwrap();
        // The account space is servable only once its customer has
        // confirmed the emailed activation link.
        service
            .address
            .activate_customer(&root, "account-state@example.com")
            .await
            .unwrap();
        let live_remote = format!(
            "{}/",
            service.address.access_service_url.trim_end_matches('/')
        );
        let root_did = root.did();
        let delegation =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &profile.did())
                .await
                .unwrap();
        async fn account_operator(
            profile: &Profile,
            store: &crate::space::SpaceStore,
            profile_name: &str,
            profile_dir: &Directory,
        ) -> Operator<NativeSpace> {
            operator_with_profile(
                profile,
                &store.account_dir(),
                profile_name,
                profile_dir.clone(),
            )
            .await
            .unwrap()
        }
        async fn attach(
            profile: &Profile,
            store: &crate::space::SpaceStore,
            profile_name: &str,
            profile_dir: &Directory,
            remote: &str,
            at: u64,
        ) {
            let attachment = tonk_account::AccountProviderRecord::attach(remote, at).unwrap();
            let operator = account_operator(profile, store, profile_name, profile_dir).await;
            profile
                .credential()
                .site(crate::account::ACCOUNT_LINK_SITE)
                .save(attachment.encode().unwrap())
                .perform(&operator)
                .await
                .unwrap();
        }
        profile
            .save(UcanDelegation(delegation.clone()))
            .perform(&account_operator(&profile, &store, &profile_name, &profile_dir).await)
            .await
            .unwrap();
        let local_root = crate::identity::LocalRoot {
            credential_id: "cli-account-repoint-credential".to_string(),
            root_did: root_did.to_string(),
            delegation_cid: delegation.proof_cids()[0].to_string(),
            delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        };
        profile
            .credential()
            .site(crate::identity::LOCAL_ROOT_SITE)
            .save(serde_json::to_vec(&local_root).unwrap())
            .perform(&account_operator(&profile, &store, &profile_name, &profile_dir).await)
            .await
            .unwrap();
        attach(
            &profile,
            &store,
            &profile_name,
            &profile_dir,
            &live_remote,
            1,
        )
        .await;
        let outcome = ensure_with_operator(
            &profile,
            account_operator(&profile, &store, &profile_name, &profile_dir).await,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.status,
            AccountStateStatus::Ready,
            "ensure warning: {:?}",
            outcome.warning
        );

        // The link moves to an unreachable address. The mount must
        // follow it — the old strict-equality check errored here,
        // permanently.
        attach(
            &profile,
            &store,
            &profile_name,
            &profile_dir,
            "http://127.0.0.1:9/ucan/",
            2,
        )
        .await;
        let outcome = ensure_with_operator(
            &profile,
            account_operator(&profile, &store, &profile_name, &profile_dir).await,
        )
        .await
        .expect("a moved link must repoint, not refuse to mount");
        // Still Ready: the trusted base names the account, and the
        // account did not change — only where it syncs. What this test
        // guards is that the remote repoints at all, which the assert
        // below confirms by moving back to a live one and pulling.
        assert_eq!(
            outcome.status,
            AccountStateStatus::Ready,
            "ensure warning: {:?}",
            outcome.warning
        );

        // Moving back to the live address still mounts and pulls.
        attach(
            &profile,
            &store,
            &profile_name,
            &profile_dir,
            &live_remote,
            3,
        )
        .await;
        let outcome = ensure_with_operator(
            &profile,
            account_operator(&profile, &store, &profile_name, &profile_dir).await,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.status,
            AccountStateStatus::Ready,
            "ensure warning: {:?}",
            outcome.warning
        );
        service.stop().await.unwrap();
    }

    #[dialog_common::test]
    async fn it_ensures_account_state_outside_the_space_registry() {
        use dialog_common::helpers::Provisionable as _;
        use dialog_operator::Profile;
        use dialog_ucan::UcanDelegation;
        use tonk_access_service::helpers::AccessServiceAddress;

        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::space::SpaceStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let profile_name = format!("cli-account-test-{}", rand::random::<u64>());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(&profile_name)
            .at(profile_dir.clone())
            .perform(&storage)
            .await
            .unwrap();
        let root = Ed25519Signer::generate().await.unwrap();
        // The account space is servable only once its customer has
        // confirmed the emailed activation link.
        service
            .address
            .activate_customer(&root, "account-state@example.com")
            .await
            .unwrap();
        let remote = format!(
            "{}/",
            service.address.access_service_url.trim_end_matches('/')
        );
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
        // read it through the space's account operator — which found nothing
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
        let attachment = tonk_account::AccountProviderRecord::attach(&remote, 1).unwrap();
        profile
            .credential()
            .site(crate::account::ACCOUNT_LINK_SITE)
            .save(attachment.encode().unwrap())
            .perform(&account_operator)
            .await
            .unwrap();
        assert_eq!(
            super::linked_account(&profile, &account_operator)
                .await
                .unwrap()
                .expect("a linked profile reads back the account it stored"),
            root_did,
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

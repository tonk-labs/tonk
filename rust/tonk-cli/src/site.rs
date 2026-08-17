//! Site open and init.
//!
//! A *site* is a directory that contains a single dialog
//! repository named `main`. Tonk only reads and writes that one
//! repository on the `main` branch — multi-branch / multi-repo
//! UX is intentionally not exposed. A site's directory is never
//! located by walking the current directory: the caller resolves
//! it through the spot registry (see [`crate::spot`]) and passes
//! the path in directly.
//!
//! [`TonkSite`] is the assembled context every command works
//! against: profile, operator (rooted at the site directory), the
//! repository, and the opened `main` branch.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_effects::storage::Directory;
use dialog_operator::{DeriveOperator, Operator, Profile};
use dialog_reactor::{BranchSession, Reactor, ReactorError};
use dialog_repository::{Repository, RepositoryExt as _};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime};
use dialog_varsig::{Did, Principal};
use tonk_account::backup::{AccountSpotBackup, SPACE_ROOT_SITE_PREFIX, space_root_site};

/// The standard-library notation document seeded into a freshly
/// created repository: the built-in concepts, views, commands, and
/// rules. Embedded at compile time (`include_str!`, not a runtime
/// file read) so it travels with the binary and with test archives.
/// This is the same `core.yaml` the worker fetches and lowers at
/// repository creation.
const STANDARD_LIBRARY: &str = include_str!("../../tonk-core/assets/library/core.yaml");

/// Whether the seeded standard library publishes `name`.
///
/// Live schema enumeration also sees runtime/system concepts. Agent-facing
/// workflow surfaces use this to keep the application vocabulary short.
pub(crate) fn standard_library_has_name(name: &str) -> bool {
    STANDARD_LIBRARY.lines().any(|line| {
        let Some((_, anchor)) = line.split_once("!: &") else {
            return false;
        };
        anchor
            .split_ascii_whitespace()
            .next()
            .map(|anchor| anchor.trim_end_matches(':'))
            == Some(name)
    })
}

/// Name of the dialog repository tonk uses inside `.tonk/`.
pub const REPO_NAME: &str = "main";

/// The single branch tonk reads and writes against.
pub const BRANCH_NAME: &str = "main";

/// Profile name used for the local identity.
pub const PROFILE_NAME: &str = "tonk";

/// Operator derivation context — distinguishes the tonk-derived
/// operator key from operator keys derived by other tools sharing
/// the same profile (e.g. the worker). The literal stays `"slide"`
/// for backward compatibility: it salts the operator key, so
/// changing it would re-derive a new DID and lock every existing
/// repo out of its delegations.
const OPERATOR_CONTEXT: &[u8] = b"slide";

/// Name of the `.tonk/` sub-directory holding repository data.
pub const SITE_DIRNAME: &str = ".tonk";

/// Whether `root` already holds site data.
///
/// The dialog repository always lands in `<root>/<REPO_NAME>/`, so
/// that directory is the marker regardless of how the site was
/// created. Callers use it to tell "creating a site here" from
/// "adopting the site already here" — a distinction
/// [`TonkSite::init_at_with`] deliberately erases by being
/// idempotent.
///
/// A bare `root` that exists but holds no repository reads as no
/// data: an empty directory is not a site.
pub fn has_site_data(root: &Path) -> bool {
    root.join(REPO_NAME).is_dir()
}

/// An opened tonk site: profile, operator, repository, and a
/// reactor, all wired up against the on-disk `.tonk/` directory.
///
/// Branch access goes through the [`Reactor`] rather than a raw
/// `Branch` handle, so tonk shares the worker's reactive layer:
/// the first [`Self::branch`] acquire opens and caches the branch
/// (with its warm content-addressed node cache), and later
/// acquires reuse it. Tonk opens no subscriptions, so the
/// reactor's broadcast machinery is dormant; it is used purely
/// for the cached handle and the uniform query/commit surface.
pub struct TonkSite {
    /// Absolute path to the `.tonk/` directory backing this site.
    pub root: PathBuf,
    /// The user's profile (shared identity).
    pub profile: Profile,
    /// Operator rooted at [`Self::root`].
    pub operator: crate::account_authority::AccountBoundOperator,
    /// The `main` repository handle. Verifier-typed (`Credential`):
    /// commits flow through the operator's authority chain, so the
    /// repo handle itself doesn't need to carry a signer.
    pub repository: Repository,
    /// Reactive layer over the repository's branches. Owns the
    /// cached branch handles tonk reads and writes through.
    pub reactor: Reactor,
}

impl TonkSite {
    /// Open an already-existing site at the given directory.
    /// Errors if the directory exists but the dialog repository
    /// inside it is missing or unreadable.
    pub async fn open(root: &Path) -> Result<Self> {
        Self::open_with(root, default_config()).await
    }

    /// [`Self::open`] with caller-supplied [`SiteConfig`] —
    /// lets tests redirect the profile directory and pick a
    /// unique profile name without touching the user's real
    /// data dir.
    pub async fn open_with(root: &Path, config: SiteConfig) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", root.display()))?;
        // Refuse before anything opens a branch — opening it is what fails on
        // old data, and it fails deep enough to be unreadable. A site with no
        // marker predates the marker, which dates it to before the upgrade.
        let stamped = std::fs::read(root.join(tonk_account::SITE_FORMAT_FILE)).ok();
        match tonk_account::read_site_format(stamped.as_deref()) {
            Some(format) if format.is_readable() => {}
            Some(format) => bail!(
                "this spot is written in format {} and this tonk reads {}.\n\n{}",
                format.format,
                tonk_account::SITE_FORMAT,
                tonk_account::LEGACY_FORMAT_REMEDY
            ),
            None => {
                // Unmarked: either pre-upgrade data, or a site this build has
                // simply not stamped yet. Only the first is a problem, and
                // the branch open below tells them apart — so this falls
                // through to the error sniffer rather than refusing outright.
            }
        }
        let (profile, operator) = build_profile_and_operator(&root, &config).await?;

        let repository = profile
            .repository(REPO_NAME)
            .load()
            .perform(&operator)
            .await
            .with_context(|| {
                format!(
                    "failed to load repository '{REPO_NAME}' at {}",
                    root.display()
                )
            })?;

        let reactor = Reactor::new(profile.clone());
        let operator = crate::account_authority::wrap(
            operator,
            profile.clone(),
            crate::spot::SpotStore::open().context("failed to locate account state")?,
            config.require_account,
        )
        .await?;

        Ok(Self {
            root,
            profile,
            operator,
            repository,
            reactor,
        })
    }

    /// [`Self::init`] with caller-supplied [`SiteConfig`].
    ///
    /// Historical parent-relative form: the site lands at
    /// `parent/.tonk/`. Kept for the test fixtures that model the
    /// pre-registry layout; new callers go through
    /// [`Self::init_at_with`].
    pub async fn init_with(parent: &Path, config: SiteConfig) -> Result<Self> {
        let parent = parent
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", parent.display()))?;
        Self::init_at_with(&parent.join(SITE_DIRNAME), config).await
    }

    /// Initialize (or adopt) a site whose directory is exactly
    /// `root` — no `.tonk/` nesting. This is what canonical spot
    /// storage uses: the registry maps a spot name to this
    /// directory. Idempotent: an existing repository at `root` is
    /// loaded, not clobbered, which is also how `tonk spot new
    /// --site <path>` adopts pre-existing storage.
    pub async fn init_at_with(root: &Path, config: SiteConfig) -> Result<Self> {
        std::fs::create_dir_all(root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        // Record the format beside the data, so the next incompatible change
        // is detected by reading a number rather than by matching the text of
        // a decode failure.
        std::fs::write(
            root.join(tonk_account::SITE_FORMAT_FILE),
            serde_json::to_vec_pretty(&tonk_account::SiteFormat::current())?,
        )
        .with_context(|| format!("failed to stamp the site format at {}", root.display()))?;
        let root = root
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", root.display()))?;

        let (profile, operator) = build_profile_and_operator(&root, &config).await?;

        // Try to load first. If absent, create + persist a
        // repo→profile delegation chain so the profile can later
        // mint further delegations (invites, etc.) over this
        // repo. Without that root chain `profile.access().claim`
        // fails with "no delegation chain found" the moment we
        // try to mint an invite.
        let (repository, fresh) = match profile
            .repository(REPO_NAME)
            .load()
            .perform(&operator)
            .await
        {
            Ok(repository) => (repository, false),
            Err(_) => (
                bootstrap_repository(&profile, &operator, config.require_account)
                    .await
                    .with_context(|| format!("failed to bootstrap repository '{REPO_NAME}'"))?,
                true,
            ),
        };

        let reactor = Reactor::new(profile.clone());
        let operator = crate::account_authority::wrap(
            operator,
            profile.clone(),
            crate::spot::SpotStore::open().context("failed to locate account state")?,
            config.require_account,
        )
        .await?;

        let site = Self {
            root,
            profile,
            operator,
            repository,
            reactor,
        };

        // Seed the standard library into a freshly-created repo, the
        // same way the worker does at repository creation (it fetches
        // and lowers `/library/core.yaml`). Without this, a tonk-only
        // repo lacks the built-in concepts (`tonk:view`, etc.) that
        // `tonk render` / `<tonk-display>` resolve against.
        if fresh {
            site.seed_standard_library().await?;
        }

        Ok(site)
    }

    /// Lower the standard library (`tonk-core/assets/library/core.yaml`)
    /// into the main branch via the evaluate pipeline, committing the
    /// built-in concepts/views/rules in one pass.
    async fn seed_standard_library(&self) -> Result<()> {
        crate::eval::run_against_site(
            self,
            crate::eval::Source::Inline(STANDARD_LIBRARY.to_string()),
            crate::eval::Options::default(),
        )
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("failed to seed standard library: {e}"))
    }

    /// Acquire the `main` branch through the reactor, returning a
    /// [`BranchSession`] whose `handle()` is the cached dialog
    /// `Branch`. The first call opens the branch; later calls reuse
    /// the cached handle and its warm node cache.
    ///
    /// Hold the returned session for as long as you use its
    /// `handle()`: the handle borrows from the session.
    /// Acquire the site's branch, naming the remedy when the data predates
    /// this build's format.
    ///
    /// Every command reaches its data through here, so this is where an
    /// unreadable old spot becomes a sentence someone can act on rather than
    /// `missing field \`branch\`` from inside block decoding.
    pub async fn branch(&self) -> Result<BranchSession, ReactorError> {
        self.reactor
            .repository(REPO_NAME)
            .branch(BRANCH_NAME)
            .acquire(&self.operator)
            .await
            .map_err(|error| {
                let text = error.to_string();
                if tonk_account::is_legacy_format(&text) {
                    // `reason` carries the remedy, so it travels with the
                    // existing variant rather than needing a new one here.
                    return ReactorError::BranchNotFound {
                        repo: REPO_NAME.to_owned(),
                        branch: BRANCH_NAME.to_owned(),
                        reason: tonk_account::LEGACY_FORMAT_REMEDY.to_owned(),
                    };
                }
                error
            })
    }
}

/// Create the repository, mint a self → profile delegation, and
/// persist it so the profile holds the root of a chain it can
/// extend. This mirrors the worker's `create_repository`
/// startup; without it, tonk can write to the repo through the
/// operator's blanket `Subject::any()` delegation but can't
/// delegate to anyone *else* (which is what `tonk invite`
/// needs).
///
/// Returns the verifier-only `Repository<Credential>` form
/// because the rest of tonk treats every repo handle uniformly,
/// regardless of how it was bootstrapped.
async fn bootstrap_repository(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    require_account: bool,
) -> Result<Repository> {
    let signer_repo = profile
        .repository(REPO_NAME)
        .create()
        .perform(operator)
        .await
        .context("failed to create repository")?;

    let local_root = crate::identity::local_root_with_operator(profile, operator).await?;
    if require_account {
        crate::account::require_account_with_operator(profile, operator).await?;
    }
    let durable_did: dialog_varsig::Did = local_root
        .context("local root provisioning did not produce a record")?
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
    let delegation = signer_repo
        .access()
        .claim(&signer_repo)
        .delegate(durable_did.clone())
        .perform(operator)
        .await
        .context("failed to mint repo→profile delegation")?;
    let prefix = delegation.into_chain();
    let prefix_bytes = prefix
        .to_bytes()
        .context("failed to serialize repo→root delegation")?;
    profile
        .credential()
        .site(space_root_site(&signer_repo.did(), &durable_did))
        .save(prefix_bytes)
        .perform(operator)
        .await
        .context("failed to persist repo→root delegation")?;

    profile
        .access()
        .save(UcanDelegation(prefix.clone()))
        .perform(operator)
        .await
        .context("failed to persist repo→profile delegation")?;

    // The same authority, retained into the account space. The access branch
    // above is what makes this space usable HERE; the account is what makes it
    // recoverable on the next device, since a device regains access by pulling
    // the account rather than by fetching an artifact. Non-fatal: a space that
    // works but is not yet backed up beats no space at all.
    if let Err(error) =
        crate::account_state::retain_space_delegation(profile, operator, &prefix).await
    {
        eprintln!("warning: space not retained into the account space: {error:#}");
    }

    profile
        .repository(REPO_NAME)
        .load()
        .perform(operator)
        .await
        .context("failed to reload repository after bootstrap")
}

/// Mount an exact delegated repository subject at a fresh site directory
/// without writing membership, role, name, invitation, or provenance facts.
pub async fn mount_delegated_at(
    root: &Path,
    chain: DelegationChain,
    config: SiteConfig,
) -> Result<TonkSite> {
    if root.exists() {
        anyhow::bail!("a site already exists at {}", root.display());
    }
    std::fs::create_dir_all(root)
        .with_context(|| format!("failed to create {}", root.display()))?;
    let root = root
        .canonicalize()
        .with_context(|| format!("could not canonicalize {}", root.display()))?;
    let (profile, operator) = build_profile_and_operator(&root, &config).await?;
    mount_delegated_inner(&root, profile, operator, chain, config, true).await
}

/// Mount with profile/operator state already prepared by the invite parser.
///
/// Existing invites can contain subject-open device authority. It remains
/// usable for this mount but is never persisted as a reusable account backup.
pub(crate) async fn mount_delegated_with(
    root: &Path,
    profile: Profile,
    operator: Operator<NativeSpace>,
    chain: DelegationChain,
    config: SiteConfig,
) -> Result<TonkSite> {
    mount_delegated_inner(root, profile, operator, chain, config, false).await
}

async fn mount_delegated_inner(
    root: &Path,
    profile: Profile,
    operator: Operator<NativeSpace>,
    chain: DelegationChain,
    config: SiteConfig,
    require_reusable: bool,
) -> Result<TonkSite> {
    let local_root = crate::identity::local_root_with_operator(&profile, &operator)
        .await?
        .context("local root provisioning did not produce a record")?;
    if config.require_account {
        crate::account::require_account_with_operator(&profile, &operator).await?;
    }
    let account_root: Did = local_root
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
    let subject = chain
        .subject()
        .cloned()
        .context("delegated authority has no repository subject")?;
    let chain_bytes = chain
        .to_bytes()
        .context("failed to serialize delegated authority")?;
    let reusable = AccountSpotBackup {
        chain_hex: hex::encode(&chain_bytes),
        remote_url: None,
        revocation_url: None,
        name: None,
    }
    .validate_for(&account_root)
    .await;
    if require_reusable && let Err(error) = &reusable {
        anyhow::bail!("delegated prefix is not reusable account-root authority: {error}");
    }

    profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&operator)
        .await
        .context("failed to persist delegated authority")?;
    if let Ok(validated) = reusable {
        let prefix_bytes = validated
            .chain
            .to_bytes()
            .context("failed to serialize delegated prefix")?;
        profile
            .credential()
            .site(space_root_site(&subject, &account_root))
            .save(prefix_bytes)
            .perform(&operator)
            .await
            .context("failed to persist delegated account-root prefix")?;
    }

    let verifier: Ed25519Verifier = subject
        .to_string()
        .parse()
        .map_err(|error| anyhow::anyhow!("delegated subject is not an Ed25519 DID: {error:?}"))?;
    Subject::from(profile.did())
        .attenuate(Space::new(REPO_NAME))
        .create(Credential::from(verifier))
        .perform(&operator)
        .await
        .context("failed to provision delegated repository")?;

    let repository = profile
        .repository(REPO_NAME)
        .load()
        .perform(&operator)
        .await
        .context("failed to load delegated repository")?;
    let reactor = Reactor::new(profile.clone());
    let operator = crate::account_authority::wrap(
        operator,
        profile.clone(),
        crate::spot::SpotStore::open().context("failed to locate account state")?,
        config.require_account,
    )
    .await?;
    Ok(TonkSite {
        root: root.to_path_buf(),
        profile,
        operator,
        repository,
        reactor,
    })
}

/// Load the exact reusable prefix for a site, recovering it from pre-feature
/// profile authority when the dedicated credential is absent.
pub async fn account_root_prefix(site: &TonkSite, account_root: &Did) -> Result<DelegationChain> {
    account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        &site.repository.did(),
        account_root,
    )
    .await
}

/// Decode a stored prefix artifact for `account_root`.
async fn validate_prefix(bytes: Vec<u8>, account_root: &Did) -> Result<DelegationChain> {
    Ok(AccountSpotBackup {
        chain_hex: hex::encode(bytes),
        remote_url: None,
        revocation_url: None,
        name: None,
    }
    .validate_for(account_root)
    .await?
    .chain)
}

/// Read one credential site, treating absence and emptiness alike.
async fn optional_credential(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    site: String,
) -> Result<Option<Vec<u8>>> {
    match profile
        .credential()
        .site(site)
        .load::<Vec<u8>>()
        .perform(operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => Ok(None),
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if crate::account_state::credential_is_missing(&error) => Ok(None),
        Err(error) => Err(error).context("failed to load a stored authority prefix"),
    }
}

/// Resolve the reusable `subject → … → account_root` prefix, minting it
/// when this profile holds the subject's authority but no chain to the
/// account reaches it yet.
///
/// Every path that authorizes a remote request for a spot needs this exact
/// prefix, so all of them recover the same way. A spot created before the
/// account existed — or by a release that stored no prefix at all — has
/// repository authority that stops at the profile, and refusing it would
/// strand data the device demonstrably owns. Extending that authority to
/// the account root is local bookkeeping: it delegates to the root this
/// profile already holds a grant from, and publishes nothing.
pub async fn account_root_prefix_for(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain> {
    let current = space_root_site(subject, account_root);
    if let Some(bytes) = optional_credential(profile, operator, current.clone()).await? {
        return validate_prefix(bytes, account_root)
            .await
            .context("stored account-root prefix is invalid");
    }

    // A v1 credential is copied only when its signatures and terminal root
    // validate for the explicitly requested account.
    let legacy = format!("{SPACE_ROOT_SITE_PREFIX}{subject}");
    if let Some(bytes) = optional_credential(profile, operator, legacy).await?
        && let Ok(chain) = validate_prefix(bytes.clone(), account_root).await
    {
        save_prefix(profile, operator, &current, bytes)
            .await
            .context("failed to migrate the account-root prefix")?;
        return Ok(chain);
    }

    let chain = match recover_prefix(profile, operator, subject, account_root).await? {
        Some(chain) => chain,
        None => mint_prefix(profile, operator, subject, account_root).await?,
    };
    let bytes = chain
        .to_bytes()
        .context("failed to serialize the account-root prefix")?;
    let validated = validate_prefix(bytes.clone(), account_root)
        .await
        .context("recovered account-root prefix is invalid")?;
    save_prefix(profile, operator, &current, bytes)
        .await
        .context("failed to persist the account-root prefix")?;
    Ok(validated)
}

async fn save_prefix(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    site: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    profile
        .credential()
        .site(site.to_string())
        .save(bytes)
        .perform(operator)
        .await?;
    Ok(())
}

/// Rebuild the prefix from certificates this profile already holds.
async fn recover_prefix(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<Option<DelegationChain>> {
    let proof = profile
        .access()
        .prove(Subject::from(subject.clone()))
        .perform(operator)
        .await
        .context("failed to recover repository authority")?;
    let mut delegations = Vec::new();
    for certificate in proof.proofs {
        let delegation = certificate.0;
        let reached_root = delegation.audience() == account_root;
        delegations.push(delegation);
        if reached_root {
            return DelegationChain::try_from(delegations)
                .context("recovered repository authority is not a valid chain")
                .map(Some);
        }
    }
    Ok(None)
}

/// Extend held authority over `subject` to the account root.
async fn mint_prefix(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain> {
    let delegation: UcanDelegation = profile
        .access()
        .claim(Subject::from(subject.clone()))
        .delegate(account_root.clone())
        .perform(operator)
        .await
        .context("this profile cannot delegate the spot to the account root")?;
    Ok(delegation.into_chain())
}

/// Knobs for [`TonkSite::open_with`] / [`TonkSite::init_with`].
/// The defaults (via [`default_config`]) reproduce what `tonk`
/// does on a real install: profile named `tonk` in the platform
/// profile directory.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// Profile name tonk opens (or creates).
    pub profile_name: String,
    /// Where the profile directory lives. `Directory::Profile`
    /// is the platform default; tests pick `Directory::At(...)`
    /// to redirect onto a temp dir.
    pub profile_directory: Directory,
    /// Whether minting durable authority must have an account behind it.
    /// Production enables this; legacy-isolation test fixtures disable it,
    /// and get a software-generated root instead (see
    /// [`build_profile_and_operator`]).
    pub require_account: bool,
}

impl SiteConfig {
    /// Builder shortcut: same as [`default_config`] but with the
    /// profile name overridden. Lets tests namespace their
    /// profile so parallel runs don't collide.
    pub fn with_profile_name(name: impl Into<String>) -> Self {
        Self {
            profile_name: name.into(),
            profile_directory: Directory::Profile,
            require_account: false,
        }
    }
}

/// The site config tonk uses out of the box — profile named
/// [`PROFILE_NAME`] in the platform profile directory. Exposed
/// so the binary and other crate modules can pass it to
/// `*_with` constructors without reaching into `dialog-effects`
/// for [`Directory::Profile`].
pub fn default_config() -> SiteConfig {
    SiteConfig {
        profile_name: PROFILE_NAME.to_string(),
        profile_directory: Directory::Profile,
        require_account: std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none(),
    }
}

/// Open (or create) the shared profile and build a tonk
/// operator rooted at the site directory for repository data.
/// Identity lives at `config.profile_directory`; only the
/// dialog-repo blocks live under the site directory.
///
/// Exposed as `pub(crate)` so the [`crate::invite`] module can
/// reuse the same wiring when claiming an invite into a fresh
/// site (the join path provisions the space from a verifier
/// credential rather than via `profile.repository(...).open()`,
/// but the profile + operator setup is the same).
async fn derive_operator_for_profile(
    root: &Path,
    profile: &Profile,
    storage: Storage<NativeSpace>,
) -> Result<Operator<NativeSpace>> {
    let root_str = root
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", root.display()))?
        .to_owned();
    let operator = profile
        .derive(OPERATOR_CONTEXT)
        .base(Directory::At(root_str))
        .build(storage)
        .await
        .context("failed to build operator")?;
    let expiration = Timestamp::new(
        SystemTime::now() + Duration::from_secs(tonk_identity::session::SESSION_TTL_SECONDS),
    )
    .context("native session expiration is out of range")?;
    let session = profile
        .access()
        .claim(Subject::any())
        .expires(expiration)
        .delegate(operator.did())
        .perform(&operator)
        .await
        .context("failed to mint the native signing session")?;
    profile
        .access()
        .save(session)
        .perform(&operator)
        .await
        .context("failed to save the native signing session")?;
    Ok(operator)
}

pub(crate) async fn build_profile_and_operator(
    root: &Path,
    config: &SiteConfig,
) -> Result<(Profile, Operator<NativeSpace>)> {
    let storage = Storage::<NativeSpace>::default();
    let profile = Profile::open(config.profile_name.clone())
        .at(config.profile_directory.clone())
        .perform(&storage)
        .await
        .with_context(|| format!("failed to open profile '{}'", config.profile_name))?;
    let operator = derive_operator_for_profile(root, &profile, storage).await?;

    // Isolated legacy test fixtures opt into a software-generated root so they
    // still exercise the same space → root → device chain shape. Production
    // never enters this path and requires a browser/passkey handoff.
    if !config.require_account
        && crate::identity::local_root_with_operator(&profile, &operator)
            .await?
            .is_none()
    {
        let root = Ed25519Signer::generate()
            .await
            .context("failed to generate the isolated fixture root")?;
        let delegation =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &profile.did()).await?;
        let bytes = delegation
            .to_bytes()
            .context("failed to serialize fixture root")?;
        let record = crate::identity::LocalRoot {
            credential_id: "isolated-software-root".to_string(),
            root_did: root.did().to_string(),
            delegation_cid: delegation.proof_cids()[0].to_string(),
            delegation_hex: hex::encode(&bytes),
        };
        profile
            .access()
            .save(UcanDelegation(delegation))
            .perform(&operator)
            .await
            .context("failed to install fixture root grant")?;
        profile
            .credential()
            .site(crate::identity::LOCAL_ROOT_SITE)
            .save(serde_json::to_vec(&record)?)
            .perform(&operator)
            .await
            .context("failed to persist fixture root")?;
    }

    Ok((profile, operator))
}

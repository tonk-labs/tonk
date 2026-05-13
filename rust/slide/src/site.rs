//! `.tonk/` site discovery, init, and open.
//!
//! A *site* is a working directory that contains a `.tonk/`
//! sub-directory, in which a single dialog repository named
//! `main` lives. Slide only reads and writes that one
//! repository on the `main` branch — multi-branch / multi-repo
//! UX is intentionally not exposed.
//!
//! [`SlideSite`] is the assembled context every command works
//! against: profile, operator (rooted at `.tonk/`), the
//! repository, and the opened `main` branch.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_operator::{Operator, Profile};
use dialog_repository::{Branch, Repository, RepositoryExt as _};
use dialog_storage::provider::storage::{NativeSpace, Storage};

/// Name of the dialog repository slide uses inside `.tonk/`.
pub const REPO_NAME: &str = "main";

/// The single branch slide reads and writes against.
pub const BRANCH_NAME: &str = "main";

/// Profile name used for the local identity.
pub const PROFILE_NAME: &str = "tonk";

/// Operator derivation context — distinguishes the slide-derived
/// operator key from operator keys derived by other tools sharing
/// the same profile (e.g. the worker).
const OPERATOR_CONTEXT: &[u8] = b"slide";

/// Name of the `.tonk/` sub-directory holding repository data.
pub const SITE_DIRNAME: &str = ".tonk";

/// An opened slide site — profile + operator + repository +
/// branch, all wired up against the on-disk `.tonk/` directory.
pub struct SlideSite {
    /// Absolute path to the `.tonk/` directory backing this site.
    pub root: PathBuf,
    /// The user's profile (shared identity).
    pub profile: Profile,
    /// Operator rooted at [`Self::root`].
    pub operator: Operator<NativeSpace>,
    /// The `main` repository handle. Verifier-typed (`Credential`):
    /// commits flow through the operator's authority chain, so the
    /// repo handle itself doesn't need to carry a signer.
    pub repository: Repository,
    /// The opened `main` branch.
    pub branch: Branch,
}

impl SlideSite {
    /// Walk up from `start` looking for a `.tonk/` directory and
    /// open the site rooted there. Returns an error if no
    /// `.tonk/` is found between `start` and the filesystem root.
    pub async fn discover_and_open(start: &Path) -> Result<Self> {
        let root = find_site_root(start)
            .with_context(|| format!("no .tonk/ found above {}", start.display()))?;
        Self::open(&root).await
    }

    /// Open an already-existing site at the given `.tonk/`
    /// directory. Errors if the directory exists but the dialog
    /// repository inside it is missing or unreadable.
    pub async fn open(root: &Path) -> Result<Self> {
        Self::open_with(root, default_config()).await
    }

    /// Initialize a new site under `parent` — creates the
    /// `.tonk/` directory if missing, bootstraps the dialog
    /// repository, and opens the `main` branch. Idempotent: if
    /// the site already exists, returns it without changes.
    pub async fn init(parent: &Path) -> Result<Self> {
        Self::init_with(parent, default_config()).await
    }

    /// [`Self::open`] with caller-supplied [`SiteConfig`] —
    /// lets tests redirect the profile directory and pick a
    /// unique profile name without touching the user's real
    /// data dir.
    pub async fn open_with(root: &Path, config: SiteConfig) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", root.display()))?;
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

        let branch = repository
            .branch(BRANCH_NAME)
            .open()
            .perform(&operator)
            .await
            .with_context(|| format!("failed to open branch '{BRANCH_NAME}'"))?;

        Ok(Self {
            root,
            profile,
            operator,
            repository,
            branch,
        })
    }

    /// [`Self::init`] with caller-supplied [`SiteConfig`].
    pub async fn init_with(parent: &Path, config: SiteConfig) -> Result<Self> {
        let parent = parent
            .canonicalize()
            .with_context(|| format!("could not canonicalize {}", parent.display()))?;
        let root = parent.join(SITE_DIRNAME);
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;

        let (profile, operator) = build_profile_and_operator(&root, &config).await?;

        // Try to load first. If absent, create + persist a
        // repo→profile delegation chain so the profile can later
        // mint further delegations (invites, etc.) over this
        // repo. Without that root chain `profile.access().claim`
        // fails with "no delegation chain found" the moment we
        // try to mint an invite.
        let repository = match profile
            .repository(REPO_NAME)
            .load()
            .perform(&operator)
            .await
        {
            Ok(repository) => repository,
            Err(_) => bootstrap_repository(&profile, &operator)
                .await
                .with_context(|| format!("failed to bootstrap repository '{REPO_NAME}'"))?,
        };

        let branch = repository
            .branch(BRANCH_NAME)
            .open()
            .perform(&operator)
            .await
            .with_context(|| format!("failed to open branch '{BRANCH_NAME}'"))?;

        Ok(Self {
            root,
            profile,
            operator,
            repository,
            branch,
        })
    }
}

/// Create the repository, mint a self → profile delegation, and
/// persist it so the profile holds the root of a chain it can
/// extend. This mirrors the worker's `create_repository`
/// startup; without it, slide can write to the repo through the
/// operator's blanket `Subject::any()` delegation but can't
/// delegate to anyone *else* (which is what `slide invite`
/// needs).
///
/// Returns the verifier-only `Repository<Credential>` form
/// because the rest of slide treats every repo handle uniformly,
/// regardless of how it was bootstrapped.
async fn bootstrap_repository(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Repository> {
    let signer_repo = profile
        .repository(REPO_NAME)
        .create()
        .perform(operator)
        .await
        .context("failed to create repository")?;

    let delegation = signer_repo
        .access()
        .claim(&signer_repo)
        .delegate(profile.did())
        .perform(operator)
        .await
        .context("failed to mint repo→profile delegation")?;

    profile
        .access()
        .save(delegation)
        .perform(operator)
        .await
        .context("failed to persist repo→profile delegation")?;

    profile
        .repository(REPO_NAME)
        .load()
        .perform(operator)
        .await
        .context("failed to reload repository after bootstrap")
}

/// Knobs for [`SlideSite::open_with`] / [`SlideSite::init_with`].
/// The defaults (via [`default_config`]) reproduce what `slide`
/// does on a real install: profile named `tonk` in the platform
/// profile directory.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// Profile name slide opens (or creates).
    pub profile_name: String,
    /// Where the profile directory lives. `Directory::Profile`
    /// is the platform default; tests pick `Directory::At(...)`
    /// to redirect onto a temp dir.
    pub profile_directory: Directory,
}

impl SiteConfig {
    /// Builder shortcut: same as [`default_config`] but with the
    /// profile name overridden. Lets tests namespace their
    /// profile so parallel runs don't collide.
    pub fn with_profile_name(name: impl Into<String>) -> Self {
        Self {
            profile_name: name.into(),
            profile_directory: Directory::Profile,
        }
    }
}

/// The site config slide uses out of the box — profile named
/// [`PROFILE_NAME`] in the platform profile directory. Exposed
/// so the binary and other crate modules can pass it to
/// `*_with` constructors without reaching into `dialog-effects`
/// for [`Directory::Profile`].
pub fn default_config() -> SiteConfig {
    SiteConfig {
        profile_name: PROFILE_NAME.to_string(),
        profile_directory: Directory::Profile,
    }
}

/// Walk up the directory tree from `start` looking for a sibling
/// `.tonk/` directory. Returns the absolute path to that
/// directory if found.
fn find_site_root(start: &Path) -> Option<PathBuf> {
    let mut current: PathBuf = start
        .canonicalize()
        .ok()
        .or_else(|| start.is_absolute().then(|| start.to_path_buf()))?;
    loop {
        let candidate = current.join(SITE_DIRNAME);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Open (or create) the shared profile and build a slide
/// operator rooted at `.tonk/` for repository data. Identity
/// lives at `config.profile_directory`; only the dialog-repo
/// blocks live under `.tonk/`.
///
/// Exposed as `pub(crate)` so the [`crate::invite`] module can
/// reuse the same wiring when claiming an invite into a fresh
/// `.tonk/` (the join path provisions the space from a verifier
/// credential rather than via `profile.repository(...).open()`,
/// but the profile + operator setup is the same).
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

    let root_str = root
        .to_str()
        .with_context(|| format!("non-UTF-8 path: {}", root.display()))?
        .to_owned();

    let operator = profile
        .derive(OPERATOR_CONTEXT)
        .allow(Subject::any())
        .base(Directory::At(root_str))
        .build(storage)
        .await
        .context("failed to build operator")?;

    Ok((profile, operator))
}

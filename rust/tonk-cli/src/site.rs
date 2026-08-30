//! Site open and init.
//!
//! A *site* is a directory that contains a single dialog
//! repository named `main`. Tonk only reads and writes that one
//! repository on the `main` branch — multi-branch / multi-repo
//! UX is intentionally not exposed. A site's directory is never
//! located by walking the current directory: the caller resolves
//! it through the space registry (see [`crate::space`]) and passes
//! the path in directly.
//!
//! [`TonkSite`] is the assembled context every command works
//! against: profile, operator (rooted at the site directory), the
//! repository, and the opened `main` branch.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

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
use tonk_account::prefix::{
    SPACE_ROOT_SITE_PREFIX, space_root_site, validate_prefix as verify_prefix,
};

/// The standard-library notation document seeded into a freshly
/// created repository: the built-in concepts, views, commands, and
/// rules. Embedded at compile time (`include_str!`, not a runtime
/// file read) so it travels with the binary and with test archives.
/// This is the same `core.yaml` the worker fetches and lowers at
/// repository creation.
const STANDARD_LIBRARY: &str = include_str!("../../tonk-core/assets/library/core.yaml");

/// Whether the seeded standard library declares a concept (or a
/// command, its transient sibling) called `name`.
///
/// Live schema enumeration also sees these runtime concepts, and on a
/// fresh space they outnumber the author's own by forty to one.
/// Agent-facing listings use this to keep the application vocabulary
/// short — see [`crate::schema::is_system_concept`].
///
/// Only `concept!:` and `command!:` anchors count. The library also
/// anchors attributes, views, and routes, but those live in a
/// different namespace: a concept named after one of them is the
/// author's, not the library's.
pub(crate) fn standard_library_declares_concept(name: &str) -> bool {
    STANDARD_LIBRARY_CONCEPTS.contains(name)
}

/// Every concept and command anchor the library declares, scanned
/// once. See [`STANDARD_LIBRARY_PINS`] for why these are sets and not
/// a scan per lookup.
static STANDARD_LIBRARY_CONCEPTS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    STANDARD_LIBRARY
        .lines()
        .filter_map(declared_concept)
        .collect()
});

/// The concept (or command) anchor declared on one standard-library
/// line. `None` for every other line.
fn declared_concept(line: &str) -> Option<&str> {
    let anchor = line
        .strip_prefix("concept!: &")
        .or_else(|| line.strip_prefix("command!: &"))?;
    anchor
        .split_ascii_whitespace()
        .next()
        .map(|anchor| anchor.trim_end_matches(':'))
}

/// Whether the seeded standard library pins `uri` as the `this:` of
/// one of its own declarations.
///
/// Every library declaration that matters here — its views above all —
/// carries an explicit `this:`, so the set of pinned URIs is exactly
/// the set of entities the library seeded. Listings of branch data
/// (`tonk view`) use it to tell the library's twenty-five views
/// from the author's own.
///
/// Query variables (`this: ?this`, inside the library's rules) are not
/// entities and never match a claim's subject, so they are skipped.
pub(crate) fn standard_library_pins_entity(uri: &str) -> bool {
    STANDARD_LIBRARY_PINS.contains(uri)
}

/// Every entity the library pins, scanned once.
///
/// The listing that calls this asks per claim row and per renderable
/// attribute, so a scan per lookup would walk the whole embedded
/// document a few thousand times to list a handful of views.
static STANDARD_LIBRARY_PINS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| STANDARD_LIBRARY.lines().filter_map(pinned_entity).collect());

/// The `this:` value declared on one standard-library line, at any
/// indentation. `None` for every other line, and for the query
/// variables the library's rule bodies bind.
fn pinned_entity(line: &str) -> Option<&str> {
    let value = line.trim_start().strip_prefix("this:")?.trim();
    (!value.is_empty() && !value.starts_with('?')).then_some(value)
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
    /// Exact profile-local account and space registry used to open this site.
    pub account_store: crate::space::SpaceStore,
}

impl TonkSite {
    /// Open an already-existing site at the given directory.
    /// Errors if the directory exists but the dialog repository
    /// inside it is missing or unreadable.
    pub async fn open(root: &Path) -> Result<Self> {
        Self::open_with(root, default_config()?).await
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

        // Ask the data itself before anything opens a branch — opening it is
        // what fails on old data, and it fails deep enough to be unreadable.
        //
        // Reading it by path is the weak part, and deliberate for now.
        // `BranchReference::cell` addresses the same record without knowing
        // the layout, but `Cell<T>` carries a CBOR codec: `Cell<Vec<u8>>`
        // tries to decode the record *into* a byte string and fails, since
        // it is a map. Getting the bytes undecoded needs a codec-free read
        // dialog does not expose — which is what dialog-db#449 asks for.
        //
        // Branches migrate independently, so this asks about the one every
        // command below opens and says nothing about any other.
        let revision =
            std::fs::read(root.join(tonk_account::revision_path(REPO_NAME, BRANCH_NAME))).ok();
        match tonk_account::readability(revision.as_deref()) {
            tonk_account::Readability::Current => {}
            tonk_account::Readability::Legacy => {
                bail!("{}", tonk_account::LEGACY_FORMAT_REMEDY)
            }
            tonk_account::Readability::Unknown => {
                // Not a revision this build understands, and not recognisably
                // an old one either. Fall through: the branch open below
                // reports whatever is actually wrong, which beats guessing
                // migration.
            }
        }

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
            config.account_store.clone(),
            config.require_account,
        )
        .await?;

        Ok(Self {
            root,
            profile,
            operator,
            repository,
            reactor,
            account_store: config.account_store,
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
    /// `root` — no `.tonk/` nesting. This is what canonical space
    /// storage uses: the registry maps a space name to this
    /// directory. Idempotent: an existing repository at `root` is
    /// loaded, not clobbered, which is also how `tonk space new
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
                bootstrap_repository(&profile, &operator, &config)
                    .await
                    .with_context(|| format!("failed to bootstrap repository '{REPO_NAME}'"))?,
                true,
            ),
        };

        let reactor = Reactor::new(profile.clone());
        let operator = crate::account_authority::wrap(
            operator,
            profile.clone(),
            config.account_store.clone(),
            config.require_account,
        )
        .await?;

        let site = Self {
            root,
            profile,
            operator,
            repository,
            reactor,
            account_store: config.account_store,
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
    /// unreadable old space becomes a sentence someone can act on rather than
    /// `missing field \`branch\`` from inside block decoding.
    /// Acquire a named branch on this site's repository.
    ///
    /// Branches hold separate data and are migrated separately, so anything
    /// walking a whole space names each in turn rather than assuming `main`.
    pub async fn named_branch(&self, branch: &str) -> Result<BranchSession, ReactorError> {
        self.reactor
            .repository(REPO_NAME)
            .branch(branch)
            .acquire(&self.operator)
            .await
    }

    /// Acquire the site's `main` branch, naming the remedy when the data
    /// predates this build's format.
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

/// Every DID a roster row for this installation could be keyed on, most
/// specific first.
///
/// Three identities can name one installation and they are not
/// interchangeable:
///
/// - the **account** signed in here, which a linked device writes rows under;
/// - the durable **local root** this device delegates from once a passkey
///   account is linked. Living in the profile rather than the registry, it
///   survives sign-out;
/// - the **onboarding account** — the durable root before any passkey, what
///   unlinked creates delegate to and unlinked joins stamp their membership
///   with. Read from its grant's issuer, so rows it wrote stay recognizable
///   even after sign-in retires the account itself;
/// - the **profile**, this device's own key, as the last resort.
///
/// Reading and writing resolve through this same order, so a row this
/// installation writes is a row it later recognizes as its own.
///
/// Matches the worker's `member_did`, so a founder row written here and one
/// written by the browser converge to the same content-derived entity across
/// every device on the same account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    account: Option<String>,
    local_root: Option<String>,
    onboarding: Option<String>,
    profile: String,
}

impl Identity {
    /// Resolve this installation's identities from one open replica.
    ///
    /// The account comes from the registry and the local root from the
    /// profile's credential store; both are properties of the installation,
    /// not of the space, so any replica answers for all of them.
    pub async fn of(site: &TonkSite) -> Result<Self> {
        Ok(Self {
            account: site.account_store.account()?.map(|account| account.root),
            // Best effort: a device that has never been provisioned has no
            // local root, and that is not an error — it just means the row
            // to look for is the profile's.
            local_root: crate::identity::local_root_with_operator(
                &site.profile,
                site.operator.local(),
            )
            .await
            .ok()
            .flatten()
            .map(|root| root.root_did),
            onboarding: onboarding_grant_issuer(site).await,
            profile: site.profile.did().to_string(),
        })
    }

    /// The root of the account signed in here, if any.
    ///
    /// Narrower than [`Self::dids`] on purpose. "Which account am I signed
    /// in as" is a question about the registry slot, and it is the one that
    /// changes when somebody switches accounts; "can this installation act
    /// on this space" is the broader question the other identities answer.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// The identities in the order a roster row is looked for.
    pub fn dids(&self) -> impl Iterator<Item = &str> {
        self.account
            .as_deref()
            .into_iter()
            .chain(self.local_root.as_deref())
            .chain(self.onboarding.as_deref())
            .chain(std::iter::once(self.profile.as_str()))
    }

    /// The DID this installation writes a roster row under: the most
    /// specific identity it has.
    pub fn member_did(&self) -> Result<Did> {
        let did = self.dids().next().expect("the profile is always present");
        did.parse()
            .with_context(|| format!("'{did}' is not a valid DID"))
    }
}

/// The DID a roster row for this installation is keyed on.
///
/// See [`Identity`] for why there are three candidates and why this is the
/// order they are tried in.
pub async fn member_did(site: &TonkSite) -> Result<Did> {
    Identity::of(site).await?.member_did()
}

/// The onboarding account's DID, read from its persisted grant's
/// issuer. Best effort, and deliberately independent of the custodian:
/// rows the onboarding account wrote must stay recognizable after
/// sign-in retires it.
async fn onboarding_grant_issuer(site: &TonkSite) -> Option<String> {
    let bytes = site
        .profile
        .credential()
        .site(crate::onboarding::ONBOARDING_GRANT_SITE)
        .load::<Vec<u8>>()
        .perform(site.operator.local())
        .await
        .ok()?;
    let chain = DelegationChain::try_from(bytes.as_slice()).ok()?;
    Some(chain.issuer().to_string())
}

/// Stamp this installation as the space's founder.
///
/// Account-owned creation and `tonk space link` each call this once. The
/// content-derived membership entity makes retries idempotent.
pub async fn record_founder_membership(site: &TonkSite) -> Result<()> {
    let member = member_did(site).await?;
    record_founder_membership_for(site, member).await
}

/// Stamp an explicit member DID as founder.
///
/// The roster lives on the content branch, because only upstreamed branches
/// sync: a roster written to the local-only meta branch never reaches the
/// account's other devices or the people it is shared with. The write goes
/// through the reactor's cached `main` handle for the same reason the worker's
/// does — a commit through a separate handle leaves the cached one pinned at
/// its old head and wedges later sync.
pub async fn record_founder_membership_for(site: &TonkSite, member: Did) -> Result<()> {
    use tonk_schema::{MemberRole, Membership};

    let membership = Membership::new(member, site.repository.did());
    let session = site
        .branch()
        .await
        .context("failed to open the membership branch")?;
    session
        .handle()
        .transaction()
        .assert(membership.clone())
        .assert(MemberRole::founder(membership.this().clone()))
        .commit()
        .perform(&site.operator)
        .await
        .context("failed to record founder membership")?;
    Ok(())
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
    config: &SiteConfig,
) -> Result<Repository> {
    let account_store = &config.account_store;
    let require_account = config.require_account;
    let provision_account_spaces = config.provision_account_spaces;
    let local_root = crate::identity::local_root_with_operator(profile, operator).await?;
    let account_operator = if require_account {
        crate::account::require_account_with_operator_in(profile, operator, account_store).await?;
        let account_operator =
            crate::account_state::credential_operator_for_store(profile, account_store).await?;
        Some(account_operator)
    } else {
        None
    };
    // The durable owner: the passkey root once one is linked, else this
    // device's onboarding account — a real account custodied locally,
    // minted on first use, the same shape the worker gives a browser
    // from first boot. Unlinked spaces then have the same anatomy as
    // linked ones, and `tonk account login` moves them with the shared
    // account rotation instead of adopting a bespoke local form.
    let store_operator = crate::account_state::store_operator_with_config(
        profile,
        account_store,
        &config.profile_name,
        config.profile_directory.clone(),
    )
    .await?;
    let onboarding = match &local_root {
        Some(_) => None,
        None => Some(crate::onboarding::account(profile, &store_operator).await?),
    };
    let durable_did: dialog_varsig::Did = match (&local_root, &onboarding) {
        (Some(record), _) => record
            .root_did
            .parse()
            .context("stored root DID is invalid")?,
        (None, Some(secret)) => {
            use dialog_varsig::Principal as _;
            // The grant travels by exact bytes so every operator store
            // holds the same delegation; this site's operator installs
            // it here, where the space chain will be proven.
            crate::onboarding::install_grant(profile, operator)
                .await?
                .context("the onboarding account has no device grant")?;
            secret
                .signer()
                .await
                .map_err(|error| anyhow::anyhow!("the onboarding signer did not derive: {error}"))?
                .did()
        }
        (None, None) => unreachable!("an onboarding account is minted when no root exists"),
    };

    // The signer comes from an explicit seed so the seed can be sealed
    // to the account BEFORE the space exists: the custody row is the
    // copy the account's other devices recover the space from, and a
    // create that cannot record it must not produce a space that only
    // this machine can ever re-derive.
    let seed = zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
    let signer = Ed25519Signer::import(&*seed)
        .await
        .context("failed to derive the space signer")?;
    if require_account {
        let account_operator = account_operator
            .as_ref()
            .expect("account operator exists when an account is required");
        let account =
            crate::account_state::open_account_branch_in(profile, account_operator, account_store)
                .await?
                .context("the account repository is not ready to custody this space")?;
        let recipient = crate::custody::account_recipient(&account, &durable_did, account_operator)
            .await?
            .context(
                "the account has not published its encryption key yet; \
                     open /account in a signed-in browser once, then retry",
            )?;
        crate::custody::custody_space_seed(
            &account,
            &signer.did(),
            &recipient,
            &seed,
            account_operator,
        )
        .await?;
    } else if let Some(secret) = &onboarding {
        // Unlinked: the seed seals to the onboarding key on the local
        // account branch — the same branch the account mounts once the
        // device signs in, so the rows ride straight into rotation.
        let recipient = secret.secret().did();
        let account = crate::custody::open_local_account_branch(profile, &store_operator).await?;
        crate::custody::custody_space_seed(
            &account,
            &signer.did(),
            &recipient,
            &seed,
            &store_operator,
        )
        .await?;
    }

    let signer_repo = profile
        .repository(REPO_NAME)
        .create()
        .with_credential(signer)
        .perform(operator)
        .await
        .context("failed to create repository")?;
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

    if require_account {
        let account_operator = account_operator
            .as_ref()
            .expect("account operator exists when an account is required");
        if crate::account_state::open_account_branch_in(profile, account_operator, account_store)
            .await?
            .is_none()
        {
            bail!("the account repository is not ready to retain this space");
        }
        crate::account_state::retain_space_delegation_in(
            profile,
            account_operator,
            account_store,
            &prefix,
        )
        .await?;
        if provision_account_spaces {
            crate::customer::provision_in(profile, account_store, &signer_repo.did(), &prefix)
                .await
                .context("failed to provision the account-owned space")?;
        }
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
/// An invite supplies its own authority, so this path does not impose the
/// account precondition used when restoring an account-root prefix. Before an
/// account is linked, a valid profile-ending prefix remains reusable on this
/// device and can be connected to an account by the later profile union.
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
    let local_root = crate::identity::local_root_with_operator(&profile, &operator).await?;
    let require_account = config.require_account && require_reusable;
    if require_account {
        crate::account::require_account_with_operator_in(
            &profile,
            &operator,
            &config.account_store,
        )
        .await?;
    }
    let authority_root: Did = match local_root {
        Some(root) => root
            .root_did
            .parse()
            .context("stored root DID is invalid")?,
        None => {
            // Before a passkey exists the device's durable root is its
            // onboarding account, and a reusable prefix may terminate
            // there — sign-in rotation re-roots it later.
            let store_operator = crate::account_state::store_operator_with_config(
                &profile,
                &config.account_store,
                &config.profile_name,
                config.profile_directory.clone(),
            )
            .await?;
            match crate::onboarding::did(&profile, &store_operator).await? {
                Some(did) => did,
                None if require_reusable => {
                    bail!("neither a root nor an onboarding account exists on this device")
                }
                None => profile.did(),
            }
        }
    };
    let subject = chain
        .subject()
        .cloned()
        .context("delegated authority has no repository subject")?;
    let chain_bytes = chain
        .to_bytes()
        .context("failed to serialize delegated authority")?;
    let reusable = verify_prefix(&chain_bytes, &authority_root).await;
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
            .site(space_root_site(&subject, &authority_root))
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
        config.account_store.clone(),
        require_account,
    )
    .await?;
    Ok(TonkSite {
        root: root.to_path_buf(),
        profile,
        operator,
        repository,
        reactor,
        account_store: config.account_store,
    })
}

/// Load the exact reusable prefix for a site, recovering it from pre-feature
/// profile authority when the dedicated credential is absent.
pub async fn account_root_prefix(site: &TonkSite, account_root: &Did) -> Result<DelegationChain> {
    match adopt_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        &site.repository.did(),
        account_root,
    )
    .await
    {
        Ok(chain) => {
            site.profile
                .access()
                .save(UcanDelegation(chain.clone()))
                .perform(site.operator.local())
                .await
                .context("failed to retain account-root authority for this profile")?;
            Ok(chain)
        }
        Err(profile_error) if site.repository.credential().signer().is_some() => {
            let Some(dialog_credentials::Signer::Ed25519(signer)) =
                site.repository.credential().signer()
            else {
                unreachable!("tonk-cli enables only Ed25519 credentials");
            };
            let minter = Repository::from(signer.clone());
            let delegation: UcanDelegation = minter
                .access()
                .claim(&minter)
                .delegate(account_root.clone())
                .perform(site.operator.local())
                .await
                .with_context(|| {
                    format!(
                        "the profile cannot delegate this space and its repository signer failed: {profile_error}"
                    )
                })?;
            let chain = delegation.into_chain();
            site.profile
                .access()
                .save(UcanDelegation(chain.clone()))
                .perform(site.operator.local())
                .await
                .context("failed to retain repository-signed authority for this profile")?;
            let bytes = chain
                .to_bytes()
                .context("failed to serialize repository-signed account-root prefix")?;
            let validated = validate_prefix(bytes.clone(), account_root)
                .await
                .context("repository-signed account-root prefix is invalid")?;
            save_prefix(
                &site.profile,
                site.operator.local(),
                &space_root_site(&site.repository.did(), account_root),
                bytes,
            )
            .await
            .context("failed to persist repository-signed account-root prefix")?;
            Ok(validated)
        }
        Err(error) => Err(error),
    }
}

/// Decode a stored prefix for `account_root`.
async fn validate_prefix(bytes: Vec<u8>, account_root: &Did) -> Result<DelegationChain> {
    Ok(verify_prefix(&bytes, account_root).await?.chain)
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

/// Resolve the reusable `subject → … → account_root` prefix, recovering it
/// from retained delegations when no validated credential is stored yet.
///
/// Every path that authorizes a remote request for a space needs this exact
/// prefix, so all of them recover the same way. Authority claimed before the
/// account existed stops at the profile until linking retains the profile →
/// account union. Recovery composes that existing path without minting a new
/// ownership edge; explicit ownership adoption remains a separate operation.
pub async fn load_account_root_prefix_for(
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

    let chain = recover_prefix(profile, operator, subject, account_root)
        .await?
        .context("no existing authority reaches this account root")?;
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

/// Explicitly adopt authority held by `profile` into `account_root`.
///
/// This is the only boundary allowed to mint a new space-to-account prefix;
/// routine remote authorization calls [`load_account_root_prefix_for`] and
/// therefore cannot silently change ownership.
pub async fn adopt_account_root_prefix_for(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain> {
    match load_account_root_prefix_for(profile, operator, subject, account_root).await {
        Ok(chain) => Ok(chain),
        Err(_) => {
            let chain = mint_prefix(profile, operator, subject, account_root).await?;
            let bytes = chain
                .to_bytes()
                .context("failed to serialize adopted account-root prefix")?;
            let validated = validate_prefix(bytes.clone(), account_root)
                .await
                .context("adopted account-root prefix is invalid")?;
            save_prefix(
                profile,
                operator,
                &space_root_site(subject, account_root),
                bytes,
            )
            .await
            .context("failed to persist adopted account-root prefix")?;
            Ok(validated)
        }
    }
}

/// Compatibility name for callers that explicitly establish ownership.
pub async fn account_root_prefix_for(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<DelegationChain> {
    adopt_account_root_prefix_for(profile, operator, subject, account_root).await
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

/// Rebuild the prefix from delegations the profile's access branch retains.
///
/// Ask the branch prover for the account root directly. Proving as the
/// profile would stop at the shorter `subject → … → profile` path and omit
/// the union edge needed by account-bound authorization.
pub(crate) async fn recover_prefix(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    subject: &Did,
    account_root: &Did,
) -> Result<Option<DelegationChain>> {
    let access = Repository::from(profile)
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(operator)
        .await
        .context("failed to open the profile access branch")?;
    let proof = match access
        .delegations()
        .prove(
            account_root.clone(),
            dialog_ucan::Scope {
                subject: dialog_ucan_core::subject::Subject::Specific(subject.clone()),
                command: dialog_ucan_core::command::Command::parse("/use")
                    .expect("the use command is valid"),
                parameters: dialog_ucan::Parameters::default(),
            },
        )
        .perform(operator)
        .await
    {
        Ok(proof) => proof,
        Err(dialog_capability::access::AuthorizeError::UnprovenSubject { .. }) => return Ok(None),
        Err(error) => return Err(error).context("failed to recover repository authority"),
    };
    let delegations = proof
        .proofs
        .into_iter()
        .map(|certificate| certificate.0)
        .collect::<Vec<_>>();
    if delegations.is_empty() {
        return Ok(None);
    }
    DelegationChain::try_from(delegations)
        .context("recovered repository authority is not a valid chain")
        .map(Some)
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
        .context("this profile cannot delegate the space to the account root")?;
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
    /// Whether fresh account-owned repositories call `/provider/add`.
    /// Production account profiles enable this; authority-only fixtures may
    /// disable it while still exercising account-bound authorization.
    pub provision_account_spaces: bool,
    /// Profile-scoped account repository and session state.
    pub account_store: crate::space::SpaceStore,
}

impl SiteConfig {
    /// Builder shortcut: same as [`default_config`] but with the
    /// profile name overridden. Lets tests namespace their
    /// profile so parallel runs don't collide.
    pub fn with_profile_name(name: impl Into<String>) -> Result<Self> {
        Ok(Self {
            profile_name: name.into(),
            profile_directory: Directory::Profile,
            require_account: false,
            provision_account_spaces: false,
            account_store: crate::space::SpaceStore::open()
                .context("failed to locate account state")?,
        })
    }
}

/// The site config tonk uses out of the box — profile named
/// [`PROFILE_NAME`] in the platform profile directory. Exposed
/// so the binary and other crate modules can pass it to
/// `*_with` constructors without reaching into `dialog-effects`
/// for [`Directory::Profile`].
pub fn default_config() -> Result<SiteConfig> {
    Ok(SiteConfig {
        profile_name: PROFILE_NAME.to_string(),
        profile_directory: Directory::Profile,
        require_account: std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none(),
        provision_account_spaces: true,
        account_store: crate::space::SpaceStore::open()
            .context("failed to locate account state")?,
    })
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

    Ok((profile, operator))
}

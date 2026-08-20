//! Native account space inventory, pull, and directory recording.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dialog_operator::{Operator, Profile};
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Branch, SiteAddress};
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use tonk_account::{AccountStateStatus, MAIN_BRANCH};
use tonk_schema::RepositoryName;
use tonk_schema::directory::{MountBranch, MountRecord, MountRemote};
use tonk_schema::prelude::DidExt as _;

#[cfg(feature = "integration-tests")]
use crate::account;
use crate::remote::{self, DEFAULT_REMOTE};
use crate::site::TonkSite;
use crate::spot::{self, SpotStore};

/// One row rendered by `tonk account spots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSpotRow {
    /// Repository subject DID.
    pub subject: String,
    /// Name mirrored in the account directory.
    pub remote_name: Option<String>,
    /// Registry name already resolving to this subject.
    pub local_name: Option<String>,
    /// Retained for row rendering; legacy escrow ambiguity no longer occurs.
    pub ambiguous: bool,
    /// Whether the directory carries a mount record for this space.
    pub pullable: bool,
}

/// Result of pulling one account spot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullOutcome {
    /// Repository subject DID.
    pub subject: String,
    /// Local registry name.
    pub name: String,
    /// Registered site directory (canonical for a newly pulled spot).
    pub site: PathBuf,
    /// Whether the subject was already registered and no work was needed.
    pub already_local: bool,
    /// Initial-pull diagnostic when the mounted spot was retained for retry.
    pub warning: Option<String>,
}

/// Result of canonically archiving one account-space subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOutcome {
    /// Exact archived repository subject.
    pub subject: String,
    /// Whether this call committed the monotonic marker.
    pub newly_archived: bool,
    /// Compatibility projection diagnostic. The directory-backed custody
    /// model needs no provider projection, so this is normally absent.
    pub projection_warning: Option<String>,
}

/// What recording a spot in the account directory did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// No account is linked (or it has not hydrated yet), so there is
    /// no directory to record into.
    NoAccount,
    /// The site has no `main` upstream yet — a local-only spot is
    /// deliberately not listed as mountable anywhere else.
    NoUpstream,
    /// The directory already holds exactly this configuration.
    Unchanged,
    /// The directory was updated (and the change pushed best-effort).
    Recorded,
}

/// One isolated warning from a whole-registry directory sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordWarning {
    /// Registry name whose recording failed.
    pub name: String,
    /// Diagnostic suitable for stderr.
    pub message: String,
}

fn site_config(_profile: &Profile, store: &SpotStore) -> Result<crate::site::SiteConfig> {
    #[cfg(feature = "integration-tests")]
    if let Some(mut config) = account::integration_site_config(_profile) {
        config.account_store = store.clone();
        return Ok(config);
    }
    Ok(crate::site::SiteConfig {
        profile_name: crate::account_profiles::generated_dialog_profile_name(store)
            .unwrap_or_else(|| crate::site::PROFILE_NAME.to_owned()),
        profile_directory: dialog_effects::storage::Directory::Profile,
        require_account: std::env::var_os("TONK_UNSAFE_ALLOW_DEVICE_ROOT").is_none(),
        account_store: store.clone(),
    })
}

async fn open_site(
    path: &std::path::Path,
    profile: &Profile,
    store: &SpotStore,
) -> Result<TonkSite> {
    let site = TonkSite::open_with(path, site_config(profile, store)?).await?;
    if site.profile.did() != profile.did() {
        bail!("registered site profile does not match the active account profile");
    }
    Ok(site)
}

#[derive(Debug, Clone)]
struct LocalSpot {
    name: String,
    site: PathBuf,
}

async fn local_subjects(
    profile: &Profile,
    store: &SpotStore,
) -> Result<HashMap<String, LocalSpot>> {
    let registry = store.load()?;
    let mut subjects: HashMap<String, LocalSpot> = HashMap::new();
    for (name, entry) in registry.spots {
        let site = match open_site(&entry.site, profile, store).await {
            Ok(site) => site,
            Err(error) => {
                eprintln!("warning: local spot '{name}' could not be inspected: {error:#}");
                continue;
            }
        };
        let subject = site.repository.did().to_string();
        match subjects.entry(subject) {
            Entry::Vacant(slot) => {
                slot.insert(LocalSpot {
                    name,
                    site: entry.site,
                });
            }
            Entry::Occupied(slot) => eprintln!(
                "warning: local spots '{}' and '{name}' both resolve to {}; using '{}'",
                slot.get().name,
                slot.key(),
                slot.get().name
            ),
        }
    }
    Ok(subjects)
}

/// Open the account branch, hydrating the link first when this device
/// has not reached Ready yet.
///
/// `tonk account spots` must not depend on a prior `tonk account
/// status` run having done the hydration: a linked-but-unhydrated
/// profile is an ordinary state right after `tonk account link` on a
/// fresh device, and reporting it as "no account" reads as data loss.
async fn ready_account_branch(
    profile: &Profile,
    store: &SpotStore,
) -> Result<(Operator<NativeSpace>, Branch)> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    if let Some(branch) =
        crate::account_state::open_account_branch_in(profile, &operator, store).await?
    {
        return Ok((operator, branch));
    }
    let outcome =
        crate::account_state::ensure_with_operator_and_store(profile, operator, store.clone())
            .await?;
    match outcome.status {
        AccountStateStatus::Unconfigured => {
            bail!("no account is linked on this profile; run `tonk account link`")
        }
        AccountStateStatus::Unhydrated => bail!(
            "the account is linked but its first sync has not succeeded yet{}",
            outcome
                .warning
                .map(|warning| format!(": {warning}"))
                .unwrap_or_default()
        ),
        AccountStateStatus::Ready => {}
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let branch = crate::account_state::open_account_branch_in(profile, &operator, store)
        .await?
        .context("the account branch did not open after hydration")?;
    Ok((operator, branch))
}

/// List the account directory's spots and identify subjects already
/// registered locally. Reads the account DB — the same directory facts
/// the Hub renders — not the retired spot-backup escrow.
pub async fn list(profile: &Profile, store: &SpotStore) -> Result<Vec<AccountSpotRow>> {
    let (operator, branch) = ready_account_branch(profile, store).await?;
    // Freshen best-effort: an offline listing still renders the local
    // copy of the directory.
    if let Err(error) = branch.pull().download().perform(&operator).await {
        eprintln!("warning: account sync failed; listing the local copy: {error:#}");
    }
    let local = local_subjects(profile, store).await?;
    let rows = tonk_schema::directory::spaces(&branch, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .into_iter()
        .map(|space| {
            let subject = space.subject.to_string();
            AccountSpotRow {
                local_name: local.get(&subject).map(|spot| spot.name.clone()),
                pullable: space.mountable,
                remote_name: space.name,
                ambiguous: false,
                subject,
            }
        })
        .collect();
    Ok(rows)
}

fn name_error(name: Option<&str>, reason: impl std::fmt::Display) -> anyhow::Error {
    let label = name.unwrap_or("(missing)");
    anyhow::anyhow!("account spot name '{label}' cannot be used: {reason}; pass --name <slug>")
}

struct FreshPullTarget {
    path: PathBuf,
    committed: bool,
}

impl FreshPullTarget {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for FreshPullTarget {
    fn drop(&mut self) {
        if self.committed || !self.path.exists() {
            return;
        }
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!(
                "warning: failed to clean up incomplete account spot at {}: {error}",
                self.path.display()
            );
        }
    }
}

/// Pull exactly one account spot into canonical local storage.
pub async fn pull(
    profile: &Profile,
    store: &SpotStore,
    subject: &str,
    requested_name: Option<&str>,
) -> Result<PullOutcome> {
    let requested: Did = subject
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid account spot subject '{subject}': {error:?}"))?;
    let (operator, account) = ready_account_branch(profile, store).await?;
    if let Err(error) = account.pull().download().perform(&operator).await {
        eprintln!("warning: account sync failed; pulling from the local copy: {error:#}");
    }
    // Bring retained certificates into the profile's provable reach so
    // the space→root chain can be assembled below. Best-effort, like
    // the pull above: with the account unreachable, already-local
    // certificates may still prove the chain.
    if let Err(error) =
        crate::account_state::adopt_account_access_in(profile, &operator, store).await
    {
        eprintln!(
            "warning: account access adoption failed; proving from local certificates: {error:#}"
        );
    }

    let account_root: Did = crate::identity::local_root_with_operator(profile, &operator)
        .await?
        .context("no account root is recorded on this profile")?
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
    let archived = tonk_schema::account::list_account_spaces(&account, &operator)
        .await?
        .into_iter()
        .any(|row| row.account == account_root && row.subject == requested && row.archived);
    if archived {
        bail!("account space {requested} is archived");
    }

    let record = tonk_schema::directory::mount_record(&account, &requested, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .with_context(|| {
            format!(
                "the account directory has no mount record for {requested} — the spot                  is local-only on its home device or predates directory records"
            )
        })?;
    let directory_name = tonk_schema::directory::spaces(&account, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .into_iter()
        .find(|space| space.subject == requested)
        .and_then(|space| space.name);

    let local = local_subjects(profile, store).await?;
    if let Some(local) = local.get(requested.as_ref()) {
        let mut registry = store.load()?;
        if registry.unsuppress(requested.as_ref()) {
            store.save(&registry)?;
        }
        return Ok(PullOutcome {
            subject: requested.to_string(),
            name: local.name.clone(),
            site: local.site.clone(),
            already_local: true,
            warning: None,
        });
    }

    let name = requested_name
        .map(str::to_string)
        .or(directory_name)
        .ok_or_else(|| name_error(None, "the directory has no stored name"))?;
    spot::validate_name(&name).map_err(|error| name_error(Some(&name), error))?;
    let registry = store.load()?;
    if registry.spots.contains_key(&name) {
        return Err(name_error(
            Some(&name),
            "that local name is already occupied",
        ));
    }
    let target = store.canonical_site(&name);
    if target.exists() {
        // Reached most often after `tonk spot rm --keep-data`: the
        // registry forgot the name but the data still holds it, so
        // point at both ways to clear the collision rather than
        // reporting it as a bare fact.
        return Err(name_error(
            Some(&name),
            format!(
                "the canonical site {target} already exists and belongs to no \
                 registered spot; adopt it with `tonk spot new {name} --site {target}` \
                 or delete the directory",
                target = target.display()
            ),
        ));
    }

    // The space→root chain assembles from certificates the profile now
    // holds (the account pull above brought the retained delegations
    // down) — the prover walks them; no escrow artifact involved. The
    // root is the linked account's root DID — NOT the account branch's
    // subject: the account is profile main's upstream, so the local
    // branch's subject is the profile itself.
    let chain = crate::site::recover_prefix(profile, &operator, &requested, &account_root)
        .await?
        .with_context(|| {
            format!(
                "no delegation chain from {requested} to the account root is                  provable here — sync the account and retry"
            )
        })?;
    let primary = record
        .remotes
        .iter()
        .find(|remote| remote.name == DEFAULT_REMOTE)
        .or_else(|| record.remotes.first())
        .context("the mount record lists no remotes")?;
    let endpoint = match &primary.address {
        SiteAddress::Ucan(ucan) => ucan.endpoint().to_owned(),
        other => bail!("the mount record's remote is not a UCAN site: {other:?}"),
    };

    let mut fresh_target = FreshPullTarget::new(target.clone());
    let site = crate::site::mount_delegated_at(&target, chain, site_config(profile, store)?)
        .await
        .context("failed to mount account spot")?;
    remote::add_with_revocation(
        &site,
        DEFAULT_REMOTE,
        &endpoint,
        Some(requested.clone()),
        primary.revocation.as_deref(),
    )
    .await
    .context("failed to configure the account spot remote")?;
    remote::set_upstream(&site, DEFAULT_REMOTE)
        .await
        .context("failed to set the account spot upstream")?;
    let canonical_target = target
        .canonicalize()
        .context("failed to canonicalize the mounted account spot")?;
    crate::sync::pull(&site)
        .await
        .with_context(|| format!("initial pull from '{DEFAULT_REMOTE}' failed"))?;
    spot::register_pulled_space(store, &name, &canonical_target, requested.as_ref())?;
    fresh_target.commit();

    Ok(PullOutcome {
        subject: requested.to_string(),
        name,
        site: canonical_target,
        already_local: false,
        warning: None,
    })
}

/// Commit a monotonic archive fact to the canonical account repository.
pub async fn archive(
    profile: &Profile,
    store: &SpotStore,
    subject: &str,
) -> Result<ArchiveOutcome> {
    let operator = crate::account_state::operator_for_store(profile, store).await?;
    archive_with_operator(profile, store, subject, &operator).await
}

async fn archive_with_operator(
    profile: &Profile,
    store: &SpotStore,
    subject: &str,
    operator: &Operator<NativeSpace>,
) -> Result<ArchiveOutcome> {
    let subject: Did = subject
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid account space subject '{subject}': {error:?}"))?;
    let root: Did = crate::identity::local_root_with_operator(profile, operator)
        .await?
        .context("no account root is recorded on this profile")?
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
    let branch = crate::account_state::open_account_branch_in(profile, operator, store)
        .await?
        .context("account repository is not hydrated")?;
    let newly_archived =
        tonk_schema::account::archive_account_space(&branch, &root, &subject, operator).await?;
    branch
        .push()
        .perform(operator)
        .await
        .context("canonical account archive committed locally but its push failed")?;
    Ok(ArchiveOutcome {
        subject: subject.to_string(),
        newly_archived,
        projection_warning: None,
    })
}

#[cfg(feature = "integration-tests")]
#[doc(hidden)]
pub async fn archive_with_operator_for_integration_test(
    profile: &Profile,
    store: &SpotStore,
    subject: &str,
    operator: &Operator<NativeSpace>,
) -> Result<ArchiveOutcome> {
    archive_with_operator(profile, store, subject, operator).await
}

async fn repository_name(site: &TonkSite) -> Option<String> {
    let branch = site.branch().await.ok()?;
    branch
        .handle()
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(site.repository.did().this()),
            name: Term::var("name"),
        })
        .perform(&site.operator)
        .try_vec()
        .await
        .ok()?
        .into_iter()
        .next()
        .map(|row| row.name.0)
}

/// Whether the account directory lists `site`'s repository with a mount
/// record and the site's exact current content revision is known to be
/// durable on the content remote.
///
/// Both facts are local reads. An absent or unhydrated account, or local
/// changes made after the last confirmed push, answer `false` rather than
/// making a destructive command claim recoverability it cannot prove.
pub async fn has_account_backup(site: &TonkSite) -> Result<bool> {
    let store = site.operator.store();
    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;
    let Some(account) =
        crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
    else {
        return Ok(false);
    };
    let listed = tonk_schema::directory::mount_record(&account, &site.repository.did(), &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .is_some();
    if !listed {
        return Ok(false);
    }
    let Some(local_root) =
        crate::identity::local_root_with_operator(&site.profile, site.operator.local()).await?
    else {
        return Ok(false);
    };
    let account_root = local_root
        .root_did
        .parse()
        .context("stored local root DID is invalid")?;
    crate::account_sync::current_revision_is_confirmed(site, &account_root).await
}

/// Mirror one site's name and mount configuration into the account
/// directory as plain facts — [`tonk_schema::directory::record`], fed
/// from the site's own upstream configuration. The directory (not any
/// escrow artifact) is what other devices list and what `tonk account
/// spots pull` mounts from.
///
/// Idempotent and quiet: when the directory already says exactly this,
/// nothing commits, so routine `tonk eval` runs do not churn the
/// account head.
pub async fn record_site(registry_name: &str, site: &TonkSite) -> Result<RecordOutcome> {
    record_site_in(registry_name, site, &SpotStore::open()?).await
}

/// [`record_site`] against a caller-supplied store.
pub async fn record_site_in(
    registry_name: &str,
    site: &TonkSite,
    store: &SpotStore,
) -> Result<RecordOutcome> {
    let Some(upstream) = remote::upstream_remote(site).await? else {
        return Ok(RecordOutcome::NoUpstream);
    };
    let remote_record = remote::find(site, &upstream)
        .await?
        .with_context(|| format!("upstream remote '{upstream}' is not registered"))?;
    let subject = site.repository.did();
    let name = repository_name(site)
        .await
        .unwrap_or_else(|| registry_name.to_string());

    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;
    let Some(account) =
        crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
    else {
        return Ok(RecordOutcome::NoAccount);
    };

    let address = SiteAddress::from(UcanAddress::new(remote_record.endpoint.as_str()));
    let desired = MountRecord {
        remotes: vec![MountRemote {
            name: upstream.clone(),
            address,
            subject: remote_record.subject.clone(),
            revocation: remote_record.revocation_url.clone(),
        }],
        branches: vec![MountBranch {
            name: MAIN_BRANCH.to_string(),
            upstream: Some((upstream.clone(), MAIN_BRANCH.to_string())),
        }],
    };
    let current = tonk_schema::directory::mount_record(&account, &subject, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?;
    let current_name = tonk_schema::directory::spaces(&account, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .into_iter()
        .find(|space| space.subject == subject)
        .and_then(|space| space.name);
    if current.as_ref() == Some(&desired) && current_name.as_deref() == Some(name.as_str()) {
        return Ok(RecordOutcome::Unchanged);
    }

    tonk_schema::directory::record(&account, &subject, Some(&name), &desired, &operator)
        .await
        .context("failed to record the spot in the account directory")?;
    if let Err(error) = account.push().perform(&operator).await {
        eprintln!("warning: account directory updated locally; push failed: {error:#}");
    }
    Ok(RecordOutcome::Recorded)
}

fn first_registry_name_for_site<'a>(
    candidates: impl IntoIterator<Item = (&'a str, &'a Path)>,
    site_root: &Path,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .find_map(|(name, path)| (path == site_root).then_some(name))
}

/// Record the registry entry matching an already-open site.
pub(crate) async fn record_current(site: &TonkSite) -> Result<RecordOutcome> {
    let store = site.operator.store();
    let registry = store.load()?;
    let candidates: Vec<_> = registry
        .spots
        .iter()
        .filter_map(|(name, entry)| {
            entry
                .site
                .canonicalize()
                .ok()
                .map(|path| (name.as_str(), path))
        })
        .collect();
    let name = first_registry_name_for_site(
        candidates
            .iter()
            .map(|(name, path)| (*name, path.as_path())),
        &site.root,
    )
    .context("the evaluated site is not registered as a spot")?;
    record_site_in(name, site, store).await
}

/// Best-effort directory sweep of every registered spot.
pub async fn record_registered(profile: &Profile, store: &SpotStore) -> Vec<RecordWarning> {
    let registry = match store.load() {
        Ok(registry) => registry,
        Err(error) => {
            return vec![RecordWarning {
                name: "(registry)".to_string(),
                message: error.to_string(),
            }];
        }
    };
    let mut warnings = Vec::new();
    let mut inspected_subjects = HashSet::new();
    for (name, entry) in registry.spots {
        let site = match open_site(&entry.site, profile, store).await {
            Ok(site) => site,
            Err(error) => {
                warnings.push(RecordWarning {
                    name,
                    message: format!("{error:#}"),
                });
                continue;
            }
        };
        if !inspected_subjects.insert(site.repository.did().to_string()) {
            continue;
        }
        if let Err(error) = record_site_in(&name, &site, store).await {
            warnings.push(RecordWarning {
                name,
                message: format!("{error:#}"),
            });
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_pull_target_removes_partial_state_unless_committed() {
        let temp = tempfile::tempdir().unwrap();
        let incomplete = temp.path().join("incomplete");
        {
            let _target = FreshPullTarget::new(incomplete.clone());
            std::fs::create_dir_all(incomplete.join("nested")).unwrap();
            std::fs::write(incomplete.join("nested/state"), b"partial").unwrap();
        }
        assert!(!incomplete.exists());

        let retained = temp.path().join("retained");
        {
            let mut target = FreshPullTarget::new(retained.clone());
            std::fs::create_dir_all(&retained).unwrap();
            target.commit();
        }
        assert!(retained.exists());
    }

    #[test]
    fn current_site_aliases_choose_the_first_registry_name() {
        let root = PathBuf::from("/canonical/site");
        let candidates = [("alpha", root.as_path()), ("zeta", root.as_path())];
        assert_eq!(
            first_registry_name_for_site(candidates, &root),
            Some("alpha")
        );
    }
}

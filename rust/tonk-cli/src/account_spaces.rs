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
use crate::space::{self, SpaceStore};
use crate::staged_directory::StagedDirectory;

/// One row rendered by `tonk account space`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSpaceRow {
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

/// Result of pulling one account space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullOutcome {
    /// Repository subject DID.
    pub subject: String,
    /// Local registry name.
    pub name: String,
    /// Registered site directory (canonical for a newly pulled space).
    pub site: PathBuf,
    /// Whether the subject was already registered and no work was needed.
    pub already_local: bool,
    /// Initial-pull diagnostic when the mounted space was retained for retry.
    pub warning: Option<String>,
}

/// What recording a space in the account directory did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// No account is linked (or it has not hydrated yet), so there is
    /// no directory to record into.
    NoAccount,
    /// The site has no `main` upstream yet — a local-only space is
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

fn site_config(_profile: &Profile) -> Result<crate::site::SiteConfig> {
    #[cfg(feature = "integration-tests")]
    if let Some(config) = account::integration_site_config(_profile) {
        return Ok(config);
    }
    crate::site::default_config()
}

async fn open_site(path: &std::path::Path, profile: &Profile) -> Result<TonkSite> {
    let site = TonkSite::open_with(path, site_config(profile)?).await?;
    if site.profile.did() != profile.did() {
        bail!("registered site profile does not match the active account profile");
    }
    Ok(site)
}

#[derive(Debug, Clone)]
struct LocalSpace {
    name: String,
    site: PathBuf,
}

async fn local_subjects(
    profile: &Profile,
    store: &SpaceStore,
) -> Result<HashMap<String, LocalSpace>> {
    let registry = store.load()?;
    let mut subjects: HashMap<String, LocalSpace> = HashMap::new();
    for (name, entry) in registry.spaces {
        let site = match open_site(&entry.site, profile).await {
            Ok(site) => site,
            Err(error) => {
                eprintln!("warning: local space '{name}' could not be inspected: {error:#}");
                continue;
            }
        };
        let subject = site.repository.did().to_string();
        match subjects.entry(subject) {
            Entry::Vacant(slot) => {
                slot.insert(LocalSpace {
                    name,
                    site: entry.site,
                });
            }
            Entry::Occupied(slot) => eprintln!(
                "warning: local spaces '{}' and '{name}' both resolve to {}; using '{}'",
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
/// `tonk account space` must not depend on a prior `tonk account
/// status` run having done the hydration: a linked-but-unhydrated
/// profile is an ordinary state right after `tonk account login` on a
/// fresh device, and reporting it as "no account" reads as data loss.
async fn ready_account_branch(
    profile: &Profile,
    store: &SpaceStore,
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
            bail!("no account is linked on this profile; run `tonk account login`")
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

/// List the account directory's spaces and identify subjects already
/// registered locally. Reads the account DB — the same directory facts
/// the Hub renders — not the retired space-backup escrow.
pub async fn list(profile: &Profile, store: &SpaceStore) -> Result<Vec<AccountSpaceRow>> {
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
            AccountSpaceRow {
                local_name: local.get(&subject).map(|space| space.name.clone()),
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
    anyhow::anyhow!("account space name '{label}' cannot be used: {reason}; pass --name <slug>")
}

/// Pull exactly one account space into canonical local storage.
pub async fn pull(
    profile: &Profile,
    store: &SpaceStore,
    name_or_subject: &str,
    requested_name: Option<&str>,
) -> Result<PullOutcome> {
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

    let directory = tonk_schema::directory::spaces(&account, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?;
    let requested: Did = match name_or_subject.parse() {
        Ok(subject) => subject,
        Err(_) => {
            let matches: Vec<Did> = directory
                .iter()
                .filter(|space| space.name.as_deref() == Some(name_or_subject))
                .map(|space| space.subject.clone())
                .collect();
            match matches.as_slice() {
                [] => bail!("the account directory has no space named '{name_or_subject}'"),
                [subject] => subject.clone(),
                subjects => {
                    let subjects = subjects
                        .iter()
                        .map(AsRef::<str>::as_ref)
                        .collect::<Vec<_>>()
                        .join(", ");
                    bail!(
                        "account space name '{name_or_subject}' is ambiguous; matches {subjects}; pull by subject DID"
                    )
                }
            }
        }
    };

    let record = tonk_schema::directory::mount_record(&account, &requested, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
        .with_context(|| {
            format!(
                "the account directory has no mount record for {requested} — the space                  is local-only on its home device or predates directory records"
            )
        })?;
    let directory_name = directory
        .into_iter()
        .find(|space| space.subject == requested)
        .and_then(|space| space.name);

    let local = local_subjects(profile, store).await?;
    if let Some(local) = local.get(requested.as_ref()) {
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
    space::validate_name(&name).map_err(|error| name_error(Some(&name), error))?;
    let registry = store.load()?;
    if registry.spaces.contains_key(&name) {
        return Err(name_error(
            Some(&name),
            "that local name is already occupied",
        ));
    }
    let target = store.canonical_site(&name);
    if target.exists() {
        // Reached most often after `tonk space rm --keep-data`: the
        // registry forgot the name but the data still holds it, so
        // point at both ways to clear the collision rather than
        // reporting it as a bare fact.
        return Err(name_error(
            Some(&name),
            format!(
                "the canonical site {target} already exists and belongs to no \
                 registered space; adopt it with `tonk space new {name} --site {target}` \
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
    let account_root: Did = crate::identity::local_root_with_operator(profile, &operator)
        .await?
        .context("no account root is recorded on this profile")?
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
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

    let stage = StagedDirectory::beside(&target, "account-pull").with_context(|| {
        format!(
            "failed to stage account space for canonical site {}",
            target.display()
        )
    })?;
    let site = crate::site::mount_delegated_in_empty(stage.path(), chain, site_config(profile)?)
        .await
        .context("failed to mount account space")?;
    remote::add_with_revocation(
        &site,
        DEFAULT_REMOTE,
        &endpoint,
        Some(requested.clone()),
        primary.revocation.as_deref(),
    )
    .await
    .context("failed to configure the account space remote")?;
    remote::set_upstream(&site, DEFAULT_REMOTE)
        .await
        .context("failed to set the account space upstream")?;
    crate::sync::pull(&site)
        .await
        .with_context(|| format!("initial pull from '{DEFAULT_REMOTE}' failed"))?;
    // The pulled replica must carry a roster row this account can claim —
    // that row, on the space's own content branch, is the only record of who
    // owns it, and nothing beside the space is written to stand in for it.
    let role = crate::inventory::role_for_site(&site)
        .await
        .context("could not read the pulled space's roster")?;
    if !matches!(
        role,
        crate::inventory::SpaceRole::Owner | crate::inventory::SpaceRole::Member
    ) {
        bail!("pulled space has no signed membership for this account profile");
    }
    site.reactor.shutdown();
    drop(site);
    let published_target = stage.publish().with_context(|| {
        format!(
            "verified account-space publication at {target} did not settle cleanly; if that path is absent, retry the pull; if it is present, never overwrite or delete it merely to retry—verify its repository subject, run `tonk space` to inspect occupied names, and adopt it only if it is the expected subject with `tonk space new <available-name> --site {target}`",
            target = target.display()
        )
    })?;
    let canonical_target = published_target
        .canonicalize()
        .context("failed to canonicalize the mounted account space")?;
    register_published(store, &name, &canonical_target)?;

    Ok(PullOutcome {
        subject: requested.to_string(),
        name,
        site: canonical_target,
        already_local: false,
        warning: None,
    })
}

fn register_published(store: &SpaceStore, name: &str, canonical_target: &Path) -> Result<()> {
    space::register_existing_unbound(store, name, canonical_target).with_context(|| {
        format!(
            "the verified account space is safe at {}, but registering local name '{name}' failed; never overwrite the occupied entry—run `tonk space`, verify the published site's repository subject, then adopt it under an available name with `tonk space new <available-name> --site {}`",
            canonical_target.display(),
            canonical_target.display()
        )
    })
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

/// Whether the account directory lists `site`'s repository with a
/// mount record — the claim `tonk space rm` leans on when it tells
/// someone their data is recoverable with `tonk account space pull`.
///
/// Answered from the account branch's local copy of the directory; an
/// absent or unhydrated account answers `false` rather than failing
/// the command this probe is trying to make safe.
pub async fn directory_lists(site: &TonkSite) -> Result<bool> {
    let store = &site.account_store;
    let operator =
        crate::account_state::credential_operator_for_store(&site.profile, store).await?;
    let Some(account) =
        crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
    else {
        return Ok(false);
    };
    Ok(
        tonk_schema::directory::mount_record(&account, &site.repository.did(), &operator)
            .await
            .map_err(|error| anyhow::anyhow!("account directory query failed: {error:?}"))?
            .is_some(),
    )
}

/// Mirror one site's name and mount configuration into the account
/// directory as plain facts — [`tonk_schema::directory::record`], fed
/// from the site's own upstream configuration. The directory (not any
/// escrow artifact) is what other devices list and what `tonk account
/// spaces pull` mounts from.
///
/// Idempotent and quiet: when the directory already says exactly this,
/// nothing commits, so routine `tonk eval` runs do not churn the
/// account head.
pub async fn record_site(registry_name: &str, site: &TonkSite) -> Result<RecordOutcome> {
    record_site_in(registry_name, site, &SpaceStore::open()?).await
}

/// [`record_site`] against a caller-supplied store.
pub async fn record_site_in(
    registry_name: &str,
    site: &TonkSite,
    store: &SpaceStore,
) -> Result<RecordOutcome> {
    record_site_for_profile(registry_name, site, &site.profile, store, false).await
}

/// [`record_site_in`] where a failed account push is an error.
///
/// `tonk space link` uses this boundary: the space is tagged as the account's
/// only once the account directory that other devices read has actually
/// accepted the record, so a silent push failure cannot leave a space marked
/// account-owned that no other device can find.
pub async fn record_site_pushed(
    registry_name: &str,
    site: &TonkSite,
    store: &SpaceStore,
) -> Result<RecordOutcome> {
    record_site_for_profile(registry_name, site, &site.profile, store, true).await
}

async fn record_site_for_profile(
    registry_name: &str,
    site: &TonkSite,
    account_profile: &Profile,
    store: &SpaceStore,
    require_push: bool,
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
        crate::account_state::credential_operator_for_store(account_profile, store).await?;
    let Some(account) =
        crate::account_state::open_account_branch_in(account_profile, &operator, store).await?
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
        if require_push {
            account
                .push()
                .perform(&operator)
                .await
                .context("failed to push the account directory")?;
        }
        return Ok(RecordOutcome::Unchanged);
    }

    tonk_schema::directory::record(&account, &subject, Some(&name), &desired, &operator)
        .await
        .context("failed to record the space in the account directory")?;
    if let Err(error) = account.push().perform(&operator).await {
        if require_push {
            return Err(error).context("failed to push the account directory");
        }
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
    let store = &site.account_store;
    let registry = store.load()?;
    let candidates: Vec<_> = registry
        .spaces
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
    .context("the evaluated site is not registered as a space")?;
    record_site_in(name, site, store).await
}

/// Best-effort directory sweep of every registered space.
pub async fn record_registered(profile: &Profile, store: &SpaceStore) -> Vec<RecordWarning> {
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
    for (name, entry) in registry.spaces {
        let site = match open_site(&entry.site, profile).await {
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
    fn a_registration_failure_keeps_the_verified_canonical_site() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = SpaceStore::at(temp.path().join("state"));
        let canonical = store.canonical_site("garden");
        std::fs::create_dir_all(&canonical)?;
        std::fs::write(canonical.join("verified"), b"complete replica")?;
        let occupied_site = temp.path().join("already-registered");
        std::fs::create_dir(&occupied_site)?;
        std::fs::write(occupied_site.join("kept"), b"existing registration")?;
        let occupied_site = occupied_site.canonicalize()?;
        let mut registry = space::Registry::default();
        registry
            .spaces
            .insert("garden".to_owned(), space::SpaceEntry::at(&occupied_site));
        store.save(&registry)?;

        let error = register_published(&store, "garden", &canonical)
            .expect_err("an occupied registry name must not be replaced");

        assert_eq!(
            std::fs::read(canonical.join("verified"))?,
            b"complete replica"
        );
        let retained = store.load()?;
        assert_eq!(retained.spaces["garden"].site, occupied_site);
        assert_eq!(
            std::fs::read(retained.spaces["garden"].site.join("kept"))?,
            b"existing registration"
        );
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("verified account space is safe"),
            "{rendered}"
        );
        assert!(
            rendered.contains(&canonical.display().to_string()),
            "{rendered}"
        );
        assert!(rendered.contains("run `tonk space`"), "{rendered}");
        assert!(rendered.contains("repository subject"), "{rendered}");
        assert!(rendered.contains("<available-name>"), "{rendered}");
        Ok(())
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

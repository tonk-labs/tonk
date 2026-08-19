//! Native account spot inventory, pull, and best-effort backup reconciliation.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dialog_operator::Profile;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::SiteAddress;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Did;
use tonk_account::backup::{ACCOUNT_SPOT_BACKUP_MARKER_PREFIX, AccountSpotBackup};
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

use crate::account::{self, AccountConnection};
use crate::remote::{self, DEFAULT_REMOTE};
use crate::site::TonkSite;
use crate::spot::{self, SpotStore};

/// One row rendered by `tonk account spots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSpotRow {
    /// Repository subject DID.
    pub subject: String,
    /// Name stored in the account backup.
    pub remote_name: Option<String>,
    /// Registry name already resolving to this subject.
    pub local_name: Option<String>,
    /// Whether conflicting legacy artifacts prevent selection.
    pub ambiguous: bool,
    /// Whether the selected artifact carries a usable sync remote.
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

/// Result of attempting to refresh one site's account backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupOutcome {
    /// No account provider is attached.
    NoProvider,
    /// The site has no actual `main` upstream yet.
    NoUpstream,
    /// The exact payload was already uploaded successfully.
    Unchanged,
    /// A new immutable payload was uploaded and marked locally.
    Uploaded {
        /// Content-addressed key returned by the account service.
        key: String,
    },
}

/// One isolated warning from a whole-registry backup sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupWarning {
    /// Registry name whose backup failed.
    pub name: String,
    /// Diagnostic suitable for stderr.
    pub message: String,
}

fn site_config(_profile: &Profile) -> crate::site::SiteConfig {
    #[cfg(feature = "integration-tests")]
    if let Some(config) = account::integration_site_config(_profile) {
        return config;
    }
    crate::site::default_config()
}

async fn open_site(path: &std::path::Path, profile: &Profile) -> Result<TonkSite> {
    let site = TonkSite::open_with(path, site_config(profile)).await?;
    if site.profile.did() != profile.did() {
        bail!("registered site profile does not match the active account profile");
    }
    Ok(site)
}

async fn response_bytes(response: reqwest::Response, path: &str) -> Result<Vec<u8>> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read account service response from {path}"))?
        .to_vec();
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&bytes);
        bail!("account service rejected {path} ({status}): {detail}");
    }
    Ok(bytes)
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
        let site = match open_site(&entry.site, profile).await {
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

/// List the account directory's spots and identify subjects already
/// registered locally. Reads the account DB — the same directory facts
/// the Hub renders — not the retired spot-backup escrow.
pub async fn list(profile: &Profile, store: &SpotStore) -> Result<Vec<AccountSpotRow>> {
    let operator = crate::account_state::credential_operator(profile).await?;
    let branch = crate::account_state::open_account_branch_in(profile, &operator, store)
        .await?
        .context("no account is configured on this profile")?;
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
    let operator = crate::account_state::credential_operator(profile).await?;
    let account = crate::account_state::open_account_branch_in(profile, &operator, store)
        .await?
        .context("no account is configured on this profile")?;
    if let Err(error) = account.pull().download().perform(&operator).await {
        eprintln!("warning: account sync failed; pulling from the local copy: {error:#}");
    }
    // Bring retained certificates into the profile's provable reach so
    // the space→root chain can be assembled below.
    crate::account_state::adopt_account_access_in(profile, &operator, store).await?;

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
    // down) — the prover walks them; no escrow artifact involved.
    let account_root: Did = account
        .subject()
        .did()
        .to_string()
        .parse()
        .context("account branch subject is not a DID")?;
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
    let site = crate::site::mount_delegated_at(&target, chain, site_config(profile))
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
    spot::register_existing_unbound(store, &name, &canonical_target)?;
    fresh_target.commit();

    let warning = match crate::sync::pull(&site).await {
        Ok(_) => None,
        Err(error) => Some(format!(
            "initial pull from '{DEFAULT_REMOTE}' failed: {error}; run `tonk pull` before making changes so you don't diverge from upstream"
        )),
    };
    Ok(PullOutcome {
        subject: requested.to_string(),
        name,
        site: canonical_target,
        already_local: false,
        warning,
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

async fn marker(site: &TonkSite, subject: &Did) -> Result<Option<Vec<u8>>> {
    match site
        .profile
        .credential()
        .site(format!("{ACCOUNT_SPOT_BACKUP_MARKER_PREFIX}{subject}"))
        .load::<Vec<u8>>()
        .perform(&site.operator)
        .await
    {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if crate::account_state::credential_is_missing(&error) => Ok(None),
        Err(error) => Err(error).context("failed to load the account backup marker"),
    }
}

/// Whether this device has already uploaded an account backup for
/// `site`'s repository.
///
/// Answered from the local marker credential alone, so it costs no
/// network round trip and works while logged out. The marker is
/// written only after the account service accepts the payload
/// ([`back_up_site_with_connection`]), which makes its presence a
/// claim that a backup exists — the claim `tonk spot rm` leans on
/// when it tells someone their data is recoverable.
pub async fn has_account_backup(site: &TonkSite) -> Result<bool> {
    Ok(marker(site, &site.repository.did()).await?.is_some())
}

async fn back_up_site_with_connection(
    registry_name: &str,
    site: &TonkSite,
    connection: &AccountConnection,
) -> Result<BackupOutcome> {
    let Some(upstream) = remote::upstream_remote(site).await? else {
        return Ok(BackupOutcome::NoUpstream);
    };
    let remote = remote::find(site, &upstream)
        .await?
        .with_context(|| format!("upstream remote '{upstream}' is not registered"))?;
    let name = repository_name(site)
        .await
        .unwrap_or_else(|| registry_name.to_string());
    let chain = crate::site::account_root_prefix(site, &connection.root_did).await?;
    let subject = chain
        .subject()
        .cloned()
        .context("account-root prefix has no repository subject")?;
    let artifact = AccountSpotBackup {
        chain_hex: hex::encode(chain.to_bytes()?),
        remote_url: Some(remote.endpoint),
        revocation_url: remote.revocation_url,
        name: Some(name),
    };
    artifact.validate_for(&connection.root_did).await?;
    let bytes = serde_json::to_vec(&artifact)?;
    let content_key = blake3::hash(&bytes).to_hex().to_string();
    if marker(site, &subject).await?.as_deref() == Some(content_key.as_bytes()) {
        return Ok(BackupOutcome::Unchanged);
    }

    let arguments = [("chain".to_string(), Promised::String(hex::encode(&bytes)))]
        .into_iter()
        .collect();
    let response = connection
        .signed_post(
            &site.profile,
            "chains/put",
            vec!["account".into(), "chain".into(), "put".into()],
            arguments,
        )
        .await?;
    let response = response_bytes(response, "chains/put").await?;
    #[derive(serde::Deserialize)]
    struct PutResponse {
        key: String,
    }
    let put: PutResponse = serde_json::from_slice(&response)
        .context("account service returned an invalid put result")?;
    site.profile
        .credential()
        .site(format!("{ACCOUNT_SPOT_BACKUP_MARKER_PREFIX}{subject}"))
        .save(content_key.into_bytes())
        .perform(&site.operator)
        .await
        .context("failed to save the account backup marker")?;
    Ok(BackupOutcome::Uploaded { key: put.key })
}

/// Refresh one registered site's account backup.
pub async fn back_up_site(registry_name: &str, site: &TonkSite) -> Result<BackupOutcome> {
    let Some(connection) = account::optional_connection(&site.profile).await? else {
        return Ok(BackupOutcome::NoProvider);
    };
    back_up_site_with_connection(registry_name, site, &connection).await
}

fn first_registry_name_for_site<'a>(
    candidates: impl IntoIterator<Item = (&'a str, &'a Path)>,
    site_root: &Path,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .find_map(|(name, path)| (path == site_root).then_some(name))
}

/// Refresh the registry entry matching an already-open site.
pub(crate) async fn back_up_current(site: &TonkSite) -> Result<BackupOutcome> {
    let store = SpotStore::open()?;
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
    back_up_site(name, site).await
}

/// Best-effort sweep of every registered spot.
pub async fn back_up_registered(profile: &Profile, store: &SpotStore) -> Vec<BackupWarning> {
    let registry = match store.load() {
        Ok(registry) => registry,
        Err(error) => {
            return vec![BackupWarning {
                name: "(registry)".to_string(),
                message: error.to_string(),
            }];
        }
    };
    let connection = match account::connection_for_store(profile, store).await {
        Ok(connection) => connection,
        Err(error) => {
            return vec![BackupWarning {
                name: "(account)".to_string(),
                message: format!("{error:#}"),
            }];
        }
    };
    let mut warnings = Vec::new();
    let mut inspected_subjects = HashSet::new();
    for (name, entry) in registry.spots {
        let site = match open_site(&entry.site, profile).await {
            Ok(site) => site,
            Err(error) => {
                warnings.push(BackupWarning {
                    name,
                    message: format!("{error:#}"),
                });
                continue;
            }
        };
        if !inspected_subjects.insert(site.repository.did().to_string()) {
            continue;
        }
        if let Err(error) = back_up_site_with_connection(&name, &site, &connection).await {
            warnings.push(BackupWarning {
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

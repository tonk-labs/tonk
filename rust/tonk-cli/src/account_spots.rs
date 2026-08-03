//! Native account spot inventory, pull, and best-effort backup reconciliation.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dialog_operator::Profile;
use dialog_query::{Output as _, Query, Term};
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Did;
use tonk_account::backup::{
    ACCOUNT_SPOT_BACKUP_MARKER_PREFIX, ACCOUNT_SPOTS_CAPABILITY_HEADER,
    ACCOUNT_SPOTS_CAPABILITY_V1, AccountSpotBackup, AccountSpotSummary,
};
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

async fn get_artifact_bytes(
    profile: &Profile,
    connection: &AccountConnection,
    key: &str,
) -> Result<Vec<u8>> {
    let arguments = [("key".to_string(), Promised::String(key.to_string()))]
        .into_iter()
        .collect();
    let response = connection
        .signed_post(
            profile,
            "chains/get",
            vec!["account".into(), "chain".into(), "get".into()],
            arguments,
        )
        .await?;
    response_bytes(response, "chains/get").await
}

async fn get_artifact(
    profile: &Profile,
    connection: &AccountConnection,
    key: &str,
) -> Result<AccountSpotBackup> {
    let bytes = get_artifact_bytes(profile, connection, key).await?;
    serde_json::from_slice(&bytes).context("account service returned an invalid spot backup")
}

fn legacy_rows(artifacts: Vec<(String, AccountSpotBackup, Did)>) -> Vec<AccountSpotSummary> {
    let mut groups: BTreeMap<String, Vec<(String, AccountSpotBackup, Did)>> = BTreeMap::new();
    for (key, backup, subject) in artifacts {
        groups
            .entry(subject.to_string())
            .or_default()
            .push((key, backup, subject));
    }
    groups
        .into_values()
        .map(|candidates| {
            let (first_key, first, subject) = &candidates[0];
            if candidates
                .iter()
                .skip(1)
                .any(|(_, candidate, _)| candidate != first)
            {
                AccountSpotSummary {
                    subject: subject.to_string(),
                    key: None,
                    name: None,
                    remote_url: None,
                    revocation_url: None,
                    ambiguous: true,
                }
            } else {
                AccountSpotSummary {
                    subject: subject.to_string(),
                    key: Some(first_key.clone()),
                    name: first.name.clone(),
                    remote_url: first.remote_url.clone(),
                    revocation_url: first.revocation_url.clone(),
                    ambiguous: false,
                }
            }
        })
        .collect()
}

async fn legacy_inventory(
    profile: &Profile,
    connection: &AccountConnection,
    keys: Vec<String>,
) -> Result<Vec<AccountSpotSummary>> {
    let mut artifacts = Vec::new();
    for key in keys {
        // Fetch/status failures mean the inventory is incomplete and must be
        // surfaced. Successfully fetched generic or malformed legacy blobs
        // are compatibility noise and remain isolated from valid spots.
        let bytes = get_artifact_bytes(profile, connection, &key).await?;
        let Ok(backup) = serde_json::from_slice::<AccountSpotBackup>(&bytes) else {
            continue;
        };
        let Ok(validated) = backup.validate_for(&connection.root_did).await else {
            continue;
        };
        artifacts.push((key, backup, validated.subject));
    }
    Ok(legacy_rows(artifacts))
}

async fn remote_inventory(
    profile: &Profile,
    connection: &AccountConnection,
) -> Result<Vec<AccountSpotSummary>> {
    let response = connection
        .signed_post(
            profile,
            "chains/list",
            vec!["account".into(), "chain".into(), "list".into()],
            BTreeMap::new(),
        )
        .await?;
    let supports_spots = response
        .headers()
        .get(ACCOUNT_SPOTS_CAPABILITY_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(ACCOUNT_SPOTS_CAPABILITY_V1);
    let bytes = response_bytes(response, "chains/list").await?;
    let keys: Vec<String> =
        serde_json::from_slice(&bytes).context("account service returned invalid chain keys")?;
    if !supports_spots {
        return legacy_inventory(profile, connection, keys).await;
    }

    let response = connection
        .signed_post(
            profile,
            "chains/spots",
            vec!["account".into(), "chain".into(), "spots".into()],
            BTreeMap::new(),
        )
        .await?;
    let bytes = response_bytes(response, "chains/spots").await?;
    let mut spots: Vec<AccountSpotSummary> =
        serde_json::from_slice(&bytes).context("account service returned invalid account spots")?;
    spots.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(spots)
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

/// List remote account spots and identify subjects already registered locally.
pub async fn list(profile: &Profile, store: &SpotStore) -> Result<Vec<AccountSpotRow>> {
    let connection = account::connection_for_store(profile, store).await?;
    let local = local_subjects(profile, store).await?;
    let mut rows: Vec<_> = remote_inventory(profile, &connection)
        .await?
        .into_iter()
        .map(|summary| AccountSpotRow {
            local_name: local.get(&summary.subject).map(|spot| spot.name.clone()),
            pullable: !summary.ambiguous && summary.key.is_some() && summary.remote_url.is_some(),
            subject: summary.subject,
            remote_name: summary.name,
            ambiguous: summary.ambiguous,
        })
        .collect();
    rows.sort_by(|left, right| left.subject.cmp(&right.subject));
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
    let connection = account::connection_for_store(profile, store).await?;
    let summaries = remote_inventory(profile, &connection).await?;
    let summary = summaries
        .into_iter()
        .find(|summary| summary.subject == requested.to_string())
        .with_context(|| format!("no account spot is backed up for {requested}"))?;
    if summary.ambiguous {
        bail!("account spot {requested} has conflicting legacy backups and cannot be pulled");
    }
    let key = summary
        .key
        .as_deref()
        .context("account spot has no selected backup artifact")?;

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
        .or_else(|| summary.name.clone())
        .ok_or_else(|| name_error(None, "the backup has no stored name"))?;
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
        return Err(name_error(
            Some(&name),
            format!("the canonical site {} already exists", target.display()),
        ));
    }

    let artifact = get_artifact(profile, &connection, key).await?;
    let validated = artifact
        .validate_for(&connection.root_did)
        .await
        .context("account spot backup is invalid")?;
    if validated.subject != requested {
        bail!("account spot backup subject does not match {requested}");
    }
    let remote_url = artifact
        .remote_url
        .as_deref()
        .context("account spot backup has no usable sync remote")?;

    let mut fresh_target = FreshPullTarget::new(target.clone());
    let site = crate::site::mount_delegated_at(&target, validated.chain, site_config(profile))
        .await
        .context("failed to mount account spot")?;
    remote::add_with_revocation(
        &site,
        DEFAULT_REMOTE,
        remote_url,
        Some(requested.clone()),
        artifact.revocation_url.as_deref(),
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

    #[test]
    fn it_marks_materially_different_legacy_backups_ambiguous() {
        let subject: Did = "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL"
            .parse()
            .unwrap();
        let first = AccountSpotBackup {
            chain_hex: "aa".to_string(),
            remote_url: Some("https://one.example/".to_string()),
            revocation_url: None,
            name: None,
        };
        let second = AccountSpotBackup {
            remote_url: Some("https://two.example/".to_string()),
            ..first.clone()
        };
        let rows = legacy_rows(vec![
            ("a".to_string(), first, subject.clone()),
            ("b".to_string(), second, subject),
        ]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].ambiguous);
        assert!(rows[0].key.is_none());
    }
}

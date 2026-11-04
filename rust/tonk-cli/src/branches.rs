//! Select the replica's active branch through Dialog's branch registry.

use anyhow::{Result, bail};
use dialog_effects::MethodExt as _;
use dialog_effects::branch::prelude::*;
use dialog_repository::schema::{Branch as BranchConcept, Replica};
use dialog_repository::{REGISTRY, RepositoryMemoryExt as _, Revision, registry};
use serde::Serialize;

use crate::site::{BRANCH_NAME, TonkSite};

/// A locally available branch.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchRow {
    /// Local branch name.
    pub name: String,
    /// Whether CLI commands currently use this branch.
    pub active: bool,
    /// Its exact head, if it has one.
    pub revision: Option<String>,
}

/// Resolve the active reference. Older sites default to `main`.
pub async fn active_name(site: &TonkSite) -> Result<String> {
    let registry = site
        .repository
        .subject()
        .registry()
        .open()
        .perform(&site.operator)
        .await?;
    let operator = site.operator.inner().build_authority(site.repository.did());
    let Some(active) = registry::active(&registry, &operator, &site.operator).await? else {
        return Ok(BRANCH_NAME.to_owned());
    };
    let replica = Replica::new(site.operator.profile_did(), site.repository.did());
    if active == BranchConcept::new(&replica, BRANCH_NAME).this {
        return Ok(BRANCH_NAME.to_owned());
    }
    for branch in registry::list(&registry, &operator, &site.operator).await? {
        if branch.this == active && branch.name.0 != REGISTRY {
            return Ok(branch.name.0);
        }
    }
    bail!("the selected branch is not available on this device; use `tonk switch main`")
}

/// List available branches, including the legacy default before it is registered.
pub async fn list(site: &TonkSite) -> Result<Vec<BranchRow>> {
    let active = active_name(site).await?;
    let registry = site
        .repository
        .subject()
        .registry()
        .open()
        .perform(&site.operator)
        .await?;
    let operator = site.operator.inner().build_authority(site.repository.did());
    let mut names: std::collections::BTreeSet<String> =
        registry::list(&registry, &operator, &site.operator)
            .await?
            .into_iter()
            .map(|branch| branch.name.0)
            .filter(|name| name != REGISTRY)
            .collect();
    names.insert(BRANCH_NAME.to_owned());
    let mut rows = Vec::new();
    for name in names {
        let branch = site
            .repository
            .branch(&name)
            .load()
            .perform(&site.operator)
            .await?;
        rows.push(BranchRow {
            active: name == active,
            name,
            revision: branch
                .revision()
                .map(|revision| revision.version().to_string()),
        });
    }
    Ok(rows)
}

/// Register a branch without modifying its existing head.
pub(crate) async fn register(site: &TonkSite, name: &str) -> Result<()> {
    site.repository
        .subject()
        .writer()
        .branches()
        .branch(name)
        .create()
        .perform(&site.operator)
        .await?;
    Ok(())
}

/// Start a fresh branch at the exact signed revision, without replaying claims.
pub(crate) async fn create(site: &TonkSite, name: &str, revision: Revision) -> Result<()> {
    site.repository
        .subject()
        .writer()
        .branches()
        .branch(name)
        .create()
        .revision(revision)
        .perform(&site.operator)
        .await?;
    Ok(())
}

/// Select an existing local branch. No remote is changed or inherited.
pub async fn switch(site: &TonkSite, name: &str) -> Result<()> {
    if name == REGISTRY || dialog_effects::branch::invalid_name(name).is_some() {
        bail!("invalid application branch name '{name}'");
    }
    site.repository
        .branch(name)
        .load()
        .perform(&site.operator)
        .await?;
    register(site, name).await?;
    let replica = Replica::new(site.operator.profile_did(), site.repository.did());
    let branch = BranchConcept::new(&replica, name);
    site.repository
        .subject()
        .writer()
        .branches()
        .switch(branch.this)
        .perform(&site.operator)
        .await?;
    Ok(())
}

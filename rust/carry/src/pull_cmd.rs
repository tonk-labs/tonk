//! `carry pull` — fetch the configured remote and three-way merge into
//! the local repository.

use crate::site::Site;
use anyhow::{Context, Result, bail};

/// Execute `carry pull`.
pub async fn execute(site: &Site) -> Result<()> {
    if site.branch.upstream().is_none() {
        bail!(
            "this repository has no remote configured; run `carry remote add <NAME> <URL> \
             --subject <DID>` first"
        );
    }

    let result = site
        .branch
        .pull()
        .perform(&site.operator)
        .await
        .context("pull failed")?;

    match result {
        Some(rev) => {
            eprintln!("Pulled. Local is now at {}.", rev.tree);
        }
        None => {
            eprintln!("Already up to date.");
        }
    }
    Ok(())
}

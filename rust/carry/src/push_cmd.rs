//! `carry push` — fast-forward the configured remote with local changes.
//!
//! Carry v1 operates at the repository level and hides branches; this
//! command pushes the single hidden branch to whatever upstream was
//! wired by `carry remote add`.

use crate::site::Site;
use anyhow::{Context, Result, bail};

/// Execute `carry push`.
pub async fn execute(site: &Site) -> Result<()> {
    if site.branch.upstream().is_none() {
        bail!(
            "this repository has no remote configured; run `carry remote add <NAME> <URL> \
             --subject <DID>` first"
        );
    }

    let result = site
        .branch
        .push()
        .perform(&site.operator)
        .await
        .context("push failed")?;

    match result {
        Some(rev) => {
            eprintln!("Pushed. Remote is now at {}.", rev.tree);
            Ok(())
        }
        None => {
            bail!(
                "push could not fast-forward: the remote has diverged from your local \
                 history. Run `carry pull` to merge remote changes, then try again."
            );
        }
    }
}

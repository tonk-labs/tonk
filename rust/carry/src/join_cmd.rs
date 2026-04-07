//! `carry join <TOKEN>` — redeem an invite token to join a repository.
//!
//! Decodes the token, saves the delegation chain under the local profile,
//! and — if the token includes an access service URL — sets up the sync
//! remote and pulls the latest data.
//!
//! Because the invite was created for this profile's DID, the delegation
//! chain's audience is already our DID. No re-delegation is needed.

use crate::invite_cmd;
use crate::remote_cmd::HIDDEN_BRANCH;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;
use std::path::Path;

/// Execute `carry join <token> [--repo <REPO>]`.
pub async fn execute(
    token: &str,
    site_flag: Option<&Path>,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    let decoded = invite_cmd::decode_token(token)?;

    // Resolve or create the .carry/ site
    let site = match Site::resolve(site_flag, profile_location.clone()).await {
        Ok(site) => site,
        Err(_) => {
            let parent = if let Some(p) = site_flag {
                if p.ends_with(".carry") {
                    p.parent()
                        .context("--repo .carry path has no parent")?
                        .to_path_buf()
                } else {
                    p.to_path_buf()
                }
            } else {
                std::env::current_dir().context("Failed to determine current directory")?
            };
            Site::init(&parent, profile_location, None).await?
        }
    };

    // Save the delegation chain. The audience is already our profile DID
    // (Alice created the invite specifically for us), so the operator can
    // use it immediately.
    site.profile
        .save(decoded.chain)
        .perform(&site.operator)
        .await
        .context("Failed to save delegation chain")?;

    eprintln!("Joined repository as {}", site.did());

    // If the token includes an access URL, wire up sync and pull.
    if let Some(url) = decoded.url {
        eprintln!("Configuring sync remote...");

        let remote = site
            .repo
            .remote("origin")
            .create(SiteAddress::Ucan(UcanAddress::new(&url)))
            .subject(decoded.subject)
            .perform(&site.operator)
            .await
            .context("Failed to register remote")?;

        let remote_branch = remote
            .branch(HIDDEN_BRANCH)
            .open()
            .perform(&site.operator)
            .await
            .context("Failed to open remote branch")?;

        site.branch
            .set_upstream(remote_branch)
            .perform(&site.operator)
            .await
            .context("Failed to set upstream")?;

        match site.branch.pull().perform(&site.operator).await {
            Ok(Some(rev)) => eprintln!("Pulled. Local is now at {}.", rev.tree()),
            Ok(None) => eprintln!("Remote is empty; nothing to pull yet."),
            Err(e) => eprintln!("warning: pull failed ({}); run `carry pull` to retry.", e),
        }
    }

    Ok(())
}

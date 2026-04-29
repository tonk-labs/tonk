//! `carry join [<INVITE-URL>]` -- join a space using an invite URL.
//!
//! When an invite URL is provided:
//!
//! - If the URL has a `#` fragment (open invite), the fragment contains the
//!   ephemeral private key. The joiner redelegates from the ephemeral key to
//!   their own profile DID, extending the delegation chain.
//!
//! - If the URL has no fragment (scoped invite), the delegation was issued
//!   directly to this profile's DID. The chain is used as-is after verifying
//!   the audience matches.
//!
//! When no URL is provided, self-provisions an upstream for the space.

use crate::remote_cmd::HIDDEN_BRANCH;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;
use std::path::Path;
use tonk_invite::Invite as TonkInvite;

/// Execute `carry join [<invite-url>] [--repo <REPO>]`.
pub async fn execute(
    invite_url: Option<&str>,
    site_flag: Option<&Path>,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    let invite_url = match invite_url {
        Some(url) => url,
        None => {
            anyhow::bail!("Self-provisioning is not yet implemented. Provide an invite URL.");
        }
    };

    let parsed = TonkInvite::parse_url(invite_url)
        .await
        .context("Failed to parse invite URL")?;

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

    // For scoped invites, verify the audience matches before claiming.
    if matches!(parsed.audience, tonk_invite::InviteAudience::Scoped) {
        let audience = parsed.chain.audience();
        let our_did = site.profile.did();
        if *audience != our_did {
            anyhow::bail!(
                "Cannot join: this invite was issued to {} but this repository is {}",
                audience,
                our_did
            );
        }
    }

    let remote_url = parsed.remote_url.clone();
    let subject = parsed.subject().clone();
    let claimed = parsed
        .claim(&site.profile.did())
        .await
        .context("Failed to claim invite")?;
    let chain = claimed.chain;

    site.profile
        .save(dialog_ucan::UcanDelegation(chain))
        .perform(&site.operator)
        .await
        .context("Failed to save delegation chain")?;

    eprintln!("Joined repository as {}", site.did());

    // Configure sync remote from the remote URL and pull.
    if let Some(ref remote_url) = remote_url {
        eprintln!("Configuring sync remote...");

        let remote = site
            .repo
            .remote("origin")
            .create(SiteAddress::Ucan(UcanAddress::new(remote_url.as_str())))
            .subject(subject)
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
            Ok(Some(rev)) => eprintln!("Pulled. Local is now at {}.", rev.tree),
            Ok(None) => eprintln!("Remote is empty; nothing to pull yet."),
            Err(e) => eprintln!("warning: pull failed ({}); run `carry pull` to retry.", e),
        }
    }

    Ok(())
}

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

use crate::site::Site;
use anyhow::{Context, Result};
use std::path::Path;
use tonk_invite::Invite as TonkInvite;

/// Execute `carry join <invite-url> [--repo <REPO>]`.
pub async fn execute(
    invite_url: Option<&str>,
    site_flag: Option<&Path>,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    let invite_url = invite_url.context("Provide an invite URL.")?;

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

    let claimed = parsed
        .claim(&site.profile.did())
        .await
        .context("Failed to claim invite")?;

    site.profile
        .save(dialog_ucan::UcanDelegation(claimed.chain))
        .perform(&site.operator)
        .await
        .context("Failed to save delegation chain")?;

    eprintln!("Joined repository as {}", site.did());

    Ok(())
}

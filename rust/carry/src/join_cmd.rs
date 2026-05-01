//! `carry join [<INVITE-URL>]` -- redeem an invite into a local
//! replica of the invited space.
//!
//! Carry's join model mirrors tonk-worker's: redeeming an invite
//! creates a `.carry/` whose local repo DID equals the invited
//! subject's DID (via a verifier-only credential), so every
//! recipient and device that joins the same invite converges on
//! the same `Replica.this = hash(profile, subject)` and the same
//! sigil glyph.
//!
//! Two outcomes:
//!
//! - **Fresh** — no `.carry/` was discoverable from the working
//!   directory (or the user passed `--repo` to a path without one).
//!   A new `.carry/` is created keyed to the invited subject; an
//!   `origin` remote is configured to track the invite's access
//!   service; main branch is wired to track `origin/main`.
//!
//! - **Renewing** — an existing `.carry/` was found whose subject
//!   DID matches the invite's subject. The delegation chain is
//!   saved (so the recipient picks up any new access this invite
//!   carries — e.g. an extended delegation) and the working tree
//!   is pulled. The remote/upstream wiring already exists.
//!
//! Joining a different space's `.carry/` is rejected — the user
//! is told to leave the directory or pass `--repo` to a fresh
//! location. Silently mutating an unrelated `.carry/` would
//! confuse the storage and the meta-branch shape.

use crate::site::Site;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tonk_invite::Invite as TonkInvite;

#[derive(Clone, Copy)]
enum Outcome {
    Fresh,
    Renewing,
}

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

    let invited_subject = parsed.subject().clone();
    let remote_url = parsed.remote_url.clone();

    // Resolve an existing `.carry/` if one is reachable; otherwise
    // create a fresh one keyed to the invited subject. Joining a
    // `.carry/` that already mirrors a *different* subject is
    // rejected — the storage and meta-branch facts are tied to a
    // single subject, and silently overwriting them would confuse
    // every other tool reading the same directory.
    let (site, outcome) = match Site::resolve(site_flag, profile_location.clone()).await {
        Ok(site) => {
            if site.repo.did() == invited_subject {
                (site, Outcome::Renewing)
            } else {
                bail!(
                    "This .carry/ is for a different space ({}); the invite is for {}. \
                     Run from outside it, or pass --repo <PATH> to use a fresh location.",
                    site.repo.did(),
                    invited_subject,
                );
            }
        }
        Err(_) => {
            let parent = parent_for_new_site(site_flag)?;
            let site =
                Site::init_from_invite(&parent, invited_subject.clone(), profile_location, None)
                    .await?;
            (site, Outcome::Fresh)
        }
    };

    // For scoped invites, verify the audience matches before claiming.
    if matches!(parsed.audience, tonk_invite::InviteAudience::Scoped) {
        let audience = parsed.chain.audience();
        let our_did = site.profile.did();
        if *audience != our_did {
            bail!(
                "Cannot join: this invite was issued to {} but this profile is {}",
                audience,
                our_did
            );
        }
    }

    // Always claim and save the delegation chain. Idempotent at
    // the dialog layer — re-saving the same chain is a no-op,
    // re-saving an extended one adds a fresh proof.
    let claimed = parsed
        .claim(&site.profile.did())
        .await
        .context("Failed to claim invite")?;
    site.profile
        .save(dialog_ucan::UcanDelegation(claimed.chain))
        .perform(&site.operator)
        .await
        .context("Failed to save delegation chain")?;

    match outcome {
        Outcome::Fresh => eprintln!("Joined repository as {}", site.did()),
        Outcome::Renewing => eprintln!("Renewed access to {}", site.did()),
    }

    // Configure the sync remote and pull. On a fresh join, route
    // through `remote_cmd::execute` so the dialog-side remote
    // creation and the meta-branch `Remote` + `TrackingBranch`
    // facts land together. On renewal the remote is already wired;
    // skip the add and go straight to the pull.
    if let Some(url) = remote_url {
        if matches!(outcome, Outcome::Fresh) {
            eprintln!("Configuring sync remote...");
            crate::remote_cmd::execute(
                &site,
                crate::remote_cmd::RemoteAddOptions {
                    name: "origin".to_string(),
                    url: url.to_string(),
                    subject: Some(invited_subject.to_string()),
                    s3_endpoint: None,
                    s3_region: None,
                    s3_bucket: None,
                    s3_access_key: None,
                    s3_secret_key: None,
                    set_upstream: true,
                },
            )
            .await?;
        }

        match site.branch.pull().perform(&site.operator).await {
            Ok(Some(rev)) => eprintln!("Pulled. Local is now at {}.", rev.tree),
            Ok(None) => eprintln!("Remote is empty; nothing to pull yet."),
            Err(e) => eprintln!("warning: pull failed ({}); run `carry pull` to retry.", e),
        }
    }

    Ok(())
}

/// Resolve the parent directory for a fresh `.carry/`.
///
/// `--repo .carry` and `--repo /path/to/.carry` both resolve to
/// the parent of `.carry/`; `--repo /path` and no flag use the
/// path / cwd as the parent directly.
fn parent_for_new_site(site_flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = site_flag {
        if p.ends_with(".carry") {
            Ok(p.parent()
                .context("--repo .carry path has no parent")?
                .to_path_buf())
        } else {
            Ok(p.to_path_buf())
        }
    } else {
        std::env::current_dir().context("Failed to determine current directory")
    }
}

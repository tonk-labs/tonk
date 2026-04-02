//! `carry join <TOKEN>` — redeem an invite token to join a repository.
//!
//! Decodes the invite token, imports the membership credential, saves
//! the delegation chain under the local profile, and establishes a
//! local access chain from the membership credential through to the
//! operator.

use crate::invite_cmd;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::credential::Credential;
use dialog_credentials::credential::export::CredentialExport;
use dialog_varsig::Principal;
use std::path::Path;

/// Execute `carry join <token> [--repo <REPO>]`.
pub async fn execute(token: &str, site_flag: Option<&Path>) -> Result<()> {
    // Decode the invite token
    let (cred_bytes, chain) = invite_cmd::decode_token(token)?;

    // Import the membership credential
    let cred_export =
        CredentialExport::try_from(cred_bytes).context("Invalid credential in invite token")?;
    let credential = Credential::import(cred_export)
        .await
        .context("Failed to import membership credential")?;

    let membership_did = credential.did();
    eprintln!("Membership credential: {}", membership_did);

    // Resolve or create the .carry/ site
    let site = match Site::resolve(site_flag).await {
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
            Site::init(&parent).await?
        }
    };

    // Save the delegation chain under the local profile.
    // This gives the profile proof of access to the repo subject.
    site.profile
        .save(chain)
        .perform(&site.operator)
        .await
        .context("Failed to save delegation chain")?;

    eprintln!("Joined repository as {}", site.profile.did());

    Ok(())
}

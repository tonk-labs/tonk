//! `carry status` — show current repository information.

use crate::site::Site;
use anyhow::Result;
use std::path::Path;

/// Execute `carry status [--repo <REPO>]`.
pub async fn execute(
    site_flag: Option<&Path>,
    format: &str,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    let site = Site::resolve(site_flag, profile_location).await?;

    match format {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "repo": site.root().display().to_string(),
                    "profile": site.did(),
                    "repository": site.repo_did(),
                }))?
            );
        }
        _ => {
            println!("Repo: {}", site.root().display());
            println!("Profile: {}", site.did());
            println!("Repository: {}", site.repo_did());
        }
    }

    Ok(())
}

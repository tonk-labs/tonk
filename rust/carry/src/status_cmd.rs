//! `carry status` — show current repository information.

use crate::site::Site;
use anyhow::Result;
use std::path::Path;

/// Execute `carry status [--repo <REPO>]`.
pub async fn execute(site_flag: Option<&Path>, format: &str) -> Result<()> {
    let site = Site::resolve(site_flag)?;
    let active_did = site.active_space_did()?;

    match format {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "repo": site.root().display().to_string(),
                    "did": active_did,
                }))?
            );
        }
        _ => {
            println!("Repo: {}", site.root().display());
            if let Some(did) = active_did {
                println!("DID: {}", did);
            } else {
                println!("No repository. Run `carry init` to create one.");
            }
        }
    }

    Ok(())
}

//! `carry status` — show current site and space information.

use crate::site::Site;
use anyhow::Result;
use std::path::Path;

/// Execute `carry status [--site <SITE>]`.
pub async fn execute(site_flag: Option<&Path>, format: &str) -> Result<()> {
    let site = Site::resolve(site_flag)?;
    let spaces = site.list_spaces()?;
    let active_did = site.active_space_did()?;

    match format {
        "json" => {
            let space_list: Vec<serde_json::Value> = spaces
                .iter()
                .map(|s| {
                    let is_active = active_did.as_deref() == Some(&s.did);
                    serde_json::json!({
                        "did": s.did,
                        "active": is_active,
                        "path": s.dir().display().to_string(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "site": site.root().display().to_string(),
                    "spaces": space_list,
                }))?
            );
        }
        _ => {
            println!("Site: {}", site.root().display());
            if spaces.is_empty() {
                println!("No spaces. Run `carry init` to create one.");
            } else {
                println!("Spaces:");
                for s in &spaces {
                    let marker = if active_did.as_deref() == Some(&s.did) {
                        " (active)"
                    } else {
                        ""
                    };
                    println!("  {}{}", s.did, marker);
                }
            }
        }
    }

    Ok(())
}

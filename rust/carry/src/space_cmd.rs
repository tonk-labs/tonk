//! `carry space` — manage spaces within a `.carry/` repository.
//!
//! Spaces are isolated namespaces within a single carry repo, each with
//! its own Ed25519 identity and data store. Use spaces to keep workstreams
//! separate within the same project.

use crate::schema;
use crate::site::Site;
use anyhow::Result;
use dialog_query::claim::{Attribute as ClaimAttribute, Claim, Relation};
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

/// Execute `carry space list`.
pub async fn list(site: &Site, format: &str) -> Result<()> {
    let spaces = site.list_spaces()?;
    let active_did = site.active_space_did()?;

    if spaces.is_empty() {
        println!("No spaces. Run `carry space create` or `carry init` to create one.");
        return Ok(());
    }

    // Collect labels for all spaces
    let mut entries = Vec::new();
    for space in &spaces {
        let label = site.space_label(space).await.unwrap_or(None);
        let is_active = active_did.as_deref() == Some(&space.did);
        entries.push((space, label, is_active));
    }

    match format {
        "json" => {
            let space_list: Vec<serde_json::Value> = entries
                .iter()
                .map(|(space, label, is_active)| {
                    let mut obj = serde_json::json!({
                        "did": space.did,
                        "active": is_active,
                    });
                    if let Some(label) = label {
                        obj["label"] = serde_json::Value::String(label.clone());
                    }
                    obj
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&space_list)?);
        }
        _ => {
            for (space, label, is_active) in &entries {
                let active_marker = if *is_active { "* " } else { "  " };
                let label_display = label
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default();
                println!("{}{}{}", active_marker, space.did, label_display);
            }
        }
    }

    Ok(())
}

/// Execute `carry space create [LABEL]`.
pub async fn create(site: &Site, label: Option<String>, format: &str) -> Result<()> {
    let space = site.create_space()?;
    site.set_active_space(&space.did)?;

    // If a label is provided, assert it as a claim
    if let Some(ref label_str) = label {
        let mut session = space.open_session().await?;
        let entity = schema::derive_entity("space")?;
        let attr = ClaimAttribute::from_str("xyz.tonk.carry/label")
            .map_err(|e| anyhow::anyhow!("Invalid attribute: {:?}", e))?;
        let value = dialog_query::Value::String(label_str.clone());
        let relation = Relation::new(attr, entity, value);
        let mut transaction = session.edit();
        relation.assert(&mut transaction);
        session.commit(transaction).await?;
    }

    match format {
        "json" => {
            let mut obj = serde_json::json!({
                "did": space.did,
                "active": true,
            });
            if let Some(ref label_str) = label {
                obj["label"] = serde_json::Value::String(label_str.clone());
            }
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        _ => {
            let label_display = label
                .as_ref()
                .map(|l| format!(" ({})", l))
                .unwrap_or_default();
            println!("Created space {}{}", space.did, label_display);
            println!("Switched to new space (now active)");
        }
    }

    Ok(())
}

/// Execute `carry space switch <DID|LABEL>`.
pub async fn switch(site: &Site, target: &str) -> Result<()> {
    let space = site.resolve_space(target).await?;

    // Check if already active
    if let Some(current) = site.active_space_did()?
        && current == space.did
    {
        let label = site.space_label(&space).await.unwrap_or(None);
        let label_display = label
            .as_ref()
            .map(|l| format!(" ({})", l))
            .unwrap_or_default();
        println!("Already on space {}{}", space.did, label_display);
        return Ok(());
    }

    site.set_active_space(&space.did)?;
    let label = site.space_label(&space).await.unwrap_or(None);
    let label_display = label
        .as_ref()
        .map(|l| format!(" ({})", l))
        .unwrap_or_default();
    println!("Switched to space {}{}", space.did, label_display);

    Ok(())
}

/// Execute `carry space active`.
pub async fn active(site: &Site, format: &str) -> Result<()> {
    let active_did = site.active_space_did()?;

    match active_did {
        None => {
            println!("No active space. Run `carry space create` or `carry init` to create one.");
        }
        Some(ref did) => {
            let space = site
                .space_by_did(did)
                .with_context(|| format!("Active space {} not found on disk", did))?;
            let label = site.space_label(&space).await.unwrap_or(None);

            match format {
                "json" => {
                    let mut obj = serde_json::json!({
                        "did": space.did,
                    });
                    if let Some(ref label_str) = label {
                        obj["label"] = serde_json::Value::String(label_str.clone());
                    }
                    println!("{}", serde_json::to_string_pretty(&obj)?);
                }
                _ => {
                    let label_display = label
                        .as_ref()
                        .map(|l| format!(" ({})", l))
                        .unwrap_or_default();
                    println!("{}{}", space.did, label_display);
                }
            }
        }
    }

    Ok(())
}

/// Execute `carry space delete <DID|LABEL> [--yes]`.
pub async fn delete(site: &Site, target: &str, skip_confirm: bool) -> Result<()> {
    let space = site.resolve_space(target).await?;

    // Prevent deleting the active space
    if let Some(active_did) = site.active_space_did()?
        && active_did == space.did
    {
        anyhow::bail!(
            "Cannot delete the active space. Switch to another space first:\n  carry space switch <DID|LABEL>"
        );
    }

    let label = site.space_label(&space).await.unwrap_or(None);
    let label_display = label
        .as_ref()
        .map(|l| format!(" ({})", l))
        .unwrap_or_default();

    if !skip_confirm {
        print!(
            "Delete space {}{}? This cannot be undone. [y/N] ",
            space.did, label_display
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    site.delete_space(&space)?;
    println!("Deleted space {}{}", space.did, label_display);

    Ok(())
}

use anyhow::Context as _;

/// Resolve a `Site` from the optional `--site` flag, for space subcommands
/// that only need site-level access (not a full `SiteContext`).
pub fn resolve_site(site_flag: Option<&Path>) -> Result<Site> {
    Site::resolve(site_flag)
}

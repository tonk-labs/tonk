//! `carry init` — create a new `.carry/` repository.
//!
//! Creates a `.carry/` directory and a first space. Optionally asserts a
//! label claim on the space. If a repository already exists, reports its
//! status without creating additional spaces — use `carry space create`
//! for that.

use crate::schema;
use crate::site::Site;
use anyhow::Result;
use dialog_query::claim::{Claim, Relation};
use std::path::Path;
use std::str::FromStr;

/// Execute `carry init [<name>] [--site <SITE>]`.
pub async fn execute(name: Option<String>, site_path: Option<&Path>) -> Result<()> {
    let parent = if let Some(p) = site_path {
        p.to_path_buf()
    } else {
        std::env::current_dir()?
    };

    // If a .carry/ directory already exists, report status and return
    if parent.join(".carry").is_dir() {
        let site = Site::open(&parent)?;
        let spaces = site.list_spaces()?;
        let active_did = site.active_space_did()?;

        println!("Repository already exists at {}", site.root().display());
        println!(
            "{} space{}",
            spaces.len(),
            if spaces.len() == 1 { "" } else { "s" }
        );
        if let Some(ref did) = active_did {
            if let Some(space) = site.space_by_did(did) {
                let label = site.space_label(&space).await.unwrap_or(None);
                let label_display = label
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default();
                println!("Active: {}{}", did, label_display);
            } else {
                println!("Active: {}", did);
            }
        }
        println!("\nUse `carry space create` to add more spaces.");
        return Ok(());
    }

    // Create the .carry/ directory
    let site = Site::init(&parent)?;

    // Create the first space
    let space = site.create_space()?;
    site.set_active_space(&space.did)?;

    // If a name is provided, assert it as a label claim
    if let Some(ref label) = name {
        let mut session = space.open_session().await?;
        let entity = schema::derive_entity("space")?;
        let attr = dialog_query::claim::Attribute::from_str("xyz.tonk.carry/label")
            .map_err(|e| anyhow::anyhow!("Invalid attribute: {:?}", e))?;
        let value = dialog_query::Value::String(label.clone());
        let relation = Relation::new(attr, entity, value);
        let mut transaction = session.edit();
        relation.assert(&mut transaction);
        session.commit(transaction).await?;
    }

    // Print result
    let dir_display = space.dir().display();
    if let Some(label) = name {
        println!("Seeded {} repository in {}", label, dir_display);
    } else {
        println!("Seeded repository in {}", dir_display);
    }

    Ok(())
}

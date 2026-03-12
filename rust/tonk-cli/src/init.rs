//! `carry init` — create a new `.carry/` repository.
//!
//! Creates a `.carry/` directory, generates an Ed25519 keypair, and
//! initializes a space directory. Optionally asserts a label claim.
//!
//! # TODO: Behavior when site already has spaces
//!
//! Currently `carry init` reuses the first/active space if one exists.
//! With multispace support, consider:
//! - `carry init` with existing spaces could warn or prompt
//! - `carry space create` becomes the explicit way to add spaces
//! - `carry init` might only create the site, not a space, if spaces exist

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

    // Create or open the .carry/ directory
    let site = if parent.join(".carry").is_dir() {
        Site::open(&parent)?
    } else {
        Site::init(&parent)?
    };

    // Check if there's already a space
    let existing_spaces = site.list_spaces()?;

    let space = if existing_spaces.is_empty() {
        // Create a new space
        let space = site.create_space()?;
        site.set_active_space(&space.did)?;
        space
    } else {
        // Use existing active space or first space
        if let Ok(active) = site.active_space() {
            active
        } else {
            let space = &existing_spaces[0];
            site.set_active_space(&space.did)?;
            space.clone()
        }
    };

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

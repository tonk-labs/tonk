//! `carry init` — create a new `.carry/` repository.
//!
//! Creates a `.carry/` directory and bootstraps pre-registered concepts
//! (attribute, concept, bookmark) so they can be queried and used
//! immediately.

use crate::schema;
use crate::site::Site;
use anyhow::Result;
use std::path::Path;

/// Execute `carry init [<name>] [--repo <REPO>]`.
///
/// `profile_location`: `None` for production (platform data dir),
/// `Some(loc)` for test isolation.
pub async fn execute(
    name: Option<String>,
    _admin_dids: Vec<String>,
    site_path: Option<&Path>,
    profile_location: Option<crate::identity_cmd::ProfileLocation>,
) -> Result<()> {
    let parent = if let Some(p) = site_path {
        p.to_path_buf()
    } else {
        std::env::current_dir()?
    };

    // If a .carry/ directory already exists, report status and return
    if parent.join(".carry").is_dir() {
        let site = Site::open(&parent, profile_location).await?;
        println!("Repository already exists at {}", site.root().display());
        println!("DID: {}", site.did());
        return Ok(());
    }

    // Create site (initializes .carry/ directory and identity)
    let site = Site::init(&parent, profile_location).await?;

    // Open a session for bootstrapping
    let mut session = site.open_session().await?;

    // Bootstrap pre-registered concepts (attribute, concept, bookmark)
    schema::bootstrap_builtins(&mut session).await?;

    // If a name is provided, assert it as a label claim
    if let Some(ref label) = name {
        let entity = schema::derive_entity("space")?;
        let name_attr = schema::dialog_meta::Name::the();
        let value = dialog_query::Value::String(label.clone());
        let mut transaction = session.edit();
        transaction.associate(name_attr, entity, value);
        session.commit(transaction).await?;
    }

    // Print result
    let dir_display = site.root().display();
    if let Some(label) = name {
        println!("Initialized {} repository in {}", label, dir_display);
    } else {
        println!("Initialized repository in {}", dir_display);
    }
    eprintln!("Identity: {}", site.did());

    Ok(())
}

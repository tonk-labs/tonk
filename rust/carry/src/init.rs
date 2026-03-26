//! `carry init` — create a new `.carry/` repository.
//!
//! Creates a `.carry/` directory and a first space with delegation-based
//! authority. The space key is ephemeral: it powerline-delegates to admin
//! identities, then is discarded. All subsequent operations use the user's
//! passkey-derived identity from `~/.carry/identity`.
//!
//! Bootstraps pre-registered concepts (attribute, concept, bookmark) so
//! they can be queried and used immediately.

use crate::identity_cmd;
use crate::schema;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_query::Attribute;
use dialog_query::claim::{Claim, Relation};
use std::path::Path;
use tonk_space::Did;

/// Execute `carry init [<name>] [--admin <DID>...] [--repo <REPO>]`.
pub async fn execute(
    name: Option<String>,
    admin_dids: Vec<String>,
    site_path: Option<&Path>,
) -> Result<()> {
    let parent = if let Some(p) = site_path {
        p.to_path_buf()
    } else {
        std::env::current_dir()?
    };

    // If a .carry/ directory already exists, report status and return
    if parent.join(".carry").is_dir() {
        let site = Site::open(&parent)?;
        let active_did = site.active_space_did()?;

        println!("Repository already exists at {}", site.root().display());
        if let Some(ref did) = active_did {
            if let Some(space) = site.space_by_did(did) {
                let label = site.space_label(&space).await.unwrap_or(None);
                let label_display = label
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default();
                println!("DID: {}{}", did, label_display);
            } else {
                println!("DID: {}", did);
            }
        }
        return Ok(());
    }

    // Ensure we have a local identity (auto-trigger passkey flow if needed)
    let identity = identity_cmd::ensure_identity().await?;

    // Build list of admin DIDs (always includes the local identity)
    let mut all_admins: Vec<Did> = vec![identity.did()];
    for did_str in &admin_dids {
        let did: Did = did_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid admin DID '{}': {:?}", did_str, e))?;
        // Deduplicate
        if !all_admins.iter().any(|d| d.to_string() == did.to_string()) {
            all_admins.push(did);
        }
    }

    // Create the .carry/ directory
    let site = Site::init(&parent)?;

    // Create the first space with delegation to admin(s)
    let (space, _proofs) = site
        .create_delegated_space(&all_admins)
        .await
        .context("Failed to create delegated space")?;
    site.set_active_space(&space.did)?;

    // Open a session for bootstrapping
    let mut session = space.open_session().await?;

    // Bootstrap pre-registered concepts (attribute, concept, bookmark)
    schema::bootstrap_builtins(&mut session).await?;

    // If a name is provided, assert it as a label claim
    if let Some(ref label) = name {
        let entity = schema::derive_entity("space")?;
        let name_attr = schema::dialog_meta::Name::selector();
        let value = dialog_query::Value::String(label.clone());
        let relation = Relation::new(name_attr, entity, value);
        let mut transaction = session.edit();
        relation.assert(&mut transaction);
        session.commit(transaction).await?;
    }

    // Print result
    let dir_display = space.dir().display();
    if let Some(label) = name {
        println!("Initialized {} repository in {}", label, dir_display);
    } else {
        println!("Initialized repository in {}", dir_display);
    }
    eprintln!("Identity: {}", identity.did());
    if all_admins.len() > 1 {
        eprintln!("Admins: {} total", all_admins.len());
    }

    Ok(())
}

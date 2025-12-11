use anyhow::{bail, Context, Result};
use dialoguer::{Input, Password};
use std::path::PathBuf;
use tonk_space::{AuthMethod, Issuer, RemoteState, RestStorageConfig, Revision, S3Authority, Space};

use crate::authority;
use crate::keystore::Keystore;
use crate::state;

/// Get the storage path for the active space's facts database
fn get_active_space_info() -> Result<(String, PathBuf)> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;

    let space_did = state::get_active_space(&authority.did)?
        .context("No active space. Please run 'tonk space create' or 'tonk space set' first")?;

    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let path = home
        .join(".tonk")
        .join("operator")
        .join(&operator_did)
        .join("session")
        .join(&authority.did)
        .join("space")
        .join(&space_did)
        .join("facts");

    Ok((space_did, path))
}

/// Add a remote to the active space
pub async fn add(
    name: String,
    endpoint: Option<String>,
    bucket: Option<String>,
    region: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
) -> Result<()> {
    // Get active space info
    let (space_did, storage_path) = get_active_space_info()?;

    // Prompt for missing values
    let endpoint = match endpoint {
        Some(e) => e,
        None => Input::new()
            .with_prompt("Endpoint (e.g., https://s3.amazonaws.com)")
            .interact_text()?,
    };

    let bucket = match bucket {
        Some(b) => b,
        None => Input::new()
            .with_prompt("Bucket name")
            .interact_text()?,
    };

    let region = match region {
        Some(r) => r,
        None => Input::new()
            .with_prompt("Region")
            .default("us-east-1".to_string())
            .interact_text()?,
    };

    let access_key_id = match access_key_id {
        Some(k) => k,
        None => Input::new()
            .with_prompt("Access Key ID")
            .interact_text()?,
    };

    let secret_access_key = match secret_access_key {
        Some(s) => s,
        None => Password::new()
            .with_prompt("Secret Access Key")
            .interact()?,
    };

    // Use space DID as key prefix
    let key_prefix = Some(space_did.clone());

    // Create remote state
    let remote_state = RemoteState {
        site: name.clone().into(),
        address: RestStorageConfig {
            endpoint,
            auth_method: AuthMethod::S3(S3Authority {
                access_key_id,
                secret_access_key,
                session_token: None,
                region,
                public_read: false,
                expires: 86400, // 24 hours
            }),
            bucket: Some(bucket),
            key_prefix: key_prefix.clone(),
            headers: vec![],
            timeout_seconds: None,
        },
    };

    // Get operator issuer
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let issuer = Issuer::from_secret(&operator.to_bytes());

    // Open the space and add remote
    let mut space = Space::open(space_did.clone(), issuer, storage_path).await?;
    space.add_remote(remote_state).await?;

    println!("Remote '{}' added to space", name);
    println!("Key prefix: {}", space_did);

    Ok(())
}

/// Format a revision for display
fn format_revision(rev: &Revision) -> String {
    format!("{}:{}", rev.period, rev.moment)
}

/// Open the active space
async fn open_active_space() -> Result<Space> {
    let (space_did, storage_path) = get_active_space_info()?;

    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let issuer = Issuer::from_secret(&operator.to_bytes());

    let space = Space::open(space_did, issuer, storage_path).await?;
    Ok(space)
}

/// Pull changes from the upstream remote
pub async fn pull() -> Result<()> {
    let mut space = open_active_space().await?;

    let before = space.revision().await;
    println!("Current revision: {}", format_revision(&before));

    match space.pull().await {
        Ok(Some(_old_upstream)) => {
            let after = space.revision().await;
            println!("Pulled changes: {} -> {}", format_revision(&before), format_revision(&after));
        }
        Ok(None) => {
            println!("Already up to date");
        }
        Err(e) => {
            bail!("Pull failed: {}", e);
        }
    }

    Ok(())
}

/// Push local changes to the upstream remote
pub async fn push() -> Result<()> {
    let mut space = open_active_space().await?;

    let before = space.revision().await;
    println!("Current revision: {}", format_revision(&before));

    match space.push().await {
        Ok(Some(old_upstream)) => {
            println!(
                "Pushed changes: remote {} -> {}",
                format_revision(&old_upstream),
                format_revision(&before)
            );
        }
        Ok(None) => {
            println!("Nothing to push (already in sync)");
        }
        Err(e) => {
            bail!("Push failed: {}", e);
        }
    }

    Ok(())
}

/// Sync with upstream (pull then push)
pub async fn sync() -> Result<()> {
    let mut space = open_active_space().await?;

    let before = space.revision().await;
    println!("Current revision: {}", format_revision(&before));

    // Pull first
    match space.pull().await {
        Ok(Some(_)) => {
            let after_pull = space.revision().await;
            println!("Pulled: {} -> {}", format_revision(&before), format_revision(&after_pull));
        }
        Ok(None) => {
            println!("Pull: already up to date");
        }
        Err(e) => {
            bail!("Pull failed: {}", e);
        }
    }

    // Then push
    let current = space.revision().await;
    match space.push().await {
        Ok(Some(old_upstream)) => {
            println!(
                "Pushed: remote {} -> {}",
                format_revision(&old_upstream),
                format_revision(&current)
            );
        }
        Ok(None) => {
            println!("Push: nothing to push");
        }
        Err(e) => {
            bail!("Push failed: {}", e);
        }
    }

    let final_rev = space.revision().await;
    println!("Final revision: {}", format_revision(&final_rev));

    Ok(())
}

/// Show upstream remote information
pub async fn show() -> Result<()> {
    let space = open_active_space().await?;

    match space.upstream_info().await {
        Some((site, branch_id, revision)) => {
            println!("Upstream configured:");
            println!("  Site:     {}", site);
            println!("  Branch:   {}", branch_id);
            if let Some(rev) = revision {
                println!("  Revision: {}", format_revision(&rev));
            } else {
                println!("  Revision: (not fetched)");
            }
        }
        None => {
            println!("No upstream configured");
            println!("Use 'tonk remote add <name>' to add a remote");
        }
    }

    Ok(())
}

/// Remove the upstream remote (not yet implemented)
pub async fn delete() -> Result<()> {
    let space = open_active_space().await?;

    if space.has_upstream().await {
        bail!("Remote deletion is not yet supported by the underlying storage layer.\nPlease recreate the space without a remote if you need to remove it.");
    } else {
        println!("No upstream configured - nothing to delete");
    }

    Ok(())
}

/// Edit the upstream remote configuration (not yet implemented)
pub async fn edit() -> Result<()> {
    let space = open_active_space().await?;

    if space.has_upstream().await {
        bail!("Remote editing is not yet supported.\nTo change remote configuration, you would need to delete and re-add the remote.\nHowever, remote deletion is also not yet supported by the underlying storage layer.");
    } else {
        println!("No upstream configured - nothing to edit");
        println!("Use 'tonk remote add <name>' to add a remote");
    }

    Ok(())
}

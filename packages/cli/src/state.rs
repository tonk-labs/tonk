use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Get the .tonk directory path
fn tonk_dir() -> Result<PathBuf> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".tonk"))
}

/// Get the operator DID
fn get_operator_did() -> Result<String> {
    let keystore = crate::keystore::Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    Ok(operator.to_did_key())
}

/// Get the operator directory path: ~/.tonk/operator/{operator-did}/
fn operator_dir() -> Result<PathBuf> {
    let operator_did = get_operator_did()?;
    Ok(tonk_dir()?.join("operator").join(operator_did))
}

/// Get the active session DID from ~/.tonk/operator/{operator-did}/session/@active
pub fn get_active_session() -> Result<Option<String>> {
    let active_file = operator_dir()?.join("session").join("@active");

    if !active_file.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&active_file)
        .context("Failed to read active session file")?;

    let did = content.trim().to_string();
    if did.is_empty() {
        Ok(None)
    } else {
        Ok(Some(did))
    }
}

/// Set the active session DID in ~/.tonk/operator/{operator-did}/session/@active
pub fn set_active_session(authority_did: &str) -> Result<()> {
    let active_file = operator_dir()?.join("session").join("@active");

    // Ensure parent directory exists
    if let Some(parent) = active_file.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&active_file, authority_did)
        .context("Failed to write active session file")?;

    Ok(())
}

/// Get all session DIDs (authorities) from ~/.tonk/operator/{operator-did}/session/
pub fn list_sessions() -> Result<Vec<String>> {
    let session_dir = operator_dir()?.join("session");

    if !session_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    for entry in fs::read_dir(&session_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("did:key:") {
                    sessions.push(name.to_string());
                }
            }
        }
    }

    sessions.sort();
    Ok(sessions)
}

/// Get the active space DID for a session from ~/.tonk/operator/{operator-did}/session/{authority}/space/@active
pub fn get_active_space(authority_did: &str) -> Result<Option<String>> {
    let active_file = operator_dir()?
        .join("session")
        .join(authority_did)
        .join("space")
        .join("@active");

    if !active_file.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&active_file)
        .context("Failed to read active space file")?;

    let did = content.trim().to_string();
    if did.is_empty() {
        Ok(None)
    } else {
        Ok(Some(did))
    }
}

/// Set the active space DID for a session in ~/.tonk/operator/{operator-did}/session/{authority}/space/@active
pub fn set_active_space(authority_did: &str, space_did: &str) -> Result<()> {
    let active_file = operator_dir()?
        .join("session")
        .join(authority_did)
        .join("space")
        .join("@active");

    // Ensure parent directory exists
    if let Some(parent) = active_file.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&active_file, space_did)
        .context("Failed to write active space file")?;

    Ok(())
}

/// Add a space to a session by creating ~/.tonk/operator/{operator-did}/session/{authority}/space/{space-did}/
pub fn add_space_to_session(authority_did: &str, space_did: &str) -> Result<()> {
    let space_dir = operator_dir()?
        .join("session")
        .join(authority_did)
        .join("space")
        .join(space_did);

    fs::create_dir_all(&space_dir)
        .context("Failed to create space directory")?;

    Ok(())
}

/// List all space DIDs for a session from ~/.tonk/operator/{operator-did}/session/{authority}/space/
pub fn list_spaces_for_session(authority_did: &str) -> Result<Vec<String>> {
    let spaces_dir = operator_dir()?
        .join("session")
        .join(authority_did)
        .join("space");

    if !spaces_dir.exists() {
        return Ok(Vec::new());
    }

    let mut spaces = Vec::new();

    for entry in fs::read_dir(&spaces_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip the @active file (active space marker)
        if path.is_file() {
            continue;
        }

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("did:key:") {
                    spaces.push(name.to_string());
                }
            }
        }
    }

    spaces.sort();
    Ok(spaces)
}

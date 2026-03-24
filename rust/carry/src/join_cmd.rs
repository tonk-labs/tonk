//! `carry join <TOKEN>` — redeem an invite token to join a space.
//!
//! Decodes the invite token, verifies the delegation grants, creates local
//! space directories with the collaborator's credentials, and stores the
//! delegations in the space DB.

use crate::identity_cmd;
use crate::schema;
use crate::site::Site;
use anyhow::{Context, Result};
use std::path::Path;
use tonk_space::{Timestamp, decode_invite, verify_envelope};

/// Execute `carry join <token> [--repo <REPO>]`.
pub async fn execute(token: &str, site_flag: Option<&Path>) -> Result<()> {
    // Decode and validate the token structure
    let envelope = decode_invite(token).context("Failed to decode invite token")?;

    // Load the local identity (must exist and match the invited DID)
    let operator = identity_cmd::require_identity()?;
    if operator.did().to_string() != envelope.invited {
        anyhow::bail!(
            "Local identity {} does not match invited DID {}.\n\
             Run `carry identity` to check your identity.",
            operator.did(),
            envelope.invited
        );
    }

    // Verify all grants cryptographically
    let now = Timestamp::now().to_unix();
    let delegations =
        verify_envelope(&envelope, now).context("Invite token verification failed")?;

    // Resolve or create the .carry/ site
    let site = match Site::resolve(site_flag) {
        Ok(site) => site,
        Err(_) => {
            let parent = if let Some(p) = site_flag {
                p.to_path_buf()
            } else {
                std::env::current_dir().context("Failed to determine current directory")?
            };
            Site::init(&parent)?
        }
    };

    let mut joined_count = 0;

    for (grant, delegation) in envelope.grants.iter().zip(delegations.iter()) {
        let space_did = &grant.space;

        // Create local space directory if it doesn't exist
        let space_ref = if let Some(existing) = site.space_by_did(space_did) {
            existing
        } else {
            // Create a new space directory for this space DID.
            // The collaborator uses their own identity key as credentials.
            let space = site.create_space_from_key(operator.signer())?;

            // Bootstrap builtins in the new space
            let mut session = space.open_session().await?;
            schema::bootstrap_builtins(&mut session).await?;

            space
        };

        // Store the delegation in the space DB
        let mut session = space_ref.open_session().await?;
        let mut tx = session.edit();
        dialog_query::claim::Claim::assert(delegation.clone(), &mut tx);
        session.commit(tx).await?;

        joined_count += 1;
    }

    // Set the first joined space as active if we only joined one
    if joined_count == 1 {
        let space_did = &envelope.grants[0].space;
        site.set_active_space(space_did)?;
    }

    eprintln!("Joined {} space(s) as {}", joined_count, envelope.invited);

    Ok(())
}

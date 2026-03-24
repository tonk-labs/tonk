//! `carry join <TOKEN>` — redeem an invite token to join a space.
//!
//! Decodes the invite token, verifies the delegation grants, creates local
//! space directories with the collaborator's credentials, and stores the
//! delegations in the space DB.

use crate::schema;
use crate::site::Site;
use anyhow::{Context, Result};
use std::path::Path;
use tonk_space::{Timestamp, decode_invite, verify_envelope};

/// Execute `carry join <token> [--repo <REPO>]`.
pub async fn execute(token: &str, site_flag: Option<&Path>) -> Result<()> {
    // Decode and validate the token structure
    let envelope = decode_invite(token).context("Failed to decode invite token")?;

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

    // Generate the collaborator's operator keypair
    let collab_key = ed25519_dalek::SigningKey::generate(&mut rand_0_8::rngs::OsRng);
    let collab_operator = tonk_space::Operator::from_secret(collab_key.to_bytes());

    // Verify the local operator matches the invited DID
    if collab_operator.did().to_string() != envelope.invited {
        // The collaborator needs a stable identity that matches the invited DID.
        // For now, we need the invited DID's credentials to already exist locally,
        // or we generate fresh (which won't match). This is a placeholder until
        // `carry identity` exists.
        //
        // For v1: check if there's already a space with credentials whose DID
        // matches the invited DID. If not, error with guidance.
        let spaces = site.list_spaces()?;
        let mut found_operator = None;
        for space in &spaces {
            if let Ok(op) = space.load_operator() {
                if op.did().to_string() == envelope.invited {
                    found_operator = Some(op);
                    break;
                }
            }
        }

        let collab_operator = found_operator.ok_or_else(|| {
            anyhow::anyhow!(
                "No local identity matching invited DID {}.\n\
                 Your local operator DID must match the invite. \
                 A `carry identity` command will address this in a future release.",
                envelope.invited
            )
        })?;

        return join_spaces(&site, &collab_operator, &delegations, &envelope).await;
    }

    join_spaces(&site, &collab_operator, &delegations, &envelope).await
}

async fn join_spaces(
    site: &Site,
    _collab_operator: &tonk_space::Operator,
    delegations: &[tonk_space::Delegation],
    envelope: &tonk_space::InviteEnvelopeV1,
) -> Result<()> {
    let mut joined_count = 0;

    for (grant, delegation) in envelope.grants.iter().zip(delegations.iter()) {
        let space_did = &grant.space;

        // Create local space directory if it doesn't exist
        let space_ref = if let Some(existing) = site.space_by_did(space_did) {
            existing
        } else {
            // Create a new space directory for this space DID.
            // The collaborator uses their own keypair as credentials.
            let collab_key = ed25519_dalek::SigningKey::generate(&mut rand_0_8::rngs::OsRng);
            let space = site.create_space_from_key(&collab_key)?;

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

    eprintln!(
        "Joined {} space(s) as {}",
        joined_count, envelope.invited
    );

    Ok(())
}

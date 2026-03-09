//! `carry invite <INVITED_DID>` — create an invite token for a collaborator.
//!
//! Mints a UCAN delegation from the inviter's identity to the invited DID,
//! includes the upstream proof chain (space → admin → ... → inviter),
//! wraps it in a self-contained invite token, and prints it to stdout.

use crate::identity_cmd;
use crate::site::SiteContext;
use anyhow::{Context, Result};
use tonk_space::{create_invite, encode_invite};

/// Execute `carry invite <invited_did>`.
pub async fn execute(ctx: &SiteContext, invited_did: &str) -> Result<()> {
    // Parse the invited DID
    let invited: tonk_space::Did = invited_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid DID '{}': {:?}", invited_did, e))?;

    // Load the inviter's identity
    let identity = identity_cmd::require_identity()?;

    // Load the space's proof chain (our authority over this space)
    let upstream_proofs = ctx.space.load_proofs()?;

    let space_did: tonk_space::Did =
        ctx.space.did.parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse space DID '{}': {:?}", ctx.space.did, e)
        })?;

    // Read repo label for the hint field
    let repo_hint = ctx.site.space_label(&ctx.space).await.ok().flatten();

    // Create the invite (delegation + envelope)
    let (envelope, delegation) =
        create_invite(&identity, &space_did, &invited, repo_hint, &upstream_proofs)
            .await
            .context("Failed to create invite")?;

    // Store the delegation in the space DB for audit
    let mut session = ctx.space.open_session().await?;
    let mut tx = session.edit();
    dialog_query::claim::Claim::assert(delegation, &mut tx);
    session.commit(tx).await?;

    // Encode and print the token
    let token = encode_invite(&envelope).context("Failed to encode invite token")?;

    eprintln!("Invite token (share securely with your collaborator):");
    println!("{}", token);

    Ok(())
}

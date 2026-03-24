//! `carry invite <INVITED_DID>` — create an invite token for a collaborator.
//!
//! Mints a UCAN delegation from the active space to the invited DID, wraps it
//! in a self-contained invite token, and prints it to stdout.

use crate::site::SiteContext;
use anyhow::{Context, Result};
use tonk_space::{create_invite, encode_invite};

/// Execute `carry invite <invited_did>`.
pub async fn execute(ctx: &SiteContext, invited_did: &str) -> Result<()> {
    // Parse the invited DID
    let invited: tonk_space::Did = invited_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid DID '{}': {:?}", invited_did, e))?;

    // Load the space operator (the space's signing key)
    let space_operator = ctx.space.load_operator()?;
    let space_did = space_operator.did();

    // Read repo label for the hint field
    let repo_hint = ctx.site.space_label(&ctx.space).await.ok().flatten();

    // Create the invite (delegation + envelope)
    let (envelope, delegation) = create_invite(&space_operator, &space_did, &invited, repo_hint)
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

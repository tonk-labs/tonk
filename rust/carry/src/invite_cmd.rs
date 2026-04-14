//! `carry invite [<MEMBER>]` -- CLI command for generating invite URLs.
//!
//! Delegates to [`crate::invite`] for platform-independent primitives.

use crate::invite;
use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::DelegationChain;
use dialog_varsig::{Did, Principal};

// Re-export for callers that import from invite_cmd.
pub use crate::invite::{parse_invite_url, redelegate};

/// The result of creating an invite.
pub struct Invite {
    /// The invite URL to share.
    pub url: String,
    /// The delegation chain (for asserting into the space).
    pub chain: DelegationChain,
    /// The member DID that was delegated to.
    pub audience: Did,
}

/// Execute `carry invite [<MEMBER>] [--url <BASE>]`.
pub async fn execute(site: &Site, member: Option<&str>, base_url: Option<&str>) -> Result<()> {
    let audience: Option<Did> = member
        .map(|m| m.parse().with_context(|| format!("invalid DID: {}", m)))
        .transpose()?;

    let invite = create_invite(site, audience.as_ref(), base_url).await?;

    println!("{}", invite.url);

    Ok(())
}

/// Build an invite URL delegating repo access.
///
/// If `audience` is `None`, generates an ephemeral keypair and embeds its
/// private key in the URL fragment.
pub async fn create_invite(
    site: &Site,
    audience: Option<&Did>,
    base_url: Option<&str>,
) -> Result<Invite> {
    let base = base_url.unwrap_or(invite::DEFAULT_BASE_URL);

    let (target_did, secret_seed) = match audience {
        Some(did) => (did.clone(), None),
        None => {
            let ephemeral = Ed25519Signer::generate()
                .await
                .context("Failed to generate ephemeral keypair")?;
            let did = ephemeral.did();
            let exported = ephemeral
                .export()
                .await
                .context("Failed to export ephemeral key")?;
            let seed_bytes = match exported {
                dialog_credentials::KeyExport::Extractable(bytes) => bytes,
                #[allow(unreachable_patterns)]
                _ => anyhow::bail!("Ephemeral key is not extractable"),
            };
            (did, Some(seed_bytes))
        }
    };

    let chain = site
        .profile
        .access()
        .claim(&site.repo)
        .delegate(target_did.clone())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation")?;

    let remote_url = resolve_access_url(site).await;

    let url = invite::build_invite_url(
        base,
        &chain,
        remote_url.as_deref(),
        secret_seed.as_deref(),
    )?;

    Ok(Invite {
        url,
        chain,
        audience: target_did,
    })
}

/// Resolve the access service URL from the repo's upstream remote, if any.
pub async fn resolve_access_url(site: &Site) -> Option<String> {
    use dialog_repository::{SiteAddress, UpstreamState};

    let upstream = site.branch.upstream()?;
    let remote_name = match upstream {
        UpstreamState::Remote { name, .. } => name,
        _ => return None,
    };

    let remote = site
        .repo
        .remote(remote_name)
        .load()
        .perform(&site.operator)
        .await
        .ok()?;

    match remote.address().site() {
        SiteAddress::Ucan(ucan) => Some(ucan.endpoint().to_string()),
        _ => None,
    }
}

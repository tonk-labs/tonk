//! `carry invite [<MEMBER>]` -- CLI command for generating invite URLs.
//!
//! Builds a delegation chain via dialog, then hands it to `tonk_invite::Invite`
//! to serialize as a URL. The URL format and claim semantics live in
//! `tonk-invite` so that web UIs and the CLI agree by construction.

use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal};
use tonk_invite::{Invite as TonkInvite, InviteAudience};
use url::Url;

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
/// If `audience` is `None`, generates an ephemeral keypair (open invite); the
/// seed is embedded in the URL fragment. Otherwise, the chain audience is the
/// passed DID (scoped invite).
pub async fn create_invite(
    site: &Site,
    audience: Option<&Did>,
    base_url: Option<&str>,
) -> Result<Invite> {
    let base = base_url.unwrap_or(tonk_invite::DEFAULT_BASE_URL);

    let (target_did, invite_audience) = match audience {
        Some(did) => (did.clone(), InviteAudience::Scoped),
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
            let seed: [u8; 32] = seed_bytes.try_into().map_err(|v: Vec<u8>| {
                anyhow::anyhow!("ephemeral seed must be 32 bytes, got {}", v.len())
            })?;
            (did, InviteAudience::Open { seed })
        }
    };

    let delegation = site
        .profile
        .access()
        .claim(&site.repo)
        .delegate(target_did.clone())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation")?;
    let chain = delegation.into_chain();

    let remote_url = resolve_access_url(site)
        .await
        .map(|s| Url::parse(&s).context("invalid access URL"))
        .transpose()?;

    let tonk_invite = TonkInvite::new(chain.clone(), invite_audience, remote_url)
        .await
        .context("Failed to construct invite")?;
    let url = tonk_invite
        .to_url(base)
        .context("Failed to serialize invite URL")?;

    Ok(Invite {
        url,
        chain,
        audience: target_did,
    })
}

/// Resolve the access service URL from the repo's upstream remote, if any.
pub async fn resolve_access_url(site: &Site) -> Option<String> {
    use dialog_repository::{SiteAddress, Upstream};

    let upstream = site.branch.upstream()?;
    let remote_name = match upstream {
        Upstream::Remote { remote: name, .. } => name,
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

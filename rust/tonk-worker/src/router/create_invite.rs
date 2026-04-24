//! `POST /api/repository/:repo/invite`: mint an invite URL for a repo.
//!
//! Mirrors the native `carry invite` command at
//! `tonk-carry/rust/carry/src/invite_cmd.rs`: issues a fresh delegation
//! from the caller's profile, reads the repo's `origin` remote to fold
//! the access-service URL into the invite, and serializes via the
//! shared [`tonk_invite::Invite`] primitive so invites minted here are
//! redeemable by any tonk-compatible client.
//!
//! Two modes, distinguished by whether the request body names an
//! `audience` DID:
//!
//! - **audience-scoped** — body `{ "audience": "did:key:..." }`: chain's
//!   final audience is that DID; only that identity can claim.
//! - **audience-open** (default) — body absent or `{}`: generates an
//!   ephemeral Ed25519 key, embeds its seed in the URL fragment. Any
//!   redeemer can claim by redelegating from the ephemeral key.
//!
//! The `base_url` field in the body controls the URL's scheme+host+path
//! prefix — typically `<window.origin>/join` from the UI so that
//! invites minted on a local dev deployment open against that same
//! deployment rather than production.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_credentials::{Ed25519Signer, key::KeyExport};
use dialog_repository::{RepositoryExt as _, SiteAddress, Upstream};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_invite::{Invite, InviteAudience};
use url::Url;

use super::AppState;
use crate::TonkWorkerError;

/// Body of `POST /api/repository/:repo/invite`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreateInviteRequest {
    /// Base URL to embed in the invite link (e.g. `https://tonk.xyz/join`
    /// or `<window.origin>/join`). Falls back to
    /// [`tonk_invite::DEFAULT_BASE_URL`] when absent — but UI callers
    /// should always pass this so that links open against the minting
    /// deployment rather than the canonical `tonk.xyz` host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Recipient DID for an audience-scoped invite. Absent → open
    /// invite with an ephemeral key embedded in the URL fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Did>,
}

/// Response body of `POST /api/repository/:repo/invite`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateInviteResponse {
    /// The invite URL to share.
    pub url: String,
    /// The DID the delegation chain terminates at — the recipient's DID
    /// for audience-scoped invites, or the ephemeral key's DID for
    /// audience-open invites.
    pub audience: Did,
}

/// Generate an ephemeral Ed25519 signer with an extractable seed.
///
/// On wasm the default `Ed25519Signer::generate()` produces a
/// non-extractable WebCrypto key whose seed can't be read, which can't
/// be embedded in the invite URL. The [`ExtractableKey`] variant opts
/// in to extractable generation. On native, the regular `generate` is
/// already extractable.
///
/// [`ExtractableKey`]: dialog_credentials::key::ExtractableKey
async fn generate_ephemeral() -> Result<(Ed25519Signer, [u8; 32]), TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let signer = {
        use dialog_credentials::key::ExtractableKey;
        <Ed25519Signer as ExtractableKey>::generate()
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("failed to generate ephemeral key: {e}"))
            })?
    };
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let signer = Ed25519Signer::generate()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to generate ephemeral key: {e}")))?;

    let exported = signer
        .export()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to export ephemeral key: {e}")))?;

    let seed: [u8; 32] = match exported {
        KeyExport::Extractable(bytes) => bytes.as_slice().try_into().map_err(|_| {
            TonkWorkerError::Internal(format!(
                "ephemeral seed has unexpected length {}, want 32",
                bytes.len()
            ))
        })?,
        #[allow(unreachable_patterns)]
        _ => {
            return Err(TonkWorkerError::Internal(
                "ephemeral key was generated non-extractable; \
                 cannot embed seed in invite URL"
                    .into(),
            ));
        }
    };

    Ok((signer, seed))
}

/// Mint an invite URL for `repo`.
#[wasm_compat]
pub async fn create_invite(
    State(state): State<AppState>,
    Path(repo_name): Path<String>,
    body_bytes: Bytes,
) -> Result<Json<CreateInviteResponse>, TonkWorkerError> {
    log!("POST /api/repository/{}/invite", repo_name);

    // Empty body = defaults (open invite, canonical base URL).
    // Non-empty body is parsed here so JSON errors come back as the
    // structured `Router` variant rather than axum's default
    // plain-text `JsonRejection`. Matches `put_repository`.
    let request: CreateInviteRequest = if body_bytes.is_empty() {
        CreateInviteRequest::default()
    } else {
        serde_json::from_slice(&body_bytes)
            .map_err(|e| TonkWorkerError::Router(format!("Invalid request body: {e}")))?
    };

    let base_url = request
        .base_url
        .as_deref()
        .unwrap_or(tonk_invite::DEFAULT_BASE_URL);

    let tonk = state.read().await;

    // 1. Load the repo. 404 on miss — creating the invite only makes
    // sense against an existing repo.
    let repository = tonk
        .profile
        .repository(&repo_name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", repo_name, e))
        })?;

    // 2. Resolve the audience DID. `None` → generate an ephemeral
    // signer and remember the seed so we can embed it below.
    let (audience_did, ephemeral_seed) = match request.audience {
        Some(did) => (did, None),
        None => {
            let (signer, seed) = generate_ephemeral().await?;
            (signer.did(), Some(seed))
        }
    };

    // 3. Issue a delegation from the profile (which already holds a
    // chain granting access to this repo) to the audience.
    let delegation: UcanDelegation = tonk
        .profile
        .access()
        .claim(&repository)
        .delegate(audience_did.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;

    let chain = delegation.into_chain();

    // 4. Look up the repo's origin remote URL, if any, so redeemers
    // can sync immediately. Mirrors carry's `resolve_access_url` but
    // uses the worker's `Upstream` type rather than `UpstreamState`.
    let remote_url = resolve_remote_url(&tonk, &repository).await;

    // 5. Assemble + serialize via the shared primitive so the URL
    // format stays in lockstep with the claim path.
    let audience = match ephemeral_seed {
        Some(seed) => InviteAudience::Open { seed },
        None => InviteAudience::Scoped,
    };

    let invite = Invite::new(chain, audience, remote_url)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to assemble invite: {e}")))?;

    let url = invite
        .to_url(base_url)
        .map_err(|e| TonkWorkerError::Router(format!("failed to serialize invite URL: {e}")))?;

    log!(
        "Minted invite for repo '{}' audience {}",
        repo_name,
        audience_did,
    );

    Ok(Json(CreateInviteResponse {
        url,
        audience: audience_did,
    }))
}

/// Probe `main.upstream` and, when it points at a remote, pull the
/// UCAN access-service URL off the remote's site address. Anything
/// else (no upstream, non-UCAN address, load failures) resolves to
/// `None` — invites without a remote become local-only, which the
/// claim side already tolerates.
async fn resolve_remote_url<R>(
    tonk: &crate::worker::TonkState,
    repository: &dialog_repository::Repository<R>,
) -> Option<Url>
where
    R: Principal + Clone,
{
    let main = repository
        .branch("main")
        .open()
        .perform(&tonk.operator)
        .await
        .ok()?;

    let remote_name = match main.upstream()? {
        Upstream::Remote { remote, .. } => remote,
        _ => return None,
    };

    let remote = repository
        .remote(remote_name.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .ok()?;

    match remote.address().site() {
        SiteAddress::Ucan(ucan) => Url::parse(ucan.endpoint()).ok(),
        _ => None,
    }
}

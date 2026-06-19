//! `POST /api/repository/:repo/invite`: mint an invite URL for a repo.
//!
//! Two modes, distinguished by whether the request body names an
//! `audience` DID:
//!
//! - **audience-scoped** — body `{ "audience": "did:key:..." }`: only
//!   that identity can claim. Response tagged `"scoped"`, echoes the DID.
//! - **audience-open** (default) — body absent or `{}`: generates an
//!   ephemeral Ed25519 key, embeds its seed in the URL fragment. Any
//!   redeemer can claim by redelegating from the ephemeral key.
//!
//! `base_url` controls the minted URL's prefix — typically
//! `<window.origin>/join` from the UI so links open against the minting
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
    /// Base URL to embed in the invite link. Falls back to
    /// [`tonk_invite::DEFAULT_BASE_URL`] when absent. Typed as [`Url`]
    /// so malformed values reject at deserialize time with a 400.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Url>,

    /// Recipient DID for an audience-scoped invite. Absent → open invite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Did>,
}

/// Response body of `POST /api/repository/:repo/invite`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreateInviteResponse {
    /// Only `audience` can claim.
    Scoped {
        /// Minted invite URL.
        url: Url,
        /// Echo of the requested recipient DID.
        audience: Did,
    },
    /// Anyone with the URL can claim.
    Open {
        /// Minted invite URL, with the ephemeral seed in the fragment.
        url: Url,
    },
}

impl CreateInviteResponse {
    /// The minted invite URL, uniformly for both variants.
    pub fn url(&self) -> &Url {
        match self {
            Self::Scoped { url, .. } | Self::Open { url } => url,
        }
    }
}

/// Generate an ephemeral Ed25519 signer with an extractable seed.
///
/// Wasm's default `Ed25519Signer::generate` produces a non-extractable
/// WebCrypto key whose seed can't be embedded in the invite URL; the
/// [`ExtractableKey`] variant opts in to extractable generation.
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
        other => {
            return Err(TonkWorkerError::Internal(format!(
                "ephemeral key export returned an unexpected variant ({other:?}); \
                 expected KeyExport::Extractable so the seed can be embedded in the invite URL"
            )));
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

    // Parse inline so JSON errors become structured `Router` (400) rather
    // than axum's default plain-text `JsonRejection`.
    let request: CreateInviteRequest = if body_bytes.is_empty() {
        CreateInviteRequest::default()
    } else {
        serde_json::from_slice(&body_bytes)
            .map_err(|e| TonkWorkerError::Router(format!("Invalid request body: {e}")))?
    };

    let base_url = request
        .base_url
        .as_ref()
        .map(Url::as_str)
        .unwrap_or(tonk_invite::DEFAULT_BASE_URL);

    let tonk = state.read().await;

    let repository = tonk
        .profile
        .repository(&repo_name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", repo_name, e))
        })?;

    let (audience_did, audience) = match request.audience {
        Some(did) => (did, InviteAudience::Scoped),
        None => {
            let (signer, seed) = generate_ephemeral().await?;
            (signer.did(), InviteAudience::Open { seed })
        }
    };

    let delegation: UcanDelegation = tonk
        .profile
        .access()
        .claim(&repository)
        .delegate(audience_did.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;

    let remote_url = resolve_remote_url(&tonk, &repository).await?;

    let invite = Invite::new(delegation.into_chain(), audience, remote_url)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to assemble invite: {e}")))?;

    let url_str = invite
        .to_url(base_url)
        .map_err(|e| TonkWorkerError::Router(format!("failed to serialize invite URL: {e}")))?;
    let url = Url::parse(&url_str).map_err(|e| {
        TonkWorkerError::Internal(format!(
            "invite URL serializer produced an unparseable value: {e}"
        ))
    })?;

    log!(
        "Minted invite for repo '{}' audience {}",
        repo_name,
        audience_did,
    );

    let response = match invite.audience {
        InviteAudience::Open { .. } => CreateInviteResponse::Open { url },
        InviteAudience::Scoped => CreateInviteResponse::Scoped {
            url,
            audience: audience_did,
        },
    };
    Ok(Json(response))
}

/// Probe `main`'s upstream and, when it points at a remote, pull the
/// UCAN access-service URL off the remote's site address.
///
/// - `Ok(None)` — legitimate "this repo has no remote to advertise"
///   (branch has no upstream, non-remote upstream, or non-UCAN site).
///   The claim side tolerates invites without a remote.
/// - `Err(...)` — branch/remote load failed or the stored UCAN endpoint
///   won't parse. Failing the whole mint is the right move: silently
///   demoting to local-only would mask config drift the inviter can't
///   see (redeemers would hit a downstream sync error with no link to
///   the root cause).
///
/// `main` is hardcoded; see `project_main_branch_implicit_creation`
/// memory note on why `.open()` is used here despite not being
/// strictly read-only.
pub(crate) async fn resolve_remote_url<R>(
    tonk: &crate::worker::TonkState,
    repository: &dialog_repository::Repository<R>,
) -> Result<Option<Url>, TonkWorkerError>
where
    R: Principal + Clone,
{
    resolve_remote_url_with(repository, &tonk.operator).await
}

/// [`resolve_remote_url`] against a bare operator rather than the whole
/// [`TonkState`] — for callers (e.g. the invite command handler) that
/// must not hold the state guard across this await.
pub(crate) async fn resolve_remote_url_with<R>(
    repository: &dialog_repository::Repository<R>,
    operator: &crate::worker::DefaultOperator,
) -> Result<Option<Url>, TonkWorkerError>
where
    R: Principal + Clone,
{
    let main = repository
        .branch("main")
        .open()
        .perform(operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "failed to probe branch 'main' while resolving remote URL: {e}"
            ))
        })?;

    let remote_name = match main.upstream() {
        Some(Upstream::Remote { remote, .. }) => remote,
        Some(_) | None => return Ok(None),
    };

    let remote = repository
        .remote(remote_name.as_str())
        .load()
        .perform(operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "branch 'main' upstream names remote '{remote_name}' but it failed to load: {e}"
            ))
        })?;

    match remote.address().site() {
        SiteAddress::Ucan(ucan) => Url::parse(ucan.endpoint()).map(Some).map_err(|e| {
            TonkWorkerError::Internal(format!(
                "remote '{remote_name}' has unparseable UCAN endpoint '{}': {e}",
                ucan.endpoint()
            ))
        }),
        _ => Ok(None),
    }
}

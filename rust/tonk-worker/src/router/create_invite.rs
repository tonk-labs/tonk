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
use tonk_invite::{Invite, InviteAudience, shortcut::ShortcutRequest};
use tonk_schema::Invitation;
use url::Url;

use super::AppState;
use crate::TonkWorkerError;

/// Name of the content branch on a repository — the branch that syncs
/// across replicas, where roster/governance facts must live.
const CONTENT_BRANCH: &str = "main";

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
pub(crate) async fn generate_ephemeral() -> Result<(Ed25519Signer, [u8; 32]), TonkWorkerError> {
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

    // Record the invitation on the repo's content branch: the durable
    // half of the invite. The content branch syncs across replicas, so
    // the record converges where the roster lives. The URL (with its
    // secret fragment) is never stored — only chain-derivable facts. A
    // failure here fails the mint; the claim path can self-heal a missing
    // record, but a mint that can't write its own repo's content branch
    // is broken enough to surface.
    //
    // Route through the *reactor's* cached `main` handle (keyed by the
    // routing key, the `{repo}` param) rather than a fresh
    // `repository.branch().open()`: background sync pulls/publishes through
    // the reactor's cached handle, so a commit on a separate handle leaves
    // it pinned at a stale head and the next pull's CAS fails forever.
    let invitation = Invitation::from_chain(&invite.chain)
        .expect("Invite invariant: chain has a specific subject");
    tonk.reactor
        .repository(&repo_name)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(invitation)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to record invitation: {e}")))?;

    let url_str = invite
        .to_url(base_url)
        .map_err(|e| TonkWorkerError::Router(format!("failed to serialize invite URL: {e}")))?;

    // Shorten against the link's own origin — the only origin that can
    // serve the relative redirect back — when the caller supplied it
    // (the UI always sends `window.origin`). The long URL is fully
    // functional, so a failed PUT degrades to it rather than failing
    // the mint. The hardcoded default base is never PUT to, keeping
    // tests and offline mints network-free.
    let url_str = if request.base_url.is_some() {
        match shorten(&url_str).await {
            Ok(short) => short,
            Err(e) => {
                log!("invite shortcut failed; using the full URL: {e}");
                url_str
            }
        }
    } else {
        url_str
    };
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

/// Shorten a minted invite URL via the shortcut service on its own
/// origin: PUT the path + query, assemble `{origin}/@/{hash}` with the
/// seed fragment re-attached (the fragment never goes on the wire).
async fn shorten(url: &str) -> Result<String, TonkWorkerError> {
    let request = ShortcutRequest::new(url)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to derive shortcut: {e}")))?;
    let hash = put_shortcut(request.endpoint.as_str(), request.target.clone()).await?;
    request
        .short_url(&hash)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to assemble short URL: {e}")))
}

/// PUT a shortcut target, returning the hash the service responds with.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn put_shortcut(endpoint: &str, target: String) -> Result<String, TonkWorkerError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let init = RequestInit::new();
    init.set_method("PUT");
    init.set_body(&target.into());
    let request = Request::new_with_str_and_init(endpoint, &init)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut request: {e:?}")))?;

    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service-worker scope".to_owned()))?;
    let response: Response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut PUT: {e:?}")))?;
    if !response.ok() {
        return Err(TonkWorkerError::Internal(format!(
            "shortcut PUT returned HTTP {}",
            response.status()
        )));
    }
    let text = JsFuture::from(
        response
            .text()
            .map_err(|e| TonkWorkerError::Internal(format!("shortcut response: {e:?}")))?,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("shortcut response: {e:?}")))?;
    text.as_string()
        .ok_or_else(|| TonkWorkerError::Internal("shortcut response is not a string".to_owned()))
}

/// PUT a shortcut target, returning the hash the service responds with.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn put_shortcut(endpoint: &str, target: String) -> Result<String, TonkWorkerError> {
    let response = reqwest::Client::new()
        .put(endpoint)
        .body(target)
        .send()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut PUT: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(TonkWorkerError::Internal(format!(
            "shortcut PUT returned HTTP {status}: {detail}"
        )));
    }
    response
        .text()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut response: {e}")))
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

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use tonk_invite::Invite;
    use tonk_schema::Invitation;
    use tonk_schema::prelude::DidExt as _;

    use crate::router::tests::{content_invitations, put_repo, test_state};
    use crate::router::{CreateInviteResponse, api_router_with_state};

    /// Minting an invite records an `Invitation` on the repo's content
    /// branch whose entity matches what a claimer derives from the
    /// URL, and whose inviter is the minting profile.
    #[dialog_common::test]
    async fn it_records_the_invitation_on_mint() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);

        // Create the repo; address it by the minted routing key.
        let key = put_repo(&app, "test-mint-invitation").await;

        // Mint an open invite.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/invite"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let minted: CreateInviteResponse = serde_json::from_slice(&bytes).unwrap();

        // The claimer-side derivation from the URL matches the record.
        let parsed = Invite::parse_url(minted.url().as_str()).await.unwrap();
        let expected = Invitation::from_chain(&parsed.chain).unwrap();

        let invitations = content_invitations(&state, &key).await;
        assert_eq!(invitations.len(), 1, "exactly the minted invitation");
        assert_eq!(invitations[0].this, expected.this);

        let profile_entity = {
            let guard = state.read().await;
            guard.profile.did().this()
        };
        assert_eq!(invitations[0].inviter.0, profile_entity);
    }
}

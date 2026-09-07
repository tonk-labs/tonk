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
//! Every returned link also carries the organic channel and a hashed space
//! token used by the page-side, closed PostHog attribution schema.

use ::axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_capability::Subject;
use dialog_credentials::{Ed25519Signer, key::KeyExport};
use dialog_effects::Use;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{
    LoadRemoteError, RemoteRepository, RepositoryExt as _, SiteAddress, Upstream,
};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_invite::{Invite, InviteAudience, home_address_meta, shortcut::ShortcutRequest};
use tonk_schema::{Invitation, InvitationExecution, Remote as RemoteConcept};
use url::Url;

pub use tonk_worker_api::{CreateInviteRequest, CreateInviteResponse};

use super::AppState;
use crate::{TonkWorkerError, axum::RequestOrigin};

/// Name of the content branch on a repository — the branch that syncs
/// across replicas, where roster/governance facts must live.
const CONTENT_BRANCH: &str = "main";

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
    Extension(origin): Extension<RequestOrigin>,
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

    let shorten_explicitly = request.base_url.is_some();
    let base_url = match request.base_url.clone() {
        Some(base_url) => base_url,
        None => origin
            .url()
            .join("join")
            .map_err(|error| TonkWorkerError::Internal(error.to_string()))?,
    };

    let tonk = state.read().await;

    if super::account::provider(&tonk).await.is_none() {
        return Err(TonkWorkerError::Forbidden(
            "create an account or log in before sharing".into(),
        ));
    }

    let repository = tonk
        .profile
        .repository(&repo_name)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", repo_name, e))
        })?;

    let (audience_did, audience) = match request.recipient_root {
        Some(did) => (did, InviteAudience::Scoped),
        None => {
            let (signer, seed) = generate_ephemeral().await?;
            (signer.did(), InviteAudience::Open { seed })
        }
    };

    let remote = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote,
        RemoteRequirement::Refused(reason) => {
            let reason = explain_refusal(&tonk, reason).await;
            return Err(TonkWorkerError::Conflict(format!(
                "cannot mint an invite for '{repo_name}': {} ({})",
                reason.detail(),
                reason.code()
            )));
        }
    };

    // The leaf is signed with the space's upstream in its `home.address`
    // meta, so the endpoint rides inside the signed grant.
    let delegation: UcanDelegation = tonk
        .profile
        .access()
        .claim(Subject::from(repository.did()).attenuate(Use))
        .delegate(audience_did.clone())
        .meta(home_address_meta(&remote.access_url))
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to create delegation: {e}")))?;

    let invite = Invite::new(
        delegation.into_chain(),
        audience,
        Some(remote.access_url.clone()),
    )
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
    let kind = match &invite.audience {
        InviteAudience::Open { .. } => "open",
        InviteAudience::Scoped => "scoped",
    };
    let execution = InvitationExecution::new(&invitation, kind);
    tonk.reactor
        .repository(&repo_name)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(invitation)
        .assert(execution)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to record invitation: {e}")))?;

    retain_invite_authority(&tonk, &repo_name, &invite.chain).await?;

    let url_str = invite
        .to_url(base_url.as_str())
        .map_err(|e| TonkWorkerError::Router(format!("failed to serialize invite URL: {e}")))?;
    let url_str =
        tonk_analytics::launch::space_referral_url(&url_str, &repo_name).map_err(|e| {
            TonkWorkerError::Internal(format!("failed to add invite referral attribution: {e}"))
        })?;

    // Shorten against the link's own origin — the only origin that can
    // serve the relative redirect back — when the caller supplied it
    // (the UI always sends `window.origin`). The long URL is fully
    // functional, so a failed PUT degrades to it rather than failing
    // the mint. The hardcoded default base is never PUT to, keeping
    // tests and offline mints network-free.
    let url_str = if shorten_explicitly {
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
            recipient_root: audience_did,
        },
    };
    Ok(Json(response))
}

/// Retain an invite's delegation chain, plus the profile-to-account union,
/// into the repository's content branch.
///
/// Retaining is what makes the invite revocable: it decomposes every
/// certificate on the chain into `dialog.ucan/*` facts and an envelope blob,
/// which is what a later [`prove`] search walks to rebuild the exact path
/// through the invite hop. A chain that is only serialized into a URL leaves
/// nothing on the branch, so revocation has nothing to find.
///
/// The union edge (`profile -> account`) rides along because the branch is a
/// shared, synced surface: without it a second device of the same account can
/// walk only as far as this device's profile key and stops, so the minting
/// device would be the only one that could ever revoke. It is subject-open,
/// so retaining it once per mint is content-addressed and free after the
/// first.
///
/// Best effort on the union half only: a profile with no account root has no
/// union to mint, and that must not fail a mint that is otherwise complete.
///
/// [`prove`]: dialog_repository::Delegations::prove
pub(super) async fn retain_invite_authority(
    tonk: &crate::TonkState,
    repo_name: &str,
    chain: &dialog_ucan_core::DelegationChain,
) -> Result<(), TonkWorkerError> {
    // The reactor's cached handle, for the same stale-head reason the
    // invitation transaction above routes through it.
    let session = tonk
        .reactor
        .repository(repo_name)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to open '{repo_name}' content branch: {e}"))
        })?;

    let mut chains = vec![UcanDelegation(chain.clone())];
    match super::identity::local_root(tonk).await {
        Ok(root) => {
            let signer = tonk.profile.signer().signer().clone();
            match tonk_account::delegations::mint_account_union(&signer, &root.root_did).await {
                Ok(union) => chains.push(UcanDelegation(union)),
                Err(e) => log!("invite union edge was not minted: {e}"),
            }
        }
        Err(e) => log!("no account root on this profile, minting invite without a union: {e}"),
    }

    session
        .handle()
        .delegations()
        .retain_all(chains)
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to retain the invite delegation: {e}"))
        })?;
    Ok(())
}

/// Shorten a minted invite URL via the shortcut service on its own
/// origin: PUT the path + query, assemble `{origin}/@/{hash}` with the
/// seed fragment re-attached (the fragment never goes on the wire).
///
/// Shared with the `tonk:invite` command handler in [`super::repository`],
/// the other mint path, so both shorten identically.
pub(super) async fn shorten(url: &str) -> Result<String, TonkWorkerError> {
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

/// Why a space cannot produce a shareable invite.
///
/// Both variants mean an invite that would fail its recipient: one that
/// can never sync. [`Self::UnshareableRemote`] is terminal;
/// [`Self::NotSynced`] names something the share prompt can attach, so it
/// offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteRefusal {
    /// `main` has no upstream at all, and the account has a provider to
    /// attach one to. Repairable by attaching a remote.
    NotSynced,
    /// No upstream, and nobody has registered: the account this device
    /// has held since first boot is not a customer of any provider.
    /// Repairable by registering, which is what the bar offers.
    NeedsAccount,
    /// No upstream, and the account enrolled but never confirmed the
    /// emailed link. Repairable in the user's inbox, not in the bar —
    /// attaching now would wire a remote the service refuses.
    NeedsActivation,
    /// No upstream, and the account's service was withdrawn. Terminal.
    Suspended,
    /// `main` tracks something that is not a remote, or a remote whose site
    /// address is not a UCAN endpoint. An invite URL has no way to express
    /// either, so there is nothing to offer.
    UnshareableRemote,
}

impl RemoteRefusal {
    /// The stable class string carried on `xyz.tonk.share/blocked`. Wire
    /// vocabulary the bar branches on, so it comes from `tonk-worker-api`
    /// rather than a literal here that agrees with the bar's by luck.
    pub(crate) fn code(self) -> &'static str {
        use tonk_worker_api::share;
        match self {
            Self::NotSynced => share::BLOCKED_NOT_SYNCED,
            Self::NeedsAccount => share::BLOCKED_NEEDS_ACCOUNT,
            Self::NeedsActivation => share::BLOCKED_NEEDS_ACTIVATION,
            Self::Suspended => share::BLOCKED_SUSPENDED,
            Self::UnshareableRemote => share::BLOCKED_UNSHAREABLE_REMOTE,
        }
    }

    /// The sentence shown to the user.
    pub(crate) fn detail(self) -> &'static str {
        match self {
            Self::NotSynced => "This space only exists on this device.",
            Self::NeedsAccount => {
                "Sharing needs an account, so the people you share with have somewhere to sync from."
            }
            Self::NeedsActivation => "Check your email and confirm your address, then share again.",
            Self::Suspended => "This account's sync service has been suspended.",
            Self::UnshareableRemote => "This space's sync server can't be shared.",
        }
    }
}

/// Say WHY a space has no upstream, given what this profile's account has
/// registered.
///
/// `resolve_remote_url` sees only the repository, so every unsynced space
/// reads as [`RemoteRefusal::NotSynced`] — "attach a remote". That is
/// the right answer only when there is a provider to attach to. A device
/// has an account from first boot, so the interesting cases are the ones
/// before registration finishes, and each wants a different remedy:
/// register, go confirm an email, or nothing at all.
///
/// Only `NotSynced` is refined. Every other refusal already knows its
/// own cause.
pub(crate) async fn explain_refusal(
    tonk: &crate::worker::TonkState,
    refusal: RemoteRefusal,
) -> RemoteRefusal {
    use crate::router::customer::{Registration, registration};

    if !matches!(refusal, RemoteRefusal::NotSynced) {
        return refusal;
    }
    match registration(tonk).await {
        Registration::Served { .. } => RemoteRefusal::NotSynced,
        Registration::AwaitingActivation { .. } => RemoteRefusal::NeedsActivation,
        Registration::Suspended => RemoteRefusal::Suspended,
        Registration::Unregistered => RemoteRefusal::NeedsAccount,
    }
}

/// Explicit operational endpoints attached to one invite-ready remote.
#[derive(Debug, Clone)]
pub(crate) struct RemoteExecutionUrls {
    /// UCAN access-service endpoint advertised in the invite.
    pub(crate) access_url: Url,
}

/// Operational endpoints attached to the actual configured sync upstream.
#[derive(Debug, Clone)]
pub(crate) struct ConfiguredRemoteExecutionUrls {
    /// UCAN access-service endpoint used by synchronization.
    pub(crate) access_url: Url,
}

/// The outcome of probing a repository for a configured UCAN sync endpoint.
#[derive(Debug, Clone)]
pub(crate) enum ConfiguredRemoteRequirement {
    /// A UCAN endpoint suitable for synchronization and backup.
    Ready(ConfiguredRemoteExecutionUrls),
    /// No usable configured upstream. See [`RemoteRefusal`].
    Refused(RemoteRefusal),
}

/// The outcome of probing a repository for an invite-ready sync endpoint.
#[derive(Debug, Clone)]
pub(crate) enum RemoteRequirement {
    /// A UCAN endpoint an invite can advertise.
    Ready(RemoteExecutionUrls),
    /// No such endpoint. See [`RemoteRefusal`].
    Refused(RemoteRefusal),
}

pub(crate) async fn resolve_configured_remote_url_with<R>(
    repository: &dialog_repository::Repository<R>,
    operator: &crate::worker::DefaultOperator,
) -> Result<ConfiguredRemoteRequirement, TonkWorkerError>
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
        None => {
            return Ok(ConfiguredRemoteRequirement::Refused(
                RemoteRefusal::NotSynced,
            ));
        }
        Some(_) => {
            return Ok(ConfiguredRemoteRequirement::Refused(
                RemoteRefusal::UnshareableRemote,
            ));
        }
    };

    let meta = repository
        .branch("meta")
        .open()
        .perform(operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to open meta while resolving remote execution: {error}"
            ))
        })?;
    let remotes: Vec<RemoteConcept> = meta
        .query()
        .select(Query::<RemoteConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::var("origin"),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to query remote metadata: {error:?}"))
        })?;
    let remote_concept = remotes
        .into_iter()
        .find(|concept| concept.name.0 == remote_name);
    let remote = load_or_recover_remote(
        repository,
        operator,
        remote_name.as_str(),
        remote_concept.as_ref(),
    )
    .await?;

    let access_url = match remote.address().site() {
        SiteAddress::Ucan(ucan) => Url::parse(ucan.endpoint()).map_err(|e| {
            TonkWorkerError::Internal(format!(
                "remote '{remote_name}' has unparseable UCAN endpoint '{}': {e}",
                ucan.endpoint()
            ))
        })?,
        _ => {
            return Ok(ConfiguredRemoteRequirement::Refused(
                RemoteRefusal::UnshareableRemote,
            ));
        }
    };

    Ok(ConfiguredRemoteRequirement::Ready(
        ConfiguredRemoteExecutionUrls { access_url },
    ))
}

/// Load the named dialog remote, rebuilding a missing address cell only from
/// the replica's persisted remote concept. The metadata is the same signed
/// configuration mirrored by `ensure_remote_config`; the current deployment
/// origin is deliberately not used as a fallback.
async fn load_or_recover_remote<R>(
    repository: &dialog_repository::Repository<R>,
    operator: &crate::worker::DefaultOperator,
    remote_name: &str,
    concept: Option<&RemoteConcept>,
) -> Result<RemoteRepository, TonkWorkerError>
where
    R: Principal + Clone,
{
    match repository
        .remote(remote_name)
        .load()
        .perform(operator)
        .await
    {
        Ok(remote) => Ok(remote),
        Err(LoadRemoteError::NotFound { .. }) => {
            let concept = concept.ok_or_else(|| {
                TonkWorkerError::Internal(format!(
                    "branch 'main' upstream names missing remote '{remote_name}', and meta has no recovery record"
                ))
            })?;
            let subject: Did = concept.subject.0.to_string().parse().map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "remote '{remote_name}' has an invalid subject in meta: {error}"
                ))
            })?;
            let address = concept.address.decode().map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "remote '{remote_name}' has an invalid address in meta: {error:?}"
                ))
            })?;
            repository
                .remote(remote_name)
                .create(address)
                .subject(subject)
                .perform(operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to recover remote '{remote_name}' from meta: {error}"
                    ))
                })
        }
        Err(error) => Err(TonkWorkerError::Internal(format!(
            "branch 'main' upstream names remote '{remote_name}' but it failed to load: {error}"
        ))),
    }
}

/// Probe `main` for an invite-ready endpoint.
pub(crate) async fn resolve_remote_url<R>(
    tonk: &crate::worker::TonkState,
    repository: &dialog_repository::Repository<R>,
) -> Result<RemoteRequirement, TonkWorkerError>
where
    R: Principal + Clone,
{
    resolve_remote_url_with(repository, &tonk.operator).await
}

/// [`resolve_remote_url`] against a bare operator rather than the whole
/// [`TonkState`] — for callers that must not hold state across this await.
pub(crate) async fn resolve_remote_url_with<R>(
    repository: &dialog_repository::Repository<R>,
    operator: &crate::worker::DefaultOperator,
) -> Result<RemoteRequirement, TonkWorkerError>
where
    R: Principal + Clone,
{
    match resolve_configured_remote_url_with(repository, operator).await? {
        ConfiguredRemoteRequirement::Refused(reason) => Ok(RemoteRequirement::Refused(reason)),
        ConfiguredRemoteRequirement::Ready(remote) => {
            Ok(RemoteRequirement::Ready(RemoteExecutionUrls {
                access_url: remote.access_url,
            }))
        }
    }
}

#[cfg(test)]
mod refusal_copy_tests {
    use super::RemoteRefusal;

    #[test]
    fn it_uses_space_in_user_facing_refusals() {
        for detail in [
            RemoteRefusal::NotSynced.detail(),
            RemoteRefusal::NeedsAccount.detail(),
            RemoteRefusal::NeedsActivation.detail(),
            RemoteRefusal::Suspended.detail(),
            RemoteRefusal::UnshareableRemote.detail(),
        ] {
            assert!(!detail.to_ascii_lowercase().contains("spot"), "{detail}");
        }
        assert_eq!(
            RemoteRefusal::NotSynced.detail(),
            "This space only exists on this device."
        );
        assert_eq!(
            RemoteRefusal::UnshareableRemote.detail(),
            "This space's sync server can't be shared."
        );
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dialog_remote_ucan_s3::UcanAddress;
    use dialog_repository::{RepositoryExt as _, SiteAddress};
    use tower::ServiceExt;

    use tonk_invite::Invite;
    use tonk_schema::Invitation;
    use tonk_schema::prelude::DidExt as _;

    use crate::axum::RequestOrigin;
    use crate::router::tests::{attach_remote, content_invitations, put_repo, test_state};
    use crate::router::{CreateInviteResponse, api_router_with_state};

    #[dialog_common::test]
    async fn it_recovers_a_missing_dialog_remote_from_replica_metadata() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "recover-missing-dialog-remote").await;
        let tonk = state.read().await;
        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let address = SiteAddress::from(UcanAddress::new("https://sync.example.test/ucan/"));
        let replica = tonk_schema::Replica::new(tonk.profile.did(), repository.did());
        let concept = replica.remote("origin", repository.did(), &address);

        let recovered =
            super::load_or_recover_remote(&repository, &tonk.operator, "origin", Some(&concept))
                .await
                .expect("signed replica metadata repairs the missing address cell");

        assert_eq!(recovered.address().site(), &address);
        assert_eq!(recovered.did(), repository.did());
        assert!(
            repository
                .remote("origin")
                .load()
                .perform(&tonk.operator)
                .await
                .is_ok()
        );
    }

    /// Minting an invite records an `Invitation` on the repo's content
    /// branch whose entity matches what a claimer derives from the
    /// URL, and whose inviter is the minting profile.
    #[dialog_common::test]
    async fn it_records_the_invitation_on_mint() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);

        // Create the repo; address it by the minted routing key. The
        // route now refuses a local-only repo, so give it a remote first.
        let key = put_repo(&app, "test-mint-invitation").await;
        attach_remote(&app, &key, "https://sync.example.test/ucan/").await;

        // Mint an open invite.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/invite"))
                    .method("POST")
                    .header("content-type", "application/json")
                    .extension(
                        RequestOrigin::parse("https://local.example/invite").expect("valid origin"),
                    )
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
        assert!(minted.url().query_pairs().any(|(name, value)| {
            name == tonk_analytics::launch::CHANNEL_PARAMETER && value == "reshare"
        }));
        assert!(minted.url().query_pairs().any(|(name, value)| {
            name == tonk_analytics::launch::SPACE_PARAMETER
                && value == tonk_analytics::anonymize(&key)
        }));

        // The claimer-side derivation from the URL matches the record.
        let parsed = Invite::parse_url(minted.url().as_str()).await.unwrap();
        let expected = Invitation::from_chain(&parsed.chain).unwrap();

        // The minted leaf names the space's upstream in its signed meta,
        // so the endpoint survives without the `remote=` parameter.
        let embedded = tonk_invite::home_address(&parsed.chain).unwrap();
        assert_eq!(
            embedded.map(String::from),
            Some("https://sync.example.test/ucan/".to_owned())
        );

        let invitations = content_invitations(&state, &key).await;
        assert_eq!(invitations.len(), 1, "exactly the minted invitation");
        assert_eq!(invitations[0].this, expected.this);

        let root_entity = {
            let guard = state.read().await;
            crate::router::identity::root_did(&guard)
                .await
                .expect("test profile has a root")
                .this()
        };
        assert_eq!(invitations[0].inviter.0, root_entity);
    }

    /// A space created without a remote refuses, and says which case it was.
    #[dialog_common::test]
    async fn it_refuses_a_repository_with_no_upstream() {
        use crate::router::create_invite::{RemoteRefusal, RemoteRequirement, resolve_remote_url};

        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-no-upstream").await;

        let tonk = state.read().await;
        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .expect("repository loads");

        let requirement = resolve_remote_url(&tonk, &repository)
            .await
            .expect("probe succeeds");

        assert!(matches!(
            requirement,
            RemoteRequirement::Refused(RemoteRefusal::NotSynced)
        ));
    }

    #[dialog_common::test]
    async fn it_names_the_refusal_classes() {
        use crate::router::create_invite::RemoteRefusal;

        assert_eq!(RemoteRefusal::NotSynced.code(), "not-synced");
        assert_eq!(
            RemoteRefusal::UnshareableRemote.code(),
            "unshareable-remote"
        );
        assert_eq!(
            RemoteRefusal::NotSynced.detail(),
            "This space only exists on this device."
        );
        assert_eq!(
            RemoteRefusal::UnshareableRemote.detail(),
            "This space's sync server can't be shared."
        );
    }

    /// The HTTP mint route refuses a local-only repository rather than
    /// answering with an invite that can never sync.
    #[dialog_common::test]
    async fn it_rejects_a_mint_for_a_local_only_repository() {
        let (app, _state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-http-local-only").await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repository/{key}/invite"))
                    .extension(
                        RequestOrigin::parse("https://local.example/invite").expect("valid origin"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    /// The HTTP route is a second boundary behind the FABB account check: a
    /// stale client must not be able to mint from an unattached profile.
    #[dialog_common::test]
    async fn it_rejects_a_mint_without_an_account() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-http-account-required").await;
        attach_remote(&app, &key, "https://sync.example.test/ucan/").await;
        {
            let tonk = state.read().await;
            crate::router::account::detach_test_account(&tonk)
                .await
                .expect("the test account detaches");
        }

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repository/{key}/invite"))
                    .extension(
                        RequestOrigin::parse("https://local.example/invite").expect("valid origin"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            content_invitations(&state, &key).await.is_empty(),
            "a provider-free profile records no invitation"
        );
    }

    #[dialog_common::test]
    async fn it_uses_the_request_origin_and_rejects_unknown_fields() {
        let (app, _state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "test-request-origin").await;
        attach_remote(&app, &key, "https://sync.example.test/ucan/").await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repository/{key}/invite"))
                    .header("content-type", "application/json")
                    .extension(
                        RequestOrigin::parse(
                            "https://staging.example.test/source?ignored=yes#ignored",
                        )
                        .expect("valid origin"),
                    )
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
        assert_eq!(
            minted.url().origin().ascii_serialization(),
            "https://staging.example.test"
        );
        assert_eq!(minted.url().path(), "/join");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repository/{key}/invite"))
                    .header("content-type", "application/json")
                    .extension(
                        RequestOrigin::parse("https://local.example/invite").expect("valid origin"),
                    )
                    .body(Body::from(r#"{"baseURL":"https://wrong.example/join"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

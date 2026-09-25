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

use dialog_credentials::{Ed25519Signer, key::KeyExport};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{LoadRemoteError, RemoteRepository, SiteAddress, Upstream};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal};
use tonk_common::log;
use tonk_invite::shortcut::ShortcutRequest;
use tonk_schema::Remote as RemoteConcept;
use url::Url;

use crate::TonkWorkerError;

/// Name of the content branch on a repository — the branch that syncs
/// across replicas, where roster/governance facts must live.
const CONTENT_BRANCH: &str = "main";

/// Generate an ephemeral Ed25519 signer and the seed it was made from.
///
/// The seed is what the invite URL carries, and only a signer generated
/// as [`Extractable`] can give it back: a sealed one exports opaque
/// handles on wasm. The signer handed on is sealed, imported from that
/// seed, so nothing downstream holds an extractable key.
///
/// [`Extractable`]: dialog_credentials::Extractable
pub(crate) async fn generate_ephemeral() -> Result<(Ed25519Signer, [u8; 32]), TonkWorkerError> {
    use dialog_credentials::Extractable;
    use dialog_credentials::key::ExtractableKey;

    let extractable = <Ed25519Signer<Extractable> as ExtractableKey>::generate()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to generate ephemeral key: {e}")))?;
    let exported = ExtractableKey::export(&extractable)
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
    let signer = Ed25519Signer::import(&seed)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("failed to import ephemeral key: {e}")))?;

    Ok((signer, seed))
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

/// Shorten a minted invite URL via the shortcut service on the link's
/// origin: PUT the path + query, assemble `{origin}/@/{hash}` with the
/// seed fragment re-attached (the fragment never goes on the wire).
///
/// The answer is verified twice before the short link replaces the long
/// one: the returned hash must be the target's own content address
/// (`short_url` checks), and a probe `GET` of the stored shortcut must
/// actually redirect back to the target (`probe_shortcut`). A
/// content-addressed blob store passes the first — it stores the bytes
/// and answers with the same blake3 a shortener would — and only the
/// probe exposes that it serves bytes instead of a redirect. Either
/// failure means the host does not provide shortening; the caller falls
/// back to the fully functional long URL.
///
/// Shared with the `tonk:invite` command handler in [`super::repository`],
/// the other mint path, so both shorten identically.
/// How long each leg of a shortcut attempt may take.
///
/// The service answers a `PUT /@` in ~10ms, so this is a hang detector,
/// not a budget: a shortcut host that stops answering (a captive portal,
/// a stalled origin, a dropped connection) must not pin the mint on a
/// convenience. Both legs get their own timeout, so the worst case is
/// twice this — still far inside the share control's own 15s backstop
/// (`tonk_fab::logic::SHARE_TIMEOUT_MS`), which is what has to stay true
/// for the long-URL fallback to reach the clipboard.
pub(super) const SHORTCUT_TIMEOUT_MS: u32 = tonk_invite::shortcut::TIMEOUT_MS;

pub(super) async fn shorten(url: &str) -> Result<String, TonkWorkerError> {
    let request = ShortcutRequest::new(url)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to derive shortcut: {e}")))?;
    let hash = put_shortcut(request.endpoint.as_str(), request.target.clone()).await?;
    let short = request
        .short_url(&hash)
        .map_err(|e| TonkWorkerError::Internal(format!("failed to assemble short URL: {e}")))?;
    probe_shortcut(&request, &hash).await?;
    Ok(short)
}

/// `AbortSignal.timeout(SHORTCUT_TIMEOUT_MS)`, or `None` on a runtime
/// without it.
///
/// `web-sys` generates this static as a NON-catching binding, so calling
/// it where `AbortSignal.timeout` is absent throws straight through the
/// wasm frame instead of returning an error — killing the mint that the
/// timeout exists to protect, which is precisely the failure the whole
/// best-effort shortcut is written to avoid. Feature-detect instead: no
/// signal means an untimed request, exactly what it was before the
/// timeout existed, and the share control's own backstop still ends the
/// wait. Never a thrown mint.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn shortcut_timeout_signal() -> Option<web_sys::AbortSignal> {
    use wasm_bindgen::JsValue;

    let constructor =
        js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("AbortSignal")).ok()?;
    let timeout = js_sys::Reflect::get(&constructor, &JsValue::from_str("timeout")).ok()?;
    timeout
        .is_function()
        .then(|| web_sys::AbortSignal::timeout_with_u32(SHORTCUT_TIMEOUT_MS))
}

/// Probe the stored shortcut: `HEAD {origin}/@/{hash}` must redirect
/// back to the stored target. HEAD, not GET — the landing URL is the
/// whole answer, so there is no reason to download the app shell behind
/// it (the same choice `<tonk-invite-link>` documents). The browser
/// fetch follows the redirect; `redirected` plus the landing URL is the
/// proof.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn probe_shortcut(request: &ShortcutRequest, hash: &str) -> Result<(), TonkWorkerError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let probe = request
        .probe_url(hash)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe URL: {e}")))?;
    let init = RequestInit::new();
    init.set_method("HEAD");
    init.set_signal(shortcut_timeout_signal().as_ref());
    let probe_request = Request::new_with_str_and_init(&probe, &init)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe request: {e:?}")))?;
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| TonkWorkerError::Internal("not in a service-worker scope".to_owned()))?;
    let response: Response = JsFuture::from(global.fetch_with_request(&probe_request))
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe HEAD: {e:?}")))?;
    if !response.redirected() {
        return Err(TonkWorkerError::Internal(format!(
            "the shortcut host answered the probe without redirecting (HTTP {})",
            response.status()
        )));
    }
    request
        .verify_resolved(&response.url())
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe: {e}")))
}

/// Probe the stored shortcut without following the redirect: a
/// conforming service answers 3xx with a `Location` that resolves back
/// to the stored target. HEAD — the landing URL is the whole answer.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn probe_shortcut(request: &ShortcutRequest, hash: &str) -> Result<(), TonkWorkerError> {
    let probe = request
        .probe_url(hash)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe URL: {e}")))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(SHORTCUT_TIMEOUT_MS.into()))
        .build()
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe client: {e}")))?;
    let response = client
        .head(&probe)
        .send()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe HEAD: {e}")))?;
    if !response.status().is_redirection() {
        return Err(TonkWorkerError::Internal(format!(
            "the shortcut host answered the probe without redirecting (HTTP {})",
            response.status()
        )));
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            TonkWorkerError::Internal("the shortcut probe redirect carries no Location".to_owned())
        })?;
    let resolved = tonk_invite::shortcut::resolve_location(&probe, location)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe: {e}")))?;
    request
        .verify_resolved(&resolved)
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut probe: {e}")))
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
    init.set_signal(shortcut_timeout_signal().as_ref());
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
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(SHORTCUT_TIMEOUT_MS.into()))
        .build()
        .map_err(|e| TonkWorkerError::Internal(format!("shortcut PUT client: {e}")))?
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
pub(crate) async fn resolve_remote_url<'a, R>(
    tonk: &'a crate::worker::TonkState,
    repository: &'a dialog_repository::Repository<R>,
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

    use dialog_remote_ucan::UcanAddress;
    use dialog_repository::{RepositoryExt as _, SiteAddress};

    use crate::router::tests::{put_repo, test_state};

    #[dialog_common::test]
    async fn it_recovers_a_missing_dialog_remote_from_replica_metadata() {
        let state: crate::router::AppState =
            std::sync::Arc::new(tokio::sync::RwLock::new(test_state().await));
        let key = put_repo(&state, "recover-missing-dialog-remote").await;
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

    /// A space created without a remote refuses, and says which case it was.
    #[dialog_common::test]
    async fn it_refuses_a_repository_with_no_upstream() {
        use crate::router::create_invite::{RemoteRefusal, RemoteRequirement, resolve_remote_url};

        let state: crate::router::AppState =
            std::sync::Arc::new(tokio::sync::RwLock::new(test_state().await));
        let key = put_repo(&state, "test-no-upstream").await;

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
}

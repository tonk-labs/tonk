//! Linking one local-only space into the signed-in account.
//!
//! This is the only ownership transition tonk performs: a space that belongs
//! to no account becomes a space the signed-in account owns. It never runs in
//! reverse and never moves a space between accounts — a synced space stays
//! with its owner so the shares already handed out keep working.
//!
//! Nothing here is destructive, so a failed attempt leaves a usable local
//! space and a retry converges: every step is either idempotent or guarded by
//! the state the previous run left behind. Ownership is settled by the founder
//! row on the space's own content branch, confirmed against the retained
//! `subject → … → account root` chain, so an interrupted run is visible as
//! exactly what it is — a half-finished link, not a finished one.

use std::fs::{File, OpenOptions};
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use axum::Router;
use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse as _, Redirect, Response};
use axum::routing::get;
use dialog_capability::Subject;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::{Branch, Upstream};
use dialog_ucan::UcanDelegation;
use dialog_varsig::{Did, Principal as _};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tonk_schema::Invitation;
use tonk_schema::prelude::DidExt as _;

use crate::inventory::{Roster, SpaceRole};
use crate::remote::DEFAULT_REMOTE;
use crate::site::SiteConfig;
use crate::space::SpaceStore;

const BROWSER_LINK_STATE_FILE: &str = "local-space-link-v1.json";
const BROWSER_LINK_LOCK_FILE: &str = "local-space-link.lock";
const BROWSER_LINK_STATE_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserLinkState {
    version: u8,
    name: String,
    space: String,
    account: String,
    service_did: String,
    service_url: String,
    #[serde(default)]
    consent: Option<String>,
    invite: Option<String>,
    browser_published: bool,
}

#[derive(Debug)]
struct BrowserLinkLock {
    _file: File,
}

/// Browser-delivery choices for `tonk space link`.
#[derive(Clone, Debug)]
pub struct BrowserLinkOptions {
    /// Open the approval page through the OS browser handler.
    pub open_browser: bool,
    /// Explicit approval page for local/staging deployments.
    pub via: Option<String>,
}

enum BrowserMessage {
    Approval {
        encoded: String,
        correlation: String,
        continuation: tokio::sync::oneshot::Sender<std::result::Result<String, String>>,
    },
    Provisioned {
        encoded: String,
        correlation: String,
        continuation: tokio::sync::oneshot::Sender<std::result::Result<String, String>>,
    },
    Completion {
        encoded: String,
        correlation: String,
    },
    Denied {
        reason: String,
        correlation: String,
    },
}

struct BrowserApproval {
    validated: tonk_invite::local_space_link::ValidatedLocalSpaceLinkApproval,
    encoded: String,
    invite: String,
}

#[derive(Clone)]
struct BrowserCallbackState {
    messages: tokio::sync::mpsc::UnboundedSender<BrowserMessage>,
    shutdown: Arc<tokio::sync::Notify>,
}

struct BrowserCallback {
    url: url::Url,
    listener: tokio::net::TcpListener,
}

impl BrowserCallback {
    async fn bind() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .context("failed to bind the local-space link callback")?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            url: format!("http://127.0.0.1:{port}").parse()?,
            listener,
        })
    }

    fn serve(
        self,
    ) -> (
        tokio::sync::mpsc::UnboundedReceiver<BrowserMessage>,
        tokio::task::JoinHandle<()>,
    ) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let state = BrowserCallbackState {
            messages: sender,
            shutdown: shutdown.clone(),
        };
        let app = Router::new()
            .route("/", get(local_link_bridge).post(local_link_delivery))
            .with_state(state);
        let server = axum::serve(self.listener, app).with_graceful_shutdown(async move {
            shutdown.notified().await;
        });
        let task = tokio::spawn(async move {
            let _ = server.await;
        });
        (receiver, task)
    }
}

async fn local_link_bridge() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<meta charset="utf-8"><meta name="referrer" content="no-referrer">
<title>Tonk local space link</title>
<p id="status">Returning to Tonk…</p>
<script>
const fields = new URLSearchParams(location.hash.slice(1));
history.replaceState(null, "", location.pathname);
if (!["approve", "provisioned", "complete", "deny"].some(k => fields.has(k))) {
  document.querySelector("#status").textContent = "No link response was provided.";
} else {
  const form = document.createElement("form"); form.method = "post"; form.hidden = true;
  for (const [name, value] of fields) {
    const input = document.createElement("input"); input.name = name; input.value = value;
    form.appendChild(input);
  }
  document.body.appendChild(form); form.submit();
}
</script>"##,
    )
}

async fn local_link_delivery(
    State(state): State<BrowserCallbackState>,
    Form(fields): Form<std::collections::HashMap<String, String>>,
) -> Response {
    let correlation = fields.get("correlation").cloned().unwrap_or_default();
    if let Some(encoded) = fields.get("approve") {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if state
            .messages
            .send(BrowserMessage::Approval {
                encoded: encoded.clone(),
                correlation,
                continuation: sender,
            })
            .is_err()
        {
            return (
                axum::http::StatusCode::GONE,
                "terminal is no longer waiting",
            )
                .into_response();
        }
        return match receiver.await {
            Ok(Ok(target)) => match target.parse::<axum::http::HeaderValue>() {
                Ok(_) => Redirect::to(&target).into_response(),
                Err(_) => (
                    axum::http::StatusCode::BAD_REQUEST,
                    "continuation URL was invalid",
                )
                    .into_response(),
            },
            Ok(Err(error)) => (axum::http::StatusCode::BAD_REQUEST, error).into_response(),
            Err(_) => (axum::http::StatusCode::GONE, "terminal stopped").into_response(),
        };
    }
    if let Some(encoded) = fields.get("provisioned") {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if state
            .messages
            .send(BrowserMessage::Provisioned {
                encoded: encoded.clone(),
                correlation,
                continuation: sender,
            })
            .is_err()
        {
            return (
                axum::http::StatusCode::GONE,
                "terminal is no longer waiting",
            )
                .into_response();
        }
        return match receiver.await {
            Ok(Ok(target)) => Redirect::to(&target).into_response(),
            Ok(Err(error)) => (axum::http::StatusCode::BAD_REQUEST, error).into_response(),
            Err(_) => (axum::http::StatusCode::GONE, "terminal stopped").into_response(),
        };
    }
    if let Some(encoded) = fields.get("complete") {
        let _ = state.messages.send(BrowserMessage::Completion {
            encoded: encoded.clone(),
            correlation,
        });
        state.shutdown.notify_one();
        return Html("<p>Space linked. You can return to your terminal.</p>").into_response();
    }
    let reason = fields
        .get("deny")
        .cloned()
        .unwrap_or_else(|| "the browser sent no link outcome".into());
    let _ = state.messages.send(BrowserMessage::Denied {
        reason,
        correlation,
    });
    state.shutdown.notify_one();
    Html("<p>Space link declined. You can return to your terminal.</p>").into_response()
}

fn browser_link_state_path(root: &std::path::Path) -> PathBuf {
    root.join(BROWSER_LINK_STATE_FILE)
}

fn load_browser_link_state(root: &std::path::Path) -> Result<Option<BrowserLinkState>> {
    let path = browser_link_state_path(root);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read link recovery at {}", path.display()));
        }
    };
    let state: BrowserLinkState =
        serde_json::from_slice(&bytes).context("local-space link recovery is malformed")?;
    anyhow::ensure!(
        state.version == BROWSER_LINK_STATE_VERSION,
        "unsupported local-space link recovery version {}",
        state.version
    );
    Ok(Some(state))
}

/// Whether this local site completed browser-owned publication and should
/// continue proving remote requests with its own space authority.
///
/// The browser deliberately gives the CLI no account-wide grant. This durable
/// completion receipt selects the already-retained local signer path; pending
/// recovery metadata never changes authorization behavior.
pub fn uses_local_authority(root: &std::path::Path) -> Result<bool> {
    Ok(load_browser_link_state(root)?.is_some_and(|state| state.browser_published))
}

fn save_browser_link_state(root: &std::path::Path, state: &BrowserLinkState) -> Result<()> {
    crate::connections::atomic_public(
        root,
        BROWSER_LINK_STATE_FILE,
        &serde_json::to_vec_pretty(state)?,
    )
    .context("failed to save local-space link recovery")
}

fn lock_browser_link(root: &std::path::Path) -> Result<BrowserLinkLock> {
    let path = root.join(BROWSER_LINK_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open local-space link lock at {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.try_lock()
        .context("another local-space link is already running for this space")?;
    Ok(BrowserLinkLock { _file: file })
}

fn validate_browser_link_state(
    state: &BrowserLinkState,
    name: &str,
    subject: &Did,
    service: &tonk_invite::local_space_link::TrustedService,
) -> Result<Did> {
    anyhow::ensure!(
        state.name == name && state.space == subject.as_str(),
        "link recovery belongs to a different local space"
    );
    anyhow::ensure!(
        state.service_did == service.did.as_str() && state.service_url == service.url.as_str(),
        "link recovery belongs to a different Tonk service"
    );
    state
        .account
        .parse()
        .context("link recovery contains an invalid account DID")
}

/// A named, locally controlled, local-only space ready to request browser
/// adoption. Constructing this value performs no account or network work.
pub struct LocalSpaceLinkCandidate {
    site: crate::site::TonkSite,
    name: String,
    subject: Did,
}

impl LocalSpaceLinkCandidate {
    /// Exact registry name resolved for this handoff.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Existing repository DID, which must survive the ownership transition.
    pub fn subject(&self) -> &Did {
        &self.subject
    }

    /// Sign a browser request with the existing repository identity.
    pub async fn request(
        &self,
        recipient: &Did,
        callback: url::Url,
        correlation: String,
        service: &tonk_invite::local_space_link::TrustedService,
        now: dialog_ucan_core::time::Timestamp,
    ) -> Result<tonk_invite::local_space_link::LocalSpaceLinkRequest> {
        let Some(dialog_credentials::Signer::Ed25519(owner)) =
            self.site.repository.credential().signer()
        else {
            bail!("this device cannot sign for the selected local space")
        };
        tonk_invite::local_space_link::LocalSpaceLinkRequest::issue(
            owner,
            recipient,
            callback,
            correlation,
            self.name.clone(),
            service,
            now,
        )
        .await
    }
}

/// Resolve and prove one new local-space link independently of ambient
/// selected-space or cached-account state.
pub async fn prepare_local_space_link(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
) -> Result<LocalSpaceLinkCandidate> {
    prepare_local_space_link_with_state(store, config, name, None).await
}

async fn prepare_local_space_link_with_state(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
    recovery: Option<&BrowserLinkState>,
) -> Result<LocalSpaceLinkCandidate> {
    let registry = store.load()?;
    let entry = registry
        .spaces
        .get(name)
        .with_context(|| format!("unknown space '{name}'"))?
        .clone();
    if entry.connection.is_some() || crate::connections::binding_at(&entry.site)?.is_some() {
        bail!("scoped connection access cannot be adopted as account ownership");
    }
    let mut site_config = config.clone();
    site_config.require_account = false;
    site_config.provision_account_spaces = false;
    let site = crate::site::TonkSite::open_with(&entry.site, site_config).await?;
    if site.is_scoped() {
        bail!("scoped connection access cannot be adopted as account ownership");
    }
    let subject = site.repository.did();
    let expected_account = match recovery {
        Some(state) => Some(
            state
                .account
                .parse::<Did>()
                .context("link recovery contains an invalid account DID")?,
        ),
        None => None,
    };
    let roster = crate::inventory::read_roster(&site).await?;
    if let Some(founder) = roster.founder()
        && expected_account.as_ref().map(Did::as_str) != Some(founder.did.as_str())
    {
        bail!(already_owned_message(name, &founder.did));
    }
    match configured_upstream(&site).await? {
        None => {}
        Some(Upstream::Remote { remote, branch, .. })
            if recovery.is_some()
                && branch == crate::site::BRANCH_NAME
                && crate::remote::find(&site, &remote)
                    .await?
                    .is_some_and(|record| {
                        recovery.is_some_and(|state| record.endpoint == state.service_url)
                    }) => {}
        Some(_) => {
            bail!("only a local-only space with no content upstream can be linked to an account")
        }
    }
    let profile_proof = site
        .profile
        .access()
        .prove(Subject::from(subject.clone()))
        .perform(site.operator.local())
        .await;
    if let Err(error) = profile_proof
        && site.repository.credential().signer().is_none()
    {
        return Err(error).context("this device cannot prove authority over this space");
    }
    let invitations = all_invitations(&site).await?;
    match recovery.and_then(|state| state.invite.as_deref()) {
        Some(invite_url) => {
            let invite = tonk_invite::Invite::parse_url(invite_url)
                .await
                .context("stored local-space invite is invalid")?;
            let expected = Invitation::from_chain(&invite.chain)
                .context("stored local-space invite has no subject")?;
            anyhow::ensure!(
                invitations.iter().all(|invitation| invitation == &expected),
                "a space with another recorded share cannot finish linking"
            );
        }
        None => anyhow::ensure!(
            invitations.is_empty(),
            "a space with recorded shares cannot be linked to an account"
        ),
    }
    let ours = crate::site::Identity::of(&site).await?;
    if roster.members.iter().any(|member| {
        !ours.local_dids().any(|did| did == member.did)
            && expected_account.as_ref().map(Did::as_str) != Some(member.did.as_str())
    }) {
        bail!("a space with another durable member cannot be linked to an account");
    }
    Ok(LocalSpaceLinkCandidate {
        site,
        name: name.to_owned(),
        subject,
    })
}

/// Link one local-only space through explicit browser account selection.
pub async fn execute_browser(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
    options: &BrowserLinkOptions,
) -> Result<LinkOutcome> {
    let page = crate::handoff::approval_page(options.via.as_deref())?;
    let defaults = crate::deployment::discover(&page).await?;
    let service = tonk_invite::local_space_link::TrustedService::new(
        defaults
            .service_did
            .as_deref()
            .context("deployment configuration has no service identity")?
            .parse()
            .context("deployment service DID is invalid")?,
        defaults.access_remote.clone(),
    )?;
    let registered_root = store
        .load()?
        .spaces
        .get(name)
        .with_context(|| format!("unknown space '{name}'"))?
        .site
        .clone();
    let initial_recovery = load_browser_link_state(&registered_root)?;
    let candidate =
        prepare_local_space_link_with_state(store, config, name, initial_recovery.as_ref()).await?;
    let _lock = lock_browser_link(&candidate.site.root)?;
    let mut recovery = load_browser_link_state(&candidate.site.root)?;
    let candidate =
        prepare_local_space_link_with_state(store, config, name, recovery.as_ref()).await?;
    if let Some(state) = recovery.as_ref() {
        validate_browser_link_state(state, name, candidate.subject(), &service)?;
        if state.browser_published {
            return finish_browser_publication(store, config, name, &service, state).await;
        }
    }
    let callback = BrowserCallback::bind().await?;
    let callback_url = callback.url.clone();
    let recipient = dialog_credentials::Ed25519Signer::generate().await?;
    let correlation = hex::encode(rand::random::<[u8; 32]>());
    let request_wire = candidate
        .request(
            &recipient.did(),
            callback_url,
            correlation.clone(),
            &service,
            dialog_ucan_core::time::Timestamp::now(),
        )
        .await?;
    let request_encoded =
        tonk_invite::local_space_link::encode_transport(&request_wire.to_bytes()?)?;
    let request = request_wire
        .validate(&service, dialog_ucan_core::time::Timestamp::now())
        .await?;
    let mut approval_url: url::Url = page.parse()?;
    approval_url.set_fragment(None);
    approval_url.set_query(None);
    approval_url
        .query_pairs_mut()
        .append_pair("intent", "local-space-link")
        .append_pair("request", &request_encoded);
    let approval_url = approval_url.to_string();
    let (mut messages, server) = callback.serve();

    println!("Approve this space in Tonk:\n{approval_url}");
    if options.open_browser && webbrowser::open(&approval_url).is_err() {
        eprintln!("warning: could not open a browser; use the URL above");
    }

    let mut approved: Option<BrowserApproval> = None;
    let mut replay = tonk_invite::local_space_link::LocalSpaceLinkReplayGuard::default();
    let outcome = tokio::time::timeout(Duration::from_secs(5 * 60), async {
        loop {
            let message = messages
                .recv()
                .await
                .context("the browser callback closed without an outcome")?;
            match message {
                BrowserMessage::Approval {
                    encoded,
                    correlation: received,
                    continuation,
                } => {
                    if let Err(error) =
                        tonk_invite::local_space_link::validate_cancellation(&request, &received)
                    {
                        let _ = continuation.send(Err(error.to_string()));
                        continue;
                    }
                    let result = async {
                        let bytes = tonk_invite::local_space_link::decode_transport(&encoded)
                            .context("browser approval is not valid base58")?;
                        let expected_account = recovery
                            .as_ref()
                            .map(|state| state.account.parse::<Did>())
                            .transpose()
                            .context("link recovery contains an invalid account DID")?;
                        let approval =
                            tonk_invite::local_space_link::LocalSpaceLinkApproval::from_bytes(
                                &bytes,
                            )?
                            .validate(
                                &request,
                                approved
                                    .as_ref()
                                    .map(|approval| &approval.validated.account)
                                    .or(expected_account.as_ref()),
                                dialog_ucan_core::time::Timestamp::now(),
                            )
                            .await?;
                        replay.consume(&approval)?;
                        let rechecked = prepare_local_space_link_with_state(
                            store,
                            config,
                            name,
                            recovery.as_ref(),
                        )
                        .await?;
                        anyhow::ensure!(
                            rechecked.subject() == &request.space,
                            "the selected space changed while awaiting approval"
                        );
                        let mut state = match recovery.clone() {
                            Some(state) => {
                                validate_browser_link_state(
                                    &state,
                                    name,
                                    rechecked.subject(),
                                    &service,
                                )?;
                                anyhow::ensure!(
                                    state.account == approval.account.as_str(),
                                    "this space is already pending approval for another account"
                                );
                                state
                            }
                            None => BrowserLinkState {
                                version: BROWSER_LINK_STATE_VERSION,
                                name: name.to_owned(),
                                space: rechecked.subject().to_string(),
                                account: approval.account.to_string(),
                                service_did: service.did.to_string(),
                                service_url: service.url.to_string(),
                                consent: None,
                                invite: None,
                                browser_published: false,
                            },
                        };
                        // The account choice is durable before minting any
                        // space authority, so an interrupted retry cannot pick
                        // a different owner.
                        save_browser_link_state(&rechecked.site.root, &state)?;
                        let consent = match state.consent.clone() {
                            Some(consent) => consent,
                            None => {
                                let prefix = crate::site::direct_account_root_prefix(
                                    &rechecked.site,
                                    &approval.account,
                                )
                                .await?;
                                let consent = tonk_invite::local_space_link::encode_transport(
                                    &prefix.to_bytes()?,
                                )?;
                                state.consent = Some(consent.clone());
                                save_browser_link_state(&rechecked.site.root, &state)?;
                                consent
                            }
                        };
                        let invite_url = match state.invite.clone() {
                            Some(invite) => invite,
                            None => {
                                let base = defaults.ceremony_origin.join("/join")?;
                                let invite = crate::invite::mint_targeted_unrecorded(
                                    &rechecked.site,
                                    Some(base.as_str()),
                                    Some(service.url.as_str()),
                                    approval.account.as_str(),
                                )
                                .await
                                .map_err(anyhow::Error::new)?;
                                state.invite = Some(invite.url.clone());
                                save_browser_link_state(&rechecked.site.root, &state)?;
                                invite.url
                            }
                        };
                        recovery = Some(state);
                        let mut continuation_url: url::Url = page.parse()?;
                        continuation_url.set_fragment(None);
                        continuation_url.set_query(None);
                        continuation_url
                            .query_pairs_mut()
                            .append_pair("intent", "local-space-link")
                            .append_pair("request", &request_encoded)
                            .append_pair("approval", &encoded)
                            .append_pair("consent", &consent)
                            .append_pair("invite", &invite_url);
                        Ok::<_, anyhow::Error>((
                            BrowserApproval {
                                validated: approval,
                                encoded,
                                invite: invite_url,
                            },
                            continuation_url.to_string(),
                        ))
                    }
                    .await;
                    match result {
                        Ok((approval, target)) => {
                            let _ = continuation.send(Ok(target));
                            approved = Some(approval);
                        }
                        Err(error) => {
                            let detail = format!("local-space link approval failed: {error:#}");
                            let _ = continuation.send(Err(detail));
                        }
                    }
                }
                BrowserMessage::Provisioned {
                    encoded,
                    correlation: received,
                    continuation,
                } => {
                    let result = async {
                        tonk_invite::local_space_link::validate_cancellation(&request, &received)?;
                        let approval = approved
                            .as_ref()
                            .context("browser provisioned a link that was not approved")?;
                        let bytes = tonk_invite::local_space_link::decode_transport(&encoded)
                            .context("browser provisioning receipt is not valid base58")?;
                        let receipt =
                            tonk_invite::local_space_link::LocalSpaceLinkCompletion::from_bytes(
                                &bytes,
                            )?
                            .validate(
                                &approval.validated,
                                dialog_ucan_core::time::Timestamp::now(),
                            )
                            .await?;
                        anyhow::ensure!(
                            receipt.publication == "provisioned",
                            "browser returned the wrong publication stage"
                        );
                        let state = recovery
                            .as_ref()
                            .context("approved link recovery was not saved")?;
                        publish_browser_local(store, config, name, &service, state).await?;
                        let mut continuation_url: url::Url = page.parse()?;
                        continuation_url.set_fragment(None);
                        continuation_url.set_query(None);
                        continuation_url
                            .query_pairs_mut()
                            .append_pair("intent", "local-space-link")
                            .append_pair("request", &request_encoded)
                            .append_pair("approval", &approval.encoded)
                            .append_pair("invite", &approval.invite)
                            .append_pair("provisioned", &encoded);
                        Ok::<_, anyhow::Error>(continuation_url.to_string())
                    }
                    .await;
                    match result {
                        Ok(target) => {
                            let _ = continuation.send(Ok(target));
                        }
                        Err(error) => {
                            let detail = format!("local-space publication failed: {error:#}");
                            let _ = continuation.send(Err(detail));
                        }
                    }
                }
                BrowserMessage::Completion {
                    encoded,
                    correlation: received,
                } => {
                    tonk_invite::local_space_link::validate_cancellation(&request, &received)?;
                    let approval = approved
                        .as_ref()
                        .context("browser completed a link that was not approved")?;
                    let bytes = tonk_invite::local_space_link::decode_transport(&encoded)
                        .context("browser completion is not valid base58")?;
                    let completion =
                        tonk_invite::local_space_link::LocalSpaceLinkCompletion::from_bytes(
                            &bytes,
                        )?
                        .validate(
                            &approval.validated,
                            dialog_ucan_core::time::Timestamp::now(),
                        )
                        .await?;
                    let state = save_browser_completion(
                        &candidate.site.root,
                        recovery
                            .as_ref()
                            .context("approved link recovery was not saved")?,
                        &approval.validated,
                        &approval.invite,
                        &completion,
                    )?;
                    recovery = Some(state.clone());
                    break finish_browser_publication(store, config, name, &service, &state).await;
                }
                BrowserMessage::Denied {
                    reason,
                    correlation: received,
                } => {
                    tonk_invite::local_space_link::validate_cancellation(&request, &received)?;
                    bail!("space link declined: {reason}");
                }
            }
        }
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "timed out waiting for local-space approval; rerun `tonk space link {name}` to resume"
        )
    })?;
    server.abort();
    outcome
}

fn save_browser_completion(
    root: &std::path::Path,
    recovery: &BrowserLinkState,
    approval: &tonk_invite::local_space_link::ValidatedLocalSpaceLinkApproval,
    invite: &str,
    completion: &tonk_invite::local_space_link::ValidatedLocalSpaceLinkCompletion,
) -> Result<BrowserLinkState> {
    anyhow::ensure!(
        approval.account == completion.account,
        "account changed before publication"
    );
    anyhow::ensure!(
        completion.space == approval.request.space,
        "space changed before publication"
    );
    anyhow::ensure!(
        recovery.invite.as_deref() == Some(invite),
        "approved invite changed before publication"
    );
    anyhow::ensure!(
        completion.publication == format!("{}:{}", completion.space.repo_key(), completion.space),
        "browser returned the wrong publication stage"
    );
    let mut state = recovery.clone();
    state.browser_published = true;
    save_browser_link_state(root, &state)?;
    Ok(state)
}

async fn finish_browser_publication(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
    service: &tonk_invite::local_space_link::TrustedService,
    state: &BrowserLinkState,
) -> Result<LinkOutcome> {
    anyhow::ensure!(
        state.browser_published,
        "browser publication is not complete"
    );
    publish_browser_local(store, config, name, service, state).await
}

async fn publish_browser_local(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
    service: &tonk_invite::local_space_link::TrustedService,
    state: &BrowserLinkState,
) -> Result<LinkOutcome> {
    let candidate = prepare_local_space_link_with_state(store, config, name, Some(state)).await?;
    let account = validate_browser_link_state(state, name, &candidate.subject, service)?;
    let invite_url = state
        .invite
        .as_deref()
        .context("link recovery has no targeted space authority")?;
    let invite = tonk_invite::Invite::parse_url(invite_url).await?;
    anyhow::ensure!(
        matches!(invite.audience, tonk_invite::InviteAudience::Scoped)
            && invite.subject() == &candidate.subject
            && invite.chain.audience() == &account
            && invite.remote_url.as_ref() == Some(&service.url),
        "approved space authority does not match the completed link"
    );
    let already_linked = crate::inventory::read_roster(&candidate.site)
        .await?
        .founder()
        .is_some();
    publication_stage(PublicationStage::Founder, async {
        candidate
            .site
            .profile
            .access()
            .save(UcanDelegation(invite.chain.clone()))
            .perform(candidate.site.operator.local())
            .await
            .context("failed to retain approved space authority")?;
        let _ = crate::site::account_root_prefix(&candidate.site, &account).await?;
        crate::invite::record_invitation(&candidate.site, &invite.chain, &invite.audience)
            .await
            .map_err(anyhow::Error::new)?;
        crate::site::record_founder_membership_for(&candidate.site, account.clone()).await
    })
    .await
    .map_err(anyhow::Error::new)?;
    publication_stage(
        PublicationStage::Remote,
        ensure_remote(
            &candidate.site,
            DEFAULT_REMOTE,
            service.url.as_str(),
            &candidate.subject,
        ),
    )
    .await
    .map_err(anyhow::Error::new)?;
    publication_stage(
        PublicationStage::Upstream,
        ensure_upstream(&candidate.site, DEFAULT_REMOTE),
    )
    .await
    .map_err(anyhow::Error::new)?;
    publication_stage(PublicationStage::Push, async {
        crate::sync::push(&candidate.site)
            .await
            .map_err(anyhow::Error::new)
    })
    .await
    .map_err(anyhow::Error::new)?;
    Ok(LinkOutcome {
        subject: candidate.subject.to_string(),
        name: candidate.name,
        site: candidate.site.root,
        account: account.to_string(),
        already_linked,
    })
}

/// Successful link result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOutcome {
    /// Repository subject, unchanged by the link.
    pub subject: String,
    /// Registered space name, unchanged by the link.
    pub name: String,
    /// Local site directory, unchanged by the link.
    pub site: PathBuf,
    /// Account root the space now belongs to.
    pub account: String,
    /// Whether the space already belonged to this account.
    pub already_linked: bool,
}

/// Stable checkpoints in account publication of an already-local space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationStage {
    /// Record the signed account as founder and provision its authority.
    Founder,
    /// Ensure the account's content service is registered under the selected remote.
    Remote,
    /// Point the content and metadata branches at the selected remote.
    Upstream,
    /// Publish content and metadata to the configured service.
    Push,
    /// Retain custody/authority and publish the account directory entry.
    AccountDirectory,
}

impl PublicationStage {
    /// Stable stage identifier used in recovery output and fault tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Founder => "founder",
            Self::Remote => "remote",
            Self::Upstream => "upstream",
            Self::Push => "push",
            Self::AccountDirectory => "accountDirectory",
        }
    }
}

impl std::fmt::Display for PublicationStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A locally registered space whose account publication stopped at one stage.
#[derive(Debug, Error)]
#[error("stage '{stage}' failed: {source}")]
pub struct PublicationError {
    /// Exact idempotent publication stage that did not settle successfully.
    pub stage: PublicationStage,
    /// Underlying failure at that stage.
    #[source]
    pub source: anyhow::Error,
}

/// Explain why an account-owned space cannot be linked somewhere else.
pub fn already_owned_message(name: &str, owner: &str) -> String {
    format!(
        "\"{name}\" already belongs to an account, so it stays there.\n\n\
         Once a space is synced with an account, it stays owned by that account.\n\
         This keeps existing shares working.\n\n\
         Share it instead:\n  tonk invite\n\n\
         owner account: {owner}"
    )
}

/// Link one genuinely local-only space into the signed-in account.
pub async fn execute(store: &SpaceStore, config: &SiteConfig, name: &str) -> Result<LinkOutcome> {
    let prepared = prepare(store, config, name).await?;
    publish(store, name, prepared)
        .await
        .map_err(anyhow::Error::new)
}

/// Finish publication for a freshly created, already registered local space.
///
/// Once `space::create` returns, the local site, DID, registry entry, and
/// directory binding are durable. Every later failure is therefore returned
/// as a typed partial outcome and is safe to continue with [`execute`].
pub async fn publish_created(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
) -> std::result::Result<LinkOutcome, PublicationError> {
    let prepared = prepare(store, config, name)
        .await
        .map_err(|source| PublicationError {
            stage: PublicationStage::Founder,
            source,
        })?;
    publish(store, name, prepared).await
}

struct PreparedPublication {
    site: crate::site::TonkSite,
    subject: Did,
    account_root: Did,
    account: String,
    access: String,
    remote: String,
    already_linked: bool,
}

async fn prepare(
    store: &SpaceStore,
    config: &SiteConfig,
    name: &str,
) -> Result<PreparedPublication> {
    let registry = store.load()?;
    let entry = registry
        .spaces
        .get(name)
        .with_context(|| format!("unknown space '{name}'"))?
        .clone();
    if entry.connection.is_some() || crate::connections::binding_at(&entry.site)?.is_some() {
        bail!("scoped connection access cannot be adopted as account ownership");
    }
    let account = registry
        .account
        .clone()
        .context("no account is signed in; run `tonk account login` first")?;
    let account_root: Did = account
        .root
        .parse()
        .context("the signed-in account root is invalid")?;

    let mut site_config = config.clone();
    site_config.require_account = false;
    let site = crate::site::TonkSite::open_with(&entry.site, site_config).await?;
    if site.is_scoped() {
        bail!("scoped connection access cannot be adopted as account ownership");
    }
    let subject = site.repository.did();

    // Ownership is the space's own answer, not the registry's, and it is
    // settled before anything about this account's hosting is consulted: who
    // a space belongs to does not depend on where we would have put it. A
    // founder row for somebody else is final; a founder row for us is only
    // finished once the chain behind it is here too, so an interrupted run
    // resumes instead of reporting a link that never completed.
    let roster = crate::inventory::read_roster(&site).await?;
    let mut already_linked = false;
    if let Some(founder) = roster.founder() {
        if founder.did != account.root {
            bail!(already_owned_message(name, &founder.did));
        }
        if holds_account_chain(&site, &subject, &account_root).await {
            already_linked = true;
        }
    }

    match crate::account::status_in(&site.profile, store).await? {
        crate::account::AccountStatus::Registered { root_did, .. } if root_did == account.root => {}
        _ => bail!("this account is signed out; run `tonk account login` first"),
    }

    let access = account
        .access_remote
        .clone()
        .context("the account has no content endpoint; sign in again")?;
    let remote = preflight(&site, &roster, &access, already_linked)
        .await?
        .unwrap_or_else(|| DEFAULT_REMOTE.to_owned());

    Ok(PreparedPublication {
        site,
        subject,
        account_root,
        account: account.root,
        access,
        remote,
        already_linked,
    })
}

async fn publish(
    store: &SpaceStore,
    name: &str,
    prepared: PreparedPublication,
) -> std::result::Result<LinkOutcome, PublicationError> {
    let PreparedPublication {
        site,
        subject,
        account_root,
        account,
        access,
        remote,
        already_linked,
    } = prepared;

    // Authority first: the account root can only host what it can prove it
    // was given, and this is the one boundary allowed to mint that grant.
    let prefix = publication_stage(PublicationStage::Founder, async {
        let prefix = crate::site::account_root_prefix(&site, &account_root).await?;
        crate::customer::provision_in(&site.profile, store, &subject, &prefix).await?;
        crate::site::record_founder_membership(&site).await?;
        Ok(prefix)
    })
    .await?;

    publication_stage(
        PublicationStage::Remote,
        ensure_remote(&site, &remote, &access, &subject),
    )
    .await?;
    publication_stage(PublicationStage::Upstream, ensure_upstream(&site, &remote)).await?;
    publication_stage(PublicationStage::Push, async {
        crate::sync::push(&site).await?;
        Ok(())
    })
    .await?;

    publication_stage(PublicationStage::AccountDirectory, async {
        let operator =
            crate::account_state::credential_operator_for_store(&site.profile, store).await?;
        let Some(account_branch) =
            crate::account_state::open_account_branch_in(&site.profile, &operator, store).await?
        else {
            bail!("the account repository is not ready to hold this space");
        };
        crate::account_state::retain_space_delegation_in(&site.profile, &operator, store, &prefix)
            .await?;
        // The seed rides the same boundary: a space the account hosts is a
        // space the account can re-derive. Sealing needs only the published
        // public key; an account that predates it links anyway — custody
        // catches up at the next `tonk account login`.
        if !crate::custody::has_custody(&account_branch, &subject, &operator).await? {
            match crate::custody::account_recipient(&account_branch, &account_root, &operator)
                .await?
            {
                Some(recipient) => {
                    if let Some(seed) = crate::custody::site_seed(&site).await? {
                        crate::custody::custody_space_seed(
                            &account_branch,
                            &subject,
                            &recipient,
                            &seed,
                            &operator,
                        )
                        .await?;
                    }
                }
                None => eprintln!(
                    "warning: the account has not published its encryption key; \
                     the space seed stays uncustodied until it does"
                ),
            }
        }
        crate::account_spaces::record_site_pushed(name, &site, store).await?;

        if crate::inventory::role_for_site(&site).await? != SpaceRole::Owner {
            bail!("the space is not signed as owned by this device after linking");
        }
        Ok(())
    })
    .await?;

    Ok(LinkOutcome {
        subject: subject.to_string(),
        name: name.to_owned(),
        site: site.root,
        account,
        already_linked,
    })
}

async fn ensure_remote(
    site: &crate::site::TonkSite,
    name: &str,
    access: &str,
    subject: &Did,
) -> Result<()> {
    crate::remote::ensure(site, name, access, subject.clone()).await?;
    Ok(())
}

async fn ensure_upstream(site: &crate::site::TonkSite, expected_remote: &str) -> Result<()> {
    match configured_upstream(site).await? {
        Some(Upstream::Remote { remote, branch, .. })
            if remote == expected_remote && branch == crate::site::BRANCH_NAME => {}
        Some(Upstream::Remote { remote, branch, .. }) => bail!(
            "the space already tracks '{remote}/{branch}'; refusing to replace it with \
             '{expected_remote}/{main}'",
            main = crate::site::BRANCH_NAME,
        ),
        Some(Upstream::Local { branch, .. }) => bail!(
            "the space already tracks local branch '{branch}'; refusing to replace it with \
             '{expected_remote}/{main}'",
            main = crate::site::BRANCH_NAME,
        ),
        None => {}
    }
    // Re-run even when main already points at origin: an interrupted prior
    // attempt may still need to wire metadata or assert its tracking record.
    crate::remote::set_upstream(site, expected_remote).await?;
    Ok(())
}

async fn configured_upstream(site: &crate::site::TonkSite) -> Result<Option<Upstream>> {
    let session = site
        .branch()
        .await
        .context("failed to inspect the space's upstream")?;
    Ok(session.handle().upstream())
}

async fn publication_stage<T>(
    stage: PublicationStage,
    future: impl Future<Output = Result<T>>,
) -> std::result::Result<T, PublicationError> {
    let outcome = future
        .await
        .map_err(|source| PublicationError { stage, source })?;
    inject_failure_after(stage)?;
    Ok(outcome)
}

#[cfg(feature = "integration-tests")]
fn inject_failure_after(stage: PublicationStage) -> std::result::Result<(), PublicationError> {
    const ENV: &str = "TONK_TEST_SPACE_NEW_FAIL_STAGE";
    if std::env::var(ENV).ok().as_deref() == Some(stage.as_str()) {
        return Err(PublicationError {
            stage,
            source: anyhow::anyhow!(
                "injected failure after the stage completed; its outcome must be treated as unknown"
            ),
        });
    }
    Ok(())
}

#[cfg(not(feature = "integration-tests"))]
fn inject_failure_after(_stage: PublicationStage) -> std::result::Result<(), PublicationError> {
    Ok(())
}

/// Whether the retained `subject → … → account root` chain is on this device.
///
/// The chain is what the access service validates and the roster is its
/// legible, synced mirror, so a founder row with no chain behind it is an
/// unfinished link rather than an ownership claim. This reads only what the
/// profile already holds — it never mints, so it cannot answer yes by
/// establishing the ownership it was asked to confirm.
async fn holds_account_chain(
    site: &crate::site::TonkSite,
    subject: &Did,
    account_root: &Did,
) -> bool {
    crate::site::load_account_root_prefix_for(
        &site.profile,
        site.operator.local(),
        subject,
        account_root,
    )
    .await
    .is_ok()
}

/// Refuse anything that is not genuinely local-only.
///
/// An upstream already pointing at this account's own content service is the
/// one exception: that is what a half-finished link leaves behind, and a
/// retry has to be able to get past it.
async fn preflight(
    site: &crate::site::TonkSite,
    roster: &Roster,
    access: &str,
    already_linked: bool,
) -> Result<Option<String>> {
    let existing_remote = match configured_upstream(site).await? {
        Some(Upstream::Remote { remote, branch, .. }) if branch == crate::site::BRANCH_NAME => {
            let endpoint = crate::remote::find(site, &remote)
                .await?
                .map(|record| record.endpoint);
            if endpoint.as_deref() != Some(access) {
                bail!(
                    "only a local-only space with no content upstream, or an interrupted \
                     link to this account's content endpoint, can be linked to an account"
                );
            }
            Some(remote)
        }
        Some(_) => bail!(
            "only a local-only space with no content upstream, or an interrupted link to \
             this account's content endpoint, can be linked to an account"
        ),
        None => None,
    };
    // Once founder ownership and its retained account chain agree, this is no
    // longer an ownership transition. Shares and members created afterwards
    // are expected; a retry only needs to finish the idempotent hosting and
    // account-directory steps below. Keep the upstream check above so a retry
    // never silently republishes a space through a different service.
    if already_linked {
        return Ok(existing_remote);
    }
    let profile_proof = site
        .profile
        .access()
        .prove(Subject::from(site.repository.did().clone()))
        .perform(site.operator.local())
        .await;
    if let Err(error) = profile_proof
        && site.repository.credential().signer().is_none()
    {
        return Err(error).context("this device cannot prove authority over this space");
    }
    if has_invitations(site).await? {
        bail!("a space with recorded shares cannot be linked to an account");
    }
    // Every identity this installation could have written a row under — the
    // account, the local root, the profile — counts as us; anything else is
    // a member this link would silently carry into the account.
    let ours = crate::site::Identity::of(site).await?;
    if roster
        .members
        .iter()
        .any(|member| !ours.dids().any(|did| did == member.did))
    {
        bail!("a space with another durable member cannot be linked to an account");
    }
    Ok(existing_remote)
}

/// Whether the space records any share it has already handed out.
///
/// Reads the content branch, where the worker writes invitations, and the
/// meta branch, where CLI releases through this one wrote them.
async fn has_invitations(site: &crate::site::TonkSite) -> Result<bool> {
    Ok(!all_invitations(site).await?.is_empty())
}

async fn all_invitations(site: &crate::site::TonkSite) -> Result<Vec<Invitation>> {
    let content = site
        .branch()
        .await
        .context("failed to inspect the space's shares")?;
    let mut invitations = invitations_on(site, content.handle()).await?;
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .context("failed to inspect local-space metadata")?;
    invitations.extend(invitations_on(site, &meta).await?);
    Ok(invitations)
}

async fn invitations_on(site: &crate::site::TonkSite, branch: &Branch) -> Result<Vec<Invitation>> {
    Ok(branch
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::from(site.repository.did().this()),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?)
}

#[cfg(test)]
mod local_space_link_tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use dialog_ucan_core::time::Timestamp;

    fn config(root: &std::path::Path, store: SpaceStore) -> SiteConfig {
        let profile = root.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        SiteConfig {
            profile_name: format!("local-space-link-{:x}", rand::random::<u64>()),
            profile_directory: Directory::At(profile.to_string_lossy().into_owned()),
            require_account: false,
            provision_account_spaces: false,
            account_store: store,
        }
    }

    #[dialog_common::test]
    async fn local_space_link_resolves_the_named_local_space_and_validates_consent() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = SpaceStore::at(temp.path().join("state"));
        let config = config(temp.path(), store.clone());
        crate::space::create(&store, "other", None, None, config.clone()).await?;
        crate::space::create(&store, "garden", None, None, config.clone()).await?;
        let unrelated =
            crate::space::AccountRecord::new(Ed25519Signer::generate().await?.did().to_string());
        store.set_account(Some(unrelated.clone()))?;
        assert_eq!(store.load()?.account, Some(unrelated.clone()));
        let candidate = prepare_local_space_link(&store, &config, "garden").await?;
        assert_eq!(candidate.name(), "garden");
        assert_eq!(
            candidate.subject().to_string(),
            crate::site::TonkSite::open_with(&store.load()?.spaces["garden"].site, config.clone())
                .await?
                .repository
                .did()
                .to_string()
        );

        let recipient = Ed25519Signer::generate().await?;
        let account = Ed25519Signer::generate().await?;
        let consent =
            crate::site::direct_account_root_prefix(&candidate.site, &account.did()).await?;
        assert_eq!(consent.proofs().count(), 1);
        assert_eq!(consent.subject(), Some(candidate.subject()));
        assert_eq!(consent.audience(), &account.did());
        let service_signer = Ed25519Signer::generate().await?;
        let service = tonk_invite::local_space_link::TrustedService::new(
            service_signer.did(),
            "https://access.example/ucan/".parse()?,
        )?;
        let request = candidate
            .request(
                &recipient.did(),
                "http://127.0.0.1:45123/link".parse()?,
                "0123456789abcdef0123456789abcdef".into(),
                &service,
                Timestamp::now(),
            )
            .await?;
        let request = request.validate(&service, Timestamp::now()).await?;
        let approval = tonk_invite::local_space_link::LocalSpaceLinkApproval::issue(
            &request,
            &account,
            Timestamp::now(),
        )
        .await?;
        let approval = approval
            .validate(&request, Some(&account.did()), Timestamp::now())
            .await?;
        let state = BrowserLinkState {
            version: BROWSER_LINK_STATE_VERSION,
            name: "garden".into(),
            space: request.space.to_string(),
            account: account.did().to_string(),
            service_did: service.did.to_string(),
            service_url: service.url.to_string(),
            consent: None,
            invite: Some("approved-invite".into()),
            browser_published: false,
        };
        save_browser_link_state(&candidate.site.root, &state)?;
        let state_path = candidate.site.root.join(BROWSER_LINK_STATE_FILE);
        let before = std::fs::read(&state_path)?;
        for publication in [
            "provisioned".to_owned(),
            "arbitrary-revision".to_owned(),
            format!("{}:{}", request.space.repo_key(), request.space),
        ] {
            let receipt = tonk_invite::local_space_link::LocalSpaceLinkCompletion::issue(
                &approval,
                &account,
                publication.clone(),
                Timestamp::now(),
            )
            .await?
            .validate(&approval, Timestamp::now())
            .await?;
            let result = save_browser_completion(
                &candidate.site.root,
                &state,
                &approval,
                "approved-invite",
                &receipt,
            );
            if publication == "provisioned" || publication == "arbitrary-revision" {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("wrong publication stage")
                );
                assert_eq!(std::fs::read(&state_path)?, before);
            } else {
                assert!(result?.browser_published);
                assert_ne!(std::fs::read(&state_path)?, before);
            }
        }
        let mut replay = tonk_invite::local_space_link::LocalSpaceLinkReplayGuard::default();
        replay.consume(&approval)?;
        assert!(replay.consume(&approval).is_err());
        assert_eq!(store.load()?.account, Some(unrelated));
        Ok(())
    }

    #[dialog_common::test]
    async fn local_space_link_cancellation_and_owned_space_leave_state_unchanged() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = SpaceStore::at(temp.path().join("state"));
        let config = config(temp.path(), store.clone());
        crate::space::create(&store, "garden", None, None, config.clone()).await?;
        let candidate = prepare_local_space_link(&store, &config, "garden").await?;
        let before = std::fs::read(store.registry_path())?;
        let recipient = Ed25519Signer::generate().await?;
        let service_signer = Ed25519Signer::generate().await?;
        let service = tonk_invite::local_space_link::TrustedService::new(
            service_signer.did(),
            "https://access.example/ucan/".parse()?,
        )?;
        let request = candidate
            .request(
                &recipient.did(),
                "http://localhost:45124/link".parse()?,
                "abcdef0123456789abcdef0123456789".into(),
                &service,
                Timestamp::now(),
            )
            .await?
            .validate(&service, Timestamp::now())
            .await?;
        tonk_invite::local_space_link::validate_cancellation(&request, &request.correlation)?;
        assert_eq!(std::fs::read(store.registry_path())?, before);

        crate::site::record_founder_membership(&candidate.site).await?;
        assert!(
            prepare_local_space_link(&store, &config, "garden")
                .await
                .err()
                .expect("owned space must be rejected")
                .to_string()
                .contains("already belongs to an account")
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn local_space_link_recovery_is_same_account_serialized_and_stage_idempotent()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = SpaceStore::at(temp.path().join("state"));
        let config = config(temp.path(), store.clone());
        crate::space::create(&store, "garden", None, None, config.clone()).await?;
        let candidate = prepare_local_space_link(&store, &config, "garden").await?;
        let original_subject = candidate.subject().clone();
        let original_site = candidate.site.root.clone();
        let account = Ed25519Signer::generate().await?.did();
        let other_account = Ed25519Signer::generate().await?.did();
        let service = tonk_invite::local_space_link::TrustedService::new(
            Ed25519Signer::generate().await?.did(),
            "https://access.example/ucan/".parse()?,
        )?;
        assert!(
            !uses_local_authority(&original_site)?,
            "pending or absent link state must not bypass account authority"
        );
        let state = BrowserLinkState {
            version: BROWSER_LINK_STATE_VERSION,
            name: "garden".into(),
            space: original_subject.to_string(),
            account: account.to_string(),
            service_did: service.did.to_string(),
            service_url: service.url.to_string(),
            consent: None,
            invite: None,
            browser_published: true,
        };

        save_browser_link_state(&original_site, &state)?;
        assert_eq!(
            load_browser_link_state(&original_site)?,
            Some(state.clone())
        );
        assert!(
            uses_local_authority(&original_site)?,
            "only a completed browser publication selects local space authority"
        );
        let first_lock = lock_browser_link(&original_site)?;
        assert!(
            lock_browser_link(&original_site)
                .expect_err("a concurrent owner selection must fail")
                .to_string()
                .contains("already running")
        );
        drop(first_lock);
        let _retry_lock = lock_browser_link(&original_site)?;

        crate::site::record_founder_membership_for(&candidate.site, account.clone()).await?;
        assert!(
            prepare_local_space_link(&store, &config, "garden")
                .await
                .is_err()
        );
        let resumed =
            prepare_local_space_link_with_state(&store, &config, "garden", Some(&state)).await?;
        assert_eq!(resumed.subject(), &original_subject);
        assert_eq!(resumed.site.root, original_site);

        ensure_remote(
            &resumed.site,
            DEFAULT_REMOTE,
            service.url.as_str(),
            resumed.subject(),
        )
        .await?;
        ensure_upstream(&resumed.site, DEFAULT_REMOTE).await?;
        let resumed_again =
            prepare_local_space_link_with_state(&store, &config, "garden", Some(&state)).await?;
        assert_eq!(resumed_again.subject(), &original_subject);

        let conflicting = BrowserLinkState {
            account: other_account.to_string(),
            ..state
        };
        assert!(
            prepare_local_space_link_with_state(&store, &config, "garden", Some(&conflicting))
                .await
                .err()
                .expect("a different account cannot take over partial publication")
                .to_string()
                .contains("already belongs to an account")
        );
        Ok(())
    }
}

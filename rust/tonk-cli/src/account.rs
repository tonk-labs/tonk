//! Native account status and browser-assisted device linking.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialog_operator::Profile;
use dialog_storage::provider::storage::NativeSpace;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use tonk_account::{AccountProviderRecord, AccountStateStatus};
use url::Url;

/// Production account API used unless explicitly overridden.
pub const DEFAULT_SERVICE_URL: &str = "https://accounts.tonk.xyz";
/// Production account page, where the revoke ceremony runs.
pub const DEFAULT_ACCOUNT_PAGE: &str = "https://tonk.network/settings";
/// Production link ceremony page: it reads `?audience=` and `?callback=`
/// and posts the grant back to the waiting CLI.
pub const DEFAULT_LINK_PAGE: &str = "https://tonk.network/settings/link";

/// Open the browser's passkey-protected, review-first account deletion flow.
pub async fn open_deletion(
    profile: &Profile,
    account_url: &str,
    open_browser: bool,
) -> Result<String> {
    let status = status(profile).await?;
    if !matches!(status, AccountStatus::Registered { .. }) {
        bail!("no account is linked to this profile");
    }
    let url = format!("{}#delete-account", account_url.trim_end_matches('/'));
    if open_browser && webbrowser::open(&url).is_err() {
        bail!("could not open the account deletion page; open {url}");
    }
    Ok(url)
}

/// Open the browser's passkey-protected review for deleting one owned space.
pub async fn open_space_deletion(
    profile: &Profile,
    account_url: &str,
    subject: &str,
    open_browser: bool,
) -> Result<String> {
    let status = status(profile).await?;
    if !matches!(status, AccountStatus::Registered { .. }) {
        bail!("no account is linked to this profile");
    }
    subject
        .parse::<Did>()
        .context("space subject is not a valid DID")?;
    let mut url = Url::parse(account_url).context("account page URL is invalid")?;
    url.query_pairs_mut().append_pair("delete-space", subject);
    url.set_fragment(Some("delete-account"));
    let url = url.to_string();
    if open_browser && webbrowser::open(&url).is_err() {
        bail!("could not open the space deletion page; open {url}");
    }
    Ok(url)
}
/// Credential-store key for optional provider attachment metadata.
pub const ACCOUNT_LINK_SITE: &str = tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE;

// Linking is complete once credentials are durable; repository hydration is
// best-effort and must not leave the link command waiting indefinitely.

/// How long retryable post-link account synchronization may run before the
/// command reports the durable local lifecycle state and returns.
const POST_LINK_SYNC_DEADLINE: Duration = Duration::from_secs(10);

async fn ensure_after_link(
    profile: &Profile,
    operator: dialog_operator::Operator<NativeSpace>,
    store: crate::space::SpaceStore,
    deadline: Duration,
) -> Result<crate::account_state::EnsureOutcome> {
    let status_operator = operator.clone();
    match tokio::time::timeout(
        deadline,
        crate::account_state::ensure_with_operator_and_store(profile, operator, store.clone()),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => {
            let status =
                crate::account_state::status_with_operator_in(profile, &status_operator, &store)
                    .await?;
            let warning = match status {
                AccountStateStatus::Ready => {
                    "latest account synchronization did not finish within 10 seconds; committed changes will retry"
                }
                AccountStateStatus::Unconfigured | AccountStateStatus::Unhydrated => {
                    "the account repository did not answer within 10 seconds; first sync will retry"
                }
            };
            Ok(crate::account_state::EnsureOutcome {
                status,
                warning: Some(warning.to_string()),
            })
        }
    }
}

/// Descriptive registration name for a native CLI device.
pub fn default_device_name() -> String {
    let info = os_info::get();
    format_device_name(info.os_type(), info.version())
}

fn format_device_name(os_type: os_info::Type, version: &os_info::Version) -> String {
    let os_name = match os_type {
        os_info::Type::Macos => "macOS".to_string(),
        os_info::Type::Unknown => "unknown OS".to_string(),
        os_type => os_type.to_string(),
    };
    let version = match version {
        os_info::Version::Unknown => "(version unknown)".to_string(),
        version => version.to_string(),
    };
    let mut name = format!("Tonk CLI on {os_name} {version}");
    if name.len() > 100 {
        let mut end = 100;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// Current native profile account-link state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStatus {
    /// No provider-neutral local root has been provisioned.
    MissingRoot {
        /// Native device DID.
        device_did: String,
    },
    /// A local root exists with no attached provider.
    Unregistered {
        /// Durable root DID.
        root_did: String,
        /// Native device DID.
        device_did: String,
    },
    /// Optional provider services are attached to the local root.
    Registered {
        /// Durable root DID.
        root_did: String,
        /// Native device DID.
        device_did: String,
        /// Attached provider base URL.
        provider: String,
        /// Configuration/hydration state of the account repository.
        account_state: AccountStateStatus,
    },
}

/// Local sign-in phase, readable without opening a Dialog profile.
pub use crate::account_session::LocalPhase as SignInPhase;

/// Inspect one store's sign-in phase without creating profile state.
///
/// Cheap enough for read-only commands: it reads the session sidecar and
/// nothing else, so bare `tonk space` can say whether an account is signed in
/// without provisioning an identity for an installation that has none.
pub fn sign_in_phase(store: &crate::space::SpaceStore) -> Result<SignInPhase> {
    crate::account_session::inspect_local(store)
}

/// Inputs controlling a browser handoff.
#[derive(Debug, Clone)]
pub struct LinkOptions {
    /// Account API base URL.
    pub service_url: String,
    /// Human-readable name displayed for browser confirmation.
    pub device_name: String,
    /// Whether to ask the OS to open the handoff URL.
    pub open_browser: bool,
    /// Where account state lives. Defaults to the install's own store when
    /// absent; a caller running outside an install supplies its own.
    pub store: Option<crate::space::SpaceStore>,
    /// Send the approval URL here instead of relying on the OS to open it.
    ///
    /// `webbrowser::open` is the product path; this exists for callers that
    /// drive the ceremony themselves and so must see the URL the CLI built,
    /// which carries a callback address only it knows.
    pub announce: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// The browser page that runs the authorization ceremony, when it is
    /// not the default account page (staging or local development).
    ///
    /// Authorization always flows through a loopback callback: the CLI
    /// binds a listener, passes it as `callback=`, and the page posts the
    /// grant straight back — no remote registry, so nothing but the
    /// browser and this process ever holds it.
    pub via: Option<String>,
}

/// Successful browser handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOutcome {
    /// URL printed and optionally opened for the user.
    pub url: String,
    /// Root DID now delegated to this profile.
    pub root_did: String,
    /// Native profile DID.
    pub device_did: String,
    /// Account repository lifecycle after the immediate ensure attempt.
    pub account_state: AccountStateStatus,
    /// Diagnostic when the persisted link remains unhydrated.
    pub warning: Option<String>,
    /// Account service this device attached to.
    ///
    /// The page's own deployment when it named one, and only otherwise the
    /// flag — the same precedence the attachment itself was recorded under,
    /// so a caller matching endpoints against a provider matches the one
    /// this device actually uses.
    pub service_url: String,
}

async fn decode_provider(
    _root_did: &dialog_varsig::Did,
    bytes: Result<Vec<u8>, dialog_effects::credential::CredentialError>,
) -> Result<Option<AccountProviderRecord>> {
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) if crate::account_state::credential_is_missing(&error) => return Ok(None),
        Err(error) => return Err(error).context("failed to load the account provider"),
    };
    AccountProviderRecord::decode(&bytes).context("stored account provider is unusable")
}

/// Load the provider attachment through an already-mounted site operator and
/// caller-supplied profile store.
pub(crate) async fn stored_provider_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<Option<AccountProviderRecord>> {
    stored_provider_for_store(profile, operator, store).await
}

async fn stored_provider_for_store(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<Option<AccountProviderRecord>> {
    {
        let guard = crate::account_session::exclusive_transition_guard(store)?;
        crate::account_session::ensure_initialized(profile, operator, &guard).await?;
    }
    let guard = crate::account_session::shared_remote_guard(store)?;
    if crate::account_session::active_guarded(profile, operator, &guard)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let Some(root) = crate::identity::local_root_with_operator(profile, operator).await? else {
        return Ok(None);
    };
    let root_did = parse_root_did(&root.root_did)?;
    let bytes = profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .load::<Vec<u8>>()
        .perform(operator)
        .await;
    decode_provider(&root_did, bytes).await
}

/// What to tell a device that has no account when it asks for durable
/// authority. Names the one command that provisions both the root and the
/// account it belongs to.
const ACCOUNT_REQUIRED: &str = "A Tonk account is required; run `tonk account login`";

/// Refuse unless one explicit profile store holds an account attachment.
pub(crate) async fn require_account_with_operator_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<()> {
    match stored_provider_in(profile, operator, store).await? {
        Some(_) => Ok(()),
        None => bail!(ACCOUNT_REQUIRED),
    }
}

/// Disconnect provider services while preserving this profile's root,
/// delegations, account repository, and spaces.
pub async fn logout(profile: &Profile) -> Result<()> {
    let operator = crate::account_state::credential_operator(profile).await?;
    logout_with_operator(profile, &operator).await
}

/// Disconnect only the account session owned by `store`.
pub async fn logout_in(profile: &Profile, store: &crate::space::SpaceStore) -> Result<()> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    logout_with_operator_in(profile, &operator, store).await
}

async fn logout_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
) -> Result<()> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    logout_with_operator_in(profile, operator, &store).await
}

async fn logout_with_operator_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for account in
        crate::account_session::logout_transition_for_store(profile, operator, store).await?
    {
        if let Err(error) = crate::account_session::deliver_detach(profile, &account, now).await {
            eprintln!(
                "warning: logged out locally; the provider was not notified                  and may list this device until it is revoked: {error:#}"
            );
        }
    }
    Ok(())
}

fn parse_root_did(root_did: &str) -> Result<dialog_varsig::Did> {
    root_did.parse().context("stored local root DID is invalid")
}

/// Read the current profile's local account status.
///
/// Reads only durable local state. Reporting status must not depend on the
/// account remote being reachable: an offline device is still linked, and a
/// mount failure should be reported as `unhydrated` rather than failing the
/// command. Hydration is [`crate::account_state::ensure`]'s job, and the
/// paths that need it call it directly.
pub async fn status(profile: &Profile) -> Result<AccountStatus> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    status_in(profile, &store).await
}

/// Read account status from one explicit native profile store.
pub async fn status_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<AccountStatus> {
    let device_did = profile.did().to_string();
    let Some(root) = crate::identity::local_root_in(profile, store).await? else {
        return Ok(AccountStatus::MissingRoot { device_did });
    };
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    match stored_provider_in(profile, &operator, store).await? {
        None => Ok(AccountStatus::Unregistered {
            root_did: root.root_did,
            device_did,
        }),
        Some(provider) => {
            let account_state = if provider.remote().is_some() {
                crate::account_state::status_in(profile, store).await?
            } else {
                AccountStateStatus::Unconfigured
            };
            Ok(AccountStatus::Registered {
                root_did: root.root_did,
                device_did,
                provider: provider.provider().to_owned(),
                account_state,
            })
        }
    }
}

/// Validate a delegation the browser returned as an `account → this profile`
/// grant.
///
/// One proof, subject-open (a powerline, so it covers everything the
/// account holds), addressed to this profile, and signed by the issuer it
/// names. Checked before anything is written: a browser-delivered grant
/// arrives over an untrusted path.
pub async fn validate_account_grant(profile: &Profile, bytes: &[u8]) -> Result<DelegationChain> {
    let chain =
        DelegationChain::try_from(bytes).context("authorization is not a delegation container")?;
    if chain.proof_cids().len() != 1 {
        bail!("authorization must carry exactly one delegation");
    }
    if chain.subject().is_some() {
        bail!("authorization must be subject-open, so it covers every space the account holds");
    }
    if chain.audience() != &profile.did() {
        bail!(
            "authorization is addressed to {}, not this profile",
            chain.audience()
        );
    }
    chain
        .proofs()
        .next()
        .context("authorization is missing its proof")?
        .verify_signature(&dialog_credentials::DidKeyResolver)
        .await
        .context("authorization signature is invalid")?;
    Ok(chain)
}

/// Turn one validated browser response into the immutable generation that is
/// checkpointed and replayed after a crash. This performs no local writes.
async fn account_from_callback(
    profile: &Profile,
    options: &LinkOptions,
    authorization: CallbackAuthorization,
) -> Result<crate::account_session::ActiveAccount> {
    let grant_bytes = hex::decode(&authorization.delegation_hex)
        .context("authorization delegation is not hex")?;
    let chain = validate_account_grant(profile, &grant_bytes).await?;
    let account_did = chain.issuer().clone();
    let attachment_id = authorization.attachment_id.trim();
    if attachment_id.is_empty() {
        bail!("authorization is missing its service attachment generation");
    }
    let provider_url = Some(authorization.service_url.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(&options.service_url);
    let remote = authorization.remote.trim().to_owned();
    let attached_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let provider = AccountProviderRecord::attach(provider_url, &remote, attached_at)
        .context("authorization returned an unusable provider")?;
    Ok(crate::account_session::ActiveAccount {
        provider: provider.provider().to_owned(),
        credential_id: authorization.credential_id,
        root_did: account_did.to_string(),
        delegation_cid: chain.proof_cids()[0].to_string(),
        delegation_hex: hex::encode(grant_bytes),
        remote: Some(remote),
        attachment_id: attachment_id.to_owned(),
        attached_at,
    })
}

async fn recorded_account_grant(
    profile: &Profile,
    account: &crate::account_session::ActiveAccount,
) -> Result<(Vec<u8>, DelegationChain)> {
    let grant_bytes =
        hex::decode(&account.delegation_hex).context("recorded account delegation is not hex")?;
    let chain = validate_account_grant(profile, &grant_bytes).await?;
    if chain.issuer().as_ref() != account.root_did
        || chain.proof_cids()[0].to_string() != account.delegation_cid
    {
        bail!("recorded account delegation does not match its recorded generation");
    }
    Ok((grant_bytes, chain))
}

/// Validate and project one exact staged generation into the compatibility
/// credential records. Replaying the same values is idempotent.
async fn project_staged_account(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    account: &crate::account_session::ActiveAccount,
) -> Result<()> {
    if account.attachment_id.trim().is_empty() {
        bail!("staged account activation has no service attachment generation");
    }
    let (_, chain) = recorded_account_grant(profile, account).await?;
    let _root_did: Did = account
        .root_did
        .parse()
        .context("staged account root DID is invalid")?;
    let remote = account.remote.as_deref().unwrap_or_default();
    let provider = AccountProviderRecord::attach(&account.provider, remote, account.attached_at)
        .context("staged account provider is unusable")?;
    if provider.provider() != account.provider {
        bail!("staged account provider is not canonical");
    }

    profile
        .access()
        .save(UcanDelegation(chain))
        .perform(operator)
        .await
        .context("failed to install the account grant")?;
    crate::identity::save_local_root_with_operator(
        profile,
        operator,
        account.credential_id.clone(),
        account.delegation_hex.clone(),
    )
    .await?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(provider.encode()?)
        .perform(operator)
        .await
        .context("failed to persist the account link")?;
    Ok(())
}

/// Resume or complete one exact staged generation. The activation guard keeps
/// logout and other login attempts behind projection and final promotion.
async fn complete_staged_account(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    account: &crate::account_session::ActiveAccount,
) -> Result<()> {
    let guard =
        crate::account_session::stage_activation(profile, operator, store, account.clone()).await?;
    project_staged_account(profile, operator, account).await?;
    crate::account_session::finalize_activation(profile, operator, guard, account).await?;
    Ok(())
}

/// Hydrate and converge authority after the canonical account is already active.
/// Failure here never reopens the browser ceremony.
async fn hydrate_activated_account(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    account: &crate::account_session::ActiveAccount,
    url: String,
) -> Result<LinkOutcome> {
    let ensured = ensure_after_link(
        profile,
        operator.clone(),
        store.clone(),
        POST_LINK_SYNC_DEADLINE,
    )
    .await?;

    Ok(LinkOutcome {
        url,
        root_did: account.root_did.clone(),
        device_did: profile.did().to_string(),
        account_state: ensured.status,
        warning: ensured.warning,
        service_url: account.provider.clone(),
    })
}

/// Authorize this device through a loopback callback.
///
/// The browser runs the ceremony and posts the grant straight back to a
/// listener on this machine, so no remote registry holds it in between. The
/// grant is validated before anything is written — see
/// [`validate_account_grant`] — so a page returning something addressed
/// elsewhere, scoped to one space, or unsigned installs no authority here.
async fn link_via_callback(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    options: &LinkOptions,
    page: &str,
) -> Result<LinkOutcome> {
    let callback = crate::callback::Callback::bind().await?;
    let url = login_url(
        page,
        profile.did().as_ref(),
        callback.url(),
        &options.device_name,
    );

    println!("Open this URL to approve the device:\n{url}");
    if options.open_browser && webbrowser::open(&url).is_err() {
        eprintln!("Could not open a browser; use the URL above.");
    }
    // A caller that drives the ceremony itself (tests, or an embedder with
    // its own browser control) receives the URL rather than relying on the
    // OS to open it.
    if let Some(announce) = options.announce.as_ref() {
        let _ = announce.send(url.clone());
    }

    // One listener for the whole wait, so Ctrl-C lands as a cancellation
    // rather than whatever the default handler does mid-await.
    let redirect_origin = Url::parse(page)
        .ok()
        .map(|page| page.origin().ascii_serialization());
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let received = tokio::select! {
        result = callback.receive(redirect_origin) => result?,
        signal = &mut ctrl_c => {
            signal.context("failed to listen for Ctrl-C")?;
            bail!("account login cancelled");
        }
    };
    let bytes = match received {
        crate::callback::Authorization::Granted(bytes) => bytes,
        crate::callback::Authorization::Denied(reason) => {
            bail!("authorization was declined in the browser: {reason}");
        }
    };
    let authorization: CallbackAuthorization =
        serde_json::from_slice(&bytes).context("authorization payload is not readable")?;
    let account = account_from_callback(profile, options, authorization).await?;
    complete_staged_account(profile, operator, store, &account).await?;
    hydrate_activated_account(profile, operator, store, &account, url).await
}

/// What the authorizing page posts back to the waiting CLI.
///
/// The delegation alone would leave the device authorized but unable to find
/// the account repository, so the descriptor rides along. The credential id
/// names the passkey for display.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallbackAuthorization {
    delegation_hex: String,
    remote: String,
    #[serde(default)]
    credential_id: String,
    /// The service-issued attachment generation the approving page
    /// registered this device under. Required so recovery and detach target
    /// the exact service row rather than guessing from the delegation CID.
    #[serde(default)]
    attachment_id: String,
    /// The account service the approving page's deployment uses. Absent
    /// from pages that predate it; the `--service-url` flag (or its
    /// production default) stands in then.
    #[serde(default)]
    service_url: String,
}

/// Start or resume a browser handoff and activate its fresh generation.
pub async fn link(profile: &Profile, options: &LinkOptions) -> Result<LinkOutcome> {
    let operator = crate::account_state::credential_operator(profile).await?;
    link_with_operator(profile, &operator, options).await
}

/// Start or resume linking against one explicit native profile store.
pub async fn link_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
    options: &LinkOptions,
) -> Result<LinkOutcome> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let mut options = options.clone();
    options.store = Some(store.clone());
    link_with_operator(profile, &operator, &options).await
}

/// [`link`] against a caller-supplied operator.
///
/// [`link`] resolves one from the global install, mounting the profile by
/// name, which a caller that already holds a profile cannot satisfy. This
/// form takes the operator so the whole flow is reachable from a test.
pub async fn link_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    options: &LinkOptions,
) -> Result<LinkOutcome> {
    let store = match options.store.clone() {
        Some(store) => store,
        None => crate::space::SpaceStore::open().context("failed to locate account state")?,
    };
    {
        let guard = crate::account_session::exclusive_transition_guard(&store)?;
        crate::account_session::ensure_initialized(profile, operator, &guard).await?;
    }
    let state = {
        let guard = crate::account_session::shared_remote_guard(&store)?;
        crate::account_session::load_guarded(profile, operator, &guard).await?
    };
    if let Some(account) = state.active {
        recorded_account_grant(profile, &account).await?;
        return hydrate_activated_account(profile, operator, &store, &account, String::new()).await;
    }
    match state.pending_login {
        Some(crate::account_session::PendingLogin::Activating { account }) => {
            complete_staged_account(profile, operator, &store, &account).await?;
            hydrate_activated_account(profile, operator, &store, &account, String::new()).await
        }
        Some(crate::account_session::PendingLogin::Waiting { .. }) => {
            bail!(
                "an older account login is pending but cannot be resumed; run `tonk account logout` and try again"
            )
        }
        None => {
            let page = options.via.as_deref().unwrap_or(DEFAULT_LINK_PAGE);
            link_via_callback(profile, operator, &store, options, page).await
        }
    }
}

/// One device row, from the account space's own facts.
#[derive(Debug, Clone)]
pub struct DeviceRow {
    /// The device's DID.
    pub did: String,
    /// Display name described at link time.
    pub name: String,
    /// Link time, seconds since the epoch.
    pub created_at: u64,
}

/// Authenticated provider attachment used by account-scoped CLI modules.
pub(crate) struct AccountConnection {
    pub(crate) service_url: String,
    pub(crate) root_did: Did,
    pub(crate) link: DelegationChain,
}

async fn connection_from_provider(
    profile: &Profile,
    provider: AccountProviderRecord,
    store: &crate::space::SpaceStore,
) -> Result<AccountConnection> {
    let service_url = provider.provider().to_string();
    let root = crate::identity::local_root_in(profile, store)
        .await?
        .context("the provider attachment has no local root")?;
    let bytes = hex::decode(root.delegation_hex).context("stored local-root hex is invalid")?;
    let link = DelegationChain::try_from(bytes.as_slice())
        .context("stored local-root delegation is invalid")?;
    let root_did = link.issuer().clone();
    Ok(AccountConnection {
        service_url,
        root_did,
        link,
    })
}

pub(crate) async fn optional_connection_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<Option<AccountConnection>> {
    #[cfg(feature = "integration-tests")]
    if let Some((service_url, link, _)) = integration_connections()
        .lock()
        .expect("integration connection registry")
        .get(profile.did().as_ref())
        .cloned()
    {
        return Ok(Some(AccountConnection {
            service_url,
            root_did: link.issuer().clone(),
            link,
        }));
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let Some(provider) = stored_provider_in(profile, &operator, store).await? else {
        return Ok(None);
    };
    Ok(Some(
        connection_from_provider(profile, provider, store).await?,
    ))
}

#[cfg(feature = "integration-tests")]
type IntegrationConnection = (String, DelegationChain, crate::site::SiteConfig);
#[cfg(feature = "integration-tests")]
type IntegrationConnections =
    std::sync::Mutex<std::collections::HashMap<String, IntegrationConnection>>;

#[cfg(feature = "integration-tests")]
fn integration_connections() -> &'static IntegrationConnections {
    static CONNECTIONS: std::sync::OnceLock<IntegrationConnections> = std::sync::OnceLock::new();
    CONNECTIONS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(feature = "integration-tests")]
pub(crate) fn integration_site_config(profile: &Profile) -> Option<crate::site::SiteConfig> {
    integration_connections()
        .lock()
        .expect("integration connection registry")
        .get(profile.did().as_ref())
        .map(|(_, _, config)| config.clone())
}

#[cfg(feature = "integration-tests")]
/// Install an already-created account attachment into an isolated test profile.
#[doc(hidden)]
pub async fn attach_for_integration_test(
    profile: &Profile,
    operator: &crate::account_authority::AccountBoundOperator,
    config: crate::site::SiteConfig,
    service_url: &str,
    credential_id: &str,
    link: DelegationChain,
    remote: &str,
) -> Result<()> {
    use dialog_ucan::UcanDelegation;

    let root_did = link.issuer().clone();
    let record = crate::identity::LocalRoot {
        credential_id: credential_id.to_string(),
        root_did: root_did.to_string(),
        delegation_cid: link.proof_cids()[0].to_string(),
        delegation_hex: hex::encode(link.to_bytes()?),
    };
    profile
        .access()
        .save(UcanDelegation(link.clone()))
        .perform(operator)
        .await?;
    profile
        .credential()
        .site(crate::identity::LOCAL_ROOT_SITE)
        .save(serde_json::to_vec(&record)?)
        .perform(operator)
        .await?;
    let provider = AccountProviderRecord::attach(
        service_url,
        remote,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(provider.encode()?)
        .perform(operator)
        .await?;
    let session = crate::account_session::AccountSessionState {
        version: 1,
        active: Some(crate::account_session::ActiveAccount {
            provider: service_url.trim_end_matches('/').to_string(),
            credential_id: credential_id.to_string(),
            root_did: root_did.to_string(),
            delegation_cid: record.delegation_cid.clone(),
            delegation_hex: record.delegation_hex.clone(),
            remote: Some(remote.to_owned()),
            attachment_id: record.delegation_cid.clone(),
            attached_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }),
        pending_login: None,
    };
    crate::account_session::install_for_integration_test(profile, operator, &session).await?;
    integration_connections()
        .lock()
        .expect("integration connection registry")
        .insert(
            profile.did().to_string(),
            (service_url.to_string(), link, config),
        );
    Ok(())
}

/// List the devices authorized under this profile's account, from the
/// account branch's own facts. The recorded provider is authoritative; a
/// `service_url` names one only to cross-check it against the active
/// account.
///
/// Fresh by default, never hostage to the remote: a tightly bounded
/// account pull runs first, and a remote that is slow or unanswering
/// degrades to a stderr note over local facts rather than a hang. Set
/// `TONK_OFFLINE=1` to skip the pull entirely. One row per device — a
/// device described more than once keeps its earliest link time.
pub async fn devices(profile: &Profile, service_url: Option<&str>) -> Result<Vec<DeviceRow>> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    devices_in(profile, &store, service_url).await
}

/// List devices through one explicit account profile store.
pub async fn devices_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
    service_url: Option<&str>,
) -> Result<Vec<DeviceRow>> {
    let connection = optional_connection_in(profile, store)
        .await?
        .context("no active account; run `tonk account login`")?;
    if let Some(service_url) = service_url
        && connection.service_url.trim_end_matches('/') != service_url.trim_end_matches('/')
    {
        bail!("requested provider does not match the active account");
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    freshen_account(profile, &operator, store, "listing local facts").await;
    let branch = account_branch(profile, &operator, store).await?;
    let links = tonk_schema::device_link::device_links(&branch, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("failed to query device links: {error:?}"))?;
    let mut rows: std::collections::BTreeMap<String, DeviceRow> = Default::default();
    for (link, did) in links {
        let row = DeviceRow {
            did,
            name: link.title.0,
            created_at: link.created_at.0,
        };
        match rows.get_mut(&row.did) {
            Some(existing) if existing.created_at <= row.created_at => {}
            Some(existing) => *existing = row,
            None => {
                rows.insert(row.did.clone(), row);
            }
        }
    }
    let mut devices: Vec<DeviceRow> = rows.into_values().collect();
    devices.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(devices)
}

/// Explicitly pull the account so every local view reads current facts.
///
/// The read verbs (`devices`, `status`, `spaces`) deliberately never
/// touch the remote — local answers stay instant and a sick remote
/// cannot hang them. This is the verb that freshens what they read,
/// bounded and honest: a remote that does not answer is an error naming
/// it, not a wait.
pub async fn sync(profile: &Profile) -> Result<crate::account_state::EnsureOutcome> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    sync_in(profile, &store).await
}

/// [`sync`] through one explicit account profile store.
pub async fn sync_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
) -> Result<crate::account_state::EnsureOutcome> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    tokio::time::timeout(
        Duration::from_secs(60),
        crate::account_state::ensure_with_operator_and_store(profile, operator, store.clone()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the account remote did not answer in time"))?
}

/// Sync the account best-effort, under the same hard deadline the link
/// flow uses: reads that follow serve local facts either way, so a slow
/// or unreachable remote must degrade to slightly stale rather than
/// hang the command. Reachable remotes answer well inside the bound.
///
/// `TONK_OFFLINE=1` skips the attempt: the opt-out for air-gapped work
/// and scripts that want local answers with no network at all.
async fn freshen_account(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    doing: &str,
) {
    if std::env::var_os("TONK_OFFLINE").is_some_and(|value| !value.is_empty() && value != "0") {
        return;
    }
    match tokio::time::timeout(
        POST_LINK_SYNC_DEADLINE,
        crate::account_state::ensure_with_operator_and_store(
            profile,
            operator.clone(),
            store.clone(),
        ),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => eprintln!("warning: account sync failed; {doing}: {error:#}"),
        Err(_) => eprintln!("warning: the account remote did not answer in time; {doing}"),
    }
}

/// The mounted account branch, or what to run when there is none.
///
/// Bounded: mounting resolves the remote head, and dialog's remote
/// transport has no client deadline of its own, so a socket that
/// accepts and never answers would park the command forever — sampled
/// exactly there when `tonk account devices` hung the e2e suite. A
/// deadline turns that into an error naming the remote.
async fn account_branch(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
) -> Result<dialog_repository::Branch> {
    tokio::time::timeout(
        Duration::from_secs(30),
        crate::account_state::open_account_branch_in(profile, operator, store),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the account remote did not answer while mounting"))??
    .context("the account repository is not mounted; run `tonk account login`")
}

/// Publish a revocation everywhere it could still be honoured — the
/// account's own access service plus every service a directory space
/// syncs through. Every endpoint must accept it: a partial publication
/// is the dangerous outcome, so a refusal anywhere is reported rather
/// than swallowed.
async fn publish_revocation(
    profile: &Profile,
    branch: &dialog_repository::Branch,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::space::SpaceStore,
    artifact: &[u8],
) -> Result<()> {
    use dialog_remote_ucan_s3::UcanAddress;

    let mut endpoints = std::collections::BTreeSet::new();
    if let Some(provider) = stored_provider_in(profile, operator, store).await?
        && let Some(remote) = provider.remote()
    {
        endpoints.insert(UcanAddress::new(remote).endpoint().to_string());
    }
    endpoints.extend(
        tonk_schema::directory::access_endpoints(branch, operator)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot enumerate the services this revocation must reach: {error:?}"
                )
            })?,
    );
    if endpoints.is_empty() {
        bail!("this profile has no access service to publish a revocation to");
    }
    let client = reqwest::Client::new();
    for endpoint in &endpoints {
        let response = client
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/cbor")
            .body(artifact.to_vec())
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .with_context(|| format!("failed to reach the access service at {endpoint}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("access service {endpoint} rejected the revocation ({status}): {text}");
        }
        let receipt: tonk_account::customer::RevokeReceipt = response
            .json()
            .await
            .with_context(|| format!("{endpoint} returned an unreadable revoke receipt"))?;
        let _ = receipt.revoked;
    }
    Ok(())
}

/// Retract a revoked device's link rows from the account space, so the
/// row leaves every device's list the way the authority left the chain.
/// Returns whether anything was retracted.
async fn retract_device_rows(
    branch: &dialog_repository::Branch,
    operator: &dialog_operator::Operator<NativeSpace>,
    target: &str,
) -> Result<bool> {
    let links = tonk_schema::device_link::device_links(branch, operator)
        .await
        .map_err(|error| anyhow::anyhow!("failed to query device links: {error:?}"))?;
    let mut transaction = branch.transaction();
    let mut retracting = false;
    for (link, did) in links {
        if did == target {
            transaction = transaction.retract(link);
            retracting = true;
        }
    }
    if retracting {
        transaction
            .commit()
            .perform(operator)
            .await
            .context("failed to retract the revoked device's rows")?;
    }
    Ok(retracting)
}

/// The browser URL that asks the account to delegate to this CLI profile.
///
/// The same page `account login` opens, with two query parameters instead of a
/// fragment secret: `audience` is the profile the account should delegate to,
/// and `callback` is the loopback URL the page posts the grant back to. Both
/// are percent-encoded — a callback URL contains `:` and `/`, and an
/// unencoded one would truncate the parameter at the first `&` a port or path
/// happened to introduce.
///
/// Neither value is secret. The audience is a public DID, and the callback
/// points at loopback on this machine, so nothing here needs the fragment
/// treatment the link handoff gives its bearer token.
fn login_url(base: &str, audience: &str, callback: &str, name: &str) -> String {
    format!(
        "{}?audience={}&callback={}&name={}",
        base.trim_end_matches('#').trim_end_matches('/'),
        urlencoding::encode(audience),
        urlencoding::encode(callback),
        urlencoding::encode(name),
    )
}

/// Inputs for revoking a device.
pub struct RevokeOptions {
    /// Account service base URL, cross-checked against the active account.
    pub service_url: String,
}

/// How a revocation request resolved. The caller owns the messaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    /// The revocation was published and the device's rows retracted.
    Revoked,
    /// The device has no rows left to retract; its grant was already
    /// withdrawn.
    AlreadyRevoked,
}

/// Revoke a device under this profile's own account grant.
///
/// No browser and no passkey: a device link is a powerline, so this
/// device can prove for the account subject — which is exactly the
/// authority revoking an account-issued grant requires. The artifact is
/// published to every access service that honours it, then the device's
/// rows are retracted from the account space so the list converges on
/// every device.
///
/// The order encodes what each failure costs. For another device the
/// publication comes first, because enforcement lives in the revocation
/// index and a row that disappears over a device still reaching storage
/// would be a lie. For this device itself the retraction and its push
/// come first, because a device that has just revoked itself can no
/// longer push anything.
pub async fn revoke(
    profile: &Profile,
    options: &RevokeOptions,
    did: &str,
) -> Result<RevokeOutcome> {
    let store = crate::space::SpaceStore::open().context("failed to locate account state")?;
    revoke_in(profile, &store, options, did).await
}

/// Revoke a device through one explicit account profile store.
pub async fn revoke_in(
    profile: &Profile,
    store: &crate::space::SpaceStore,
    options: &RevokeOptions,
    did: &str,
) -> Result<RevokeOutcome> {
    let connection = optional_connection_in(profile, store)
        .await?
        .context("no active account; run `tonk account login`")?;
    if connection.service_url.trim_end_matches('/') != options.service_url.trim_end_matches('/') {
        bail!("requested provider does not match the active account");
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let link = connection.link.clone();

    if profile.did().as_ref() == did {
        let branch = account_branch(profile, &operator, store).await?;
        let target = link.proof_cids()[0];
        let artifact = tonk_identity::revocation::mint_self_revocation(
            profile.signer().signer().clone(),
            &link,
            &target,
        )
        .await
        .context("failed to sign self-revocation")?;
        match retract_device_rows(&branch, &operator, did).await {
            Ok(true) => {
                if let Err(error) = branch.push().perform(&operator).await {
                    eprintln!("warning: this device's rows were retracted but not pushed: {error}");
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("warning: this device's rows were not retracted: {error:#}"),
        }
        publish_revocation(profile, &branch, &operator, store, &artifact).await?;
        return Ok(RevokeOutcome::Revoked);
    }

    // Sync first: revoking another device needs its grant retained here,
    // to rebuild the path that says why it may be revoked.
    freshen_account(profile, &operator, store, "revoking from local facts").await;
    let branch = account_branch(profile, &operator, store).await?;
    let target: Did = did.parse().context("device DID is invalid")?;
    let listed = tonk_schema::device_link::device_links(&branch, &operator)
        .await
        .map_err(|error| anyhow::anyhow!("failed to query device links: {error:?}"))?
        .into_iter()
        .any(|(_, audience)| audience == did);
    let proof = branch
        .delegations()
        .prove(target, tonk_account::delegations::account_scope(&link))
        .perform(&operator)
        .await;
    let proof = match proof {
        Ok(proof) => proof,
        Err(error) if listed => {
            bail!("no retained grant reaches {did}: {error}");
        }
        Err(_) => bail!("no device {did} under this account"),
    };
    if !listed {
        // The grant is retained but its rows are gone — a prior
        // revocation already took them.
        return Ok(RevokeOutcome::AlreadyRevoked);
    }
    let mut certificates = proof.proofs.into_iter();
    let first = certificates
        .next()
        .with_context(|| format!("the grant for {did} is empty"))?;
    let mut path = DelegationChain::new(first.0);
    for certificate in certificates {
        path = path
            .push(certificate.0)
            .context("proved certificates do not chain")?;
    }
    let target_cid = path.proof_cids()[0];
    let artifact = tonk_identity::revocation::mint_delegated_revocation(
        profile.signer().signer().clone(),
        &path,
        &target_cid,
        &link,
    )
    .await
    .with_context(|| format!("cannot revoke {did}"))?;
    publish_revocation(profile, &branch, &operator, store, &artifact).await?;
    match retract_device_rows(&branch, &operator, did).await {
        Ok(true) => {
            if let Err(error) = branch.push().perform(&operator).await {
                eprintln!("warning: the revoked device's retraction was not pushed: {error}");
            }
        }
        Ok(false) => {}
        Err(error) => eprintln!("warning: the revoked device's rows were not retracted: {error:#}"),
    }
    Ok(RevokeOutcome::Revoked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_operator::DeriveOperator as _;

    async fn account_state_fixture(
        ready: bool,
    ) -> (
        tempfile::TempDir,
        Profile,
        dialog_operator::Operator<NativeSpace>,
        crate::space::SpaceStore,
    ) {
        use dialog_capability::Subject;
        use dialog_effects::storage::Directory;
        use dialog_storage::provider::storage::Storage;
        use dialog_varsig::Principal as _;

        let temp = tempfile::tempdir().unwrap();
        let store = crate::space::SpaceStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let profile_name = format!("cli-account-timeout-test-{}", rand::random::<u64>());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(&profile_name)
            .at(profile_dir)
            .perform(&storage)
            .await
            .unwrap();
        std::fs::create_dir_all(store.account_dir()).unwrap();
        let account_dir = store.account_dir().canonicalize().unwrap();
        let operator = profile
            .derive(b"tonk/account-state/v1")
            .allow(Subject::any())
            .base(Directory::At(account_dir.to_string_lossy().into()))
            .build(storage)
            .await
            .unwrap();
        let root = dialog_credentials::Ed25519Signer::generate().await.unwrap();
        let root_did = root.did();
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &profile.did())
            .await
            .unwrap();
        let delegation_cid = link.proof_cids()[0].to_string();
        let delegation_hex = hex::encode(link.to_bytes().unwrap());
        let provider = AccountProviderRecord::attach(
            "https://accounts.example",
            "https://content.example/ucan/",
            1,
        )
        .unwrap();
        let account = crate::account_session::ActiveAccount {
            provider: provider.provider().to_owned(),
            credential_id: "credential".to_owned(),
            root_did: root_did.to_string(),
            delegation_cid,
            delegation_hex,
            remote: Some("https://content.example/ucan/".to_owned()),
            attachment_id: "attachment".to_owned(),
            attached_at: 1,
        };
        let guard =
            crate::account_session::stage_activation(&profile, &operator, &store, account.clone())
                .await
                .unwrap();
        crate::identity::save_local_root_with_operator(
            &profile,
            &operator,
            "credential".to_string(),
            account.delegation_hex.clone(),
        )
        .await
        .unwrap();
        profile
            .access()
            .save(dialog_ucan::UcanDelegation(link))
            .perform(&operator)
            .await
            .unwrap();
        profile
            .credential()
            .site(ACCOUNT_LINK_SITE)
            .save(provider.encode().unwrap())
            .perform(&operator)
            .await
            .unwrap();
        crate::account_session::finalize_activation(&profile, &operator, guard, &account)
            .await
            .unwrap();
        if ready {
            profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .save(root_did.as_str().as_bytes().to_vec())
                .perform(&operator)
                .await
                .unwrap();
        }
        (temp, profile, operator, store)
    }

    #[dialog_common::test]
    async fn it_preserves_ready_when_post_link_sync_times_out() {
        let (_temp, profile, operator, store) = account_state_fixture(true).await;

        let outcome = ensure_after_link(&profile, operator, store, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(outcome.status, AccountStateStatus::Ready);
        assert_eq!(
            outcome.warning.as_deref(),
            Some(
                "latest account synchronization did not finish within 10 seconds; committed changes will retry"
            )
        );
    }

    #[dialog_common::test]
    async fn it_keeps_unhydrated_when_first_sync_times_out() {
        let (_temp, profile, operator, store) = account_state_fixture(false).await;

        let outcome = ensure_after_link(&profile, operator, store, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(outcome.status, AccountStateStatus::Unhydrated);
        assert_eq!(
            outcome.warning.as_deref(),
            Some("the account repository did not answer within 10 seconds; first sync will retry")
        );
    }

    #[test]
    fn it_formats_the_cli_device_name_with_os_version() {
        assert_eq!(
            format_device_name(os_info::Type::Macos, &os_info::Version::Semantic(15, 6, 0)),
            "Tonk CLI on macOS 15.6.0"
        );
        assert_eq!(
            format_device_name(
                os_info::Type::Ubuntu,
                &os_info::Version::Custom("24.04 LTS".to_string())
            ),
            "Tonk CLI on Ubuntu 24.04 LTS"
        );
    }

    #[test]
    fn it_falls_back_for_unknown_cli_os_metadata() {
        assert_eq!(
            format_device_name(os_info::Type::Linux, &os_info::Version::Unknown),
            "Tonk CLI on Linux (version unknown)"
        );
        assert_eq!(
            format_device_name(os_info::Type::Unknown, &os_info::Version::Unknown),
            "Tonk CLI on unknown OS (version unknown)"
        );
    }

    #[test]
    fn it_bounds_the_cli_device_name_without_splitting_utf8() {
        let label = format_device_name(
            os_info::Type::Linux,
            &os_info::Version::Custom("é".repeat(100)),
        );

        assert!(label.starts_with("Tonk CLI on Linux "));
        assert!(label.len() <= 100);
        assert!(!label.is_empty());
    }

    #[dialog_common::test]
    async fn it_logs_out_by_tombstoning_only_the_provider_attachment() {
        use dialog_capability::Subject;
        use dialog_effects::storage::Directory;
        use dialog_storage::provider::storage::Storage;

        let temp = tempfile::tempdir().unwrap();
        let store = crate::space::SpaceStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let profile_name = format!("cli-account-logout-test-{}", rand::random::<u64>());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(&profile_name)
            .at(profile_dir)
            .perform(&storage)
            .await
            .unwrap();
        std::fs::create_dir_all(store.account_dir()).unwrap();
        let account_dir = store.account_dir().canonicalize().unwrap();
        let operator = profile
            .derive(b"tonk/account-state/v1")
            .allow(Subject::any())
            .base(Directory::At(account_dir.to_string_lossy().into()))
            .build(storage)
            .await
            .unwrap();
        let device_did = profile.did();
        let local_root = crate::identity::LocalRoot {
            credential_id: "credential".to_string(),
            root_did: device_did.to_string(),
            delegation_cid: "delegation".to_string(),
            delegation_hex: "00".to_string(),
        };
        let local_root_bytes = serde_json::to_vec(&local_root).unwrap();
        let provider = AccountProviderRecord::attach("https://accounts.example", "", 1)
            .unwrap()
            .encode()
            .unwrap();
        let trusted_base = b"trusted-base".to_vec();
        profile
            .credential()
            .site(crate::identity::LOCAL_ROOT_SITE)
            .save(local_root_bytes.clone())
            .perform(&operator)
            .await
            .unwrap();
        profile
            .credential()
            .site(ACCOUNT_LINK_SITE)
            .save(provider)
            .perform(&operator)
            .await
            .unwrap();
        profile
            .credential()
            .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
            .save(trusted_base.clone())
            .perform(&operator)
            .await
            .unwrap();
        let sentinel = store.account_dir().join("sentinel");
        std::fs::write(&sentinel, b"keep").unwrap();

        assert!(
            stored_provider_in(&profile, &operator, &store)
                .await
                .unwrap()
                .is_some()
        );

        logout_with_operator_in(&profile, &operator, &store)
            .await
            .unwrap();

        assert!(
            stored_provider_in(&profile, &operator, &store)
                .await
                .unwrap()
                .is_none()
        );

        logout_with_operator_in(&profile, &operator, &store)
            .await
            .unwrap();

        assert_eq!(
            profile
                .credential()
                .site(ACCOUNT_LINK_SITE)
                .load::<Vec<u8>>()
                .perform(&operator)
                .await
                .unwrap(),
            Vec::<u8>::new()
        );
        assert_eq!(
            profile
                .credential()
                .site(crate::identity::LOCAL_ROOT_SITE)
                .load::<Vec<u8>>()
                .perform(&operator)
                .await
                .unwrap(),
            local_root_bytes
        );
        assert_eq!(
            profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .load::<Vec<u8>>()
                .perform(&operator)
                .await
                .unwrap(),
            trusted_base
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(profile.did(), device_did);
    }

    /// The login URL carries the audience and callback as query parameters,
    /// both percent-encoded. A raw callback URL contains `:` and `/`, and an
    /// unencoded one would truncate at the first `&` a port or path added.
    #[test]
    fn it_passes_the_audience_and_callback_as_encoded_query_parameters() {
        let url = login_url(
            DEFAULT_LINK_PAGE,
            "did:key:zProfile",
            "http://127.0.0.1:54321",
            "Kitchen laptop",
        );
        assert_eq!(
            url,
            "https://tonk.network/settings/link\
             ?audience=did%3Akey%3AzProfile\
             &callback=http%3A%2F%2F127.0.0.1%3A54321\
             &name=Kitchen%20laptop"
        );
    }

    /// A grant addressed to another device installs nothing here. The page
    /// is not trusted to address it correctly just because it delivered it.
    #[dialog_common::test]
    async fn it_refuses_a_grant_addressed_elsewhere() {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_varsig::Principal as _;

        let storage = dialog_storage::provider::storage::Storage::<NativeSpace>::default();
        let profile = Profile::open("link-audience-test")
            .perform(&storage)
            .await
            .unwrap();
        let account = Ed25519Signer::generate().await.unwrap();
        let elsewhere = Ed25519Signer::generate().await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(account))
            .audience(&elsewhere.did())
            .subject(UcanSubject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let bytes = DelegationChain::new(grant).to_bytes().unwrap();

        let error = validate_account_grant(&profile, &bytes)
            .await
            .expect_err("a grant for another audience must be refused");
        assert!(
            error.to_string().contains("not this profile"),
            "the error must name the mismatch, got {error}"
        );
    }

    /// A grant scoped to one space is refused: the account's authority is a
    /// powerline, and accepting a narrowed one would silently install less
    /// than the device asked for.
    #[dialog_common::test]
    async fn it_refuses_a_grant_scoped_to_one_subject() {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_varsig::Principal as _;

        let storage = dialog_storage::provider::storage::Storage::<NativeSpace>::default();
        let profile = Profile::open("link-subject-test")
            .perform(&storage)
            .await
            .unwrap();
        let account = Ed25519Signer::generate().await.unwrap();
        let space = Ed25519Signer::generate().await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(account))
            .audience(&profile.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let bytes = DelegationChain::new(grant).to_bytes().unwrap();

        let error = validate_account_grant(&profile, &bytes)
            .await
            .expect_err("a subject-scoped grant must be refused");
        assert!(
            error.to_string().contains("subject-open"),
            "the error must say what shape was required, got {error}"
        );
    }

    /// A modern callback must name the exact service generation. Without it,
    /// restart recovery and logout would have to guess which row to target.
    #[dialog_common::test]
    async fn it_refuses_a_callback_without_an_attachment_generation() {
        use dialog_credentials::Ed25519Signer;
        use dialog_effects::storage::Directory;

        let temp = tempfile::tempdir().unwrap();
        let storage = dialog_storage::provider::storage::Storage::<NativeSpace>::default();
        let profile = Profile::open(format!("link-generation-test-{}", rand::random::<u64>()))
            .at(Directory::At(
                temp.path().join("profiles").to_string_lossy().into(),
            ))
            .perform(&storage)
            .await
            .unwrap();
        let service_url = "https://accounts.example/ucan/".to_string();
        let account = Ed25519Signer::generate().await.unwrap();
        let authorized =
            tonk_identity::ceremony::authorize_device(account, profile.did(), &service_url)
                .await
                .unwrap();

        let error = account_from_callback(
            &profile,
            &LinkOptions {
                service_url: service_url.clone(),
                device_name: "generation test".to_owned(),
                open_browser: false,
                store: None,
                announce: None,
                via: None,
            },
            CallbackAuthorization {
                delegation_hex: authorized.delegation_hex,
                remote: "https://accounts.example/ucan/".to_owned(),
                credential_id: authorized.root_did,
                attachment_id: "  ".to_owned(),
                service_url,
            },
        )
        .await
        .expect_err("a callback without an exact generation must be refused");

        assert!(
            error.to_string().contains("attachment generation"),
            "unexpected missing-generation error: {error:#}"
        );
    }

    struct RecoveryFixture {
        _temp: tempfile::TempDir,
        store: crate::space::SpaceStore,
        profile_dir: dialog_effects::storage::Directory,
        profile_name: String,
        account_dir: std::path::PathBuf,
        service_url: String,
        account: crate::account_session::ActiveAccount,
    }

    impl RecoveryFixture {
        async fn new() -> (Self, Profile, dialog_operator::Operator<NativeSpace>) {
            use dialog_capability::Subject;
            use dialog_credentials::Ed25519Signer;
            use dialog_effects::storage::Directory;
            use dialog_storage::provider::storage::Storage;

            let temp = tempfile::tempdir().unwrap();
            let store = crate::space::SpaceStore::at(temp.path().join("state"));
            let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
            let profile_name = format!("cli-account-recovery-test-{}", rand::random::<u64>());
            let storage = Storage::<NativeSpace>::default();
            let profile = Profile::open(&profile_name)
                .at(profile_dir.clone())
                .perform(&storage)
                .await
                .unwrap();
            std::fs::create_dir_all(store.account_dir()).unwrap();
            let account_dir = store.account_dir().canonicalize().unwrap();
            let operator = profile
                .derive(b"tonk/account-state/v1")
                .allow(Subject::any())
                .base(Directory::At(account_dir.to_string_lossy().into()))
                .build(storage)
                .await
                .unwrap();
            let service_url = "http://127.0.0.1:9/ucan/".to_string();
            let signer = Ed25519Signer::generate().await.unwrap();
            let authorized =
                tonk_identity::ceremony::authorize_device(signer, profile.did(), &service_url)
                    .await
                    .unwrap();
            let grant_bytes = hex::decode(&authorized.delegation_hex).unwrap();
            let grant = validate_account_grant(&profile, &grant_bytes)
                .await
                .unwrap();
            let account = crate::account_session::ActiveAccount {
                provider: service_url.trim_end_matches('/').to_owned(),
                credential_id: authorized.root_did,
                root_did: grant.issuer().to_string(),
                delegation_cid: grant.proof_cids()[0].to_string(),
                delegation_hex: hex::encode(grant_bytes),
                remote: Some("https://accounts.example/ucan/".to_owned()),
                attachment_id: "service-generation-7".to_owned(),
                attached_at: 7,
            };
            (
                Self {
                    _temp: temp,
                    store,
                    profile_dir,
                    profile_name,
                    account_dir,
                    service_url,
                    account,
                },
                profile,
                operator,
            )
        }

        async fn reopen(&self) -> (Profile, dialog_operator::Operator<NativeSpace>) {
            use dialog_capability::Subject;
            use dialog_effects::storage::Directory;
            use dialog_storage::provider::storage::Storage;

            let storage = Storage::<NativeSpace>::default();
            let profile = Profile::open(&self.profile_name)
                .at(self.profile_dir.clone())
                .perform(&storage)
                .await
                .unwrap();
            let operator = profile
                .derive(b"tonk/account-state/v1")
                .allow(Subject::any())
                .base(Directory::At(self.account_dir.to_string_lossy().into()))
                .build(storage)
                .await
                .unwrap();
            (profile, operator)
        }

        fn options(
            &self,
            announce: Option<tokio::sync::mpsc::UnboundedSender<String>>,
        ) -> LinkOptions {
            LinkOptions {
                service_url: self.service_url.clone(),
                device_name: "recovery test".to_owned(),
                open_browser: false,
                store: Some(self.store.clone()),
                announce,
                via: Some("http://127.0.0.1:3000/link".to_owned()),
            }
        }
    }

    fn assert_no_browser_announcement(
        announced: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        assert!(
            matches!(
                announced.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                    | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ),
            "recovery must not announce a new browser handoff"
        );
    }

    /// A process reopening after the browser callback finishes the exact
    /// durable generation instead of starting another handoff.
    #[dialog_common::test]
    async fn it_resumes_the_exact_pending_attachment_after_reopen() {
        let (fixture, profile, operator) = RecoveryFixture::new().await;
        let staged = crate::account_session::stage_activation(
            &profile,
            &operator,
            &fixture.store,
            fixture.account.clone(),
        )
        .await
        .unwrap();
        drop(staged);
        drop(operator);
        drop(profile);

        let (profile, operator) = fixture.reopen().await;
        let (announce, mut announced) = tokio::sync::mpsc::unbounded_channel();
        // A deadlock guard, not a stopwatch. Waiting for a browser
        // means waiting on the announcement channel forever, which no
        // budget rescues — `assert_no_browser_announcement` below is
        // what actually proves recovery took the durable generation.
        // At one second this was measuring how loaded the machine was:
        // reopening the store, deriving keys and loading the guarded
        // state is real work, and it failed in a full `--lib` run while
        // passing alone.
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            link_with_operator(&profile, &operator, &fixture.options(Some(announce))),
        )
        .await
        .expect("pending recovery must not wait for a browser")
        .unwrap();

        assert_eq!(outcome.root_did, fixture.account.root_did);
        assert_no_browser_announcement(&mut announced);
        let guard = crate::account_session::shared_remote_guard(&fixture.store).unwrap();
        let state = crate::account_session::load_guarded(&profile, &operator, &guard)
            .await
            .unwrap();
        assert_eq!(state.active, Some(fixture.account.clone()));
        assert!(state.pending_login.is_none());
    }

    /// Replaying all compatibility writes after a failed final commit must be
    /// idempotent and must preserve the exact staged generation.
    #[dialog_common::test]
    async fn it_replays_projection_after_a_pre_commit_finalization_failure() {
        let (fixture, profile, operator) = RecoveryFixture::new().await;
        let staged = crate::account_session::stage_activation(
            &profile,
            &operator,
            &fixture.store,
            fixture.account.clone(),
        )
        .await
        .unwrap();
        drop(staged);
        let state_file = std::fs::read_dir(fixture.store.account_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".json"))
            })
            .unwrap();
        let tmp = state_file.with_extension("json.tmp");
        std::fs::create_dir(&tmp).unwrap();

        let error = link_with_operator(&profile, &operator, &fixture.options(None))
            .await
            .expect_err("the obstructed final commit must fail after projection");
        assert!(error.to_string().contains("account-session temp file"));
        assert!(
            crate::identity::local_root_with_operator(&profile, &operator)
                .await
                .unwrap()
                .is_some(),
            "the projection reached the local-root write before finalization"
        );
        std::fs::remove_dir(&tmp).unwrap();
        drop(operator);
        drop(profile);

        let (profile, operator) = fixture.reopen().await;
        let outcome = link_with_operator(&profile, &operator, &fixture.options(None))
            .await
            .expect("replaying the completed projection must converge");
        assert_eq!(outcome.root_did, fixture.account.root_did);
    }

    /// A crash after Active but before the outer command's registry write
    /// reconciles from the canonical generation without another browser.
    #[dialog_common::test]
    async fn it_reconciles_an_active_generation_after_outer_command_restart() {
        let (fixture, profile, operator) = RecoveryFixture::new().await;
        complete_staged_account(&profile, &operator, &fixture.store, &fixture.account)
            .await
            .unwrap();
        drop(operator);
        drop(profile);

        let (profile, operator) = fixture.reopen().await;
        let (announce, mut announced) = tokio::sync::mpsc::unbounded_channel();
        let outcome = link_with_operator(&profile, &operator, &fixture.options(Some(announce)))
            .await
            .expect("active recovery must finish the interrupted outer command");
        assert_eq!(outcome.root_did, fixture.account.root_did);
        assert_no_browser_announcement(&mut announced);
    }

    #[dialog_common::test]
    async fn it_rejects_active_state_whose_grant_names_another_root() {
        let (fixture, profile, operator) = RecoveryFixture::new().await;
        let mut corrupt = fixture.account.clone();
        corrupt.root_did = profile.did().to_string();
        let guard = crate::account_session::stage_activation(
            &profile,
            &operator,
            &fixture.store,
            corrupt.clone(),
        )
        .await
        .unwrap();
        crate::account_session::finalize_activation(&profile, &operator, guard, &corrupt)
            .await
            .unwrap();

        let error = link_with_operator(&profile, &operator, &fixture.options(None))
            .await
            .expect_err("active recovery must bind its grant to the recorded root");
        assert!(
            error.to_string().contains("recorded generation"),
            "unexpected corrupt-active error: {error:#}"
        );
    }
}

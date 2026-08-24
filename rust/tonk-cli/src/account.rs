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
pub const DEFAULT_ACCOUNT_PAGE: &str = "https://tonk.network/account";
/// Production link ceremony page: it reads `?audience=` and `?callback=`
/// and posts the grant back to the waiting CLI.
pub const DEFAULT_LINK_PAGE: &str = "https://tonk.network/account/link";

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

/// How long the post-link hydration phase may run before the command
/// reports "waiting for first sync" and returns.
const HYDRATION_DEADLINE: Duration = Duration::from_secs(10);

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
/// nothing else, so `tonk space list` can say whether an account is signed in
/// without provisioning an identity for an installation that has none.
pub fn sign_in_phase(store: &crate::spot::SpotStore) -> Result<SignInPhase> {
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
    pub store: Option<crate::spot::SpotStore>,
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
    root_did: &dialog_varsig::Did,
    bytes: Result<Vec<u8>, dialog_effects::credential::CredentialError>,
) -> Result<Option<AccountProviderRecord>> {
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) if crate::account_state::credential_is_missing(&error) => return Ok(None),
        Err(error) => return Err(error).context("failed to load the account provider"),
    };
    AccountProviderRecord::decode(&bytes, root_did)
        .await
        .context("stored account provider is unusable")
}

/// Load the provider attachment through an already-mounted site operator.
pub(crate) async fn stored_provider_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
) -> Result<Option<AccountProviderRecord>> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    stored_provider_for_store(profile, operator, &store).await
}

/// [`stored_provider_with_operator`] against a caller-supplied store.
pub(crate) async fn stored_provider_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<Option<AccountProviderRecord>> {
    stored_provider_for_store(profile, operator, store).await
}

async fn stored_provider_for_store(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
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
const ACCOUNT_REQUIRED: &str = "A Tonk account is required; run `tonk account link`";

/// Refuse unless this profile holds an account.
///
/// The precondition every durable operation shares, in the shape the browser
/// worker uses it (`router::account::require_account`): durable authority is
/// only ever issued to an account, so what it mints stays revocable and what
/// it creates gets backed up. `Unhydrated` and `Unconfigured` accounts pass —
/// an account that exists but has not synchronized is still an account.
pub(crate) async fn require_account_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
) -> Result<()> {
    match stored_provider_with_operator(profile, operator).await? {
        Some(_) => Ok(()),
        None => bail!(ACCOUNT_REQUIRED),
    }
}

/// Refuse unless one explicit profile store holds an account attachment.
pub(crate) async fn require_account_with_operator_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<()> {
    match stored_provider_in(profile, operator, store).await? {
        Some(_) => Ok(()),
        None => bail!(ACCOUNT_REQUIRED),
    }
}

/// Disconnect provider services while preserving this profile's root,
/// delegations, account repository, and spots.
pub async fn logout(profile: &Profile) -> Result<()> {
    let operator = crate::account_state::credential_operator(profile).await?;
    logout_with_operator(profile, &operator).await
}

/// Disconnect only the account session owned by `store`.
pub async fn logout_in(profile: &Profile, store: &crate::spot::SpotStore) -> Result<()> {
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    logout_with_operator_in(profile, &operator, store).await
}

async fn logout_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
) -> Result<()> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    logout_with_operator_in(profile, operator, &store).await
}

async fn logout_with_operator_in(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
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
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    status_in(profile, &store).await
}

/// Read account status from one explicit native profile store.
pub async fn status_in(profile: &Profile, store: &crate::spot::SpotStore) -> Result<AccountStatus> {
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
            let account_state = if provider.descriptor().is_some() {
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
            bail!("account link cancelled");
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
    let grant_bytes = hex::decode(&authorization.delegation_hex)
        .context("authorization delegation is not hex")?;
    let chain = validate_account_grant(profile, &grant_bytes).await?;
    let account_did = chain.issuer().clone();
    let root_did = account_did.to_string();

    // Install the inbound half so this device can act, and record the root
    // the way a linked device does.
    profile
        .access()
        .save(UcanDelegation(chain))
        .perform(operator)
        .await
        .context("failed to install the account grant")?;
    crate::identity::save_local_root_with_operator(
        profile,
        operator,
        authorization.credential_id.clone(),
        authorization.delegation_hex.clone(),
    )
    .await?;

    // The descriptor tells this device WHERE the account repository lives; a
    // delegation only says who may act. Persisting it is what lets the
    // account mount and sync at all. The provider URL prefers what the
    // page delivered — the page knows its deployment — over the flag,
    // whose default names production regardless of where the ceremony ran.
    let provider_url = Some(authorization.service_url.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(&options.service_url)
        .to_owned();
    let provider = AccountProviderRecord::attach(
        &provider_url,
        &hex::decode(&authorization.descriptor_hex)
            .context("authorization descriptor is not hex")?,
        &account_did,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .await
    .context("authorization returned an unusable repository descriptor")?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(provider.encode()?)
        .perform(operator)
        .await
        .context("failed to persist the account link")?;

    let store = match options.store.clone() {
        Some(store) => store,
        None => crate::spot::SpotStore::open().context("failed to locate account state")?,
    };
    // The canonical session was initialized empty before the ceremony ran,
    // so the persisted root and record must be projected into an active
    // session explicitly — everything account-bound reads through it.
    let attachment_id = Some(authorization.attachment_id.clone()).filter(|id| !id.is_empty());
    crate::account_session::activate_link(profile, operator, &store, attachment_id).await?;

    // Mount the account, then retain BOTH halves of the union into it and
    // push. The page mints only the inbound grant; storing both ends here
    // keeps the writes where the account repository is already mounted, and
    // means a later device pulling the account inherits what this profile
    // holds rather than only what the account issued.
    // `ensure` mounts the account, adopts it as the access upstream, and
    // syncs — the dance that turns a grant into usable, shared authority.
    // The whole phase runs under a hard deadline: the link is complete
    // once credentials are durable, and a slow or unreachable remote must
    // degrade to "waiting for first sync" rather than hang the command.
    let hydration = tokio::time::timeout(HYDRATION_DEADLINE, async {
        let account_state = match crate::account_state::ensure_with_operator_and_store(
            profile,
            operator.clone(),
            store.clone(),
        )
        .await
        {
            Ok(outcome) => outcome.status,
            Err(_) => AccountStateStatus::Unhydrated,
        };
        let mut warning = None;
        if let Some(branch) =
            crate::account_state::open_account_branch_in(profile, operator, &store).await?
        {
            let signer = profile.signer().signer().clone();
            let union =
                tonk_account::delegations::mint_account_union(&signer, &account_did).await?;
            let inbound = DelegationChain::try_from(grant_bytes.as_slice())
                .context("account grant is not a delegation container")?;
            for (label, chain) in [("account grant", inbound), ("profile union", union)] {
                if let Err(error) =
                    tonk_account::delegations::retain_space_delegation(&branch, &chain, operator)
                        .await
                {
                    warning = Some(format!(
                        "{label} was not retained into the account: {error}"
                    ));
                }
            }
            if warning.is_none()
                && let Err(error) = branch.push().perform(operator).await
            {
                warning = Some(format!("account was authorized but not pushed: {error}"));
            }
        }
        Ok::<_, anyhow::Error>((account_state, warning))
    })
    .await;
    let (account_state, warning) = match hydration {
        Ok(result) => result?,
        Err(_) => (
            AccountStateStatus::Unhydrated,
            Some("the account repository did not answer in time; sync will retry".to_string()),
        ),
    };

    Ok(LinkOutcome {
        url,
        root_did,
        device_did: profile.did().to_string(),
        account_state,
        warning,
        service_url: provider_url,
    })
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
    descriptor_hex: String,
    #[serde(default)]
    credential_id: String,
    /// The service-issued attachment generation the approving page
    /// registered this device under. Absent from pages that predate
    /// registration-at-approval; the delegation CID stands in then.
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
    store: &crate::spot::SpotStore,
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
        None => crate::spot::SpotStore::open().context("failed to locate account state")?,
    };
    {
        let guard = crate::account_session::exclusive_transition_guard(&store)?;
        crate::account_session::ensure_initialized(profile, operator, &guard).await?;
    }
    let state = {
        let guard = crate::account_session::shared_remote_guard(&store)?;
        crate::account_session::load_guarded(profile, operator, &guard).await?
    };
    if state.active.is_some() {
        bail!("an account is already active; run `tonk account logout` before linking another");
    }
    let page = options.via.as_deref().unwrap_or(DEFAULT_LINK_PAGE);
    link_via_callback(profile, operator, options, page).await
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
    store: &crate::spot::SpotStore,
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
    store: &crate::spot::SpotStore,
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
    descriptor: &[u8],
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
        descriptor,
        &root_did,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .await?;
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
            descriptor_hex: Some(hex::encode(descriptor)),
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
/// The account is synced first, best-effort: rows described on other
/// devices arrive with the pull, and offline the list still serves what
/// is local. One row per device — a device described more than once
/// keeps its earliest link time.
pub async fn devices(profile: &Profile, service_url: Option<&str>) -> Result<Vec<DeviceRow>> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    devices_in(profile, &store, service_url).await
}

/// List devices through one explicit account profile store.
pub async fn devices_in(
    profile: &Profile,
    store: &crate::spot::SpotStore,
    service_url: Option<&str>,
) -> Result<Vec<DeviceRow>> {
    let connection = optional_connection_in(profile, store)
        .await?
        .context("no active account; run `tonk account link`")?;
    if let Some(service_url) = service_url
        && connection.service_url.trim_end_matches('/') != service_url.trim_end_matches('/')
    {
        bail!("requested provider does not match the active account");
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    if let Err(error) = crate::account_state::ensure_with_operator_and_store(
        profile,
        operator.clone(),
        store.clone(),
    )
    .await
    {
        eprintln!("warning: account sync failed; listing local facts: {error:#}");
    }
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

/// The mounted account branch, or what to run when there is none.
async fn account_branch(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    store: &crate::spot::SpotStore,
) -> Result<dialog_repository::Branch> {
    crate::account_state::open_account_branch_in(profile, operator, store)
        .await?
        .context("the account repository is not mounted; run `tonk account link`")
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
    store: &crate::spot::SpotStore,
    artifact: &[u8],
) -> Result<()> {
    use dialog_remote_ucan_s3::UcanAddress;

    let mut endpoints = std::collections::BTreeSet::new();
    if let Some(provider) = stored_provider_in(profile, operator, store).await?
        && let Some(descriptor) = provider.descriptor()
    {
        endpoints.insert(
            UcanAddress::new(descriptor.remote().as_str())
                .endpoint()
                .to_string(),
        );
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
/// The same page `account link` opens, with two query parameters instead of a
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
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    revoke_in(profile, &store, options, did).await
}

/// Revoke a device through one explicit account profile store.
pub async fn revoke_in(
    profile: &Profile,
    store: &crate::spot::SpotStore,
    options: &RevokeOptions,
    did: &str,
) -> Result<RevokeOutcome> {
    let connection = optional_connection_in(profile, store)
        .await?
        .context("no active account; run `tonk account link`")?;
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
    if let Err(error) = crate::account_state::ensure_with_operator_and_store(
        profile,
        operator.clone(),
        store.clone(),
    )
    .await
    {
        eprintln!("warning: account sync failed; revoking from local facts: {error:#}");
    }
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
        let store = crate::spot::SpotStore::at(temp.path().join("state"));
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
        let provider = AccountProviderRecord::attach_unconfigured("https://accounts.example", 1)
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
            stored_provider_with_operator(&profile, &operator)
                .await
                .unwrap()
                .is_some()
        );

        logout_with_operator(&profile, &operator).await.unwrap();

        assert!(
            stored_provider_with_operator(&profile, &operator)
                .await
                .unwrap()
                .is_none()
        );

        logout_with_operator(&profile, &operator).await.unwrap();

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
            "https://tonk.network/account/link\
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
}

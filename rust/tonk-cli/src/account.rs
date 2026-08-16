//! Native account status and browser-assisted device linking.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialog_operator::Profile;
use dialog_storage::provider::storage::NativeSpace;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Did;
use rand::RngCore;
use serde::Deserialize;
use tonk_account::handoff::{ConsumedLink, LinkCreateRequest, LinkSecretRequest};
use tonk_account::{AccountProviderRecord, AccountStateStatus};

/// Production account API used unless explicitly overridden.
pub const DEFAULT_SERVICE_URL: &str = "https://accounts.tonk.xyz";
/// Production top-document ceremony route.
pub const DEFAULT_ACCOUNT_URL: &str = "https://tonk.spot/account/link";
/// Production account page, where the revoke ceremony runs. Distinct
/// from [`DEFAULT_ACCOUNT_URL`]: `/account/link` is the link handoff and
/// consumes a fragment secret, so a `?revoke=` sent there dead-ends.
pub const DEFAULT_ACCOUNT_PAGE: &str = "https://tonk.spot/account";
/// Credential-store key for optional provider attachment metadata.
pub const ACCOUNT_LINK_SITE: &str = tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE;

/// Linking is complete once credentials are durable; repository hydration is
/// best-effort and must not leave the handoff command waiting indefinitely.
const ACCOUNT_STATE_ENSURE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Inputs controlling a browser handoff.
#[derive(Debug, Clone)]
pub struct LinkOptions {
    /// Account API base URL.
    pub service_url: String,
    /// Top-document account route base URL.
    pub account_url: String,
    /// Human-readable name displayed for browser confirmation.
    pub device_name: String,
    /// Whether to ask the OS to open the handoff URL.
    pub open_browser: bool,
    /// Whether to drop undeliverable detach intents and link anyway.
    pub abandon_detach: bool,
    /// Authorize through a loopback callback against this page instead of the
    /// service's link registry.
    ///
    /// The default flow registers a handoff with the account service and
    /// polls it. With a page named here, the CLI instead binds a loopback
    /// listener, passes it as `callback=`, and the page posts the grant
    /// straight back — no remote registry, so nothing but the browser and
    /// this process ever holds it.
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

async fn stored_provider(profile: &Profile) -> Result<Option<AccountProviderRecord>> {
    let operator = crate::account_state::credential_operator(profile).await?;
    stored_provider_with_operator(profile, &operator).await
}

async fn retry_pending_detaches(profile: &Profile) -> Result<crate::account_session::FlushOutcome> {
    let operator = crate::account_state::credential_operator(profile).await?;
    crate::account_session::flush_pending(profile, &operator).await
}

/// Disconnect provider services while preserving this profile's root,
/// delegations, account repository, and spots.
pub async fn logout(profile: &Profile) -> Result<()> {
    let operator = crate::account_state::credential_operator(profile).await?;
    logout_with_operator(profile, &operator).await
}

async fn logout_with_operator(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    crate::account_session::logout_transition(profile, operator, now).await?;
    if let Ok(outcome) = crate::account_session::flush_pending(profile, operator).await
        && let Some(warning) = outcome.warning
    {
        eprintln!("warning: logged out locally; {warning}");
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
    if let Ok(outcome) = retry_pending_detaches(profile).await
        && let Some(warning) = outcome.warning
    {
        eprintln!("warning: {warning}");
    }
    let device_did = profile.did().to_string();
    let Some(root) = crate::identity::local_root(profile).await? else {
        return Ok(AccountStatus::MissingRoot { device_did });
    };
    match stored_provider(profile).await? {
        None => Ok(AccountStatus::Unregistered {
            root_did: root.root_did,
            device_did,
        }),
        Some(provider) => {
            let account_state = if provider.descriptor().is_some() {
                crate::account_state::status(profile).await?
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
/// The same shape [`active_from_consumed`] requires of a linked grant, checked
/// here because a browser-delivered one arrives over a different path and is
/// no more trusted for it: one proof, subject-open (a powerline, so it covers
/// everything the account holds), addressed to this profile, and signed by the
/// issuer it names.
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
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .context("authorization signature is invalid")?;
    Ok(chain)
}

async fn active_from_consumed(
    profile: &Profile,
    service_url: &str,
    consumed: &ConsumedLink,
) -> Result<crate::account_session::ActiveAccount> {
    let bytes =
        hex::decode(&consumed.delegation_hex).context("invalid local-root delegation hex")?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .context("invalid local-root delegation container")?;
    if chain.proof_cids().len() != 1
        || chain.subject().is_some()
        || chain.audience() != &profile.did()
    {
        bail!("local-root delegation has an invalid shape");
    }
    let proof = chain
        .proofs()
        .next()
        .context("local-root delegation is missing its proof")?;
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .context("local-root delegation signature is invalid")?;
    let root_did = chain.issuer().clone();
    let descriptor = hex::decode(&consumed.descriptor_hex)
        .context("invalid descriptor hex from account service")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    AccountProviderRecord::attach(service_url, &descriptor, &root_did, now)
        .await
        .context("account service returned an unusable repository descriptor")?;
    Ok(crate::account_session::ActiveAccount {
        provider: service_url.trim_end_matches('/').to_string(),
        credential_id: consumed.credential_id.clone(),
        root_did: root_did.to_string(),
        delegation_cid: chain.proof_cids()[0].to_string(),
        delegation_hex: consumed.delegation_hex.clone(),
        descriptor_hex: Some(consumed.descriptor_hex.clone()),
        attachment_id: consumed.attachment_id.clone(),
        attached_at: now,
    })
}

async fn persist_pending_projection(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    account: &crate::account_session::ActiveAccount,
) -> Result<()> {
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    let guard = crate::account_session::exclusive_transition_guard(&store)?;
    crate::account_session::require_pending_activation(profile, operator, account, &guard).await?;
    let bytes =
        hex::decode(&account.delegation_hex).context("active account grant hex is invalid")?;
    let chain =
        DelegationChain::try_from(bytes.as_slice()).context("active account grant is invalid")?;
    profile
        .access()
        .save(UcanDelegation(chain))
        .perform(operator)
        .await
        .context("failed to install the local-root delegation")?;
    let root = crate::identity::LocalRoot {
        credential_id: account.credential_id.clone(),
        root_did: account.root_did.clone(),
        delegation_cid: account.delegation_cid.clone(),
        delegation_hex: account.delegation_hex.clone(),
    };
    profile
        .credential()
        .site(crate::identity::LOCAL_ROOT_SITE)
        .save(serde_json::to_vec(&root).context("failed to serialize the local root")?)
        .perform(operator)
        .await
        .context("failed to persist the local root")?;
    let root_did: Did = account
        .root_did
        .parse()
        .context("active root DID is invalid")?;
    let descriptor = hex::decode(
        account
            .descriptor_hex
            .as_deref()
            .context("active account descriptor is missing")?,
    )
    .context("active account descriptor hex is invalid")?;
    let provider = AccountProviderRecord::attach(
        &account.provider,
        &descriptor,
        &root_did,
        account.attached_at,
    )
    .await
    .context("active account descriptor is invalid")?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(
            provider
                .encode()
                .context("failed to serialize account provider")?,
        )
        .perform(operator)
        .await
        .context("failed to attach the account provider")?;
    drop(guard);
    Ok(())
}

fn new_secret() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let secret = hex::encode(bytes);
    let token_hash = blake3::hash(&bytes).to_hex().to_string();
    (secret, token_hash)
}

fn handoff_url(base: &str, secret: &str) -> String {
    format!("{}#{secret}", base.trim_end_matches('#'))
}

async fn create_remote(
    client: &reqwest::Client,
    service_url: &str,
    token_hash: &str,
    device_did: &str,
    device_name: &str,
) -> Result<()> {
    let response = client
        .post(format!("{}/links", service_url.trim_end_matches('/')))
        .json(&LinkCreateRequest {
            token_hash: token_hash.to_string(),
            device_did: device_did.to_string(),
            device_name: device_name.to_string(),
        })
        .send()
        .await
        .context("failed to create account link request")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("account service rejected the link request ({status}): {body}");
    }
    Ok(())
}

async fn consume_once(
    client: &reqwest::Client,
    service_url: &str,
    secret: &str,
) -> Result<Option<ConsumedLink>> {
    let response = client
        .post(format!(
            "{}/links/consume",
            service_url.trim_end_matches('/')
        ))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .context("failed to poll account link request")?;
    if response.status() == reqwest::StatusCode::ACCEPTED {
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("account service rejected the link poll ({status}): {body}");
    }
    Ok(Some(response.json::<ConsumedLink>().await.context(
        "account service returned an invalid link response",
    )?))
}

async fn activate_remote(
    profile: &Profile,
    account: &crate::account_session::ActiveAccount,
    token_hash: &str,
) -> Result<()> {
    let operator = crate::account_state::credential_operator(profile).await?;
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    let guard = crate::account_session::shared_remote_guard(&store)?;
    let state = crate::account_session::load_guarded(profile, &operator, &guard).await?;
    if !matches!(
        state.pending_login,
        Some(crate::account_session::PendingLogin::Activating {
            account: ref pending,
            ..
        }) if pending == account
    ) {
        bail!("account handoff was cancelled before activation");
    }
    let bytes =
        hex::decode(&account.delegation_hex).context("completed account grant hex is invalid")?;
    let link = DelegationChain::try_from(bytes.as_slice())
        .context("completed account grant is invalid")?;
    let arguments = [
        (
            "tokenHash".to_string(),
            Promised::String(token_hash.to_string()),
        ),
        (
            "attachmentId".to_string(),
            Promised::String(account.attachment_id.clone()),
        ),
    ]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "link".into(), "activate".into()],
        arguments,
    )
    .await
    .context("failed to sign account-link activation")?;
    post_invocation(&account.provider, "links/activate", body).await?;
    drop(guard);
    Ok(())
}

/// Deliver queued detach intents before a handoff opens on `provider`.
///
/// Only an undelivered detach at the same provider can block a handoff:
/// the service's one-active-generation rule rejects activation while an
/// earlier generation of this device is still active there. A detach
/// queued for a different provider says nothing about this one, and one
/// the provider can never accept is dropped by the flush itself.
async fn clear_detach_for(
    profile: &Profile,
    operator: &dialog_operator::Operator<NativeSpace>,
    provider: &str,
    abandon: bool,
) -> Result<()> {
    let flushed = crate::account_session::flush_pending(profile, operator).await?;
    if !flushed.retains(provider) {
        if let Some(warning) = flushed.warning {
            eprintln!("warning: {warning}");
        }
        return Ok(());
    }
    if abandon {
        let abandoned = crate::account_session::abandon_pending(profile, operator).await?;
        eprintln!(
            "warning: abandoned {abandoned} undelivered detach intent(s); \
             earlier devices may still be listed on the account page"
        );
        return Ok(());
    }
    bail!(
        "cannot link while a detach for {provider} is undelivered: {}\n\
         retry once the account service is reachable, or run \
         `tonk account link --abandon-detach` to link anyway",
        flushed
            .warning
            .unwrap_or_else(|| "provider retry required".to_string())
    );
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
    options: &LinkOptions,
    page: &str,
) -> Result<LinkOutcome> {
    let operator = crate::account_state::credential_operator(profile).await?;
    let callback = crate::callback::Callback::bind().await?;
    let url = login_url(page, profile.did().as_ref(), callback.url());

    println!("Open this URL to approve the device:\n{url}");
    if options.open_browser && webbrowser::open(&url).is_err() {
        eprintln!("Could not open a browser; use the URL above.");
    }

    let bytes = match callback.receive().await? {
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
        .perform(&operator)
        .await
        .context("failed to install the account grant")?;
    crate::identity::save_local_root(
        profile,
        authorization.credential_id.clone(),
        authorization.delegation_hex.clone(),
    )
    .await?;

    // The descriptor tells this device WHERE the account repository lives; a
    // delegation only says who may act. Persisting it is what lets the
    // account mount and sync at all.
    let provider = AccountProviderRecord::attach(
        &options.service_url,
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
        .perform(&operator)
        .await
        .context("failed to persist the account link")?;

    // Mount the account, then retain BOTH halves of the union into it and
    // push. The page mints only the inbound grant; storing both ends here
    // keeps the writes where the account repository is already mounted, and
    // means a later device pulling the account inherits what this profile
    // holds rather than only what the account issued.
    let account_state = match crate::account_state::ensure(profile).await {
        Ok(outcome) => outcome.status,
        Err(_) => AccountStateStatus::Unhydrated,
    };
    let mut warning = None;
    if let Some(branch) = crate::account_state::open_account_branch(profile, &operator).await? {
        let signer = profile.signer().signer().clone();
        let union = tonk_account::delegations::mint_account_union(&signer, &account_did).await?;
        let inbound = DelegationChain::try_from(grant_bytes.as_slice())
            .context("account grant is not a delegation container")?;
        for (label, chain) in [("account grant", inbound), ("profile union", union)] {
            if let Err(error) =
                tonk_account::delegations::retain_space_delegation(&branch, &chain, &operator).await
            {
                warning = Some(format!(
                    "{label} was not retained into the account: {error}"
                ));
            }
        }
        if warning.is_none()
            && let Err(error) = branch.push().perform(&operator).await
        {
            warning = Some(format!("account was authorized but not pushed: {error}"));
        }
    }

    Ok(LinkOutcome {
        url,
        root_did,
        device_did: profile.did().to_string(),
        account_state,
        warning,
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
}

/// Start or resume a browser handoff and activate its fresh generation.
pub async fn link(profile: &Profile, options: &LinkOptions) -> Result<LinkOutcome> {
    if let Some(page) = options.via.as_deref() {
        return link_via_callback(profile, options, page).await;
    }
    let operator = crate::account_state::credential_operator(profile).await?;
    let store = crate::spot::SpotStore::open().context("failed to locate account state")?;
    {
        let guard = crate::account_session::exclusive_transition_guard(&store)?;
        crate::account_session::ensure_initialized(profile, &operator, &guard).await?;
    }
    let state = {
        let guard = crate::account_session::shared_remote_guard(&store)?;
        crate::account_session::load_guarded(profile, &operator, &guard).await?
    };
    if state.active.is_some() {
        bail!("an account is already active; run `tonk account logout` before linking another");
    }
    let target_provider = match &state.pending_login {
        Some(crate::account_session::PendingLogin::Waiting { provider, .. }) => provider.clone(),
        Some(crate::account_session::PendingLogin::Activating { account, .. }) => {
            account.provider.clone()
        }
        None => options.service_url.trim_end_matches('/').to_string(),
    };
    if target_provider != options.service_url.trim_end_matches('/') {
        bail!("a pending account handoff belongs to another provider; log out to cancel it");
    }
    clear_detach_for(profile, &operator, &target_provider, options.abandon_detach).await?;
    let device_did = profile.did().to_string();
    let client = reqwest::Client::new();
    let (service_url, secret, token_hash, activating, recovered) = match state.pending_login {
        Some(crate::account_session::PendingLogin::Waiting {
            provider,
            secret,
            token_hash,
        }) => {
            match create_remote(
                &client,
                &provider,
                &token_hash,
                &device_did,
                &options.device_name,
            )
            .await
            {
                Ok(()) => (provider, secret, token_hash, None, None),
                // A waiting handoff holds no grant material, and its token
                // is one-time: a service that refuses to recreate it —
                // because that token was already spent, expired, or the
                // service never made creation idempotent — has ended that
                // handoff, not this profile's ability to link. Take the
                // completed material if the browser got there first;
                // otherwise start a fresh handoff, so a cancelled attempt
                // never strands the profile on a secret no later attempt
                // can revive.
                Err(error) => match consume_once(&client, &provider, &secret).await {
                    Ok(Some(consumed)) => (provider, secret, token_hash, None, Some(consumed)),
                    _ => {
                        eprintln!(
                            "warning: could not resume the pending handoff ({error:#}); \
                             starting a new one"
                        );
                        let (secret, token_hash) = new_secret();
                        crate::account_session::begin_login(
                            profile,
                            &operator,
                            crate::account_session::PendingLogin::Waiting {
                                provider: provider.clone(),
                                secret: secret.clone(),
                                token_hash: token_hash.clone(),
                            },
                        )
                        .await?;
                        create_remote(
                            &client,
                            &provider,
                            &token_hash,
                            &device_did,
                            &options.device_name,
                        )
                        .await?;
                        (provider, secret, token_hash, None, None)
                    }
                },
            }
        }
        Some(crate::account_session::PendingLogin::Activating {
            secret, account, ..
        }) => {
            let secret_bytes = hex::decode(&secret).context("pending link secret is invalid")?;
            let token_hash = blake3::hash(&secret_bytes).to_hex().to_string();
            (
                account.provider.clone(),
                secret,
                token_hash,
                Some(account),
                None,
            )
        }
        None => {
            let (secret, token_hash) = new_secret();
            crate::account_session::begin_login(
                profile,
                &operator,
                crate::account_session::PendingLogin::Waiting {
                    provider: options.service_url.trim_end_matches('/').to_string(),
                    secret: secret.clone(),
                    token_hash: token_hash.clone(),
                },
            )
            .await?;
            create_remote(
                &client,
                &options.service_url,
                &token_hash,
                &device_did,
                &options.device_name,
            )
            .await?;
            (
                options.service_url.trim_end_matches('/').to_string(),
                secret,
                token_hash,
                None,
                None,
            )
        }
    };
    let url = handoff_url(&options.account_url, &secret);
    let awaits_approval = activating.is_none() && recovered.is_none();
    if awaits_approval {
        println!("Open this URL to approve the device:\n{url}");
    }
    if awaits_approval && options.open_browser && webbrowser::open(&url).is_err() {
        eprintln!("Could not open a browser; use the URL above.");
    }

    // Keep one listener alive for the whole wait. Once Tokio installs its
    // SIGINT handler, dropping a per-poll listener leaves the default handler
    // replaced and swallows Ctrl-C during the backoff between polls.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    let mut delay = Duration::from_millis(500);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5 * 60);
    let consumed = if recovered.is_some() {
        recovered
    } else if activating.is_some() {
        None
    } else {
        Some(loop {
            if tokio::time::Instant::now() >= deadline {
                bail!("account link expired; run `tonk account link` again");
            }
            tokio::select! {
                result = consume_once(&client, &options.service_url, &secret) => {
                    if let Some(consumed) = result? {
                        break consumed;
                    }
                }
                signal = &mut ctrl_c => {
                    signal.context("failed to listen for Ctrl-C")?;
                    bail!("account link cancelled");
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                signal = &mut ctrl_c => {
                    signal.context("failed to listen for Ctrl-C")?;
                    bail!("account link cancelled");
                }
            }
            delay = (delay * 2).min(Duration::from_secs(5));
        })
    };
    let account = if let Some(consumed) = consumed {
        let account = active_from_consumed(profile, &service_url, &consumed).await?;
        crate::account_session::begin_login(
            profile,
            &operator,
            crate::account_session::PendingLogin::Activating {
                provider: service_url.clone(),
                secret: secret.clone(),
                account: account.clone(),
            },
        )
        .await?;
        account
    } else {
        let guard = crate::account_session::shared_remote_guard(&store)?;
        match crate::account_session::load_guarded(profile, &operator, &guard)
            .await?
            .pending_login
        {
            Some(crate::account_session::PendingLogin::Activating { account, .. }) => account,
            _ => bail!("pending activation state was lost"),
        }
    };
    persist_pending_projection(profile, &operator, &account).await?;
    activate_remote(profile, &account, &token_hash).await?;
    crate::account_session::finish_activation(profile, &operator, &account).await?;
    let root_did = account.root_did.clone();
    let ensured = match tokio::time::timeout(
        ACCOUNT_STATE_ENSURE_TIMEOUT,
        crate::account_state::ensure(profile),
    )
    .await
    {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => crate::account_state::EnsureOutcome {
            status: AccountStateStatus::Unhydrated,
            warning: Some(error.to_string()),
        },
        Err(_) => crate::account_state::EnsureOutcome {
            status: AccountStateStatus::Unhydrated,
            warning: Some("account repository hydration timed out".to_string()),
        },
    };
    Ok(LinkOutcome {
        url,
        root_did,
        device_did,
        account_state: ensured.status,
        warning: ensured.warning,
    })
}

/// One registry row from `POST /devices/list`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    /// Exact attachment generation.
    pub attachment_id: String,
    /// The device's DID.
    pub did: String,
    /// Display name registered at link time.
    pub name: String,
    /// Registry status: `active` or `revoked`.
    pub status: String,
    /// Registration time, seconds since the epoch.
    pub created_at: u64,
    /// CID of the `root → device` delegation a revocation must name.
    pub delegation_cid: String,
}

/// Authenticated provider attachment used by account-scoped CLI modules.
pub(crate) struct AccountConnection {
    pub(crate) service_url: String,
    pub(crate) root_did: Did,
    pub(crate) link: DelegationChain,
    store: crate::spot::SpotStore,
}

async fn connection_from_provider(
    profile: &Profile,
    provider: AccountProviderRecord,
) -> Result<AccountConnection> {
    let service_url = provider.provider().to_string();
    let root = crate::identity::local_root(profile)
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
        store: crate::spot::SpotStore::open().context("failed to locate account state")?,
    })
}

/// Load an attachment through the account directory owned by an explicit
/// spot store. Account-spots tests and isolated consumers use this without
/// changing process-global profile paths.
pub(crate) async fn connection_for_store(
    profile: &Profile,
    store: &crate::spot::SpotStore,
) -> Result<AccountConnection> {
    #[cfg(feature = "integration-tests")]
    if let Some((service_url, link, _)) = integration_connections()
        .lock()
        .expect("integration connection registry")
        .get(profile.did().as_ref())
        .cloned()
    {
        return Ok(AccountConnection {
            service_url,
            root_did: link.issuer().clone(),
            link,
            store: store.clone(),
        });
    }
    let operator = crate::account_state::credential_operator_for_store(profile, store).await?;
    let _ = crate::account_session::flush_pending_for_store(profile, &operator, store).await;
    let provider = stored_provider_for_store(profile, &operator, store)
        .await?
        .context("no account provider is attached; run `tonk account link`")?;
    let root = crate::identity::local_root_with_operator(profile, &operator)
        .await?
        .context("the provider attachment has no local root")?;
    let bytes = hex::decode(root.delegation_hex).context("stored local-root hex is invalid")?;
    let link = DelegationChain::try_from(bytes.as_slice())
        .context("stored local-root delegation is invalid")?;
    Ok(AccountConnection {
        service_url: provider.provider().to_string(),
        root_did: link.issuer().clone(),
        link,
        store: store.clone(),
    })
}

/// Load the attached provider and exact `root → device` link when present.
pub(crate) async fn optional_connection(profile: &Profile) -> Result<Option<AccountConnection>> {
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
            store: crate::spot::SpotStore::open().context("failed to locate account state")?,
        }));
    }
    let Some(provider) = stored_provider(profile).await? else {
        return Ok(None);
    };
    Ok(Some(connection_from_provider(profile, provider).await?))
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
        pending_detaches: Vec::new(),
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

impl AccountConnection {
    /// Sign and POST one account invocation, preserving the raw HTTP status
    /// so callers can implement rolling-deployment fallbacks.
    pub(crate) async fn signed_post(
        &self,
        profile: &Profile,
        path: &str,
        command: Vec<String>,
        arguments: std::collections::BTreeMap<String, Promised>,
    ) -> Result<reqwest::Response> {
        #[cfg(feature = "integration-tests")]
        if integration_connections()
            .lock()
            .expect("integration connection registry")
            .contains_key(profile.did().as_ref())
        {
            let body = tonk_identity::request::build_device_invocation(
                profile.signer().signer().clone(),
                &self.link,
                command,
                arguments,
            )
            .await
            .context("failed to sign the account-service request")?;
            return post_invocation_raw(&self.service_url, path, body).await;
        }

        let operator =
            crate::account_state::credential_operator_for_store(profile, &self.store).await?;
        let _ =
            crate::account_session::flush_pending_for_store(profile, &operator, &self.store).await;
        let store = self.store.clone();
        {
            let guard = crate::account_session::exclusive_transition_guard(&store)?;
            crate::account_session::ensure_initialized(profile, &operator, &guard).await?;
        }
        let guard = crate::account_session::shared_remote_guard(&store)?;
        let active = crate::account_session::active_guarded(profile, &operator, &guard)
            .await?
            .context("no active account; run `tonk account link`")?;
        if active.provider.trim_end_matches('/') != self.service_url.trim_end_matches('/')
            || active.root_did != self.root_did.to_string()
        {
            bail!("account connection does not match the active attachment");
        }
        let bytes =
            hex::decode(&active.delegation_hex).context("active account grant hex is invalid")?;
        let link = DelegationChain::try_from(bytes.as_slice())
            .context("active account grant is invalid")?;
        if link.proof_cids().len() != 1 || link.proof_cids()[0].to_string() != active.delegation_cid
        {
            bail!("active account grant does not match canonical session state");
        }
        let body = tonk_identity::request::build_device_invocation(
            profile.signer().signer().clone(),
            &link,
            command,
            arguments,
        )
        .await
        .context("failed to sign the account-service request")?;
        let response = post_invocation_raw(&self.service_url, path, body).await;
        drop(guard);
        response
    }
}

async fn post_invocation_raw(
    service_url: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response> {
    reqwest::Client::new()
        .post(format!(
            "{}/{}",
            service_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/cbor")
        .body(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("failed to reach the account service at {path}"))
}

async fn post_invocation(
    service_url: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response> {
    let response = post_invocation_raw(service_url, path, body).await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("account service rejected {path} ({status}): {text}");
    }
    Ok(response)
}

/// List the devices registered under this profile's account.
pub async fn devices(profile: &Profile, service_url: &str) -> Result<Vec<DeviceRow>> {
    let _ = retry_pending_detaches(profile).await;
    let connection = optional_connection(profile)
        .await?
        .context("no active account; run `tonk account link`")?;
    if connection.service_url.trim_end_matches('/') != service_url.trim_end_matches('/') {
        bail!("requested provider does not match the active account");
    }
    let response = connection
        .signed_post(
            profile,
            "devices/list",
            vec!["account".into(), "device".into(), "list".into()],
            std::collections::BTreeMap::new(),
        )
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("account service rejected devices/list ({status}): {text}");
    }
    response
        .json()
        .await
        .context("account service returned an invalid device list")
}

/// The browser URL that runs the revoke ceremony for `did`.
///
/// A query parameter, not a fragment: the fragment carries bearer
/// secrets in the link handoff, and a device DID is neither secret nor
/// sensitive to leak into a browser history. The DID needs no escaping —
/// `:` is a legal query character and the rest is base58.
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
fn login_url(base: &str, audience: &str, callback: &str) -> String {
    format!(
        "{}?audience={}&callback={}",
        base.trim_end_matches('#').trim_end_matches('/'),
        urlencoding::encode(audience),
        urlencoding::encode(callback),
    )
}

fn revoke_url(base: &str, did: &str) -> String {
    format!(
        "{}?revoke={did}",
        base.trim_end_matches('#').trim_end_matches('/')
    )
}

/// Inputs for a browser-assisted revocation.
pub struct RevokeOptions {
    /// Account service base URL.
    pub service_url: String,
    /// Browser page that runs the ceremony.
    pub account_url: String,
    /// Whether to ask the OS to open the ceremony URL.
    pub open_browser: bool,
}

/// How a revocation request resolved. The caller owns the messaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    /// The registry now shows the device revoked.
    Revoked,
    /// The registry already showed the device revoked; nothing to do.
    AlreadyRevoked,
}

/// Revoke a device. The current device self-signs immediately; another device
/// requires the browser/passkey ceremony, then the CLI watches projection.
pub async fn revoke(
    profile: &Profile,
    options: &RevokeOptions,
    did: &str,
) -> Result<RevokeOutcome> {
    if profile.did().as_ref() == did {
        let connection = optional_connection(profile)
            .await?
            .context("no active account; run `tonk account link`")?;
        let link = connection.link.clone();
        let target = link.proof_cids()[0];
        let artifact = tonk_identity::revocation::mint_self_revocation(
            profile.signer().signer().clone(),
            &link,
            &target,
        )
        .await
        .context("failed to sign self-revocation")?;
        let rows = devices(profile, &options.service_url).await?;
        let row = rows
            .iter()
            .find(|row| row.did == did && row.delegation_cid == target.to_string())
            .context("the active self attachment is missing from the device list")?;
        let arguments = [
            (
                "attachmentId".to_owned(),
                dialog_ucan_core::promise::Promised::String(row.attachment_id.clone()),
            ),
            (
                "did".to_owned(),
                dialog_ucan_core::promise::Promised::String(did.to_string()),
            ),
            (
                "revocation".to_owned(),
                dialog_ucan_core::promise::Promised::String(hex::encode(artifact)),
            ),
        ]
        .into_iter()
        .collect();
        let response = connection
            .signed_post(
                profile,
                "devices/revoke",
                vec!["account".into(), "device".into(), "revoke".into()],
                arguments,
            )
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            bail!("account service rejected devices/revoke ({status}): {text}");
        }
        return Ok(RevokeOutcome::Revoked);
    }

    let rows = devices(profile, &options.service_url).await?;
    let target = rows
        .iter()
        .find(|row| row.did == did)
        .with_context(|| format!("no device {did} under this account"))?;
    if target.status == "revoked" {
        return Ok(RevokeOutcome::AlreadyRevoked);
    }

    let target_attachment = target.attachment_id.clone();
    let url = format!(
        "{}&attachment={}",
        revoke_url(&options.account_url, did),
        target_attachment
    );
    println!("Approve this revocation with your passkey:\n{url}");
    if options.open_browser && webbrowser::open(&url).is_err() {
        eprintln!("Could not open a browser; use the URL above.");
    }

    // One pinned listener for the whole wait. Tokio replaces the
    // process's default SIGINT handling the first time this is polled
    // and never restores it, so a fresh `ctrl_c()` per iteration would
    // swallow any Ctrl-C that lands between polls.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    // A failed poll is not a failed revocation — the user may be
    // mid-passkey while the service hiccups — so polling tolerates
    // errors until the deadline and reports the last one then.
    let mut last_error: Option<anyhow::Error> = None;
    let mut delay = Duration::from_millis(500);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5 * 60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            match last_error {
                Some(error) => bail!(
                    "revocation was not approved in time (last poll failed: {error:#}); \
                     run `tonk account revoke` again"
                ),
                None => {
                    bail!("revocation was not approved in time; run `tonk account revoke` again")
                }
            }
        }
        tokio::select! {
            rows = devices(profile, &options.service_url) => match rows {
                Ok(rows) => {
                    last_error = None;
                    if rows.iter().any(|row| {
                        row.did == did
                            && row.attachment_id == target_attachment
                            && row.status == "revoked"
                    }) {
                        return Ok(RevokeOutcome::Revoked);
                    }
                }
                Err(error) => last_error = Some(error),
            },
            signal = &mut ctrl_c => {
                signal.context("failed to listen for Ctrl-C")?;
                bail!("revocation cancelled");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            signal = &mut ctrl_c => {
                signal.context("failed to listen for Ctrl-C")?;
                bail!("revocation cancelled");
            }
        }
        delay = (delay * 2).min(Duration::from_secs(5));
    }
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
            DEFAULT_ACCOUNT_URL,
            "did:key:zProfile",
            "http://127.0.0.1:54321",
        );
        assert_eq!(
            url,
            "https://tonk.spot/account/link\
             ?audience=did%3Akey%3AzProfile\
             &callback=http%3A%2F%2F127.0.0.1%3A54321"
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
            .issuer(account)
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
            .issuer(account)
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

    #[test]
    fn it_points_the_revoke_ceremony_at_the_named_device() {
        assert_eq!(
            revoke_url(DEFAULT_ACCOUNT_PAGE, "did:key:zDevice"),
            "https://tonk.spot/account?revoke=did:key:zDevice"
        );
    }

    /// The revoke deep link must not default to the link-handoff page:
    /// `/account/link` consumes a fragment secret and errors without
    /// one, so a `?revoke=` sent there is never read.
    #[test]
    fn it_does_not_hand_the_revoke_ceremony_to_the_link_page() {
        assert_ne!(DEFAULT_ACCOUNT_PAGE, DEFAULT_ACCOUNT_URL);
        assert!(!DEFAULT_ACCOUNT_PAGE.ends_with("/link"));
    }

    #[test]
    fn it_keeps_the_bearer_secret_in_the_fragment() {
        assert_eq!(
            handoff_url("https://tonk.spot/account/link", "secret"),
            "https://tonk.spot/account/link#secret"
        );
    }

    #[test]
    fn it_hashes_a_new_secret_before_storage() {
        let (secret, hash) = new_secret();
        let bytes = hex::decode(secret).unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(hash, blake3::hash(&bytes).to_hex().to_string());
    }

    #[test]
    fn it_parses_a_service_device_row() {
        let rows: Vec<DeviceRow> = serde_json::from_str(
            r#"[{"attachmentId":"generation","did":"did:key:z1","name":"laptop","status":"active",
                 "delegationCid":"bafy","createdAt":1753300000}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].did, "did:key:z1");
        assert_eq!(rows[0].created_at, 1_753_300_000);
    }
}

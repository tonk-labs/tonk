//! Native account status and browser-assisted device linking.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialog_operator::Profile;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan_core::DelegationChain;
use rand::RngCore;
use serde::{Deserialize, Serialize};
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateLinkRequest<'a> {
    token_hash: &'a str,
    device_did: &'a str,
    device_name: &'a str,
}

#[derive(Serialize)]
struct SecretRequest<'a> {
    secret: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsumeResponse {
    delegation_hex: String,
    credential_id: String,
    descriptor_hex: String,
}

fn storage() -> Storage<NativeSpace> {
    Storage::<NativeSpace>::default()
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

async fn stored_provider(profile: &Profile) -> Result<Option<AccountProviderRecord>> {
    let Some(root) = crate::identity::local_root(profile).await? else {
        return Ok(None);
    };
    let root_did = parse_root_did(&root.root_did)?;
    let bytes = profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .load::<Vec<u8>>()
        .perform(&storage())
        .await;
    decode_provider(&root_did, bytes).await
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

async fn persist(
    profile: &Profile,
    service_url: &str,
    credential_id: String,
    delegation_hex: String,
    descriptor_hex: &str,
) -> Result<String> {
    let root = crate::identity::save_local_root(profile, credential_id, delegation_hex).await?;
    let root_did: dialog_varsig::Did = root
        .root_did
        .parse()
        .context("stored local root DID is invalid")?;
    let descriptor =
        hex::decode(descriptor_hex).context("invalid descriptor hex from account service")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let record = AccountProviderRecord::attach(service_url, &descriptor, &root_did, now)
        .await
        .context("account service returned an unusable repository descriptor")?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(
            record
                .encode()
                .context("failed to serialize account provider")?,
        )
        .perform(&storage())
        .await
        .context("failed to attach the account provider")?;
    Ok(root.root_did)
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
        .json(&CreateLinkRequest {
            token_hash,
            device_did,
            device_name,
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
) -> Result<Option<ConsumeResponse>> {
    let response = client
        .post(format!(
            "{}/links/consume",
            service_url.trim_end_matches('/')
        ))
        .json(&SecretRequest { secret })
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
    Ok(Some(response.json::<ConsumeResponse>().await.context(
        "account service returned an invalid link response",
    )?))
}

/// Start a browser handoff, wait for its one-time result, and persist it.
pub async fn link(profile: &Profile, options: &LinkOptions) -> Result<LinkOutcome> {
    if matches!(status(profile).await?, AccountStatus::Registered { .. }) {
        bail!("this profile already has an account provider attached");
    }
    let device_did = profile.did().to_string();
    let (secret, token_hash) = new_secret();
    let client = reqwest::Client::new();
    create_remote(
        &client,
        &options.service_url,
        &token_hash,
        &device_did,
        &options.device_name,
    )
    .await?;
    let url = handoff_url(&options.account_url, &secret);
    println!("Open this URL to approve the device:\n{url}");
    if options.open_browser && webbrowser::open(&url).is_err() {
        eprintln!("Could not open a browser; use the URL above.");
    }

    let mut delay = Duration::from_millis(500);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5 * 60);
    let consumed = loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("account link expired; run `tonk account link` again");
        }
        tokio::select! {
            result = consume_once(&client, &options.service_url, &secret) => {
                if let Some(consumed) = result? {
                    break consumed;
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for Ctrl-C")?;
                bail!("account link cancelled");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
    };
    let root_did = persist(
        profile,
        &options.service_url,
        consumed.credential_id,
        consumed.delegation_hex,
        &consumed.descriptor_hex,
    )
    .await?;
    let ensured = match crate::account_state::ensure(profile).await {
        Ok(outcome) => outcome,
        Err(error) => crate::account_state::EnsureOutcome {
            status: AccountStateStatus::Unhydrated,
            warning: Some(error.to_string()),
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

async fn linked_chain(profile: &Profile) -> Result<DelegationChain> {
    stored_provider(profile)
        .await?
        .context("no account provider is attached; run `tonk account link`")?;
    let root = crate::identity::local_root(profile)
        .await?
        .context("the provider attachment has no local root")?;
    let bytes = hex::decode(root.delegation_hex).context("stored local-root hex is invalid")?;
    DelegationChain::try_from(bytes.as_slice()).context("stored local-root delegation is invalid")
}

async fn post_invocation(
    service_url: &str,
    path: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response> {
    let response = reqwest::Client::new()
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
        .with_context(|| format!("failed to reach the account service at {path}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("account service rejected {path} ({status}): {text}");
    }
    Ok(response)
}

/// List the devices registered under this profile's account.
pub async fn devices(profile: &Profile, service_url: &str) -> Result<Vec<DeviceRow>> {
    let link = linked_chain(profile).await?;
    let body = tonk_identity::request::build_device_invocation(
        profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "device".into(), "list".into()],
        std::collections::BTreeMap::new(),
    )
    .await
    .context("failed to sign the device-list request")?;
    let response = post_invocation(service_url, "devices/list", body).await?;
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
        let link = linked_chain(profile).await?;
        let target = link.proof_cids()[0];
        let artifact = tonk_identity::revocation::mint_self_revocation(
            profile.signer().signer().clone(),
            &link,
            &target,
        )
        .await
        .context("failed to sign self-revocation")?;
        let arguments = [
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
        let body = tonk_identity::request::build_device_invocation(
            profile.signer().signer().clone(),
            &link,
            vec!["account".into(), "device".into(), "revoke".into()],
            arguments,
        )
        .await
        .context("failed to build self-revoke request")?;
        post_invocation(&options.service_url, "devices/revoke", body).await?;
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

    let url = revoke_url(&options.account_url, did);
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
                    if rows.iter().any(|row| row.did == did && row.status == "revoked") {
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
            r#"[{"did":"did:key:z1","name":"laptop","status":"active",
                 "delegationCid":"bafy","createdAt":1753300000}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].did, "did:key:z1");
        assert_eq!(rows[0].created_at, 1_753_300_000);
    }
}

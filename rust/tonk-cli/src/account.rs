//! Native account status and browser-assisted device linking.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use dialog_effects::credential::CredentialError;
use dialog_operator::Profile;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Production account API used unless explicitly overridden.
pub const DEFAULT_SERVICE_URL: &str = "https://accounts.tonk.xyz";
/// Production top-document ceremony route.
pub const DEFAULT_ACCOUNT_URL: &str = "https://tonk.spot/account/link";
/// Credential-store key shared with the browser worker.
pub const ACCOUNT_LINK_SITE: &str = "tonk-account-link-v1";

/// Current native profile account-link state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStatus {
    /// No account delegation is stored for the profile.
    Unlinked {
        /// Native profile DID.
        device_did: String,
    },
    /// A root-to-profile delegation is stored.
    Linked {
        /// Account root DID that issued the delegation.
        root_did: String,
        /// Native profile DID receiving the delegation.
        device_did: String,
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
}

fn storage() -> Storage<NativeSpace> {
    Storage::<NativeSpace>::default()
}

async fn stored_link(profile: &Profile) -> Result<Option<Vec<u8>>> {
    match profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .load::<Vec<u8>>()
        .perform(&storage())
        .await
    {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
        Err(CredentialError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).context("failed to load the local account link"),
    }
}

/// Read the current profile's local account status.
pub async fn status(profile: &Profile) -> Result<AccountStatus> {
    let device_did = profile.did();
    let Some(bytes) = stored_link(profile).await? else {
        return Ok(AccountStatus::Unlinked {
            device_did: device_did.to_string(),
        });
    };
    let chain = DelegationChain::try_from(bytes.as_slice())
        .context("stored account delegation is invalid")?;
    if chain.audience() != &device_did {
        bail!("stored account delegation targets another profile");
    }
    Ok(AccountStatus::Linked {
        root_did: chain.issuer().to_string(),
        device_did: device_did.to_string(),
    })
}

async fn persist(profile: &Profile, delegation_hex: &str) -> Result<String> {
    let bytes =
        hex::decode(delegation_hex).context("invalid delegation hex from account service")?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .context("invalid delegation from account service")?;
    if chain.proof_cids().len() != 1 || chain.subject().is_some() {
        bail!("account delegation has an invalid shape");
    }
    if chain.audience() != &profile.did() {
        bail!("account delegation targets another profile");
    }
    let proof = chain
        .proofs()
        .next()
        .context("account delegation is missing its proof")?;
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .context("account delegation signature is invalid")?;

    if let Some(existing) = stored_link(profile).await? {
        let existing = DelegationChain::try_from(existing.as_slice())
            .context("stored account delegation is invalid")?;
        if existing.issuer() != chain.issuer() {
            bail!("profile is already linked to another account root");
        }
    }

    let root_did = chain.issuer().to_string();
    profile
        .save(UcanDelegation(chain))
        .perform(&storage())
        .await
        .context("failed to save account delegation")?;
    profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(bytes)
        .perform(&storage())
        .await
        .context("failed to save local account link")?;
    Ok(root_did)
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
) -> Result<Option<String>> {
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
    Ok(Some(
        response
            .json::<ConsumeResponse>()
            .await
            .context("account service returned an invalid link response")?
            .delegation_hex,
    ))
}

/// Start a browser handoff, wait for its one-time result, and persist it.
pub async fn link(profile: &Profile, options: &LinkOptions) -> Result<LinkOutcome> {
    if matches!(status(profile).await?, AccountStatus::Linked { .. }) {
        bail!("this profile is already linked to an account");
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
    let delegation_hex = loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("account link expired; run `tonk account link` again");
        }
        tokio::select! {
            result = consume_once(&client, &options.service_url, &secret) => {
                if let Some(delegation) = result? {
                    break delegation;
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
    let root_did = persist(profile, &delegation_hex).await?;
    Ok(LinkOutcome {
        url,
        root_did,
        device_did,
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
}

fn revoke_target_guard(own_did: &str, target_did: &str) -> Result<()> {
    if own_did == target_did {
        bail!("refusing to revoke the device you are using");
    }
    Ok(())
}

async fn linked_chain(profile: &Profile) -> Result<DelegationChain> {
    let bytes = stored_link(profile)
        .await?
        .context("this profile is not linked to an account; run `tonk account link`")?;
    DelegationChain::try_from(bytes.as_slice()).context("stored account delegation is invalid")
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

/// Revoke another of the account's devices.
pub async fn revoke(profile: &Profile, service_url: &str, did: &str) -> Result<()> {
    revoke_target_guard(profile.did().as_ref(), did)?;
    let link = linked_chain(profile).await?;
    let arguments = [(
        "did".to_owned(),
        dialog_ucan_core::promise::Promised::String(did.to_owned()),
    )]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    .context("failed to sign the revoke request")?;
    post_invocation(service_url, "devices/revoke", body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn it_refuses_to_revoke_the_own_device_did() {
        assert!(revoke_target_guard("did:key:same", "did:key:same").is_err());
        assert!(revoke_target_guard("did:key:same", "did:key:other").is_ok());
    }
}

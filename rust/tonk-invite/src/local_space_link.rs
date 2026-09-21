//! Signed browser consent for adopting one local-only space into an account.
//!
//! This module deliberately does not create a second authorization system.
//! The request and receipts are short-lived UCANs used to bind user consent
//! to one exact handoff. After consent, the local owner issues the ordinary
//! space delegation targeted to the selected account; the browser imports it
//! through the normal join/provision/account-directory path.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Signer, Signer};
use dialog_ucan_core::{
    DelegationBuilder, DelegationChain,
    command::Command,
    delegation::chain::check_chain,
    subject::Subject,
    time::{
        Timestamp,
        timestamp::{Duration, UNIX_EPOCH},
    },
};
use dialog_varsig::{Did, Principal};
use ipld_core::ipld::Ipld;
use url::Url;

/// Handoffs expire quickly; the resulting ordinary space delegation has its
/// own independently bounded lifetime.
pub const HANDOFF_TTL_SECONDS: u64 = 5 * 60;
/// Upper bound for any encoded request or callback payload.
pub const MAX_HANDOFF_BYTES: usize = 256 * 1024;

const REQUEST_COMMAND: &[&str] = &["link", "local-space", "request"];
const APPROVE_COMMAND: &[&str] = &["link", "local-space", "approve"];
const COMPLETE_COMMAND: &[&str] = &["link", "local-space", "complete"];
const META_CALLBACK: &str = "tonk.link.callback";
const META_CORRELATION: &str = "tonk.link.correlation";
const META_NAME: &str = "tonk.link.name";
const META_PUBLICATION: &str = "tonk.link.publication";
const META_REQUEST: &str = "tonk.link.request";
const META_SERVICE_DID: &str = "tonk.link.service.did";
const META_SERVICE_URL: &str = "tonk.link.service.url";
const META_SPACE: &str = "tonk.link.space";

/// Encode one bounded public handoff payload for URLs and callback forms.
pub fn encode_transport(bytes: &[u8]) -> Result<String> {
    ensure!(
        bytes.len() <= MAX_HANDOFF_BYTES,
        "local_space_link_payload_too_large"
    );
    Ok(bs58::encode(bytes).into_string())
}

/// Decode one bounded public handoff payload from a URL or callback form.
pub fn decode_transport(value: &str) -> Result<Vec<u8>> {
    // Base58 expands by less than 1.5x. Reject an oversized string before
    // allocating its decoded representation.
    ensure!(
        value.len() <= MAX_HANDOFF_BYTES * 3 / 2,
        "local_space_link_payload_too_large"
    );
    let bytes = bs58::decode(value)
        .into_vec()
        .context("local_space_link_payload_invalid")?;
    ensure!(
        bytes.len() <= MAX_HANDOFF_BYTES,
        "local_space_link_payload_too_large"
    );
    Ok(bytes)
}

fn command(parts: &[&str]) -> Command {
    Command(parts.iter().map(|part| (*part).to_owned()).collect())
}

fn expiry_after(now: Timestamp) -> Result<Timestamp> {
    let seconds = now
        .to_unix()
        .checked_add(HANDOFF_TTL_SECONDS)
        .context("local_space_link_time_overflow")?;
    Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds))
        .context("local_space_link_time_invalid")
}

fn text_meta(entries: impl IntoIterator<Item = (&'static str, String)>) -> BTreeMap<String, Ipld> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Ipld::String(value)))
        .collect()
}

fn meta_string<'a>(chain: &'a DelegationChain, key: &str) -> Result<&'a str> {
    match chain
        .proofs()
        .last()
        .context("local_space_link_invalid_chain")?
        .meta()
        .get(key)
    {
        Some(Ipld::String(value)) => Ok(value),
        _ => anyhow::bail!("local_space_link_missing_binding:{key}"),
    }
}

fn validate_loopback_callback(callback: &Url) -> Result<()> {
    let loopback = match callback.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    ensure!(
        callback.scheme() == "http"
            && loopback
            && callback.username().is_empty()
            && callback.password().is_none()
            && callback.fragment().is_none(),
        "local_space_link_callback_not_loopback"
    );
    Ok(())
}

fn validate_service_url(service_url: &Url) -> Result<()> {
    let loopback = match service_url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    ensure!(
        (service_url.scheme() == "https" || (service_url.scheme() == "http" && loopback))
            && service_url.host().is_some()
            && service_url.username().is_empty()
            && service_url.password().is_none()
            && service_url.fragment().is_none(),
        "local_space_link_untrusted_service"
    );
    Ok(())
}

async fn validate_chain(
    chain: &DelegationChain,
    subject: &Did,
    audience: &Did,
    exact_command: &[&str],
    now: Timestamp,
    direct: bool,
) -> Result<Timestamp> {
    let proof_count = chain.proofs().count();
    ensure!(proof_count > 0, "local_space_link_invalid_chain");
    if direct {
        ensure!(proof_count == 1, "local_space_link_overbroad_chain");
    }
    let root_subject = chain.subject().unwrap_or_else(|| chain.issuer());
    ensure!(root_subject == subject, "local_space_link_subject_mismatch");
    ensure!(
        chain.audience() == audience,
        "local_space_link_recipient_mismatch"
    );
    check_chain(chain.proofs(), subject, Some(now)).context("local_space_link_invalid_chain")?;
    let leaf = chain
        .proofs()
        .last()
        .context("local_space_link_invalid_chain")?;
    for hop in chain.proofs() {
        hop.verify_signature(&DidKeyResolver)
            .await
            .context("local_space_link_invalid_signature")?;
    }
    ensure!(
        leaf.command() == &command(exact_command) && leaf.policy().is_empty(),
        "local_space_link_scope_mismatch"
    );
    leaf.expiration().context("local_space_link_missing_expiry")
}

/// Independently trusted account-service identity and endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedService {
    /// Service principal expected by the deployment.
    pub did: Did,
    /// Exact access endpoint expected by the deployment.
    pub url: Url,
}

impl TrustedService {
    /// Construct trusted routing supplied by deployment discovery.
    pub fn new(did: Did, url: Url) -> Result<Self> {
        validate_service_url(&url)?;
        Ok(Self { did, url })
    }
}

/// Space-signed request displayed by the browser before account consent.
#[derive(Clone, Debug)]
pub struct LocalSpaceLinkRequest {
    chain: DelegationChain,
    callback: Url,
    correlation: String,
    name: String,
}

impl LocalSpaceLinkRequest {
    /// Sign a short-lived request as the existing repository identity.
    pub async fn issue(
        owner: &Ed25519Signer,
        recipient: &Did,
        callback: Url,
        correlation: String,
        name: String,
        service: &TrustedService,
        now: Timestamp,
    ) -> Result<Self> {
        validate_loopback_callback(&callback)?;
        ensure!(
            correlation.len() >= 32 && correlation.len() <= 128,
            "local_space_link_invalid_correlation"
        );
        ensure!(
            !name.trim().is_empty() && name.len() <= 256,
            "local_space_link_invalid_name"
        );
        let space = owner.did();
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(recipient)
            .subject(Subject::Specific(space.clone()))
            .command(
                REQUEST_COMMAND
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect(),
            )
            .expiration(expiry_after(now)?)
            .meta(text_meta([
                (META_CALLBACK, callback.to_string()),
                (META_CORRELATION, correlation.clone()),
                (META_NAME, name.clone()),
                (META_SERVICE_DID, service.did.to_string()),
                (META_SERVICE_URL, service.url.to_string()),
                (META_SPACE, space.to_string()),
            ]))
            .try_build()
            .await?;
        Ok(Self {
            chain: DelegationChain::new(grant),
            callback,
            correlation,
            name,
        })
    }

    /// Decode a bounded request transported by the approval URL.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        let chain = DelegationChain::try_from(bytes).context("local_space_link_invalid_chain")?;
        let callback: Url = meta_string(&chain, META_CALLBACK)?
            .parse()
            .context("local_space_link_callback_invalid")?;
        let correlation = meta_string(&chain, META_CORRELATION)?.to_owned();
        let name = meta_string(&chain, META_NAME)?.to_owned();
        Ok(Self {
            chain,
            callback,
            correlation,
            name,
        })
    }

    /// Encode the public signed request. No private key material is included.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = self.chain.to_bytes()?;
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        Ok(bytes)
    }

    /// Exact loopback callback carried by the signed request.
    pub fn callback(&self) -> &Url {
        &self.callback
    }

    /// Correlation value carried by the signed request.
    pub fn correlation(&self) -> &str {
        &self.correlation
    }

    /// Local registry name displayed for consent.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Space DID displayed before validation; callers must validate before
    /// acting on it.
    pub fn subject_hint(&self) -> Option<&Did> {
        self.chain.subject()
    }

    /// Validate ownership proof and exact deployment routing.
    pub async fn validate(
        &self,
        trusted_service: &TrustedService,
        now: Timestamp,
    ) -> Result<ValidatedLocalSpaceLinkRequest> {
        validate_loopback_callback(&self.callback)?;
        let space = self
            .chain
            .subject()
            .cloned()
            .context("local_space_link_subject_mismatch")?;
        let recipient = self.chain.audience().clone();
        let expires_at =
            validate_chain(&self.chain, &space, &recipient, REQUEST_COMMAND, now, true).await?;
        ensure!(
            meta_string(&self.chain, META_SPACE)? == space.as_str(),
            "local_space_link_subject_mismatch"
        );
        ensure!(
            meta_string(&self.chain, META_CORRELATION)? == self.correlation,
            "local_space_link_correlation_mismatch"
        );
        ensure!(
            meta_string(&self.chain, META_NAME)? == self.name,
            "local_space_link_name_mismatch"
        );
        ensure!(
            meta_string(&self.chain, META_CALLBACK)? == self.callback.as_str(),
            "local_space_link_callback_mismatch"
        );
        ensure!(
            meta_string(&self.chain, META_SERVICE_DID)? == trusted_service.did.as_str()
                && meta_string(&self.chain, META_SERVICE_URL)? == trusted_service.url.as_str(),
            "local_space_link_untrusted_service"
        );
        Ok(ValidatedLocalSpaceLinkRequest {
            space,
            recipient,
            callback: self.callback.clone(),
            correlation: self.correlation.clone(),
            name: self.name.clone(),
            service: trusted_service.clone(),
            request_cid: self.chain.proof_cids()[0].to_string(),
            expires_at,
        })
    }
}

/// A request whose ownership signature, expiry, callback, and service route
/// have all been checked.
#[derive(Clone, Debug)]
pub struct ValidatedLocalSpaceLinkRequest {
    /// Existing local space DID.
    pub space: Did,
    /// Locally retained handoff recipient DID.
    pub recipient: Did,
    /// Exact loopback callback.
    pub callback: Url,
    /// Unpredictable correlation value.
    pub correlation: String,
    /// Local registry name approved by the user.
    pub name: String,
    /// Independently trusted service route.
    pub service: TrustedService,
    /// CID of the signed request.
    pub request_cid: String,
    /// Request deadline.
    pub expires_at: Timestamp,
}

/// Account-signed explicit consent for one validated request.
#[derive(Clone, Debug)]
pub struct LocalSpaceLinkApproval {
    chain: DelegationChain,
}

impl LocalSpaceLinkApproval {
    /// Sign consent with the selected browser account.
    pub async fn issue(
        request: &ValidatedLocalSpaceLinkRequest,
        account: &Ed25519Signer,
        now: Timestamp,
    ) -> Result<Self> {
        let expiration = expiry_after(now)?.min(request.expires_at);
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(account.clone()))
            .audience(&request.recipient)
            .subject(Subject::Specific(account.did()))
            .command(
                APPROVE_COMMAND
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect(),
            )
            .expiration(expiration)
            .meta(binding_meta(request))
            .try_build()
            .await?;
        Ok(Self {
            chain: DelegationChain::new(grant),
        })
    }

    /// Extend an existing account-to-browser device grant to the exact
    /// handoff recipient. This lets an authenticated browser approve without
    /// exporting or re-deriving the account root key.
    pub async fn issue_from_device(
        request: &ValidatedLocalSpaceLinkRequest,
        account_link: DelegationChain,
        device: &Signer,
        now: Timestamp,
    ) -> Result<Self> {
        ensure!(
            account_link.audience() == &device.did(),
            "local_space_link_account_device_mismatch"
        );
        let account = account_link.issuer().clone();
        let requested_expiration = expiry_after(now)?;
        let expiration = account_link
            .expiration()
            .map_or(requested_expiration, |limit| {
                requested_expiration.min(limit)
            })
            .min(request.expires_at);
        let leaf = DelegationBuilder::new()
            .issuer(device.clone())
            .audience(&request.recipient)
            .subject(Subject::Specific(account))
            .command(
                APPROVE_COMMAND
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect(),
            )
            .expiration(expiration)
            .meta(binding_meta(request))
            .try_build()
            .await?;
        Ok(Self {
            chain: account_link
                .push(leaf)
                .context("local_space_link_invalid_account_chain")?,
        })
    }

    /// Decode a bounded callback payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        Ok(Self {
            chain: DelegationChain::try_from(bytes).context("local_space_link_invalid_approval")?,
        })
    }

    /// Encode the public signed consent proof.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = self.chain.to_bytes()?;
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        Ok(bytes)
    }

    /// Verify exact request binding and, on retries, the already-selected account.
    pub async fn validate(
        &self,
        request: &ValidatedLocalSpaceLinkRequest,
        expected_account: Option<&Did>,
        now: Timestamp,
    ) -> Result<ValidatedLocalSpaceLinkApproval> {
        let account = self.chain.issuer().clone();
        if let Some(expected) = expected_account {
            ensure!(&account == expected, "local_space_link_account_mismatch");
        }
        let expires_at = validate_chain(
            &self.chain,
            &account,
            &request.recipient,
            APPROVE_COMMAND,
            now,
            false,
        )
        .await?;
        ensure!(
            expires_at <= request.expires_at,
            "local_space_link_expiry_limited"
        );
        validate_binding(&self.chain, request)?;
        Ok(ValidatedLocalSpaceLinkApproval {
            account,
            request: request.clone(),
        })
    }
}

/// Consent token required before the CLI may mint account-targeted space
/// authority.
#[derive(Clone, Debug)]
pub struct ValidatedLocalSpaceLinkApproval {
    /// Selected browser account.
    pub account: Did,
    /// Exact request that was approved.
    pub request: ValidatedLocalSpaceLinkRequest,
}

/// Account-signed receipt emitted only after browser provisioning and account
/// directory publication have completed.
#[derive(Clone, Debug)]
pub struct LocalSpaceLinkCompletion {
    chain: DelegationChain,
}

impl LocalSpaceLinkCompletion {
    /// Sign the browser's durable publication identifier after publication.
    pub async fn issue(
        approval: &ValidatedLocalSpaceLinkApproval,
        account: &Ed25519Signer,
        publication: String,
        now: Timestamp,
    ) -> Result<Self> {
        ensure!(
            account.did() == approval.account,
            "local_space_link_account_mismatch"
        );
        ensure!(
            !publication.is_empty() && publication.len() <= 256,
            "local_space_link_publication_invalid"
        );
        let mut meta = binding_meta(&approval.request);
        meta.insert(META_PUBLICATION.to_owned(), Ipld::String(publication));
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(account.clone()))
            .audience(&approval.request.recipient)
            .subject(Subject::Specific(account.did()))
            .command(
                COMPLETE_COMMAND
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect(),
            )
            .expiration(expiry_after(now)?.min(approval.request.expires_at))
            .meta(meta)
            .try_build()
            .await?;
        Ok(Self {
            chain: DelegationChain::new(grant),
        })
    }

    /// Extend an account-to-browser device grant into a signed completion
    /// receipt after the normal browser join path has published the space.
    pub async fn issue_from_device(
        approval: &ValidatedLocalSpaceLinkApproval,
        account_link: DelegationChain,
        device: &Signer,
        publication: String,
        now: Timestamp,
    ) -> Result<Self> {
        ensure!(
            account_link.issuer() == &approval.account && account_link.audience() == &device.did(),
            "local_space_link_account_device_mismatch"
        );
        ensure!(
            !publication.is_empty() && publication.len() <= 256,
            "local_space_link_publication_invalid"
        );
        let mut meta = binding_meta(&approval.request);
        meta.insert(META_PUBLICATION.to_owned(), Ipld::String(publication));
        let requested_expiration = expiry_after(now)?;
        let expiration = account_link
            .expiration()
            .map_or(requested_expiration, |limit| {
                requested_expiration.min(limit)
            })
            .min(approval.request.expires_at);
        let leaf = DelegationBuilder::new()
            .issuer(device.clone())
            .audience(&approval.request.recipient)
            .subject(Subject::Specific(approval.account.clone()))
            .command(
                COMPLETE_COMMAND
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect(),
            )
            .expiration(expiration)
            .meta(meta)
            .try_build()
            .await?;
        Ok(Self {
            chain: account_link
                .push(leaf)
                .context("local_space_link_invalid_account_chain")?,
        })
    }

    /// Encode the public signed completion receipt.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = self.chain.to_bytes()?;
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        Ok(bytes)
    }

    /// Decode a bounded completion receipt.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_HANDOFF_BYTES,
            "local_space_link_payload_too_large"
        );
        Ok(Self {
            chain: DelegationChain::try_from(bytes)
                .context("local_space_link_invalid_completion")?,
        })
    }

    /// Verify the same account, request, recipient, route, expiry, and exact
    /// completion scope used throughout the handoff.
    pub async fn validate(
        &self,
        approval: &ValidatedLocalSpaceLinkApproval,
        now: Timestamp,
    ) -> Result<ValidatedLocalSpaceLinkCompletion> {
        let expires_at = validate_chain(
            &self.chain,
            &approval.account,
            &approval.request.recipient,
            COMPLETE_COMMAND,
            now,
            false,
        )
        .await?;
        ensure!(
            expires_at <= approval.request.expires_at,
            "local_space_link_expiry_limited"
        );
        validate_binding(&self.chain, &approval.request)?;
        let publication = meta_string(&self.chain, META_PUBLICATION)?.to_owned();
        ensure!(
            !publication.is_empty() && publication.len() <= 256,
            "local_space_link_publication_invalid"
        );
        Ok(ValidatedLocalSpaceLinkCompletion {
            account: approval.account.clone(),
            space: approval.request.space.clone(),
            publication,
        })
    }
}

/// Fully verified browser publication receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedLocalSpaceLinkCompletion {
    /// Account that adopted the space.
    pub account: Did,
    /// Space adopted by that account.
    pub space: Did,
    /// Browser-produced durable publication identifier.
    pub publication: String,
}

fn binding_meta(request: &ValidatedLocalSpaceLinkRequest) -> BTreeMap<String, Ipld> {
    text_meta([
        (META_CORRELATION, request.correlation.clone()),
        (META_NAME, request.name.clone()),
        (META_REQUEST, request.request_cid.clone()),
        (META_SERVICE_DID, request.service.did.to_string()),
        (META_SERVICE_URL, request.service.url.to_string()),
        (META_SPACE, request.space.to_string()),
    ])
}

fn validate_binding(
    chain: &DelegationChain,
    request: &ValidatedLocalSpaceLinkRequest,
) -> Result<()> {
    for (key, expected) in [
        (META_CORRELATION, request.correlation.as_str()),
        (META_NAME, request.name.as_str()),
        (META_REQUEST, request.request_cid.as_str()),
        (META_SERVICE_DID, request.service.did.as_str()),
        (META_SERVICE_URL, request.service.url.as_str()),
        (META_SPACE, request.space.as_str()),
    ] {
        ensure!(
            meta_string(chain, key)? == expected,
            "local_space_link_binding_mismatch:{key}"
        );
    }
    Ok(())
}

/// A one-use in-memory guard for the signed request CID. Durable retry state
/// may recreate the guard only for the same space/account tuple.
#[derive(Default)]
pub struct LocalSpaceLinkReplayGuard {
    consumed: BTreeSet<String>,
}

impl LocalSpaceLinkReplayGuard {
    /// Consume a consent exactly once.
    pub fn consume(&mut self, approval: &ValidatedLocalSpaceLinkApproval) -> Result<()> {
        ensure!(
            self.consumed.insert(approval.request.request_cid.clone()),
            "local_space_link_replayed"
        );
        Ok(())
    }
}

/// Validate a browser cancellation without treating it as authorization.
pub fn validate_cancellation(
    request: &ValidatedLocalSpaceLinkRequest,
    correlation: &str,
) -> Result<()> {
    ensure!(
        request.correlation == correlation,
        "local_space_link_correlation_mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod local_space_link_tests {
    use super::*;

    fn at(seconds: u64) -> Timestamp {
        Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).unwrap()
    }

    async fn fixture() -> (
        Ed25519Signer,
        Ed25519Signer,
        Ed25519Signer,
        TrustedService,
        LocalSpaceLinkRequest,
        ValidatedLocalSpaceLinkRequest,
    ) {
        let owner = Ed25519Signer::generate().await.unwrap();
        let recipient = Ed25519Signer::generate().await.unwrap();
        let account = Ed25519Signer::generate().await.unwrap();
        let service_signer = Ed25519Signer::generate().await.unwrap();
        let service = TrustedService::new(
            service_signer.did(),
            "https://access.example/ucan/".parse().unwrap(),
        )
        .unwrap();
        let request = LocalSpaceLinkRequest::issue(
            &owner,
            &recipient.did(),
            "http://127.0.0.1:43210/link".parse().unwrap(),
            "0123456789abcdef0123456789abcdef".into(),
            "garden".into(),
            &service,
            at(1_000_000),
        )
        .await
        .unwrap();
        let decoded = LocalSpaceLinkRequest::from_bytes(&request.to_bytes().unwrap()).unwrap();
        let validated = decoded.validate(&service, at(1_000_001)).await.unwrap();
        (owner, recipient, account, service, request, validated)
    }

    #[tokio::test]
    async fn local_space_link_accepts_exact_consent_and_rejects_replay() {
        let (_, _, account, _, _, request) = fixture().await;
        let approval = LocalSpaceLinkApproval::issue(&request, &account, at(1_000_002))
            .await
            .unwrap();
        let decoded = LocalSpaceLinkApproval::from_bytes(&approval.to_bytes().unwrap()).unwrap();
        let validated = decoded
            .validate(&request, Some(&account.did()), at(1_000_003))
            .await
            .unwrap();
        let completion = LocalSpaceLinkCompletion::issue(
            &validated,
            &account,
            "directory-revision-1".into(),
            at(1_000_004),
        )
        .await
        .unwrap();
        let completion = LocalSpaceLinkCompletion::from_bytes(&completion.to_bytes().unwrap())
            .unwrap()
            .validate(&validated, at(1_000_005))
            .await
            .unwrap();
        assert_eq!(completion.space, request.space);
        assert_eq!(completion.account, account.did());
        let mut replay = LocalSpaceLinkReplayGuard::default();
        replay.consume(&validated).unwrap();
        assert_eq!(
            replay.consume(&validated).unwrap_err().to_string(),
            "local_space_link_replayed"
        );
    }

    #[tokio::test]
    async fn local_space_link_accepts_device_signed_account_receipts() {
        let (_, _, account, _, _, request) = fixture().await;
        let device = Ed25519Signer::generate().await.unwrap();
        let account_link = DelegationBuilder::new()
            .issuer(Signer::from(account.clone()))
            .audience(&device.did())
            .subject(Subject::Any)
            .command(Vec::<String>::new())
            .expiration(at(1_001_000))
            .try_build()
            .await
            .unwrap();
        let account_link = DelegationChain::new(account_link);
        let device = Signer::from(device);

        let approval = LocalSpaceLinkApproval::issue_from_device(
            &request,
            account_link.clone(),
            &device,
            at(1_000_002),
        )
        .await
        .unwrap();
        let approval = LocalSpaceLinkApproval::from_bytes(&approval.to_bytes().unwrap())
            .unwrap()
            .validate(&request, Some(&account.did()), at(1_000_003))
            .await
            .unwrap();
        let completion = LocalSpaceLinkCompletion::issue_from_device(
            &approval,
            account_link,
            &device,
            "directory-revision-1".into(),
            at(1_000_004),
        )
        .await
        .unwrap();
        let completion = LocalSpaceLinkCompletion::from_bytes(&completion.to_bytes().unwrap())
            .unwrap()
            .validate(&approval, at(1_000_005))
            .await
            .unwrap();

        assert_eq!(completion.account, account.did());
        assert_eq!(completion.space, request.space);
    }

    #[tokio::test]
    async fn local_space_link_rejects_wrong_account_recipient_and_correlation() {
        let (_, _, account, service, _, request) = fixture().await;
        let other = Ed25519Signer::generate().await.unwrap();
        let approval = LocalSpaceLinkApproval::issue(&request, &account, at(1_000_002))
            .await
            .unwrap();
        assert!(
            approval
                .validate(&request, Some(&other.did()), at(1_000_003))
                .await
                .unwrap_err()
                .to_string()
                .contains("account_mismatch")
        );

        let other_recipient = Ed25519Signer::generate().await.unwrap();
        let wrong_request = LocalSpaceLinkRequest::issue(
            &other,
            &other_recipient.did(),
            "http://127.0.0.1:43211/link".parse().unwrap(),
            "abcdef0123456789abcdef0123456789".into(),
            "garden".into(),
            &service,
            at(1_000_000),
        )
        .await
        .unwrap()
        .validate(&service, at(1_000_001))
        .await
        .unwrap();
        assert!(
            approval
                .validate(&wrong_request, None, at(1_000_003))
                .await
                .unwrap_err()
                .to_string()
                .contains("recipient_mismatch")
        );
        assert!(validate_cancellation(&request, "wrong-correlation").is_err());
        validate_cancellation(&request, &request.correlation).unwrap();
    }

    #[tokio::test]
    async fn local_space_link_rejects_wrong_subject_service_expiry_and_scope() {
        let (owner, recipient, account, service, _, request) = fixture().await;
        let other_service = TrustedService::new(
            Ed25519Signer::generate().await.unwrap().did(),
            "https://other.example/ucan/".parse().unwrap(),
        )
        .unwrap();
        let fresh = LocalSpaceLinkRequest::issue(
            &owner,
            &recipient.did(),
            "http://localhost:43212/link".parse().unwrap(),
            "fedcba9876543210fedcba9876543210".into(),
            "garden".into(),
            &service,
            at(2_000_000),
        )
        .await
        .unwrap();
        assert!(
            fresh
                .validate(&other_service, at(2_000_001))
                .await
                .unwrap_err()
                .to_string()
                .contains("untrusted_service")
        );
        assert!(
            fresh
                .validate(&service, at(2_000_400))
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid_chain")
        );

        let wrong_subject = Ed25519Signer::generate().await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(wrong_subject.clone()))
            .audience(&request.recipient)
            .subject(Subject::Specific(wrong_subject.did()))
            .command(vec!["link".into()])
            .expiration(request.expires_at)
            .meta(binding_meta(&request))
            .try_build()
            .await
            .unwrap();
        let overbroad = LocalSpaceLinkApproval {
            chain: DelegationChain::new(grant),
        };
        assert!(
            overbroad
                .validate(&request, None, at(1_000_003))
                .await
                .unwrap_err()
                .to_string()
                .contains("scope_mismatch")
        );

        let approval = LocalSpaceLinkApproval::issue(&request, &account, at(1_000_002))
            .await
            .unwrap();
        let other_owner = Ed25519Signer::generate().await.unwrap();
        let other_request = LocalSpaceLinkRequest::issue(
            &other_owner,
            &recipient.did(),
            "http://127.0.0.1:43213/link".parse().unwrap(),
            "00112233445566778899aabbccddeeff".into(),
            "garden".into(),
            &service,
            at(1_000_000),
        )
        .await
        .unwrap()
        .validate(&service, at(1_000_001))
        .await
        .unwrap();
        assert!(
            approval
                .validate(&other_request, None, at(1_000_003))
                .await
                .unwrap_err()
                .to_string()
                .contains("binding_mismatch")
        );
    }
}

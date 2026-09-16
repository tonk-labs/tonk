//! Public, signed terminal requests and complete selected-space approvals.
//!
//! These messages deliver ordinary grants. They never carry private keys and do
//! not activate authority. Service trust, revocation, local cancellation and
//! create-only mailbox publication remain the caller's responsibility.

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Verifier, Signer};
use dialog_ucan_core::{
    Container, DelegationChain, InvocationBuilder, InvocationChain, delegation::chain::check_chain,
    promise::Promised, time::Timestamp,
};
use dialog_varsig::{
    AnySignature, Did, Principal, Signer as _, Verifier as _, eddsa::Ed25519Signature,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_bytes::ByteBuf;
use std::collections::BTreeMap;
use url::Url;

use crate::connection::{SpaceGrantBundle, candidate_build_scopes, grant_set_id};

mod additions;
pub use additions::{Addition, ReadAdditions};

/// Maximum approval window; unrelated to ordinary grant lifetime.
pub const REQUEST_TTL_SECONDS: u64 = 600;
/// Maximum recipient-authenticated poll lifetime.
pub const READ_TTL_SECONDS: u64 = 60;
/// Bound checked before decoding requests and poll messages.
pub const MAX_REQUEST_BYTES: usize = 4096;
/// Bound checked before decoding any complete approval.
pub const MAX_APPROVAL_BYTES: usize = 4 * 1024 * 1024;
/// Abuse bound on an explicit snapshot; the complete 4 MiB byte limit also
/// applies. Selections are never silently truncated to meet either limit.
pub const MAX_SELECTED_SPACES: usize = 1024;
const PREFIX: &str = "tonk-terminal-v1=";
const REQUEST_DOMAIN: &[u8] = b"tonk/terminal-request/v1\0";

const READ_DOMAIN: &[u8] = b"tonk/terminal-read/v1\0";

type Envelope = (ByteBuf, ByteBuf);
type RequestPayload = (
    u64,
    ByteBuf,
    String,
    String,
    u64,
    u64,
    String,
    Option<String>,
);
type BundlePayload = (String, String, String, Vec<ByteBuf>);
type ApprovalPayload = (u64, ByteBuf, String, ByteBuf, u64, bool, Vec<BundlePayload>);
type ReadPayload = (u64, String, String, ByteBuf, u64, u64);

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_ipld_dagcbor::to_vec(value).context("terminal_invalid_encoding")
}
fn decode<T: DeserializeOwned + Serialize>(bytes: &[u8], limit: usize) -> Result<T> {
    ensure!(bytes.len() <= limit, "terminal_message_too_large");
    let value: T = serde_ipld_dagcbor::from_slice(bytes).context("terminal_invalid_encoding")?;
    ensure!(encode(&value)? == bytes, "terminal_noncanonical_encoding");
    Ok(value)
}
fn message(domain: &[u8], payload: &[u8]) -> Vec<u8> {
    [domain, payload].concat()
}
fn ed25519(did: &str) -> Result<Ed25519Verifier> {
    let verifier: Ed25519Verifier = did
        .parse()
        .map_err(|_| anyhow::anyhow!("terminal_invalid_recipient"))?;
    ensure!(
        verifier.to_string() == did,
        "terminal_noncanonical_principal"
    );
    Ok(verifier)
}
async fn sign<T: Serialize>(signer: &Signer, domain: &[u8], payload: &T) -> Result<Vec<u8>> {
    ed25519(signer.did().as_str())?;
    let payload = encode(payload)?;
    let signature = signer
        .sign(&message(domain, &payload))
        .await
        .context("terminal_signing_failed")?;
    encode(&(
        ByteBuf::from(payload),
        ByteBuf::from(signature.to_bytes().to_vec()),
    ))
}
async fn verify(bytes: &[u8], limit: usize, domain: &[u8], signer: &str) -> Result<Vec<u8>> {
    let (payload, signature): Envelope = decode(bytes, limit)?;
    let signature: [u8; 64] = signature
        .as_ref()
        .try_into()
        .context("terminal_invalid_signature")?;
    ed25519(signer)?
        .verify(
            &message(domain, &payload),
            &Ed25519Signature::from_bytes(signature),
        )
        .await
        .context("terminal_invalid_signature")?;
    Ok(payload.into_vec())
}
fn timestamp(seconds: u64) -> Result<Timestamp> {
    Timestamp::try_from(seconds as i128).context("terminal_invalid_time")
}
fn identifier(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "terminal_invalid_identifier"
    );
    Ok(())
}
fn endpoint(url: &Url) -> Result<()> {
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "terminal_invalid_url"
    );
    ensure!(
        url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "terminal_invalid_url"
    );
    Ok(())
}

/// Signature-verified request whose private recipient key remains on the CLI.
#[derive(Debug, Clone)]
pub struct LinkRequest {
    bytes: Vec<u8>,
    recipient: Did,
    service: Did,
    nonce: [u8; 32],
    created_at: u64,
    deadline: u64,
    label: String,
    expected_account: Option<Did>,
}
impl LinkRequest {
    /// Sign a fresh nonce and optional explicit approving-account constraint.
    /// Never derive `expected_account` from an unrelated ambient CLI account.
    pub async fn sign(
        signer: &Signer,
        service: &Did,
        nonce: [u8; 32],
        created_at: u64,
        deadline: u64,
        label: &str,
        expected_account: Option<&Did>,
    ) -> Result<Self> {
        let bytes = sign(
            signer,
            REQUEST_DOMAIN,
            &(
                1,
                ByteBuf::from(nonce.to_vec()),
                signer.did().to_string(),
                service.to_string(),
                created_at,
                deadline,
                label.to_owned(),
                expected_account.map(ToString::to_string),
            ),
        )
        .await?;
        Self::validate(&bytes, created_at).await
    }
    /// Verify a request which is still inside its initial approval window.
    pub async fn validate(bytes: &[u8], now: u64) -> Result<Self> {
        let request = Self::inspect(bytes).await?;
        ensure!(
            request.created_at <= now.saturating_add(30) && now < request.deadline,
            "terminal_request_expired_or_future"
        );
        Ok(request)
    }
    /// Verify exact bytes and bounded time shape, allowing historical requests.
    /// Use only for existing deliveries or mailbox additions; not first writes.
    pub async fn inspect(bytes: &[u8]) -> Result<Self> {
        let (payload, _): Envelope = decode(bytes, MAX_REQUEST_BYTES)?;
        let (version, nonce, recipient, service, created_at, deadline, label, expected): RequestPayload =
            decode(&payload, MAX_REQUEST_BYTES)?;
        ensure!(version == 1, "terminal_unsupported_version");
        ensure!(
            deadline > created_at && deadline - created_at <= REQUEST_TTL_SECONDS,
            "terminal_invalid_deadline"
        );
        timestamp(deadline)?;
        ensure!(
            !label.trim().is_empty() && label.len() <= 80 && !label.chars().any(char::is_control),
            "terminal_invalid_label"
        );
        let nonce: [u8; 32] = nonce
            .as_ref()
            .try_into()
            .context("terminal_invalid_nonce")?;
        verify(bytes, MAX_REQUEST_BYTES, REQUEST_DOMAIN, &recipient).await?;
        let expected_account = expected
            .map(|did| ed25519(&did).map(|key| key.did()))
            .transpose()?;
        Ok(Self {
            bytes: bytes.to_vec(),
            recipient: ed25519(&recipient)?.did(),
            service: ed25519(&service)?.did(),
            nonce,
            created_at,
            deadline,
            label,
            expected_account,
        })
    }
    /// Put only the public signed request in a trusted approval-page fragment.
    pub fn to_url(&self, base: &str) -> Result<Url> {
        let mut url = Url::parse(base).context("terminal_invalid_url")?;
        endpoint(&url)?;
        url.set_fragment(Some(&format!(
            "{PREFIX}{}",
            bs58::encode(&self.bytes).into_string()
        )));
        Ok(url)
    }
    /// Parse and verify a public request URL without trusting its carrier origin.
    pub async fn from_url(url: &str, now: u64) -> Result<Self> {
        ensure!(
            url.len() <= MAX_REQUEST_BYTES * 2 + 2048,
            "terminal_message_too_large"
        );
        let mut url = Url::parse(url).context("terminal_invalid_url")?;
        let fragment = url
            .fragment()
            .context("terminal_missing_request")?
            .to_owned();
        url.set_fragment(None);
        endpoint(&url)?;
        let encoded = fragment
            .strip_prefix(PREFIX)
            .context("terminal_missing_request")?;
        let bytes = bs58::decode(encoded)
            .into_vec()
            .context("terminal_invalid_encoding")?;
        Self::validate(&bytes, now).await
    }
    /// Exact canonical signed bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Content-addressed mailbox identifier, including nonce and signature.
    pub fn id(&self) -> String {
        blake3::hash(&self.bytes).to_hex().to_string()
    }
    /// CLI-held public identity.
    pub fn recipient(&self) -> &Did {
        &self.recipient
    }
    /// Independently discovered service principal addressed by delivery invocations.
    pub fn service(&self) -> &Did {
        &self.service
    }
    /// Correlation nonce supplied by the CLI's secure random generator.
    pub fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }
    /// Creation time in Unix seconds.
    pub fn created_at(&self) -> u64 {
        self.created_at
    }
    /// Exclusive initial approval deadline in Unix seconds.
    pub fn deadline(&self) -> u64 {
        self.deadline
    }
    /// Bounded user-visible terminal label.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Optional explicitly requested account, never an ambient-account guess.
    pub fn expected_account(&self) -> Option<&Did> {
        self.expected_account.as_ref()
    }
}

/// Complete signature-verified initial selection addressed to one CLI key.
#[derive(Debug, Clone)]
pub struct Approval {
    bytes: Vec<u8>,
    request: LinkRequest,
    issuer: Did,
    account: Did,
    account_proof: DelegationChain,
    issued_at: u64,
    bundles: Vec<SpaceGrantBundle>,
    declined: bool,
    authorization: InvocationChain<AnySignature>,
}
impl Approval {
    /// Verify historical issuance for management and delivery pinning. This
    /// does not establish current grant or device authority; new additions
    /// require their own current signed account authorization.
    pub async fn inspect(bytes: &[u8]) -> Result<Self> {
        let (payload, _): Envelope = decode(bytes, MAX_APPROVAL_BYTES)?;
        let (_, _, _, _, issued_at, _, _): ApprovalPayload = decode(&payload, MAX_APPROVAL_BYTES)?;
        Self::validate(bytes, issued_at).await
    }
    /// Sign every selected public bundle and exact request with a browser device.
    pub async fn sign(
        signer: &Signer,
        request: &LinkRequest,
        account_proof: DelegationChain,
        bundles: Vec<SpaceGrantBundle>,
        issued_at: u64,
    ) -> Result<Self> {
        Self::sign_decision(signer, request, account_proof, bundles, issued_at, false).await
    }
    /// Sign a complete decline; no grants are included or installed.
    pub async fn sign_decline(
        signer: &Signer,
        request: &LinkRequest,
        account_proof: DelegationChain,
        issued_at: u64,
    ) -> Result<Self> {
        Self::sign_decision(signer, request, account_proof, vec![], issued_at, true).await
    }
    async fn sign_decision(
        signer: &Signer,
        request: &LinkRequest,
        account_proof: DelegationChain,
        bundles: Vec<SpaceGrantBundle>,
        issued_at: u64,
        declined: bool,
    ) -> Result<Self> {
        let mut selected = Vec::new();
        for bundle in bundles {
            let cids: Vec<String> = bundle
                .chains()
                .iter()
                .map(|chain| {
                    chain
                        .proofs()
                        .last()
                        .expect("validated nonempty chain")
                        .to_cid()
                        .to_string()
                })
                .collect();
            selected.push((
                bundle.subject().to_string(),
                bundle.remote().to_string(),
                grant_set_id(
                    bundle.subject().as_str(),
                    request.recipient().as_str(),
                    &cids,
                ),
                bundle
                    .chains()
                    .iter()
                    .map(|chain| chain.to_bytes().map(ByteBuf::from))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            ));
        }
        selected.sort_by(|left, right| left.0.cmp(&right.0));
        let payload = encode(&(
            1,
            ByteBuf::from(request.bytes().to_vec()),
            signer.did().to_string(),
            ByteBuf::from(account_proof.to_bytes()?),
            issued_at,
            declined,
            selected,
        ))?;
        let account = account_proof
            .proofs()
            .next()
            .context("terminal_invalid_account_proof")?
            .issuer();
        let invocation = InvocationBuilder::new()
            .issuer(signer.clone())
            .audience(request.service())
            .subject(account)
            .command(vec!["connection".into(), "delivery".into()])
            .arguments(BTreeMap::from([(
                "payload".into(),
                Promised::String(blake3::hash(&payload).to_hex().to_string()),
            )]))
            .proofs(account_proof.proof_cids().to_vec())
            .issued_at(timestamp(issued_at)?)
            .expiration(
                account_proof.expiration().unwrap_or(timestamp(
                    issued_at
                        .checked_add(crate::connection::DEFAULT_GRANT_TTL_SECONDS)
                        .context("terminal_invalid_time")?,
                )?),
            )
            .try_build()
            .await
            .context("terminal_signing_failed")?;
        let mut tokens = vec![encode(&invocation)?];
        tokens.extend(
            account_proof
                .export()
                .map(|(_, proof)| proof.encoded().to_vec()),
        );
        let authorization = Container::new(tokens).into_bytes()?;
        let bytes = encode(&(ByteBuf::from(payload), ByteBuf::from(authorization)))?;
        Self::validate(&bytes, issued_at).await
    }
    /// Verify the entire selection, current rooted account authority and grants.
    /// The returned signed endpoints still require independent service trust.
    /// Initial publication additionally requires `request().deadline() > now`.
    pub async fn validate(bytes: &[u8], now: u64) -> Result<Self> {
        let (payload, authorization): Envelope = decode(bytes, MAX_APPROVAL_BYTES)?;
        let (version, request, issuer, account_proof, issued_at, declined, selected): ApprovalPayload =
            decode(&payload, MAX_APPROVAL_BYTES)?;
        ensure!(version == 1, "terminal_unsupported_version");
        ensure!(
            (declined && selected.is_empty())
                || (!declined && !selected.is_empty() && selected.len() <= MAX_SELECTED_SPACES),
            "terminal_invalid_selection"
        );
        let request = LinkRequest::inspect(&request).await?;
        ensure!(
            issued_at >= request.created_at()
                && issued_at < request.deadline()
                && issued_at <= now.saturating_add(30),
            "terminal_invalid_approval_time"
        );
        let issuer = ed25519(&issuer)?.did();
        ensure!(
            &issuer != request.recipient(),
            "terminal_approver_is_recipient"
        );
        let account_proof = DelegationChain::try_from(account_proof.as_ref())
            .context("terminal_invalid_account_proof")?;
        let account = account_proof
            .proofs()
            .next()
            .context("terminal_invalid_account_proof")?
            .issuer()
            .clone();
        ensure!(
            &account != request.recipient() && account_proof.audience() == &issuer,
            "terminal_invalid_account_proof"
        );
        ensure!(
            request
                .expected_account()
                .is_none_or(|expected| expected == &account),
            "terminal_wrong_account"
        );
        check_chain(account_proof.proofs(), &account, Some(timestamp(now)?))
            .context("terminal_invalid_account_proof")?;
        for proof in account_proof.proofs() {
            ensure!(
                proof.issuer() != request.recipient() && proof.audience() != request.recipient(),
                "terminal_account_authority_addressed_to_recipient"
            );
            ensure!(
                proof.command().0.is_empty() && proof.policy().is_empty(),
                "terminal_insufficient_account_authority"
            );
            proof
                .verify_signature(&DidKeyResolver)
                .await
                .context("terminal_invalid_account_proof")?;
        }
        let authorization = InvocationChain::<AnySignature>::try_from(authorization.as_ref())
            .context("terminal_invalid_authorization")?;
        ensure!(
            authorization.issuer() == &issuer
                && authorization.subject() == &account
                && authorization.invocation.audience() == request.service()
                && authorization.command().0 == ["connection", "delivery"]
                && authorization.arguments()
                    == &BTreeMap::from([(
                        "payload".into(),
                        Promised::String(blake3::hash(&payload).to_hex().to_string())
                    )])
                && authorization.proofs() == account_proof.proof_cids(),
            "terminal_authorization_mismatch"
        );
        ensure!(
            authorization
                .invocation
                .expiration()
                .is_some_and(|deadline| deadline > timestamp(now).expect("validated timestamp")),
            "terminal_authorization_expired"
        );
        authorization
            .invocation
            .verify_signature(&DidKeyResolver)
            .await
            .context("terminal_invalid_authorization")?;
        let mut bundles = Vec::new();
        let mut previous = None;
        for (subject, remote, group_id, chains) in selected {
            ensure!(
                previous
                    .as_ref()
                    .is_none_or(|prior: &String| prior < &subject),
                "terminal_duplicate_or_unsorted_selection"
            );
            previous = Some(subject.clone());
            let subject: Did = subject.parse().context("terminal_invalid_subject")?;
            ensure!(subject != account, "terminal_account_catalogue_forbidden");
            let remote = Url::parse(&remote).context("terminal_invalid_route")?;
            let chains = chains
                .into_iter()
                .map(|bytes| {
                    DelegationChain::try_from(bytes.as_ref()).context("terminal_invalid_grants")
                })
                .collect::<Result<Vec<_>>>()?;
            let bundle = SpaceGrantBundle::validate(
                chains,
                request.recipient(),
                &candidate_build_scopes(&subject),
                &remote,
                timestamp(now)?,
            )
            .await?;
            let cids: Vec<String> = bundle
                .chains()
                .iter()
                .map(|chain| {
                    chain
                        .proofs()
                        .last()
                        .expect("validated chain")
                        .to_cid()
                        .to_string()
                })
                .collect();
            ensure!(
                group_id == grant_set_id(subject.as_str(), request.recipient().as_str(), &cids),
                "terminal_group_mismatch"
            );
            bundles.push(bundle);
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            request,
            issuer,
            account,
            account_proof,
            issued_at,
            bundles,
            declined,
            authorization,
        })
    }
    /// Standard signed invocation for service-side revocation enforcement.
    pub fn authorization(&self) -> &InvocationChain<AnySignature> {
        &self.authorization
    }
    /// Whether the authenticated browser explicitly declined the request.
    pub fn is_declined(&self) -> bool {
        self.declined
    }
    /// Exact complete signed approval bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Exact signed request approved by the browser.
    pub fn request(&self) -> &LinkRequest {
        &self.request
    }
    /// Account derived from verified rooted authority, not body metadata.
    pub fn account(&self) -> &Did {
        &self.account
    }
    /// Browser device which signed the complete selection.
    pub fn issuer(&self) -> &Did {
        &self.issuer
    }
    /// Rooted device proof; services must also check current revocations.
    pub fn account_proof(&self) -> &DelegationChain {
        &self.account_proof
    }
    /// Signed issuance time, inside the initial approval window.
    pub fn issued_at(&self) -> u64 {
        self.issued_at
    }
    /// Complete sorted selection, never a silently shortened subset.
    pub fn bundles(&self) -> &[SpaceGrantBundle] {
        &self.bundles
    }
}

/// Short-lived proof that a poller holds the exact addressed CLI key.
#[derive(Debug, Clone)]
pub struct ReadRequest {
    bytes: Vec<u8>,
    request_id: String,
    recipient: Did,
    nonce: [u8; 32],
    expires_at: u64,
}
impl ReadRequest {
    /// Sign a fresh nonce without creating server-side pending state.
    pub async fn sign(
        signer: &Signer,
        request_id: &str,
        nonce: [u8; 32],
        now: u64,
    ) -> Result<Self> {
        let bytes = sign(
            signer,
            READ_DOMAIN,
            &(
                1,
                request_id,
                signer.did().to_string(),
                ByteBuf::from(nonce.to_vec()),
                now,
                now.checked_add(READ_TTL_SECONDS)
                    .context("terminal_invalid_time")?,
            ),
        )
        .await?;
        Self::validate(&bytes, now).await
    }
    /// Verify possession, request binding, canonical bytes and current deadline.
    pub async fn validate(bytes: &[u8], now: u64) -> Result<Self> {
        let (payload, _): Envelope = decode(bytes, MAX_REQUEST_BYTES)?;
        let (version, request_id, recipient, nonce, created_at, expires_at): ReadPayload =
            decode(&payload, MAX_REQUEST_BYTES)?;
        ensure!(version == 1, "terminal_unsupported_version");
        identifier(&request_id)?;
        ensure!(
            expires_at > created_at
                && expires_at - created_at <= READ_TTL_SECONDS
                && created_at <= now.saturating_add(30)
                && now < expires_at,
            "terminal_read_expired_or_future"
        );
        verify(bytes, MAX_REQUEST_BYTES, READ_DOMAIN, &recipient).await?;
        Ok(Self {
            bytes: bytes.to_vec(),
            request_id,
            recipient: ed25519(&recipient)?.did(),
            nonce: nonce
                .as_ref()
                .try_into()
                .context("terminal_invalid_nonce")?,
            expires_at,
        })
    }
    /// Exact signed poll bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Mailbox being read; no account catalogue is exposed.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    /// Signature-authenticated reader DID, to match against mailbox recipient.
    pub fn recipient(&self) -> &Did {
        &self.recipient
    }
    /// Signed fresh correlation nonce.
    pub fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }
    /// Exclusive Unix deadline.
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::{DelegationBuilder, subject::Subject};

    async fn key(seed: u8) -> Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap().into()
    }
    async fn request(recipient: &Signer, account: Option<&Did>) -> LinkRequest {
        LinkRequest::sign(
            recipient,
            &key(9).await.did(),
            [7; 32],
            2_000_000_000,
            2_000_000_600,
            "Work terminal",
            account,
        )
        .await
        .unwrap()
    }
    async fn proof(account: &Signer, browser: &Signer) -> DelegationChain {
        DelegationChain::new(
            DelegationBuilder::new()
                .issuer(account.clone())
                .audience(&browser.did())
                .subject(Subject::Any)
                .command(vec![])
                .expiration(timestamp(2_010_000_000).unwrap())
                .try_build()
                .await
                .unwrap(),
        )
    }
    async fn bundle(recipient: &Signer, browser: &Signer, subject: &Signer) -> SpaceGrantBundle {
        let parent = DelegationBuilder::new()
            .issuer(subject.clone())
            .audience(&browser.did())
            .subject(Subject::Specific(subject.did()))
            .command(vec!["use".into()])
            .expiration(timestamp(2_010_000_000).unwrap())
            .try_build()
            .await
            .unwrap();
        let remote: Url = "https://access.example/ucan/".parse().unwrap();
        let scopes = candidate_build_scopes(&subject.did());
        let mut chains = vec![];
        for scope in &scopes {
            let leaf = DelegationBuilder::new()
                .issuer(browser.clone())
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .meta(crate::home_address_meta(&remote))
                .expiration(timestamp(2_010_000_000).unwrap())
                .try_build()
                .await
                .unwrap();
            chains.push(DelegationChain::new(parent.clone()).push(leaf).unwrap());
        }
        SpaceGrantBundle::validate(
            chains,
            &recipient.did(),
            &scopes,
            &remote,
            timestamp(2_000_000_000).unwrap(),
        )
        .await
        .unwrap()
    }

    #[dialog_common::test]
    async fn signed_request_binds_key_nonce_service_and_deadline() {
        let recipient = key(1).await;
        let request = request(&recipient, None).await;
        let url = request
            .to_url("https://tonk.network/settings/link")
            .unwrap();
        assert!(url.query().is_none());
        assert_eq!(
            LinkRequest::from_url(url.as_str(), 2_000_000_001)
                .await
                .unwrap()
                .id(),
            request.id()
        );
        assert!(
            LinkRequest::validate(request.bytes(), request.deadline())
                .await
                .is_err()
        );
        assert!(LinkRequest::inspect(request.bytes()).await.is_ok());
        let (payload, signature): Envelope = decode(request.bytes(), MAX_REQUEST_BYTES).unwrap();
        let mut fields: RequestPayload = decode(&payload, MAX_REQUEST_BYTES).unwrap();
        fields.2 = key(2).await.did().to_string();
        let altered = encode(&(ByteBuf::from(encode(&fields).unwrap()), signature)).unwrap();
        assert!(LinkRequest::inspect(&altered).await.is_err());
        let other = LinkRequest::sign(
            &recipient,
            request.service(),
            [8; 32],
            request.created_at(),
            request.deadline(),
            request.label(),
            None,
        )
        .await
        .unwrap();
        assert_ne!(other.id(), request.id());
        assert!(request.to_url("javascript:alert(1)").is_err());
    }

    #[dialog_common::test]
    async fn complete_approval_rejects_substitution_partial_selection_and_wrong_account() {
        let recipient = key(1).await;
        let account = key(2).await;
        let browser = key(3).await;
        let request = request(&recipient, Some(&account.did())).await;
        let first = bundle(&recipient, &browser, &key(4).await).await;
        let second = bundle(&recipient, &browser, &key(5).await).await;
        let approval = Approval::sign(
            &browser,
            &request,
            proof(&account, &browser).await,
            vec![first.clone(), second],
            2_000_000_001,
        )
        .await
        .unwrap();
        assert_eq!(approval.bundles().len(), 2);
        assert_eq!(approval.account(), &account.did());
        assert_eq!(
            approval.authorization().invocation.audience(),
            request.service()
        );
        assert!(
            Approval::validate(approval.bytes(), request.deadline() + 10)
                .await
                .is_ok()
        );
        let (payload, auth): Envelope = decode(approval.bytes(), MAX_APPROVAL_BYTES).unwrap();
        let mut fields: ApprovalPayload = decode(&payload, MAX_APPROVAL_BYTES).unwrap();
        fields.6.pop();
        let partial = encode(&(ByteBuf::from(encode(&fields).unwrap()), auth)).unwrap();
        assert!(Approval::validate(&partial, 2_000_000_002).await.is_err());
        let other_account = key(6).await;
        assert!(
            Approval::sign(
                &browser,
                &request,
                proof(&other_account, &browser).await,
                vec![first.clone()],
                2_000_000_001
            )
            .await
            .is_err()
        );
        let wrong_recipient = bundle(&key(7).await, &browser, &key(4).await).await;
        assert!(
            Approval::sign(
                &browser,
                &request,
                proof(&account, &browser).await,
                vec![wrong_recipient],
                2_000_000_001
            )
            .await
            .is_err()
        );
        assert!(
            Approval::sign(
                &browser,
                &request,
                proof(&account, &browser).await,
                vec![first.clone(), first],
                2_000_000_001
            )
            .await
            .is_err()
        );
    }

    #[dialog_common::test]
    async fn decline_is_signed_and_reads_cannot_substitute_the_recipient() {
        let recipient = key(1).await;
        let account = key(2).await;
        let browser = key(3).await;
        let request = request(&recipient, None).await;
        let decline = Approval::sign_decline(
            &browser,
            &request,
            proof(&account, &browser).await,
            2_000_000_001,
        )
        .await
        .unwrap();
        assert!(decline.is_declined());
        assert!(decline.bundles().is_empty());
        assert!(
            Approval::sign(
                &browser,
                &request,
                proof(&account, &browser).await,
                vec![],
                2_000_000_001
            )
            .await
            .is_err()
        );
        let read = ReadRequest::sign(&recipient, &request.id(), [8; 32], 2_000_000_002)
            .await
            .unwrap();
        assert_eq!(read.recipient(), request.recipient());
        assert!(
            ReadRequest::validate(read.bytes(), read.expires_at())
                .await
                .is_err()
        );
        let (payload, signature): Envelope = decode(read.bytes(), MAX_REQUEST_BYTES).unwrap();
        let mut fields: ReadPayload = decode(&payload, MAX_REQUEST_BYTES).unwrap();
        fields.2 = key(8).await.did().to_string();
        let forged = encode(&(ByteBuf::from(encode(&fields).unwrap()), signature)).unwrap();
        assert!(ReadRequest::validate(&forged, 2_000_000_002).await.is_err());
    }
}

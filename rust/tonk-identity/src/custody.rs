//! Custody-space invocation containers.
//!
//! The custody key is its space's root, so every invocation here is
//! self-issued — issuer, audience, and subject are all the custody DID,
//! with no proofs: a chain rooting in the subject needs nothing else.
//! The wrapped account secret lives at one well-known cell
//! (`custody/secret`); publishing and resolving it go through the
//! access service's `/ucan/` endpoint, which answers with a presigned
//! permit the caller executes directly against storage.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use dialog_credentials::Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{
    Delegation, DelegationBuilder, DelegationChain, Invocation, InvocationBuilder, InvocationChain,
};
use dialog_varsig::{AnySignature, Did, Principal};
use sha2_0_10::{Digest, Sha256};

use crate::envelope::{CUSTODY_SECRET_CELL, CUSTODY_SPACE};

/// The multihash form the memory protocol names content by:
/// varint code `0x12` (sha2-256), varint length `0x20`, digest.
fn sha256_multihash(content: &[u8]) -> Vec<u8> {
    let digest = Sha256::digest(content);
    let mut bytes = Vec::with_capacity(2 + digest.len());
    bytes.push(0x12);
    bytes.push(0x20);
    bytes.extend_from_slice(&digest);
    bytes
}

/// Sign a self-issued custody invocation.
///
/// The value itself, not a container: a caller carrying it as a block
/// in someone else's container has no use for framing, and one posting
/// it to the service wraps it with [`into_container`].
async fn sign_self_invocation(
    custody: Signer,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
    expiration: Timestamp,
) -> Result<Invocation<AnySignature>> {
    let did = custody.did();
    InvocationBuilder::new()
        .issuer(custody)
        .audience(&did)
        .subject(&did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![])
        .expiration(expiration)
        .try_build()
        .await
        .context("failed to sign the custody invocation")
}

/// Frame a proofless invocation as the container the `/ucan/` endpoint
/// reads.
pub fn into_container(invocation: Invocation<AnySignature>) -> Result<Vec<u8>> {
    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .context("failed to serialize the custody invocation")
}

async fn build_self_invocation(
    custody: Signer,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
    expiration: Timestamp,
) -> Result<Vec<u8>> {
    into_container(sign_self_invocation(custody, command, arguments, expiration).await?)
}

/// How long a deferred publish invocation stays redeemable: the
/// ceremony pre-signs it, and it waits for an email activation that can
/// take days. Bounded so a leaked queue entry is not authority forever.
pub const DEFERRED_PUBLISH_TTL_SECONDS: u64 = 60 * 60 * 24 * 30;

fn cell_arguments() -> BTreeMap<String, Promised> {
    BTreeMap::from([
        (
            "space".to_string(),
            Promised::String(CUSTODY_SPACE.to_string()),
        ),
        (
            "cell".to_string(),
            Promised::String(CUSTODY_SECRET_CELL.to_string()),
        ),
    ])
}

/// Build a `/use/put/memory/cell` container for the wrapped-secret cell.
///
/// The invocation names the content by checksum; the bytes themselves
/// travel in the presigned PUT the permit authorizes. `when` carries
/// the cell's current version for an overwrite (re-wrap, rotation) and
/// stays `None` for the first write, which the protocol makes
/// first-write-only.
pub async fn build_publish_invocation(
    custody: Signer,
    content: &[u8],
    when: Option<&[u8]>,
    expiration: Timestamp,
) -> Result<Vec<u8>> {
    into_container(sign_publish_invocation(custody, content, when, expiration).await?)
}

/// [`build_publish_invocation`] as a value, for a caller carrying it as
/// a block rather than posting it.
pub async fn sign_publish_invocation(
    custody: Signer,
    content: &[u8],
    when: Option<&[u8]>,
    expiration: Timestamp,
) -> Result<Invocation<AnySignature>> {
    let mut arguments = cell_arguments();
    arguments.insert(
        "checksum".to_string(),
        Promised::Bytes(sha256_multihash(content)),
    );
    if let Some(version) = when {
        arguments.insert("when".to_string(), Promised::Bytes(version.to_vec()));
    }
    sign_self_invocation(
        custody,
        vec![
            "use".to_string(),
            "put".to_string(),
            "memory".to_string(),
            "cell".to_string(),
        ],
        arguments,
        expiration,
    )
    .await
}

/// Pre-sign the first-write publish for `content`, redeemable for
/// [`DEFERRED_PUBLISH_TTL_SECONDS`]: what a creation ceremony queues so
/// the worker can publish the custody cell once activation lands,
/// with no further passkey assertion. Least authority: the signature
/// covers exactly this cell and this content's checksum.
pub async fn build_deferred_publish_invocation(custody: Signer, content: &[u8]) -> Result<Vec<u8>> {
    into_container(sign_deferred_publish_invocation(custody, content).await?)
}

/// [`build_deferred_publish_invocation`] as a value, for a caller
/// carrying it as a block rather than posting it.
pub async fn sign_deferred_publish_invocation(
    custody: Signer,
    content: &[u8],
) -> Result<Invocation<AnySignature>> {
    use dialog_ucan_core::time::timestamp::{Duration, SystemTime};
    let expiration =
        Timestamp::new(SystemTime::now() + Duration::from_secs(DEFERRED_PUBLISH_TTL_SECONDS))
            .context("the deferred publish expiration is out of range")?;
    sign_publish_invocation(custody, content, None, expiration).await
}

/// Build a `/use/get/memory/cell` container for the wrapped-secret cell.
pub async fn build_resolve_invocation(custody: Signer) -> Result<Vec<u8>> {
    build_self_invocation(
        custody,
        vec![
            "use".to_string(),
            "get".to_string(),
            "memory".to_string(),
            "cell".to_string(),
        ],
        cell_arguments(),
        Timestamp::five_minutes_from_now(),
    )
    .await
}

/// Mint the custody key's consent to being provided by the account —
/// the consumer powerline `/provider/add` deposits: issuer and subject
/// are the custody DID, audience is the account, command unrestricted.
pub async fn mint_custody_consent(custody: Signer, account: &Did) -> Result<DelegationChain> {
    Ok(DelegationChain::new(
        sign_custody_consent(custody, account).await?,
    ))
}

/// [`mint_custody_consent`] as a value, for a caller carrying it as a
/// block rather than as a chain of its own.
pub async fn sign_custody_consent(
    custody: Signer,
    account: &Did,
) -> Result<Delegation<AnySignature>> {
    let subject = custody.did();
    DelegationBuilder::new()
        .issuer(custody)
        .audience(account)
        .subject(UcanSubject::Specific(subject))
        .command(vec![])
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the custody consent: {e}"))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web {
    use anyhow::{Context, Result, anyhow};
    use dialog_credentials::{Ed25519Signer, Signer};
    use js_sys::Uint8Array;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    /// The presigned request the access service answers a `/ucan/`
    /// invocation with. Mirrors `dialog_remote_s3::Permit` on the wire.
    #[derive(serde::Deserialize)]
    struct Permit {
        url: String,
        method: String,
        headers: Vec<(String, String)>,
    }

    fn web_error(context: &str, value: wasm_bindgen::JsValue) -> anyhow::Error {
        anyhow!("{context}: {value:?}")
    }

    async fn fetch_bytes(request: Request) -> Result<(u16, Vec<u8>)> {
        // `fetch` off the global, so the same exchange runs from a page
        // (ceremonies) and from the service worker (the queued-publish
        // drain after activation).
        let fetch = js_sys::Reflect::get(&js_sys::global(), &"fetch".into())
            .map_err(|e| web_error("no fetch in this scope", e))?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| anyhow!("fetch is not callable in this scope"))?;
        let promise: js_sys::Promise = fetch
            .call1(&js_sys::global(), &request)
            .map_err(|e| web_error("the custody request failed to start", e))?
            .dyn_into()
            .map_err(|_| anyhow!("fetch did not answer a promise"))?;
        let response: Response = JsFuture::from(promise)
            .await
            .map_err(|e| web_error("the custody request failed", e))?
            .dyn_into()
            .map_err(|_| anyhow!("fetch answered a non-response"))?;
        let status = response.status();
        let buffer = JsFuture::from(
            response
                .array_buffer()
                .map_err(|e| web_error("no response body", e))?,
        )
        .await
        .map_err(|e| web_error("failed to read the response body", e))?;
        Ok((status, Uint8Array::new(&buffer).to_vec()))
    }

    async fn redeem(endpoint: &str, container: Vec<u8>) -> Result<Permit> {
        let init = RequestInit::new();
        init.set_method("POST");
        init.set_body(&Uint8Array::from(container.as_slice()).into());
        let request = Request::new_with_str_and_init(endpoint, &init)
            .map_err(|e| web_error("failed to build the permit request", e))?;
        request
            .headers()
            .set("Content-Type", "application/cbor")
            .map_err(|e| web_error("failed to set the container type", e))?;
        let (status, body) = fetch_bytes(request).await?;
        if status != 200 {
            let detail = String::from_utf8_lossy(&body);
            // The gate already knows the whole situation, so the client
            // does not re-derive it from a second request: the refusal is
            // read into its reason here and travels as that, which is what
            // lets a caller say "open the link in your email" instead of
            // "we couldn't log you in".
            return Err(anyhow::Error::new(super::CustodyDenial::parse(&detail)))
                .with_context(|| format!("the custody request was refused ({status})"));
        }
        serde_ipld_dagcbor::from_slice(&body).context("the permit did not decode")
    }

    async fn execute(permit: Permit, body: Option<&[u8]>) -> Result<(u16, Vec<u8>)> {
        let init = RequestInit::new();
        init.set_method(&permit.method);
        if let Some(content) = body {
            init.set_body(&Uint8Array::from(content).into());
        }
        let request = Request::new_with_str_and_init(&permit.url, &init)
            .map_err(|e| web_error("failed to build the storage request", e))?;
        for (name, value) in &permit.headers {
            request
                .headers()
                .set(name, value)
                .map_err(|e| web_error("failed to set a permit header", e))?;
        }
        fetch_bytes(request).await
    }

    /// Publish the sealed envelope into the custody space's cell:
    /// redeem a permit at the service's `/ucan/`, then execute the
    /// presigned PUT it names.
    pub async fn publish_secret(
        custody: Ed25519Signer,
        sealed: &[u8],
        endpoint: &str,
        when: Option<&[u8]>,
    ) -> Result<()> {
        let container = super::build_publish_invocation(
            Signer::from(custody),
            sealed,
            when,
            dialog_ucan_core::time::timestamp::Timestamp::five_minutes_from_now(),
        )
        .await?;
        submit_publish(&container, sealed, endpoint).await
    }

    /// Publish `sealed` with an already-signed invocation: redeem the
    /// permit and execute the presigned PUT. No signer involved — this
    /// is what drains a ceremony's pre-signed publish after activation,
    /// from the worker, with no page and no assertion.
    pub async fn submit_publish(invocation: &[u8], sealed: &[u8], endpoint: &str) -> Result<()> {
        let permit = redeem(endpoint, invocation.to_vec()).await?;
        let (status, body) = execute(permit, Some(sealed)).await?;
        if !(200..300).contains(&status) {
            let detail = String::from_utf8_lossy(&body);
            anyhow::bail!("storage refused the custody cell ({status}): {detail}");
        }
        Ok(())
    }

    /// Resolve the custody space's cell: `Ok(None)` when no wrapping
    /// was ever published there.
    pub async fn resolve_secret(custody: Signer, endpoint: &str) -> Result<Option<Vec<u8>>> {
        let container = super::build_resolve_invocation(custody).await?;
        let permit = redeem(endpoint, container).await?;
        let (status, body) = execute(permit, None).await?;
        match status {
            200..=299 => Ok(Some(body)),
            404 => Ok(None),
            status => {
                let detail = String::from_utf8_lossy(&body);
                anyhow::bail!("storage refused the custody read ({status}): {detail}");
            }
        }
    }
}

/// Why the access service refused a custody request.
///
/// The gate's answer is structured -- `{"kind":"Declined","recourse":…,
/// "reason":…}` -- and this is that answer as a type, so every caller
/// above decides on a variant instead of matching the service's prose.
/// Matching sentences made the wording load-bearing: a reworded refusal
/// silently downgraded "open the link in your email" to "check your
/// connection", with nothing failing to say so.
///
/// Travels as an `anyhow` payload, so a caller that cares downcasts and
/// the ones that do not keep printing the message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CustodyDenial {
    /// The account registered but nobody has opened the emailed link.
    /// Waiting clears it: the same request succeeds once it is opened,
    /// on this device or any other.
    #[error("the account is awaiting email confirmation")]
    AwaitingActivation,
    /// The account is suspended. Waiting does not clear it.
    #[error("the account is suspended: {0}")]
    Suspended(String),
    /// Nobody pays for this subject, so nothing serves it.
    #[error("the custody space is not provisioned: {0}")]
    NotProvisioned(String),
    /// A refusal none of the above describe, kept whole.
    #[error("the access service refused the custody invocation: {0}")]
    Other(String),
}

impl CustodyDenial {
    /// Read a refusal body into the reason it carries.
    ///
    /// Parsed, not matched: `recourse` says whether waiting helps and is
    /// the only field a client may depend on, while `reason` is prose
    /// this classifies once, here, rather than in every caller. A body
    /// that does not parse is [`CustodyDenial::Other`] holding it whole,
    /// so nothing is lost when the shape changes.
    pub fn parse(body: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            return Self::Other(body.to_owned());
        };
        let reason = value
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(body);
        let retryable = value
            .get("recourse")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|recourse| recourse == "Retry");
        // Both halves matter. `Retry` alone also covers a suspension that
        // lifts at a deadline, which no email confirms; the reason alone
        // would match a permanent refusal that merely mentions activation.
        if retryable && reason.contains("awaits email activation") {
            return Self::AwaitingActivation;
        }
        if reason.contains("is suspended") {
            return Self::Suspended(reason.to_owned());
        }
        if reason.contains("is not provisioned") {
            return Self::NotProvisioned(reason.to_owned());
        }
        Self::Other(reason.to_owned())
    }

    /// The stable tag this refusal crosses a JS boundary as.
    ///
    /// The worker answers the page through `postMessage`, which carries
    /// no Rust types, so the variant travels as this string and is read
    /// back by [`CustodyDenial::from_code`]. Values are API: the page
    /// branches on them.
    pub fn code(&self) -> &'static str {
        match self {
            Self::AwaitingActivation => "awaiting-activation",
            Self::Suspended(_) => "suspended",
            Self::NotProvisioned(_) => "not-provisioned",
            Self::Other(_) => "denied",
        }
    }

    /// The variant a `code` names, for the far side of that boundary.
    ///
    /// `None` for anything unrecognised, which a caller reports as the
    /// ordinary failure it cannot say more about.
    pub fn from_code(code: &str, reason: &str) -> Option<Self> {
        match code {
            "awaiting-activation" => Some(Self::AwaitingActivation),
            "suspended" => Some(Self::Suspended(reason.to_owned())),
            "not-provisioned" => Some(Self::NotProvisioned(reason.to_owned())),
            "denied" => Some(Self::Other(reason.to_owned())),
            _ => None,
        }
    }
}

/// The denial inside an error, whatever it was wrapped in on the way up.
///
/// `anyhow` keeps the whole chain, so a `bail!` several frames below is
/// still reachable after the layers above have added their own context.
pub fn denial_of(error: &anyhow::Error) -> Option<&CustodyDenial> {
    error.chain().find_map(|cause| cause.downcast_ref())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::{publish_secret, resolve_secret, submit_publish};

#[cfg(test)]
mod tests {
    use super::*;

    /// The refusal a second device meets while the email is unconfirmed
    /// reads as the step that clears it.
    #[dialog_common::test]
    fn it_reads_a_refusal_that_confirming_the_email_would_clear() {
        assert_eq!(
            CustodyDenial::parse(
                r#"{"kind":"Declined","recourse":"Retry","reason":"the provider of did:key:zCustody awaits email activation"}"#
            ),
            CustodyDenial::AwaitingActivation
        );
    }

    /// Refusals that waiting does NOT clear read as themselves. Reporting
    /// one as "check your email" would send someone to a link that fixes
    /// nothing.
    #[dialog_common::test]
    fn it_tells_the_other_refusals_apart() {
        assert_eq!(
            CustodyDenial::parse(
                r#"{"kind":"Declined","recourse":"None","reason":"did:key:zCustody is not provisioned"}"#
            ),
            CustodyDenial::NotProvisioned("did:key:zCustody is not provisioned".to_owned())
        );
        assert_eq!(
            CustodyDenial::parse(
                r#"{"kind":"Declined","recourse":"Retry","reason":"the subscription for did:key:zCustody is suspended: unpaid"}"#
            ),
            CustodyDenial::Suspended(
                "the subscription for did:key:zCustody is suspended: unpaid".to_owned()
            )
        );
    }

    /// A body that is not the gate's answer at all is kept whole rather
    /// than guessed at: an upstream failure is not an unconfirmed email.
    #[dialog_common::test]
    fn it_keeps_an_unparseable_refusal_whole() {
        assert_eq!(
            CustodyDenial::parse("upstream is down"),
            CustodyDenial::Other("upstream is down".to_owned())
        );
    }

    /// A suspension is retryable when it lifts at a deadline, so
    /// `recourse` alone cannot mean "confirm your email". Both halves are
    /// required, which is what keeps a timed suspension from telling
    /// someone to open a link that will not help.
    #[dialog_common::test]
    fn it_does_not_read_every_retryable_refusal_as_an_unconfirmed_email() {
        assert_ne!(
            CustodyDenial::parse(
                r#"{"kind":"Declined","recourse":"Retry","reason":"the subscription for did:key:zCustody is suspended: unpaid"}"#
            ),
            CustodyDenial::AwaitingActivation
        );
    }

    /// The variant survives the JS boundary, which carries no Rust types.
    #[dialog_common::test]
    fn it_carries_the_reason_across_a_string_boundary() {
        for denial in [
            CustodyDenial::AwaitingActivation,
            CustodyDenial::Suspended("unpaid".to_owned()),
            CustodyDenial::NotProvisioned("nobody pays".to_owned()),
            CustodyDenial::Other("something else".to_owned()),
        ] {
            let recovered = CustodyDenial::from_code(denial.code(), &denial.to_string());
            assert_eq!(
                recovered.map(|value| value.code()),
                Some(denial.code()),
                "{denial:?} crosses as its own code"
            );
        }
        assert!(
            CustodyDenial::from_code("something-new", "").is_none(),
            "an unrecognised code is not guessed at"
        );
    }

    /// The whole prefix the worker sends over the bridge, round-tripped.
    ///
    /// The link this pins is the one nothing else covers: a refusal
    /// raised deep in `resolve_secret` has to survive `?` through
    /// `LoadAccount`, the wrapping the worker adds, the split back into
    /// `(code, message)`, and the rebuild on the page. Every hop is
    /// simple; the path through all of them is what was broken.
    #[dialog_common::test]
    fn it_survives_the_whole_trip_to_the_page() {
        // As raised, several frames down.
        let raised = anyhow::Error::new(CustodyDenial::parse(
            r#"{"kind":"Declined","recourse":"Retry","reason":"the provider of did:key:zC awaits email activation"}"#,
        ))
        .context("the custody request was refused (403)")
        .context("the custody cell did not open");

        // As the worker reads it back out and puts it on the wire.
        let found = denial_of(&raised).expect("the denial survives the wrapping");
        let (code, message) = (found.code(), found.to_string());

        // As the page rebuilds it.
        assert_eq!(
            CustodyDenial::from_code(code, &message),
            Some(CustodyDenial::AwaitingActivation),
            "the page reads back the reason the service gave"
        );
    }

    /// The denial is reachable after the layers above have wrapped it.
    #[dialog_common::test]
    fn it_finds_the_denial_under_the_context_added_above_it() {
        let error = anyhow::Error::new(CustodyDenial::AwaitingActivation)
            .context("the custody request was refused (403)")
            .context("the custody cell did not open");
        assert_eq!(denial_of(&error), Some(&CustodyDenial::AwaitingActivation));
        assert_eq!(denial_of(&anyhow::anyhow!("no denial here")), None);
    }
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::promise::Promised;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    /// The activation link carries the recovery invocation, the consent
    /// and the sealed envelope, base64'd into a URL query parameter. A
    /// URL that outgrows what mail clients and browsers carry reliably
    /// (~2000 characters is the conservative floor) forces the material
    /// out of the link, so measure it rather than assume.
    #[dialog_common::test]
    async fn it_keeps_the_activation_link_inside_a_url_budget() {
        use base64::Engine as _;

        let custody = custody().await;
        let account = Ed25519Signer::generate().await.unwrap();
        let sealed = [7u8; 64];

        let recovery = build_deferred_publish_invocation(custody.clone(), &sealed)
            .await
            .unwrap();
        let consent = mint_custody_consent(custody, &account.did())
            .await
            .unwrap()
            .to_bytes()
            .unwrap();

        let payload = recovery.len() + consent.len() + sealed.len();
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(vec![0u8; payload])
            .len();

        println!(
            "recovery={} consent={} sealed={} raw={payload} base64={encoded}",
            recovery.len(),
            consent.len(),
            sealed.len(),
        );
        assert!(
            encoded < 2000,
            "the activation link's material is {encoded} base64 characters, past a safe URL budget"
        );
    }

    async fn custody() -> Signer {
        Signer::from(
            Ed25519Signer::import(&*crate::envelope::custody_seed(&[5u8; 32]))
                .await
                .unwrap(),
        )
    }

    #[dialog_common::test]
    async fn it_builds_a_self_issued_publish_the_service_verifies() {
        let custody = custody().await;
        let did = custody.did();
        let bytes =
            build_publish_invocation(custody, b"sealed", None, Timestamp::five_minutes_from_now())
                .await
                .unwrap();

        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_ucan_core::verification::VerificationContext::new(
                &dialog_ucan_core::verification::Environment::new(
                    chain.proof_store(),
                    dialog_credentials::DidKeyResolver,
                    dialog_ucan_core::revocation::UnverifiedRevocations,
                ),
            ))
            .await
            .unwrap();
        assert_eq!(chain.issuer(), &did);
        assert_eq!(chain.subject(), &did);
        assert_eq!(
            chain.command().0,
            vec![
                "use".to_string(),
                "put".to_string(),
                "memory".to_string(),
                "cell".to_string()
            ],
        );
        let arguments = chain.invocation.arguments();
        assert_eq!(
            arguments.get("space"),
            Some(&Promised::String(CUSTODY_SPACE.to_string())),
        );
        assert_eq!(
            arguments.get("cell"),
            Some(&Promised::String(CUSTODY_SECRET_CELL.to_string())),
        );
        assert_eq!(
            arguments.get("checksum"),
            Some(&Promised::Bytes(sha256_multihash(b"sealed"))),
        );
        assert_eq!(arguments.get("when"), None, "first write carries no when");
    }

    #[dialog_common::test]
    async fn it_builds_a_versioned_overwrite() {
        let bytes = build_publish_invocation(
            custody().await,
            b"resealed",
            Some(b"etag-1"),
            Timestamp::five_minutes_from_now(),
        )
        .await
        .unwrap();
        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        assert_eq!(
            chain.invocation.arguments().get("when"),
            Some(&Promised::Bytes(b"etag-1".to_vec())),
        );
    }

    #[dialog_common::test]
    async fn it_builds_a_self_issued_resolve() {
        let custody = custody().await;
        let did = custody.did();
        let bytes = build_resolve_invocation(custody).await.unwrap();
        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_ucan_core::verification::VerificationContext::new(
                &dialog_ucan_core::verification::Environment::new(
                    chain.proof_store(),
                    dialog_credentials::DidKeyResolver,
                    dialog_ucan_core::revocation::UnverifiedRevocations,
                ),
            ))
            .await
            .unwrap();
        assert_eq!(chain.issuer(), &did);
        assert_eq!(
            chain.command().0,
            vec![
                "use".to_string(),
                "get".to_string(),
                "memory".to_string(),
                "cell".to_string()
            ],
        );
    }

    /// The consent must satisfy the service's `verify_consent`: issuer
    /// and subject are the consumer, audience the provider, command a
    /// prefix of `["consumer", "provision"]` — the unrestricted
    /// powerline qualifies as-is.
    #[dialog_common::test]
    async fn it_mints_a_consent_in_the_provisioning_shape() {
        let custody = custody().await;
        let custody_did = custody.did();
        let account = Ed25519Signer::import(&[6u8; 32]).await.unwrap();
        let chain = mint_custody_consent(custody, &account.did()).await.unwrap();

        let delegation = chain.proofs().next().unwrap();
        assert_eq!(delegation.issuer(), &custody_did);
        assert_eq!(delegation.audience(), &account.did());
        assert_eq!(
            delegation.subject(),
            &UcanSubject::Specific(custody_did.clone()),
        );
        assert!(delegation.command().0.is_empty());
    }
}

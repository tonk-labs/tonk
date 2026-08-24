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
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};
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

async fn build_self_invocation(
    custody: Ed25519Signer,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
) -> Result<Vec<u8>> {
    let did = custody.did();
    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(custody))
        .audience(&did)
        .subject(&did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the custody invocation")?;
    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .context("failed to serialize the custody invocation")
}

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

/// Build a `/memory/publish` container for the wrapped-secret cell.
///
/// The invocation names the content by checksum; the bytes themselves
/// travel in the presigned PUT the permit authorizes. `when` carries
/// the cell's current version for an overwrite (re-wrap, rotation) and
/// stays `None` for the first write, which the protocol makes
/// first-write-only.
pub async fn build_publish_invocation(
    custody: Ed25519Signer,
    content: &[u8],
    when: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let mut arguments = cell_arguments();
    arguments.insert(
        "checksum".to_string(),
        Promised::Bytes(sha256_multihash(content)),
    );
    if let Some(version) = when {
        arguments.insert("when".to_string(), Promised::Bytes(version.to_vec()));
    }
    build_self_invocation(
        custody,
        vec!["memory".to_string(), "publish".to_string()],
        arguments,
    )
    .await
}

/// Build a `/memory/resolve` container for the wrapped-secret cell.
pub async fn build_resolve_invocation(custody: Ed25519Signer) -> Result<Vec<u8>> {
    build_self_invocation(
        custody,
        vec!["memory".to_string(), "resolve".to_string()],
        cell_arguments(),
    )
    .await
}

/// Mint the custody key's consent to being provided by the account —
/// the consumer powerline `/provider/add` deposits: issuer and subject
/// are the custody DID, audience is the account, command unrestricted.
pub async fn mint_custody_consent(
    custody: Ed25519Signer,
    account: &Did,
) -> Result<DelegationChain> {
    let subject = custody.did();
    let delegation = DelegationBuilder::new()
        .issuer(Signer::from(custody))
        .audience(account)
        .subject(UcanSubject::Specific(subject))
        .command(vec![])
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the custody consent: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web {
    use anyhow::{Context, Result, anyhow};
    use dialog_credentials::Ed25519Signer;
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
        let window = web_sys::window().context("no window: custody ceremonies are page-only")?;
        let response: Response = JsFuture::from(window.fetch_with_request(&request))
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
            anyhow::bail!("the access service refused the custody invocation ({status}): {detail}");
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
        let container = super::build_publish_invocation(custody, sealed, when).await?;
        let permit = redeem(endpoint, container).await?;
        let (status, body) = execute(permit, Some(sealed)).await?;
        if !(200..300).contains(&status) {
            let detail = String::from_utf8_lossy(&body);
            anyhow::bail!("storage refused the custody cell ({status}): {detail}");
        }
        Ok(())
    }

    /// Resolve the custody space's cell: `Ok(None)` when no wrapping
    /// was ever published there.
    pub async fn resolve_secret(custody: Ed25519Signer, endpoint: &str) -> Result<Option<Vec<u8>>> {
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::{publish_secret, resolve_secret};

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::promise::Promised;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    async fn custody() -> Ed25519Signer {
        Ed25519Signer::import(&*crate::envelope::custody_seed(&[5u8; 32]))
            .await
            .unwrap()
    }

    #[dialog_common::test]
    async fn it_builds_a_self_issued_publish_the_service_verifies() {
        let custody = custody().await;
        let did = custody.did();
        let bytes = build_publish_invocation(custody, b"sealed", None)
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
            vec!["memory".to_string(), "publish".to_string()],
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
        let bytes = build_publish_invocation(custody().await, b"resealed", Some(b"etag-1"))
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
            vec!["memory".to_string(), "resolve".to_string()],
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

//! Device-signed account-service invocation containers.
//!
//! The account service's `authorize` accepts requests issued by a device
//! key whose `root → device` delegation is attached as a proof, with the
//! account root as subject. This builds exactly that container from a
//! profile's live device signer and its stored `root → device` link — no
//! root key, no raw seed. Invocations carry a five-minute expiration; the
//! service refuses stale ones.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use dialog_credentials::Signer;
use dialog_ucan_core::cid::dagcbor_cid;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{
    Container, Delegation, DelegationChain, Invocation, InvocationBuilder, InvocationChain,
};
use dialog_varsig::AnySignature;
use dialog_varsig::Did;

/// Build a device-signed account-service invocation container.
///
/// `link` is the stored `root → device` delegation: its issuer is the
/// account root (the invocation subject and audience), and its single
/// proof is attached so the service can bind the device to the account.
pub async fn build_device_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
) -> Result<Vec<u8>> {
    let root_did = link.issuer().clone();
    debug_assert!(
        link.proofs().count() == 1,
        "build_device_invocation expects a single-hop root -> device link"
    );
    let delegation = link
        .proofs()
        .last()
        .context("account link carries no delegation to prove the device")?
        .clone();
    let cid = delegation.to_cid();

    let invocation = InvocationBuilder::new()
        .issuer(device.into())
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![cid])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the device invocation")?;

    let mut proofs = HashMap::new();
    proofs.insert(cid, Arc::new(delegation));
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .context("failed to serialize the device invocation")
}

/// The custody material an enrollment must carry: the passkey's space,
/// its consent to being provisioned, the pre-signed cell write, and the
/// sealed envelope that write puts there.
///
/// Values, not encodings. Each rides the enrollment as a block in its
/// container, so the only framing is the one the container itself
/// applies.
pub struct CustodyMaterial<'a> {
    /// The custody space's DID.
    pub custody: &'a Did,
    /// The custody key's consent to being provisioned by the account.
    pub consent: &'a Delegation<AnySignature>,
    /// The pre-signed cell write, proofless and self-issued.
    pub recovery: &'a Invocation<AnySignature>,
    /// The sealed envelope that write publishes.
    pub sealed: &'a [u8],
}

/// Decode a custody set from the hex a ceremony hands back.
///
/// The one place hex is understood. It arrives that way because the
/// ceremony's outputs cross into JavaScript, and it stops here: the
/// enrollment builder takes values, so nothing downstream re-parses.
pub fn decode_custody_material(
    custody: &str,
    consent: &str,
    recovery: &str,
    sealed: &str,
) -> Result<OwnedCustodyMaterial> {
    let consent = hex::decode(consent).context("the custody consent is not hex")?;
    let recovery = hex::decode(recovery).context("the publish invocation is not hex")?;
    Ok(OwnedCustodyMaterial {
        custody: custody
            .parse()
            .map_err(|error| anyhow::anyhow!("the custody DID does not parse: {error:?}"))?,
        consent: only_delegation(&consent)?,
        recovery: only_invocation(&recovery)?,
        sealed: hex::decode(sealed).context("the sealed envelope is not hex")?,
    })
}

/// The single token inside a one-token container.
///
/// The ceremony builds containers, and a carried block is a bare token —
/// the same unit the enclosing container holds. Unwrapping here keeps
/// the ceremony's outputs as they are for every other consumer.
fn only_token(bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    let mut tokens = Container::from_bytes(bytes)
        .with_context(|| format!("{what} is not a container"))?
        .into_tokens();
    if tokens.len() != 1 {
        anyhow::bail!("{what} carries {} tokens, expected one", tokens.len());
    }
    Ok(tokens.remove(0))
}

fn only_delegation(bytes: &[u8]) -> Result<Delegation<AnySignature>> {
    let token = only_token(bytes, "the custody consent")?;
    serde_ipld_dagcbor::from_slice(&token).context("the custody consent does not decode")
}

fn only_invocation(bytes: &[u8]) -> Result<Invocation<AnySignature>> {
    let token = only_token(bytes, "the publish invocation")?;
    serde_ipld_dagcbor::from_slice(&token).context("the publish invocation does not decode")
}

/// An owned custody set, for a caller that holds the material rather
/// than borrowing it from a ceremony's output.
pub struct OwnedCustodyMaterial {
    /// The custody space's DID.
    pub custody: Did,
    /// The custody key's consent to being provisioned.
    pub consent: Delegation<AnySignature>,
    /// The pre-signed cell write.
    pub recovery: Invocation<AnySignature>,
    /// The sealed envelope that write publishes.
    pub sealed: Vec<u8>,
}

impl OwnedCustodyMaterial {
    /// Borrow this set as the enrollment builders take it.
    pub fn borrow(&self) -> CustodyMaterial<'_> {
        CustodyMaterial {
            custody: &self.custody,
            consent: &self.consent,
            recovery: &self.recovery,
            sealed: &self.sealed,
        }
    }
}

/// A custody set minted from `key`, for a caller that has no ceremony:
/// tests, and the access service's own integration harness.
///
/// The same signatures over the same cell a ceremony produces, so what
/// this builds is accepted or refused for the reasons a real enrollment
/// would be.
pub async fn mint_custody_material(
    key: &dialog_credentials::Ed25519Signer,
    account: &Did,
    sealed: Vec<u8>,
) -> Result<OwnedCustodyMaterial> {
    use dialog_varsig::Principal as _;
    Ok(OwnedCustodyMaterial {
        custody: key.did(),
        consent: crate::custody::sign_custody_consent(
            dialog_credentials::Signer::from(key.clone()),
            account,
        )
        .await?,
        recovery: crate::custody::sign_deferred_publish_invocation(
            dialog_credentials::Signer::from(key.clone()),
            &sealed,
        )
        .await?,
        sealed,
    })
}

/// Build a `/customer/enroll` container for the access service.
///
/// Device-signed on the account's subject, exactly as
/// [`build_device_invocation`] does, carrying the custody material the
/// service verifies before it records anything.
pub async fn build_enroll_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    email: &str,
    custody: &CustodyMaterial<'_>,
) -> Result<Vec<u8>> {
    let recovery = serde_ipld_dagcbor::to_vec(custody.recovery)
        .context("failed to encode the publish invocation")?;
    let consent = custody.consent.encoded().to_vec();
    let sealed = custody.sealed.to_vec();

    let arguments = BTreeMap::from([
        ("email".to_string(), Promised::String(email.to_string())),
        (
            "custody".to_string(),
            Promised::String(custody.custody.to_string()),
        ),
        (
            "recovery".to_string(),
            Promised::Link(dagcbor_cid(&recovery)),
        ),
        ("consent".to_string(), Promised::Link(dagcbor_cid(&consent))),
        ("sealed".to_string(), Promised::Link(dagcbor_cid(&sealed))),
    ]);
    let invocation = build_device_invocation(
        device,
        link,
        vec!["customer".to_string(), "enroll".to_string()],
        arguments,
    )
    .await?;
    let mut tokens = Container::from_bytes(&invocation)
        .context("failed to reopen the enroll container")?
        .into_tokens();
    tokens.extend([recovery, consent, sealed]);
    Container::new(tokens)
        .to_bytes()
        .context("failed to encode the enroll container")
}

/// Build a `/provider/add` container for the access service.
///
/// The invocation is device-signed on the account's subject, and the
/// space's consent chain — its powerline to the account — is deposited
/// alongside, named by the CID of its head. The server walks the consent
/// from the consumer to the invoking customer.
pub async fn build_provider_add_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    consumer: &Did,
    consent: &DelegationChain,
    kind: Option<&str>,
) -> Result<Vec<u8>> {
    let head = consent
        .proofs()
        .next()
        .context("the consent chain carries no delegation")?;
    let mut arguments = BTreeMap::from([
        (
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        ),
        ("consent".to_string(), Promised::Link(head.to_cid())),
    ]);
    if let Some(kind) = kind {
        arguments.insert("kind".to_string(), Promised::String(kind.to_string()));
    }
    let invocation = build_device_invocation(
        device,
        link,
        vec!["provider".to_string(), "add".to_string()],
        arguments,
    )
    .await?;
    let mut tokens = Container::from_bytes(&invocation)
        .context("failed to reopen the add container")?
        .into_tokens();
    for delegation in consent.proofs() {
        tokens.push(delegation.encoded().to_vec());
    }
    Container::new(tokens)
        .to_bytes()
        .context("failed to encode the add container")
}

/// Build a `/provider/remove` container — the reverse of
/// [`build_provider_add_invocation`], and how a hosted space is
/// deleted: the invocation names the customer as its subject and the
/// space as its `consumer` argument, proving through the account's own
/// chain. No per-space artifact is deposited.
pub async fn build_provider_remove_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    consumer: &Did,
) -> Result<Vec<u8>> {
    build_device_invocation(
        device,
        link,
        vec!["provider".to_string(), "remove".to_string()],
        BTreeMap::from([(
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        )]),
    )
    .await
}

/// Build the `/void/customer/purge` invocation, signed by the account
/// root itself: proofless and self-subjected, the way a passkey ceremony
/// that has just recovered the root can sign. The service verifies the
/// chain, and the root's own signature is the whole chain.
///
/// Under `/void` deliberately: that is the destructive level of the
/// capability hierarchy, which no `/use` grant reaches.
pub async fn build_purge_invocation(root: impl Into<Signer>) -> Result<Vec<u8>> {
    use dialog_varsig::Principal as _;

    let root: Signer = root.into();
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(root)
        .audience(&root_did)
        .subject(&root_did)
        .command(PURGE_COMMAND.map(str::to_string).to_vec())
        .proofs(vec![])
        .issue_now()
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the purge invocation")?;
    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .context("failed to serialize the purge invocation")
}

/// The command [`build_purge_invocation`] signs.
pub const PURGE_COMMAND: [&str; 3] = ["void", "customer", "purge"];

/// Build the `/customer/resend` invocation: ask the service to mail the
/// activation link again.
///
/// Self-subjected and proofless, because the caller cannot sign as the
/// account they have not activated yet: the device signs as itself,
/// about the account named in the arguments. The service verifies only
/// that the signature is the issuer's own — the command is deliberately
/// unauthenticated beyond that, guarded by its rate limit and by the
/// mail going only to the address already on the row.
pub async fn build_resend_invocation(device: impl Into<Signer>, account: &Did) -> Result<Vec<u8>> {
    use dialog_varsig::Principal as _;

    let device: Signer = device.into();
    let device_did = device.did();
    let invocation = InvocationBuilder::new()
        .issuer(device)
        .audience(&device_did)
        .subject(&device_did)
        .command(vec!["customer".to_string(), "resend".to_string()])
        .arguments(BTreeMap::from([(
            "account".to_string(),
            Promised::String(account.to_string()),
        )]))
        .proofs(Vec::<ipld_core::cid::Cid>::new())
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the resend invocation")?;
    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .context("failed to serialize the resend invocation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationChain;
    use dialog_varsig::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_builds_a_device_signed_invocation_the_service_verifies() {
        let root = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let root_did = root.did();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let device_did = device.did();
        let link = crate::delegation::mint_device_delegation(root, &device_did)
            .await
            .unwrap();

        let arguments = [("chain".to_owned(), Promised::String("deadbeef".to_owned()))]
            .into_iter()
            .collect();
        let bytes = build_device_invocation(
            device,
            &link,
            vec!["account".into(), "chain".into(), "put".into()],
            arguments,
        )
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
        assert!(
            chain.invocation.expiration().is_some(),
            "device invocations must carry a ceremony expiration"
        );
        assert_eq!(chain.issuer(), &device_did);
        assert_eq!(chain.subject(), &root_did);
        assert_eq!(
            chain.command().0,
            vec![
                "account".to_string(),
                "chain".to_string(),
                "put".to_string()
            ],
        );
    }

    #[dialog_common::test]
    async fn it_builds_a_root_signed_purge_the_service_verifies() {
        let root = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let root_did = root.did();
        let bytes = build_purge_invocation(root).await.unwrap();

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
        assert_eq!(chain.issuer(), &root_did);
        assert_eq!(chain.subject(), &root_did);
        assert!(
            chain.proofs().is_empty(),
            "the root's signature is the chain"
        );
        assert_eq!(chain.command().0, PURGE_COMMAND.map(str::to_string));
        assert!(chain.invocation.expiration().is_some());
    }
}

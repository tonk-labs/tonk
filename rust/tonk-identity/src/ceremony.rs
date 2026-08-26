//! Root-authorized account ceremony payloads.
//!
//! These helpers bind every mutable account request field into a UCAN
//! invocation signed by the short-lived root signer. The root key is not
//! returned or persisted; callers receive only the signed invocation and the
//! `root → device` delegation it contains.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;
use ipld_core::cid::Cid;
use tonk_account::customer::deposit_scopes;

use crate::delegation::mint_device_delegation;

/// Output of a root-authorized account ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountCeremony {
    /// The passkey-derived root DID that signed the request.
    pub root_did: String,
    /// The device DID receiving the delegation.
    pub device_did: String,
    /// Hex-encoded `root → device` delegation chain.
    pub delegation_hex: String,
    /// Exact signed account repository descriptor, only during creation.
    pub descriptor_hex: Option<String>,
    /// Hex-encoded root-signed invocation container for the account service.
    pub invocation_hex: String,
}

/// Output of a provider-neutral local-root ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCeremony {
    /// Passkey-derived root DID.
    pub root_did: String,
    /// Device receiving the stable grant.
    pub device_did: String,
    /// Opaque WebAuthn credential identifier.
    pub credential_id: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Exact hex-encoded root → device delegation bytes.
    pub delegation_hex: String,
    /// Creation details when this ceremony created the passkey.
    pub passkey: Option<PasskeyCreationMetadata>,
    /// The account's X25519 recipient (`did:key:z6LS…`) when this
    /// ceremony held the secret, for the worker to publish as
    /// `AccountEncryptionKey`.
    pub encryption_key: Option<String>,
}

/// A fresh account, its first custody passkey, and its creation
/// invocation, produced from one in-memory secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyAccountCeremony {
    /// Material the browser persists only after credential creation succeeds.
    pub root: RootCeremony,
    /// Root-signed request submitted to the account service.
    pub account: AccountCeremony,
    /// Hex-encoded account-signed access-service deposits, when the
    /// caller named the service; empty otherwise.
    pub deposits_hex: Vec<String>,
    /// The passkey-derived custody DID — the custody space's subject.
    pub custody_did: String,
    /// Hex-encoded consent chain for `/provider/add`.
    pub consent_hex: String,
    /// Hex-encoded sealed account secret, when the custody cell could
    /// not be published during the ceremony because the account has not
    /// confirmed its email yet. The caller queues it and publishes once
    /// activation lands; `None` means the cell is already published.
    pub sealed_hex: Option<String>,
}

/// Informational metadata captured by the browser that created a passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyCreationMetadata {
    /// Browser-reported Unix time immediately after credential creation.
    pub created_at: u64,
    /// Browser and operating-system label where creation ran.
    pub created_on: String,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn root_ceremony(
    root: Ed25519Signer,
    credential_id: String,
    device_did: dialog_varsig::Did,
    passkey: Option<PasskeyCreationMetadata>,
    encryption_key: Option<String>,
) -> Result<RootCeremony> {
    let root_did = root.did().to_string();
    let delegation = mint_device_delegation(root, &device_did).await?;
    let delegation_cid = delegation.proof_cids()[0].to_string();
    let delegation_hex = hex::encode(
        delegation
            .to_bytes()
            .context("failed to serialize root to device delegation")?,
    );
    Ok(RootCeremony {
        root_did,
        device_did: device_did.to_string(),
        credential_id,
        delegation_cid,
        delegation_hex,
        passkey,
        encryption_key,
    })
}

fn strings(values: impl IntoIterator<Item = (&'static str, String)>) -> BTreeMap<String, Promised> {
    values
        .into_iter()
        .map(|(name, value)| (name.to_owned(), Promised::String(value)))
        .collect()
}

async fn build(
    root: Ed25519Signer,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
    device_did: String,
    delegation_hex: String,
    descriptor_hex: Option<String>,
) -> Result<AccountCeremony> {
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![])
        .issue_now()
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign account invocation")?;
    let container = InvocationChain::new(invocation, HashMap::new());
    let invocation_hex = hex::encode(
        container
            .to_bytes()
            .context("failed to serialize account invocation")?,
    );

    Ok(AccountCeremony {
        root_did: root_did.to_string(),
        device_did,
        delegation_hex,
        descriptor_hex,
        invocation_hex,
    })
}

/// Sign a witnessed revocation with the passkey-derived root.
///
/// The root must be an issuer in the path prefix through the target. The
/// resulting artifact carries the exact signed path and can be verified
/// without an account provider.
pub async fn sign_revocation(
    root: Ed25519Signer,
    path: &DelegationChain,
    target: &Cid,
) -> Result<String> {
    let bytes = crate::revocation::mint_root_revocation(root, path, target)
        .await
        .context("failed to sign the revocation")?;
    Ok(hex::encode(bytes))
}

/// Build account creation around an existing stable local-root grant.
#[allow(clippy::too_many_arguments)]
pub async fn create_account(
    root: Ed25519Signer,
    email: String,
    credential_id: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    delegation_hex: String,
    remote: String,
    passkey: Option<PasskeyCreationMetadata>,
) -> Result<AccountCeremony> {
    let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(&root, &remote)
        .await
        .context("failed to sign account repository descriptor")?;
    let descriptor_hex = hex::encode(descriptor.bytes());
    let device_did_string = device_did.to_string();
    let bytes = hex::decode(&delegation_hex).context("invalid existing delegation hex")?;
    let delegation = DelegationChain::try_from(bytes.as_slice())
        .context("invalid existing root to device delegation")?;
    if delegation.issuer() != &root.did() || delegation.audience() != &device_did {
        anyhow::bail!("existing delegation does not match the evaluated root and device");
    }
    let mut arguments = strings([
        ("email", email),
        ("credentialId", credential_id),
        ("deviceDid", device_did.to_string()),
        ("deviceName", device_name),
        ("delegation", delegation_hex.clone()),
        ("repositoryDescriptor", descriptor_hex.clone()),
    ]);
    if let Some(passkey) = passkey {
        arguments.insert(
            "passkeyCreatedAt".into(),
            Promised::Integer(passkey.created_at as i128),
        );
        arguments.insert(
            "passkeyCreatedOn".into(),
            Promised::String(passkey.created_on),
        );
    }
    build(
        root,
        vec!["account".into(), "create".into()],
        arguments,
        device_did_string,
        delegation_hex,
        Some(descriptor_hex),
    )
    .await
}

/// Create a passkey root and sign its account request without immediately
/// asking the new passkey for a second assertion.
/// Create an account and its first custody passkey in one ceremony
/// (`plan/Account custody.md`): generate the secret, create the
/// custody credential, seal the secret under its KEK, publish the
/// custody cell, and sign the account-creation request. The secret
/// exists only inside this function — no KEK and no wrapping is ever
/// stored anywhere; every later custody operation derives its keys
/// inside a fresh assertion.
///
/// The cell cannot publish here: the account has not enrolled yet, let
/// alone confirmed its email, and the access service serves nothing for
/// an unactivated customer. The sealed envelope comes back instead, for
/// the caller to queue and publish once activation lands
/// (`plan/account-activation-gate.md` §5). It is ciphertext under the
/// passkey's KEK, so holding it is not holding key material — but until
/// it is published the account can only be unlocked on this browser.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn create_custody_account(
    email: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    remote: String,
    created_on: Option<&str>,
    service: Option<&dialog_varsig::Did>,
) -> Result<CustodyAccountCeremony> {
    use crate::envelope::{AccountSecret, KekMethod, custody_kek, custody_signer};

    let secret = AccountSecret::generate()?;
    let root = secret.signer().await?;
    let account_did = root.did().to_string();

    let created = crate::passkey::create_custody_passkey(Some(&email), &account_did).await?;
    let credential_id = hex::encode(&created.id);
    let passkey = created_on.map(|created_on| PasskeyCreationMetadata {
        created_at: (js_sys::Date::now() / 1000.0) as u64,
        created_on: created_on.to_string(),
    });
    let evaluation = match created.evaluation {
        Some(evaluation) => evaluation,
        None => crate::passkey::evaluate_custody_passkey(Some(&created.id))
            .await?
            .evaluation
            .context("the authenticator returned no PRF outputs")?,
    };
    let custody = custody_signer(&evaluation.key).await?;
    let kek = custody_kek(&evaluation.kek);
    let sealed = kek.seal(&secret, KekMethod::Passkey)?.encode();
    let consent = crate::custody::mint_custody_consent(custody.clone(), &root.did()).await?;
    let consent_hex = hex::encode(
        consent
            .to_bytes()
            .context("failed to serialize the custody consent")?,
    );

    let deposits_hex = match service {
        Some(service) => mint_service_deposits(&root, service).await?,
        None => Vec::new(),
    };
    let encryption_key = secret.encryption_key().recipient().did().to_string();
    let root_ceremony = root_ceremony(
        root.clone(),
        credential_id.clone(),
        device_did.clone(),
        passkey.clone(),
        Some(encryption_key),
    )
    .await?;
    let account = create_account(
        root,
        email,
        credential_id,
        device_did,
        device_name,
        root_ceremony.delegation_hex.clone(),
        remote,
        passkey,
    )
    .await?;
    Ok(CustodyAccountCeremony {
        root: root_ceremony,
        account,
        deposits_hex,
        custody_did: custody.did().to_string(),
        consent_hex,
        sealed_hex: Some(hex::encode(&sealed)),
    })
}

/// One assertion, one presigned GET, one unwrap: evaluate a custody
/// passkey, resolve its cell, and open the envelope. The returned
/// secret lives only in the caller's scope; every custody operation
/// derives its keys inside a fresh user-verified assertion, and no key
/// material is ever stored.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn assert_unlock(
    endpoint: &str,
    credential_id: Option<&[u8]>,
) -> Result<(crate::envelope::AccountSecret, String)> {
    use crate::envelope::{Envelope, custody_kek, custody_signer};

    let evaluated = crate::passkey::evaluate_custody_passkey(credential_id).await?;
    let credential_id = hex::encode(evaluated.id);
    let evaluation = evaluated
        .evaluation
        .context("the authenticator returned no PRF outputs")?;
    let custody = custody_signer(&evaluation.key).await?;
    let kek = custody_kek(&evaluation.kek);
    let sealed = crate::custody::resolve_secret(custody, endpoint)
        .await?
        .context("no account custody is published for this passkey")?;
    let envelope = Envelope::decode(&sealed)
        .map_err(|error| anyhow::anyhow!("the custody cell is unreadable: {error}"))?;
    let secret = kek
        .open(&envelope)
        .map_err(|error| anyhow::anyhow!("the custody envelope did not open: {error}"))?;
    Ok((secret, credential_id))
}

/// Materialize the account signer through a custody assertion, for
/// root-signed operations: CLI approval, link completion, revocation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn unlock_root(endpoint: &str) -> Result<Ed25519Signer> {
    let (secret, _) = assert_unlock(endpoint, None).await?;
    secret.signer().await
}

/// Derive the account's X25519 recipient through a custody assertion,
/// for a device whose root record predates the key: the worker asks the
/// page for this when it needs custody set up and nothing recorded it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn publish_encryption_key(
    endpoint: &str,
    credential_id: Option<&[u8]>,
) -> Result<String> {
    let (secret, _) = assert_unlock(endpoint, credential_id).await?;
    Ok(secret.encryption_key().recipient().did().to_string())
}

/// A custody passkey enrollment's outcome: the custody DID and consent
/// the worker provisions with, and the credential the browser records.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyEnrollment {
    /// The passkey-derived custody DID — the custody space's subject.
    pub custody_did: String,
    /// Opaque WebAuthn credential identifier, hex-encoded.
    pub credential_id: String,
    /// Hex-encoded consent chain for `/provider/add`: the custody
    /// key's agreement to being provided by the account.
    pub consent_hex: String,
    /// Hex-encoded sealed account secret, when the cell could not be
    /// published yet — the new custody DID is not provisioned until the
    /// caller deposits the consent above. `None` once it is published.
    pub sealed_hex: Option<String>,
}

/// Enroll an additional custody passkey: unlock the account through an
/// existing one, create the new credential, seal the secret under its
/// KEK, and publish its cell. The caller provisions the custody DID
/// with the returned consent afterwards — best-effort and retryable,
/// where the published cell is the account's durability.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn enroll_custody(
    account_did: &str,
    label: Option<&str>,
    endpoint: &str,
) -> Result<CustodyEnrollment> {
    use crate::envelope::{Envelope, KekMethod, custody_kek, custody_signer};

    // Unlock through an existing passkey: one assertion recovers the
    // secret, and the new credential seals that same secret. Nothing is
    // read from, or written to, any local store.
    let (secret, _) = assert_unlock(endpoint, None).await?;
    let root = secret.signer().await?;
    if root.did().to_string() != account_did {
        anyhow::bail!("the asserted passkey unlocks a different account");
    }

    let created = crate::passkey::create_custody_passkey(label, account_did).await?;
    let credential_id = hex::encode(&created.id);
    let evaluation = match created.evaluation {
        Some(evaluation) => evaluation,
        None => crate::passkey::evaluate_custody_passkey(Some(&created.id))
            .await?
            .evaluation
            .context("the authenticator returned no PRF outputs")?,
    };
    let custody = custody_signer(&evaluation.key).await?;
    let kek = custody_kek(&evaluation.kek);
    let sealed = kek.seal(&secret, KekMethod::Passkey)?.encode();

    // A brand new custody DID is nobody's consumer until the caller
    // deposits the consent below, and the access service serves no
    // unprovisioned subject — so a refusal here is the expected first
    // outcome, not a failure. Hand the sealed bytes back to be queued
    // and published once provisioning lands.
    let mut sealed_hex = Some(hex::encode(&sealed));
    if let Err(publish_error) =
        crate::custody::publish_secret(custody.clone(), &sealed, endpoint, None).await
    {
        // The cell may already exist: the same credential re-enrolled
        // names the same space. Anything that opens to this account is
        // that case, and needs no republish; anything else waits.
        match crate::custody::resolve_secret(custody.clone(), endpoint).await {
            Ok(Some(existing)) => {
                let published = Envelope::decode(&existing)
                    .ok()
                    .and_then(|envelope| kek.open(&envelope).ok());
                match published {
                    Some(open) if open.signing_seed() == secret.signing_seed() => {
                        sealed_hex = None;
                    }
                    _ => anyhow::bail!("the custody space already holds a different account"),
                }
            }
            // Not published, and not already ours: leave `sealed_hex`
            // set so the caller queues it. The error itself is the
            // caller's to report, once it knows whether provisioning is
            // still pending.
            _ => {
                let _ = publish_error;
            }
        }
    } else {
        sealed_hex = None;
    }

    let consent = crate::custody::mint_custody_consent(custody.clone(), &root.did()).await?;
    let consent_hex = hex::encode(
        consent
            .to_bytes()
            .context("failed to serialize the custody consent")?,
    );
    Ok(CustodyEnrollment {
        custody_did: custody.did().to_string(),
        credential_id,
        consent_hex,
        sealed_hex,
    })
}

/// Publish a queued custody cell, with a fresh assertion.
///
/// The queued bytes were sealed under this passkey's KEK during the
/// ceremony that created it; the assertion here re-derives the custody
/// key that signs the publish, and proves the same credential is
/// present. A cell that already holds this account's envelope is
/// success — another device got there first.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn publish_queued_custody(
    custody_did: &str,
    sealed: &[u8],
    endpoint: &str,
    credential_id: Option<&[u8]>,
) -> Result<()> {
    use crate::envelope::{custody_kek, custody_signer};

    let evaluated = crate::passkey::evaluate_custody_passkey(credential_id).await?;
    let evaluation = evaluated
        .evaluation
        .context("the authenticator returned no PRF outputs")?;
    let custody = custody_signer(&evaluation.key).await?;
    if custody.did().to_string() != custody_did {
        anyhow::bail!("the asserted passkey derives a different custody space");
    }
    // Prove the queued bytes open under this passkey before publishing:
    // a cell that cannot be unsealed by the credential that owns it is
    // worse than no cell at all.
    let kek = custody_kek(&evaluation.kek);
    let envelope = crate::envelope::Envelope::decode(sealed)
        .map_err(|error| anyhow::anyhow!("the queued envelope is unreadable: {error}"))?;
    kek.open(&envelope)
        .map_err(|error| anyhow::anyhow!("the queued envelope did not open: {error}"))?;

    match crate::custody::publish_secret(custody.clone(), sealed, endpoint, None).await {
        Ok(()) => Ok(()),
        Err(publish_error) => match crate::custody::resolve_secret(custody, endpoint).await {
            Ok(Some(_)) => Ok(()),
            _ => Err(publish_error),
        },
    }
}

/// A passkey unlock's outcome: the device-link ceremony minted from
/// the unwrapped account, and the credential that unlocked it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyUnlock {
    /// The root-signed link request, exactly as `link_device` mints it.
    pub account: AccountCeremony,
    /// Opaque WebAuthn credential identifier, hex-encoded.
    pub credential_id: String,
    /// Hex-encoded account-signed access-service deposits, when the
    /// caller named the service; empty otherwise.
    pub deposits_hex: Vec<String>,
    /// The account's X25519 recipient (`did:key:z6LS…`), for the worker
    /// to publish as `AccountEncryptionKey`.
    pub encryption_key: String,
}

/// Unlock the account with a custody passkey on a fresh browser: one
/// assertion derives the custody keypair and KEK, one presigned GET
/// fetches the sealed envelope, and the unwrapped secret self-issues
/// the direct `account → device` delegation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn unlock_account(
    device_did: dialog_varsig::Did,
    device_name: String,
    endpoint: &str,
    service: Option<&dialog_varsig::Did>,
) -> Result<CustodyUnlock> {
    let (secret, credential_id) = assert_unlock(endpoint, None).await?;
    let root = secret.signer().await?;
    let encryption_key = secret.encryption_key().recipient().did().to_string();
    let deposits_hex = match service {
        Some(service) => mint_service_deposits(&root, service).await?,
        None => Vec::new(),
    };
    let account = link_device(root, device_did, device_name).await?;
    Ok(CustodyUnlock {
        account,
        credential_id,
        deposits_hex,
        encryption_key,
    })
}

/// Mint the scoped access-service deposits enrollment presents, issued
/// directly by the account root while a ceremony holds it. An
/// account-signed deposit outlives the device that carried it: revoking
/// that device leaves the service's grant standing, where a
/// device-issued chain would die with its link. Returned hex-encoded so
/// they cross the JS bridge and ride callback payloads as-is.
pub async fn mint_service_deposits(
    root: &Ed25519Signer,
    service: &dialog_varsig::Did,
) -> Result<Vec<String>> {
    let mut deposits = Vec::new();
    for scope in deposit_scopes(&root.did(), service) {
        let deposit = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(root.clone()))
            .audience(service)
            .subject(scope.subject.clone())
            .command(scope.command.segments().clone())
            .policy(scope.policy())
            .try_build()
            .await
            .context("failed to mint the access-service deposit")?;
        deposits.push(hex::encode(deposit.encoded()));
    }
    Ok(deposits)
}

/// Build the root-signed request that links a new device to an existing account.
pub async fn link_device(
    root: Ed25519Signer,
    device_did: dialog_varsig::Did,
    device_name: String,
) -> Result<AccountCeremony> {
    let device_did_string = device_did.to_string();
    let delegation = mint_device_delegation(root.clone(), &device_did).await?;
    let delegation_hex = hex::encode(
        delegation
            .to_bytes()
            .context("failed to serialize root to device delegation")?,
    );
    build(
        root,
        vec!["account".into(), "device".into(), "link".into()],
        strings([
            ("deviceDid", device_did.to_string()),
            ("deviceName", device_name),
            ("delegation", delegation_hex.clone()),
        ]),
        device_did_string,
        delegation_hex,
        None,
    )
    .await
}

/// Authorize a CLI device directly, with no account service in the loop.
///
/// The browser mints the `account → device` powerline and returns it for
/// delivery straight back to the waiting process. The descriptor rides along
/// so the device learns where the account repository lives — a delegation
/// says who may act, not where to sync from, and without it the device is
/// authorized but cannot find the account.
///
/// The remote is the account's own, so the descriptor this signs is
/// byte-identical to the established one: signing is deterministic over
/// `(version, subject, remote)`, so this reproduces the existing descriptor
/// rather than minting a competing one.
pub async fn authorize_device(
    root: Ed25519Signer,
    device_did: dialog_varsig::Did,
    remote: &str,
) -> Result<AuthorizedDevice> {
    let delegation = mint_device_delegation(root.clone(), &device_did).await?;
    let delegation_hex = hex::encode(
        delegation
            .to_bytes()
            .context("failed to serialize root to device delegation")?,
    );
    let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(&root, remote)
        .await
        .context("failed to sign the account repository descriptor")?;
    Ok(AuthorizedDevice {
        root_did: root.did().to_string(),
        device_did: device_did.to_string(),
        delegation_hex,
        descriptor_hex: hex::encode(descriptor.bytes()),
    })
}

/// What a callback authorization hands back to the waiting device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedDevice {
    /// The passkey-derived account root that issued the grant.
    pub root_did: String,
    /// The device the grant is addressed to.
    pub device_did: String,
    /// Hex-encoded `account → device` delegation chain.
    pub delegation_hex: String,
    /// Exact signed account repository descriptor, so the device knows where
    /// the account repository lives.
    pub descriptor_hex: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::InvocationChain;

    async fn fixture() -> (Ed25519Signer, dialog_varsig::Did) {
        let root = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        (root, device.did())
    }

    #[dialog_common::test]
    async fn it_binds_account_creation_fields_to_the_root_signature() {
        let (root, device) = fixture().await;
        let expected_root = root.did();
        let delegation = crate::delegation::mint_device_delegation(root.clone(), &device)
            .await
            .unwrap();
        let delegation_hex = hex::encode(delegation.to_bytes().unwrap());
        let output = create_account(
            root,
            "a@x.com".into(),
            "credential".into(),
            device.clone(),
            "laptop".into(),
            delegation_hex,
            "https://accounts.example/ucan/".into(),
            Some(PasskeyCreationMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".into(),
            }),
        )
        .await
        .unwrap();
        let bytes = hex::decode(output.invocation_hex).unwrap();
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
        assert_eq!(chain.issuer(), &expected_root);
        assert_eq!(chain.subject(), &expected_root);
        assert_eq!(
            chain.command().0,
            vec!["account".to_string(), "create".to_string()]
        );
        assert_eq!(
            chain.arguments().get("deviceDid"),
            Some(&Promised::String(device.to_string()))
        );
        assert_eq!(
            chain.arguments().get("email"),
            Some(&Promised::String("a@x.com".into()))
        );
        assert_eq!(
            chain.arguments().get("passkeyCreatedAt"),
            Some(&Promised::Integer(1_754_380_800))
        );
        assert_eq!(
            chain.arguments().get("passkeyCreatedOn"),
            Some(&Promised::String("Chrome on macOS".into()))
        );
        let descriptor_hex = output.descriptor_hex.unwrap();
        assert_eq!(
            chain.arguments().get("repositoryDescriptor"),
            Some(&Promised::String(descriptor_hex.clone()))
        );
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(
            &hex::decode(descriptor_hex).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(descriptor.account_subject(), &expected_root);

        let (root, device) = fixture().await;
        let delegation = crate::delegation::mint_device_delegation(root.clone(), &device)
            .await
            .unwrap();
        let legacy = create_account(
            root,
            "legacy@x.com".into(),
            "legacy-credential".into(),
            device,
            "old browser".into(),
            hex::encode(delegation.to_bytes().unwrap()),
            "https://accounts.example/ucan/".into(),
            None,
        )
        .await
        .unwrap();
        let bytes = hex::decode(legacy.invocation_hex).unwrap();
        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        assert!(!chain.arguments().contains_key("passkeyCreatedAt"));
        assert!(!chain.arguments().contains_key("passkeyCreatedOn"));
    }

    #[dialog_common::test]
    async fn it_builds_a_root_authorized_self_link() {
        let (root, device) = fixture().await;
        let expected_root = root.did();
        let output = link_device(root, device.clone(), "phone".into())
            .await
            .unwrap();
        let bytes = hex::decode(output.invocation_hex).unwrap();
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
        assert_eq!(chain.issuer(), &expected_root);
        assert_eq!(chain.subject(), &expected_root);
        assert_eq!(
            chain.command().0,
            vec![
                "account".to_string(),
                "device".to_string(),
                "link".to_string()
            ]
        );
        assert_eq!(
            chain.arguments().get("deviceDid"),
            Some(&Promised::String(device.to_string()))
        );
        assert!(output.descriptor_hex.is_none());
    }
}

//! Root-authorized account ceremony payloads.
//!
//! These helpers bind every mutable account request field into a UCAN
//! invocation signed by the short-lived root signer. The root key is not
//! returned or persisted; callers receive only the signed invocation and the
//! `root → device` delegation it contains.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;
use ipld_core::cid::Cid;

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
    /// `AccountSealedInbox`.
    pub encryption_key: Option<String>,
}

/// Informational metadata captured by the browser that created a passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyCreationMetadata {
    /// Browser-reported Unix time immediately after credential creation.
    pub created_at: u64,
    /// Browser and operating-system label where creation ran.
    pub created_on: String,
}

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
    root: impl Into<Signer>,
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

/// Sign an account-creation request for a root the caller already
/// holds.
///
/// The half of [`create_custody_account`] that needs no passkey: the
/// custodian has already been asserted, the secret already derived, and
/// what remains is minting the device grant and signing the request.
/// Callable off the page, which is the point — the worker holds the
/// secret now, and the browser never does.
pub async fn create_custody_request(
    root: Ed25519Signer,
    request: AccountRequest,
) -> Result<CustodyAccountRequest> {
    let AccountRequest {
        credential_id,
        device_did,
        email,
        device_name,
        remote,
        created_on,
        encryption_key,
    } = request;
    let passkey = created_on.map(|created_on| PasskeyCreationMetadata {
        created_at: now_seconds(),
        created_on,
    });
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
    Ok(CustodyAccountRequest {
        root: root_ceremony,
        account,
    })
}

/// Seconds since the epoch, however this target tells the time.
fn now_seconds() -> u64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default()
    }
}

/// What an account-creation request is for: the passkey that holds it,
/// the browser making it, and where the account lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRequest {
    /// The WebAuthn credential the custody passkey is reached through.
    pub credential_id: String,
    /// The browser receiving the account's stable grant.
    pub device_did: dialog_varsig::Did,
    /// The address the account is created for.
    pub email: String,
    /// What the browser is called in the account's device list.
    pub device_name: String,
    /// The account repository's remote, so the descriptor names it.
    pub remote: String,
    /// Browser/OS label to record with the passkey, when this request
    /// follows its creation.
    pub created_on: Option<String>,
    /// The account's X25519 recipient, published as
    /// `AccountSealedInbox`.
    pub encryption_key: String,
}

/// What signing an account-creation request produces: the root record
/// to persist, and the request to submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyAccountRequest {
    /// Material to record before the request is submitted.
    pub root: RootCeremony,
    /// The root-signed request for the account service.
    pub account: AccountCeremony,
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
    let sealed =
        crate::custody::resolve_secret(dialog_credentials::Signer::from(custody), endpoint)
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

/// Unlock an account and mint the local root record for this browser.
///
/// Current sign-in is worker-mediated. This compatibility producer remains
/// necessary for a page from before encryption-key persistence: it deliberately
/// hands the caller the key so the test can omit it from the local root record
/// and exercise the worker's assertion recovery path.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn unlock_account(
    device_did: dialog_varsig::Did,
    endpoint: &str,
) -> Result<RootCeremony> {
    let (secret, credential_id) = assert_unlock(endpoint, None).await?;
    let root = secret.signer().await?;
    let encryption_key = secret.secret().did().to_string();
    root_ceremony(root, credential_id, device_did, None, Some(encryption_key)).await
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
    Ok(secret.secret().did().to_string())
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

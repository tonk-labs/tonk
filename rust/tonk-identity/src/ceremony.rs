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

/// Output of the one-time account repository establishment ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRepositoryCeremony {
    /// The passkey-derived root DID that signed the request.
    pub root_did: String,
    /// Exact signed account repository descriptor.
    pub descriptor_hex: String,
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
}

/// A fresh passkey root and its account-creation invocation, produced from
/// one in-memory root signer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshAccountCeremony {
    /// Material the browser persists only after credential creation succeeds.
    pub root: RootCeremony,
    /// Root-signed request submitted to the account service.
    pub account: AccountCeremony,
    /// Hex-encoded account-signed access-service deposits, when the
    /// caller named the service; empty otherwise.
    pub deposits_hex: Vec<String>,
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
    })
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn create_root_material(
    label: Option<&str>,
    created_on: Option<&str>,
) -> Result<(Ed25519Signer, String, Option<PasskeyCreationMetadata>)> {
    let created = crate::passkey::create_passkey(label).await?;
    let credential_id = hex::encode(created.id);
    let passkey = created_on.map(|created_on| PasskeyCreationMetadata {
        created_at: (js_sys::Date::now() / 1000.0) as u64,
        created_on: created_on.to_string(),
    });
    let prf = match created.prf_output {
        Some(output) => output,
        None => crate::passkey::prf_output().await?,
    };
    let root = crate::derive::derive_root_signer(&prf).await?;
    Ok((root, credential_id, passkey))
}

/// Create a passkey root and delegate it to `device_did`.
///
/// `label` names the credential in the user's passkey manager: the account
/// address when an account ceremony creates this root, `None` when a spot
/// creates it before any account exists. It is metadata for the person, not
/// for the chain — no delegation, and no authority, depends on it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn create_root(
    device_did: dialog_varsig::Did,
    label: Option<&str>,
    created_on: Option<&str>,
) -> Result<RootCeremony> {
    let (root, credential_id, passkey) = create_root_material(label, created_on).await?;
    root_ceremony(root, credential_id, device_did, passkey).await
}

/// Evaluate an existing discoverable passkey and delegate its root to `device_did`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn evaluate_root(device_did: dialog_varsig::Did) -> Result<RootCeremony> {
    let evaluated = crate::passkey::evaluate_passkey().await?;
    let credential_id = hex::encode(evaluated.id);
    let prf = evaluated
        .prf_output
        .context("the authenticator returned no PRF output")?;
    let root = crate::derive::derive_root_signer(&prf).await?;
    root_ceremony(root, credential_id, device_did, None).await
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
        .issuer(root)
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

/// Sign the permanent account-service deletion request with the passkey root.
/// The verified email is repeated as a signed, human-readable confirmation.
pub async fn delete_account(
    root: Ed25519Signer,
    confirmed_email: String,
) -> Result<AccountCeremony> {
    build(
        root,
        vec!["account".into(), "delete".into()],
        strings([("confirmedEmail", confirmed_email)]),
        String::new(),
        String::new(),
        None,
    )
    .await
}

/// Sign access-service customer finalization with the passkey root.
pub async fn delete_access_customer(root: Ed25519Signer) -> Result<AccountCeremony> {
    build(
        root,
        vec!["customer".into(), "delete".into()],
        BTreeMap::new(),
        String::new(),
        String::new(),
        None,
    )
    .await
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
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(clippy::too_many_arguments)]
pub async fn create_fresh_account(
    email: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    remote: String,
    created_on: Option<&str>,
    service: Option<&dialog_varsig::Did>,
) -> Result<FreshAccountCeremony> {
    let (root, credential_id, passkey) = create_root_material(Some(&email), created_on).await?;
    let deposits_hex = match service {
        Some(service) => mint_service_deposits(&root, service).await?,
        None => Vec::new(),
    };
    let root_ceremony = root_ceremony(
        root.clone(),
        credential_id.clone(),
        device_did.clone(),
        passkey.clone(),
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
    Ok(FreshAccountCeremony {
        root: root_ceremony,
        account,
        deposits_hex,
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
            .issuer(root.clone())
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

/// Sign the one-time repository descriptor for an existing account.
pub async fn establish_account_repository(
    root: Ed25519Signer,
    remote: String,
) -> Result<AccountRepositoryCeremony> {
    let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(&root, &remote)
        .await
        .context("failed to sign account repository descriptor")?;
    let descriptor_hex = hex::encode(descriptor.bytes());
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(root)
        .audience(&root_did)
        .subject(&root_did)
        .command(vec![
            "account".into(),
            "repository".into(),
            "establish".into(),
        ])
        .arguments(strings([("repositoryDescriptor", descriptor_hex.clone())]))
        .proofs(vec![])
        .issue_now()
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign account repository invocation")?;
    let invocation = InvocationChain::new(invocation, HashMap::new());
    Ok(AccountRepositoryCeremony {
        root_did: root_did.to_string(),
        descriptor_hex,
        invocation_hex: hex::encode(
            invocation
                .to_bytes()
                .context("failed to serialize account invocation")?,
        ),
    })
}

/// Build the root-signed completion for a CLI browser handoff.
/// Authorize a CLI device directly, with no account service in the loop.
///
/// The browser mints the same `account → device` powerline `complete_link`
/// does, but returns it for delivery straight back to the waiting process
/// instead of wrapping it in a service invocation. The descriptor rides along
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

/// Complete a browser handoff: mint the `root → device` delegation and wrap
/// it in the invocation the account service consumes.
pub async fn complete_link(
    root: Ed25519Signer,
    token_hash: String,
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
        vec!["account".into(), "link".into(), "complete".into()],
        strings([
            ("tokenHash", token_hash),
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

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::InvocationChain;

    async fn fixture() -> (Ed25519Signer, dialog_varsig::Did) {
        let root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
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
            .verify(&dialog_credentials::Ed25519KeyResolver)
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
            .verify(&dialog_credentials::Ed25519KeyResolver)
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

    #[dialog_common::test]
    async fn it_signs_a_non_expiring_descriptor_for_establishment() {
        let (root, _) = fixture().await;
        let expected_root = root.did();
        let output = establish_account_repository(root, "https://accounts.example/ucan/".into())
            .await
            .unwrap();
        let invocation =
            InvocationChain::try_from(hex::decode(output.invocation_hex).unwrap().as_slice())
                .unwrap();
        invocation
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(
            invocation.command().0,
            vec![
                "account".to_string(),
                "repository".to_string(),
                "establish".to_string(),
            ]
        );
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(
            &hex::decode(&output.descriptor_hex).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(descriptor.account_subject(), &expected_root);
        assert_eq!(
            invocation.arguments().get("repositoryDescriptor"),
            Some(&Promised::String(output.descriptor_hex))
        );
    }

    #[dialog_common::test]
    async fn it_binds_a_cli_handoff_to_its_token_and_device() {
        let (root, device) = fixture().await;
        let output = complete_link(root, "hash".into(), device.clone(), "terminal".into())
            .await
            .unwrap();
        let bytes = hex::decode(output.invocation_hex).unwrap();
        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(
            chain.command().0,
            vec![
                "account".to_string(),
                "link".to_string(),
                "complete".to_string()
            ]
        );
        assert_eq!(
            chain.arguments().get("tokenHash"),
            Some(&Promised::String("hash".into()))
        );
        assert_eq!(
            chain.arguments().get("deviceDid"),
            Some(&Promised::String(device.to_string()))
        );
    }
}

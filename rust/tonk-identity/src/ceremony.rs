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
    code: String,
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
        ("code", code),
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

/// Output of the ephemeral-root genesis ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisCeremony {
    /// The account subject: a key that no longer exists anywhere.
    pub account_subject: String,
    /// Hex-encoded `root → credential`, the first passkey's enrollment.
    pub credential_delegation_hex: String,
    /// Hex-encoded `root → anchor`, the only recovery path this account
    /// will ever have that did not come through a live credential.
    pub anchor_delegation_hex: String,
    /// Root-signed account creation request.
    pub account: AccountCeremony,
}

/// Create an account whose subject is a key destroyed during the ceremony.
///
/// The passkey-derived key stops being the account and becomes one credential
/// among peers: the subject is a fresh keypair that signs the descriptor and
/// fans out to the credential and to `anchor`, then goes away. Nothing can
/// ever mint a direct `root → x` delegation again, which is what makes both
/// of those delegations load-bearing — see the anchor invariant in the spec.
///
/// The device's grant runs `root → credential → device`, so the credential
/// remains the thing that authorizes devices day to day.
#[allow(clippy::too_many_arguments)]
pub async fn create_account_with_ephemeral_root(
    credential: Ed25519Signer,
    anchor: &dialog_varsig::Did,
    email: String,
    code: String,
    credential_id: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    remote: String,
    passkey: Option<PasskeyCreationMetadata>,
) -> Result<GenesisCeremony> {
    // Generated here and dropped at the end of this function. The seed
    // zeroizes with its guard; the signer holds the only other copy, and on
    // the web target that copy is a non-extractable WebCrypto key.
    let seed = zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
    let root = Ed25519Signer::import(&*seed)
        .await
        .map_err(|err| anyhow::anyhow!("failed to generate the account root: {err}"))?;
    let account_subject = root.did().to_string();

    let credential_link = crate::credential::mint_enrollment(root.clone(), &credential.did())
        .await
        .context("failed to enrol the first credential")?;
    let anchor_link = crate::credential::mint_enrollment(root.clone(), anchor)
        .await
        .context("failed to delegate to the recovery anchor")?;
    let device_chain =
        crate::credential::extend_with_enrollment(&credential_link, credential, &device_did)
            .await
            .context("failed to grant the device through the credential")?;

    let account = create_account(
        root,
        email,
        code,
        credential_id,
        device_did,
        device_name,
        hex::encode(
            device_chain
                .to_bytes()
                .context("failed to serialize the device grant")?,
        ),
        remote,
        passkey,
    )
    .await?;

    Ok(GenesisCeremony {
        account_subject,
        credential_delegation_hex: hex::encode(
            credential_link
                .to_bytes()
                .context("failed to serialize the credential enrollment")?,
        ),
        anchor_delegation_hex: hex::encode(
            anchor_link
                .to_bytes()
                .context("failed to serialize the anchor delegation")?,
        ),
        account,
    })
}

/// Create a passkey root and sign its account request without immediately
/// asking the new passkey for a second assertion.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(clippy::too_many_arguments)]
pub async fn create_fresh_account(
    email: String,
    code: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    remote: String,
    created_on: Option<&str>,
) -> Result<FreshAccountCeremony> {
    let (root, credential_id, passkey) = create_root_material(Some(&email), created_on).await?;
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
        code,
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
    })
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
            "123456".into(),
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
            "123456".into(),
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
    async fn it_makes_the_account_subject_a_key_no_one_holds() {
        let credential = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let anchor = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();

        let genesis = create_account_with_ephemeral_root(
            credential.clone(),
            &anchor.did(),
            "a@x.com".into(),
            "123456".into(),
            "credential".into(),
            device.did(),
            "laptop".into(),
            "https://accounts.example/ucan/".into(),
            None,
        )
        .await
        .unwrap();

        let subject: dialog_varsig::Did = genesis.account_subject.parse().unwrap();
        assert_ne!(subject, credential.did(), "the passkey is not the account");
        assert_ne!(subject, device.did());
        assert_ne!(subject, anchor.did());

        // Both genesis delegations run from the subject, and neither could be
        // minted again once this ceremony returns.
        let credential_link = DelegationChain::try_from(
            hex::decode(&genesis.credential_delegation_hex)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let anchor_link = DelegationChain::try_from(
            hex::decode(&genesis.anchor_delegation_hex)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(*credential_link.issuer(), subject);
        assert_eq!(*credential_link.audience(), credential.did());
        assert_eq!(*anchor_link.issuer(), subject);
        assert_eq!(*anchor_link.audience(), anchor.did());

        // The descriptor is signed by the subject while it still exists.
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(
            &hex::decode(genesis.account.descriptor_hex.as_ref().unwrap()).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(*descriptor.account_subject(), subject);

        // And the device's grant reaches it through the credential.
        let grant = DelegationChain::try_from(
            hex::decode(&genesis.account.delegation_hex)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(grant.proof_cids().len(), 2, "root → credential → device");
        let validated = crate::delegation::validate_account_grant(&grant, &device.did())
            .await
            .unwrap();
        assert_eq!(validated.root_did, subject);
    }

    #[dialog_common::test]
    async fn it_mints_a_distinct_subject_for_every_account() {
        let credential = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let anchor = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let genesis = async || {
            create_account_with_ephemeral_root(
                credential.clone(),
                &anchor.did(),
                "a@x.com".into(),
                "123456".into(),
                "credential".into(),
                device.did(),
                "laptop".into(),
                "https://accounts.example/ucan/".into(),
                None,
            )
            .await
            .unwrap()
            .account_subject
        };

        assert_ne!(
            genesis().await,
            genesis().await,
            "the same passkey must not re-derive an account subject"
        );
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

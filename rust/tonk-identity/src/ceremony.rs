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
use dialog_ucan_core::{InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;

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
    /// Hex-encoded root-signed invocation container for the account service.
    pub invocation_hex: String,
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
        invocation_hex,
    })
}

/// Build the root-signed account-creation request and first-device delegation.
pub async fn create_account(
    root: Ed25519Signer,
    email: String,
    code: String,
    credential_id: String,
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
        vec!["account".into(), "create".into()],
        strings([
            ("email", email),
            ("code", code),
            ("credentialId", credential_id),
            ("deviceDid", device_did.to_string()),
            ("deviceName", device_name),
            ("delegation", delegation_hex.clone()),
        ]),
        device_did_string,
        delegation_hex,
    )
    .await
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
    )
    .await
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
    )
    .await
}

/// Output of the two-container rotation ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationCeremony {
    /// The account's current (old) root DID.
    pub old_root_did: String,
    /// The root DID the account rotates onto.
    pub new_root_did: String,
    /// Hex-encoded `oldRoot → newRoot` succession chain.
    pub succession_hex: String,
    /// Hex-encoded `newRoot → device` delegation for the ceremony device.
    pub device_delegation_hex: String,
    /// Hex-encoded old-root-signed rotation container.
    pub rotation_hex: String,
    /// Hex-encoded new-root-signed confirmation container.
    pub confirmation_hex: String,
}

/// Build both rotation containers, the succession chain, and a fresh
/// device link. The old root signs the rotation (account authority); the
/// new root signs the confirmation (proof the new DID is controllable, so
/// a typo cannot strand the account on an inert root).
pub async fn rotate_account(
    old_root: Ed25519Signer,
    new_root: Ed25519Signer,
    new_credential_id: String,
    device_did: dialog_varsig::Did,
) -> Result<RotationCeremony> {
    let old_root_did = old_root.did().to_string();
    let new_root_did = new_root.did().to_string();

    let succession =
        crate::delegation::mint_root_succession(old_root.clone(), &new_root.did()).await?;
    let succession_hex = hex::encode(
        succession
            .to_bytes()
            .context("failed to serialize the succession delegation")?,
    );
    let device_link =
        crate::delegation::mint_device_delegation(new_root.clone(), &device_did).await?;
    let device_delegation_hex = hex::encode(
        device_link
            .to_bytes()
            .context("failed to serialize the device delegation")?,
    );

    let rotation = build(
        old_root,
        vec!["account".into(), "rotate".into()],
        strings([
            ("newRootDid", new_root_did.clone()),
            ("newCredentialId", new_credential_id),
            ("succession", succession_hex.clone()),
            ("deviceDid", device_did.to_string()),
            ("deviceDelegation", device_delegation_hex.clone()),
        ]),
        device_did.to_string(),
        device_delegation_hex.clone(),
    )
    .await?;
    let confirmation = build(
        new_root,
        vec!["account".into(), "rotate".into(), "confirm".into()],
        strings([("oldRootDid", old_root_did.clone())]),
        device_did.to_string(),
        device_delegation_hex.clone(),
    )
    .await?;

    Ok(RotationCeremony {
        old_root_did,
        new_root_did,
        succession_hex,
        device_delegation_hex,
        rotation_hex: rotation.invocation_hex,
        confirmation_hex: confirmation.invocation_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::InvocationChain;
    use dialog_ucan_core::promise::Promised;

    async fn fixture() -> (Ed25519Signer, dialog_varsig::Did) {
        let root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        (root, device.did())
    }

    #[dialog_common::test]
    async fn it_binds_account_creation_fields_to_the_root_signature() {
        let (root, device) = fixture().await;
        let expected_root = root.did();
        let output = create_account(
            root,
            "a@x.com".into(),
            "123456".into(),
            "credential".into(),
            device.clone(),
            "laptop".into(),
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

    #[dialog_common::test]
    async fn it_builds_a_two_container_rotation() {
        let old_root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let new_root = crate::derive::derive_root_signer(&[9u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let old_did = old_root.did().to_string();
        let new_did = new_root.did().to_string();

        let ceremony = rotate_account(old_root, new_root, "cred-new".into(), device.did())
            .await
            .unwrap();
        assert_eq!(ceremony.old_root_did, old_did);
        assert_eq!(ceremony.new_root_did, new_did);

        let rotation =
            InvocationChain::try_from(hex::decode(&ceremony.rotation_hex).unwrap().as_slice())
                .unwrap();
        rotation
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(rotation.issuer().to_string(), old_did);
        assert_eq!(
            rotation.command().0,
            vec!["account".to_string(), "rotate".to_string()]
        );
        assert_eq!(
            rotation.arguments().get("newRootDid"),
            Some(&Promised::String(new_did.clone()))
        );

        let confirmation =
            InvocationChain::try_from(hex::decode(&ceremony.confirmation_hex).unwrap().as_slice())
                .unwrap();
        confirmation
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(confirmation.issuer().to_string(), new_did);
        assert_eq!(
            confirmation.command().0,
            vec![
                "account".to_string(),
                "rotate".to_string(),
                "confirm".to_string()
            ]
        );
        assert_eq!(
            confirmation.arguments().get("oldRootDid"),
            Some(&Promised::String(old_did))
        );
    }
}

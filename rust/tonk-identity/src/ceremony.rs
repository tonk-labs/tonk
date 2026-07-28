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
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn root_ceremony(
    root: Ed25519Signer,
    credential_id: String,
    device_did: dialog_varsig::Did,
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
    })
}

/// Create a provider-neutral passkey root and delegate it to `device_did`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn create_root(device_did: dialog_varsig::Did) -> Result<RootCeremony> {
    let created = crate::passkey::create_passkey().await?;
    let credential_id = hex::encode(created.id);
    let prf = match created.prf_output {
        Some(output) => output,
        None => crate::passkey::prf_output().await?,
    };
    let root = crate::derive::derive_root_signer(&prf).await?;
    root_ceremony(root, credential_id, device_did).await
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
    root_ceremony(root, credential_id, device_did).await
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
pub async fn create_account(
    root: Ed25519Signer,
    email: String,
    code: String,
    credential_id: String,
    device_did: dialog_varsig::Did,
    device_name: String,
    delegation_hex: String,
) -> Result<AccountCeremony> {
    let device_did_string = device_did.to_string();
    let bytes = hex::decode(&delegation_hex).context("invalid existing delegation hex")?;
    let delegation = DelegationChain::try_from(bytes.as_slice())
        .context("invalid existing root to device delegation")?;
    if delegation.issuer() != &root.did() || delegation.audience() != &device_did {
        anyhow::bail!("existing delegation does not match the evaluated root and device");
    }
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
}

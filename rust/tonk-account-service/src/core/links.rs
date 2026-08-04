//! One-time browser handoffs for linking native CLI profiles.

use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::store::{ActivateOutcome, LinkRequest, Store};
use tonk_account::handoff::{ConsumedLink, ResolvedLink};

/// Handoffs are deliberately short-lived bearer capabilities.
pub const LINK_TTL_SECONDS: u64 = 5 * 60;

/// Hash a raw 32-byte handoff secret for storage and lookup.
pub fn hash_secret(secret: &str) -> Result<String, CeremonyError> {
    let bytes = hex::decode(secret)
        .map_err(|_| CeremonyError::Unauthorized("invalid link secret".to_string()))?;
    if bytes.len() != 32 {
        return Err(CeremonyError::Unauthorized(
            "invalid link secret".to_string(),
        ));
    }
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn validate_hash(token_hash: &str) -> Result<(), CeremonyError> {
    let bytes = hex::decode(token_hash)
        .map_err(|_| CeremonyError::Invalid("tokenHash must be hex".to_string()))?;
    if bytes.len() != 32 {
        return Err(CeremonyError::Invalid(
            "tokenHash must encode 32 bytes".to_string(),
        ));
    }
    Ok(())
}

fn validate_device(device_did: &str, device_name: &str) -> Result<(), CeremonyError> {
    device_did
        .parse::<dialog_varsig::Did>()
        .map_err(|error| CeremonyError::Invalid(format!("invalid deviceDid: {error}")))?;
    if device_name.trim().is_empty() || device_name.len() > 100 {
        return Err(CeremonyError::Invalid(
            "deviceName must contain 1 to 100 bytes".to_string(),
        ));
    }
    Ok(())
}

/// Create a pending request from a client-computed secret hash.
pub async fn create_link<S: Store>(
    store: &S,
    token_hash: &str,
    device_did: &str,
    device_name: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    validate_hash(token_hash)?;
    validate_device(device_did, device_name)?;
    let candidate = LinkRequest {
        token_hash: token_hash.to_string(),
        device_did: device_did.to_string(),
        device_name: device_name.trim().to_string(),
        account_id: None,
        attachment_id: None,
        delegation_cid: None,
        delegation_hex: None,
        descriptor_hex: None,
        created_at: now,
        expires_at: now + LINK_TTL_SECONDS,
        completed_at: None,
        consumed_at: None,
        activated_at: None,
        cancelled_at: None,
    };
    match store.put_link(&candidate).await {
        Ok(()) => Ok(()),
        Err(crate::store::StoreError::Conflict(_)) => {
            let existing = store.link(token_hash).await?;
            if existing.as_ref().is_some_and(|link| {
                link.device_did == candidate.device_did
                    && link.device_name == candidate.device_name
                    && link.expires_at >= now
                    && link.cancelled_at.is_none()
            }) {
                Ok(())
            } else {
                Err(CeremonyError::Conflict(
                    "link token already belongs to another request".to_string(),
                ))
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Resolve live device metadata using the raw bearer secret.
pub async fn resolve_link<S: Store>(
    store: &S,
    secret: &str,
    now: u64,
) -> Result<ResolvedLink, CeremonyError> {
    let token_hash = hash_secret(secret)?;
    let link = store
        .link(&token_hash)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown link request".to_string()))?;
    if link.expires_at < now
        || link.cancelled_at.is_some()
        || link.delegation_hex.is_some()
        || link.descriptor_hex.is_some()
    {
        return Err(CeremonyError::Unauthorized(
            "link request is no longer pending".to_string(),
        ));
    }
    Ok(ResolvedLink {
        token_hash,
        device_did: link.device_did,
        device_name: link.device_name,
    })
}

/// Complete a pending handoff from a root-signed browser ceremony.
pub async fn complete_link<S: Store>(
    store: &S,
    root_did: &str,
    token_hash: &str,
    device_did: &str,
    device_name: &str,
    delegation_hex: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    validate_hash(token_hash)?;
    let link = store
        .link(token_hash)
        .await?
        .ok_or_else(|| CeremonyError::Invalid("unknown link request".to_string()))?;
    if link.cancelled_at.is_some() {
        return Err(CeremonyError::Conflict(
            "link request was cancelled".to_string(),
        ));
    }
    if let (Some(stored_account), Some(stored_cid), Some(stored_hex)) = (
        link.account_id,
        link.delegation_cid.as_deref(),
        link.delegation_hex.as_deref(),
    ) {
        let account = store
            .account_by_root(root_did)
            .await?
            .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;
        let cid = check_device_delegation(delegation_hex, root_did, device_did).await?;
        if stored_account == account.id && stored_cid == cid && stored_hex == delegation_hex {
            return Ok(());
        }
        return Err(CeremonyError::Conflict(
            "link request was completed with different grant material".to_string(),
        ));
    }
    if link.expires_at < now {
        return Err(CeremonyError::Conflict(
            "link request is no longer pending".to_string(),
        ));
    }
    if link.device_did != device_did || link.device_name != device_name {
        return Err(CeremonyError::Invalid(
            "completion does not match the pending device".to_string(),
        ));
    }
    let account = store
        .account_by_root(root_did)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;
    let descriptor = account.repository_descriptor.as_ref().ok_or_else(|| {
        CeremonyError::Conflict(tonk_account::UNESTABLISHED_ACCOUNT_CONFLICT.to_string())
    })?;
    let descriptor_hex = hex::encode(descriptor);
    let delegation_cid = check_device_delegation(delegation_hex, root_did, device_did).await?;
    let attachment_id = crate::core::devices::random_attachment_id();
    let completed = store
        .complete_link(
            token_hash,
            account.id,
            &attachment_id,
            &delegation_cid,
            delegation_hex,
            &descriptor_hex,
            now,
        )
        .await?;
    if !completed {
        let replay = store.link(token_hash).await?;
        if replay.as_ref().is_some_and(|stored| {
            stored.account_id == Some(account.id)
                && stored.delegation_cid.as_deref() == Some(delegation_cid.as_str())
                && stored.delegation_hex.as_deref() == Some(delegation_hex)
                && stored.descriptor_hex.as_deref() == Some(descriptor_hex.as_str())
                && stored.cancelled_at.is_none()
        }) {
            return Ok(());
        }
        return Err(CeremonyError::Conflict(
            "link request is no longer pending".to_string(),
        ));
    }
    Ok(())
}

/// Consume a completed delegation and descriptor once. `None` means the CLI
/// should poll again.
pub async fn consume_link<S: Store>(
    store: &S,
    secret: &str,
    now: u64,
) -> Result<Option<ConsumedLink>, CeremonyError> {
    let token_hash = hash_secret(secret)?;
    let link = store
        .link(&token_hash)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown link request".to_string()))?;
    if link.expires_at < now || link.cancelled_at.is_some() || link.activated_at.is_some() {
        return Err(CeremonyError::Unauthorized(
            "link request has expired, was cancelled, or was activated".to_string(),
        ));
    }
    if link.delegation_hex.is_none() || link.descriptor_hex.is_none() {
        return Ok(None);
    }
    let consumed = store
        .consume_link(&token_hash, now)
        .await?
        .ok_or_else(|| CeremonyError::Conflict("completed link is unavailable".to_string()))?;
    let delegation_hex = consumed.delegation_hex.ok_or_else(|| {
        CeremonyError::Internal("completed link delegation is missing".to_string())
    })?;
    let descriptor_hex = consumed.descriptor_hex.ok_or_else(|| {
        CeremonyError::Internal("completed link descriptor is missing".to_string())
    })?;
    let attachment_id = consumed.attachment_id.ok_or_else(|| {
        CeremonyError::Internal("completed link attachment is missing".to_string())
    })?;
    let bytes = hex::decode(&delegation_hex).map_err(|error| {
        CeremonyError::Internal(format!("stored link delegation is invalid: {error}"))
    })?;
    let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        CeremonyError::Internal(format!("stored link delegation is invalid: {error}"))
    })?;
    let account = store
        .account_by_root(chain.issuer().as_ref())
        .await?
        .ok_or_else(|| CeremonyError::Internal("completed link account is missing".to_string()))?;
    Ok(Some(ConsumedLink {
        attachment_id,
        delegation_hex,
        credential_id: account.credential_id,
        descriptor_hex,
    }))
}

/// Activate the exact completed handoff after its returned grant has been
/// durably recorded by the native client.
pub async fn activate_link<S: Store>(
    store: &S,
    token_hash: &str,
    attachment_id: &str,
    root_did: &str,
    device_did: &str,
    delegation_cid: &str,
    now: u64,
) -> Result<crate::store::Device, CeremonyError> {
    let link = store
        .link(token_hash)
        .await?
        .ok_or_else(|| CeremonyError::Invalid("unknown completed link".to_string()))?;
    let account = link
        .account_id
        .and_then(|id| Some(id))
        .ok_or_else(|| CeremonyError::Conflict("link is not complete".to_string()))?;
    let stored_account = store
        .account_by_root(root_did)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;
    if account != stored_account.id
        || link.device_did != device_did
        || link.attachment_id.as_deref() != Some(attachment_id)
        || link.delegation_cid.as_deref() != Some(delegation_cid)
    {
        return Err(CeremonyError::Forbidden(
            "activation does not match the completed handoff".to_string(),
        ));
    }
    match store
        .activate_completed_link(token_hash, attachment_id, now)
        .await?
    {
        ActivateOutcome::Active(device) => Ok(device),
        ActivateOutcome::ActiveDeviceConflict => Err(CeremonyError::Conflict(
            "this device is already attached; log out and retry detachment first".to_string(),
        )),
        ActivateOutcome::RevokedDelegation => Err(CeremonyError::Conflict(
            "this delegation was revoked and can never be reactivated".to_string(),
        )),
        ActivateOutcome::Cancelled => Err(CeremonyError::Conflict(
            "this completed handoff was cancelled".to_string(),
        )),
        ActivateOutcome::Unknown => Err(CeremonyError::Invalid(
            "unknown completed attachment".to_string(),
        )),
    }
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use dialog_varsig::Principal;

    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus, Store};

    const SECRET: &str = "0707070707070707070707070707070707070707070707070707070707070707";

    async fn fixture() -> (SqliteStore, String, String, String, String) {
        let store = SqliteStore::in_memory().unwrap();
        let root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&[8u8; 32])
            .await
            .unwrap();
        let root_did = root.did().to_string();
        let device_did = device.did().to_string();
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(
            &root,
            "https://accounts.example/ucan/",
        )
        .await
        .unwrap();
        let descriptor_hex = hex::encode(descriptor.bytes());
        let account_id = store
            .create_account("a@x.com", &root_did, "cred", 1)
            .await
            .unwrap();
        store
            .establish_repository_descriptor(account_id, descriptor.bytes())
            .await
            .unwrap();
        let delegation = tonk_identity::delegation::mint_device_delegation(root, &device.did())
            .await
            .unwrap();
        (
            store,
            root_did,
            device_did,
            hex::encode(delegation.to_bytes().unwrap()),
            descriptor_hex,
        )
    }

    #[dialog_common::test]
    async fn it_completes_and_consumes_a_link_once() {
        let (store, root, device, delegation, descriptor) = fixture().await;
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device, "terminal", 100)
            .await
            .unwrap();
        assert_eq!(
            resolve_link(&store, SECRET, 101).await.unwrap().device_did,
            device
        );
        complete_link(&store, &root, &hash, &device, "terminal", &delegation, 102)
            .await
            .unwrap();
        let consumed = consume_link(&store, SECRET, 103).await.unwrap().unwrap();
        assert_eq!(consumed.delegation_hex, delegation);
        assert_eq!(consumed.credential_id, "cred");
        assert_eq!(consumed.descriptor_hex, descriptor);
        assert_eq!(consumed.attachment_id.len(), 64);
        assert_eq!(
            consume_link(&store, SECRET, 104).await.unwrap(),
            Some(consumed)
        );
    }

    /// Logging out leaves the CLI device registered to its old account. A
    /// later handoff must still be able to register that same durable device
    /// identity with another account; registrations are account-scoped.
    #[dialog_common::test]
    async fn it_links_a_device_that_is_registered_to_another_account() {
        let store = SqliteStore::in_memory().unwrap();
        let old_root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&[8u8; 32])
            .await
            .unwrap();
        let old_root_did = old_root.did().to_string();
        let new_root_did = new_root.did().to_string();
        let device_did = device.did().to_string();

        let old_account = store
            .create_account("old@example.com", &old_root_did, "old-cred", 1)
            .await
            .unwrap();
        let old_descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(
            &old_root,
            "https://old.example/ucan/",
        )
        .await
        .unwrap();
        store
            .establish_repository_descriptor(old_account, old_descriptor.bytes())
            .await
            .unwrap();
        let old_delegation =
            tonk_identity::delegation::mint_device_delegation(old_root, &device.did())
                .await
                .unwrap();
        store
            .insert_device(&Device {
                id: 0,
                account_id: old_account,
                device_did: device_did.clone(),
                attachment_id: "01".repeat(32),
                delegation_cid: old_delegation.proof_cids()[0].to_string(),
                delegation_hex: hex::encode(old_delegation.to_bytes().unwrap()),
                name: "old terminal".to_string(),
                status: DeviceStatus::Active,
                created_at: 2,
            })
            .await
            .unwrap();

        let new_account = store
            .create_account("new@example.com", &new_root_did, "new-cred", 3)
            .await
            .unwrap();
        let new_descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(
            &new_root,
            "https://new.example/ucan/",
        )
        .await
        .unwrap();
        store
            .establish_repository_descriptor(new_account, new_descriptor.bytes())
            .await
            .unwrap();
        let new_delegation =
            tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
                .await
                .unwrap();
        let new_delegation_hex = hex::encode(new_delegation.to_bytes().unwrap());

        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device_did, "new terminal", 100)
            .await
            .unwrap();
        complete_link(
            &store,
            &new_root_did,
            &hash,
            &device_did,
            "new terminal",
            &new_delegation_hex,
            102,
        )
        .await
        .unwrap();

        let consumed = consume_link(&store, SECRET, 103).await.unwrap().unwrap();
        let cid = new_delegation.proof_cids()[0].to_string();
        assert!(matches!(
            activate_link(
                &store,
                &hash,
                &consumed.attachment_id,
                &new_root_did,
                &device_did,
                &cid,
                104,
            )
            .await,
            Err(CeremonyError::Conflict(_))
        ));
        store
            .detach_attachment(&"01".repeat(32), 105)
            .await
            .unwrap();
        let active = activate_link(
            &store,
            &hash,
            &consumed.attachment_id,
            &new_root_did,
            &device_did,
            &cid,
            106,
        )
        .await
        .unwrap();
        assert_eq!(active.account_id, new_account);
        assert_eq!(
            store.devices(old_account).await.unwrap()[0].status,
            DeviceStatus::Detached
        );
        assert_eq!(
            store.devices(new_account).await.unwrap()[0].delegation_hex,
            new_delegation_hex
        );
        store.revoke_device_by_cid(new_account, &cid).await.unwrap();
        assert!(matches!(
            activate_link(
                &store,
                &hash,
                &consumed.attachment_id,
                &new_root_did,
                &device_did,
                &cid,
                107,
            )
            .await,
            Err(CeremonyError::Conflict(message)) if message.contains("revoked")
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_substitution_and_rolls_back_the_device() {
        let (store, root, device, delegation, _) = fixture().await;
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device, "terminal", 100)
            .await
            .unwrap();
        assert!(
            complete_link(&store, &root, &hash, &device, "other", &delegation, 102)
                .await
                .is_err()
        );
        let account = store.account_by_root(&root).await.unwrap().unwrap();
        assert!(
            store
                .device_for_account(account.id, &device)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[dialog_common::test]
    async fn it_refuses_to_link_an_unestablished_account() {
        let store = SqliteStore::in_memory().unwrap();
        let root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&[8u8; 32])
            .await
            .unwrap();
        let root_did = root.did().to_string();
        let device_did = device.did().to_string();
        let account_id = store
            .create_account("old@x.com", &root_did, "cred", 1)
            .await
            .unwrap();
        let delegation = tonk_identity::delegation::mint_device_delegation(root, &device.did())
            .await
            .unwrap();
        let delegation = hex::encode(delegation.to_bytes().unwrap());
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device_did, "terminal", 100)
            .await
            .unwrap();

        assert!(matches!(
            complete_link(
                &store,
                &root_did,
                &hash,
                &device_did,
                "terminal",
                &delegation,
                102,
            )
            .await,
            Err(CeremonyError::Conflict(_))
        ));
        assert!(
            store
                .device_for_account(account_id, &device_did)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[dialog_common::test]
    async fn it_expires_without_registering_the_device() {
        let (store, root, device, delegation, _) = fixture().await;
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device, "terminal", 100)
            .await
            .unwrap();
        assert!(
            complete_link(&store, &root, &hash, &device, "terminal", &delegation, 401)
                .await
                .is_err()
        );
        let account = store.account_by_root(&root).await.unwrap().unwrap();
        assert!(
            store
                .device_for_account(account.id, &device)
                .await
                .unwrap()
                .is_none()
        );
    }
}

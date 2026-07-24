//! One-time browser handoffs for linking native CLI profiles.

use crate::core::CeremonyError;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::{Device, DeviceStatus, LinkRequest, Store};

/// Handoffs are deliberately short-lived bearer capabilities.
pub const LINK_TTL_SECONDS: u64 = 5 * 60;

/// Public device metadata returned to the browser after bearer resolution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLink {
    /// Hash bound into the root-signed completion invocation.
    pub token_hash: String,
    /// CLI profile DID receiving the delegation.
    pub device_did: String,
    /// Human-readable device name shown before confirmation.
    pub device_name: String,
}

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
    store
        .put_link(&LinkRequest {
            token_hash: token_hash.to_string(),
            device_did: device_did.to_string(),
            device_name: device_name.trim().to_string(),
            delegation_hex: None,
            created_at: now,
            expires_at: now + LINK_TTL_SECONDS,
            consumed_at: None,
        })
        .await?;
    Ok(())
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
    if link.expires_at < now || link.consumed_at.is_some() || link.delegation_hex.is_some() {
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
    if link.expires_at < now || link.consumed_at.is_some() || link.delegation_hex.is_some() {
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
    let delegation_cid =
        check_subject_open_delegation(delegation_hex, root_did, device_did).await?;
    let completed = store
        .complete_link(
            token_hash,
            &Device {
                account_id: account.id,
                device_did: device_did.to_string(),
                delegation_cid,
                name: device_name.to_string(),
                status: DeviceStatus::Active,
                created_at: now,
            },
            delegation_hex,
            now,
        )
        .await?;
    if !completed {
        return Err(CeremonyError::Conflict(
            "link request is no longer pending".to_string(),
        ));
    }
    Ok(())
}

/// Consume a completed delegation once. `None` means the CLI should poll again.
pub async fn consume_link<S: Store>(
    store: &S,
    secret: &str,
    now: u64,
) -> Result<Option<String>, CeremonyError> {
    let token_hash = hash_secret(secret)?;
    let link = store
        .link(&token_hash)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown link request".to_string()))?;
    if link.expires_at < now || link.consumed_at.is_some() {
        return Err(CeremonyError::Unauthorized(
            "link request has expired or was consumed".to_string(),
        ));
    }
    let completed = link.delegation_hex.is_some();
    let consumed = store.consume_link(&token_hash, now).await?;
    if completed && consumed.is_none() {
        return Err(CeremonyError::Conflict(
            "link request was already consumed".to_string(),
        ));
    }
    Ok(consumed)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use dialog_varsig::Principal;

    use super::*;
    use crate::store::Store;
    use crate::store::sqlite::SqliteStore;

    const SECRET: &str = "0707070707070707070707070707070707070707070707070707070707070707";

    async fn fixture() -> (SqliteStore, String, String, String) {
        let store = SqliteStore::in_memory().unwrap();
        let root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&[8u8; 32])
            .await
            .unwrap();
        let root_did = root.did().to_string();
        let device_did = device.did().to_string();
        store
            .create_account("a@x.com", &root_did, "cred", 1)
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
        )
    }

    #[dialog_common::test]
    async fn it_completes_and_consumes_a_link_once() {
        let (store, root, device, delegation) = fixture().await;
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
        assert_eq!(
            consume_link(&store, SECRET, 103).await.unwrap(),
            Some(delegation)
        );
        assert!(consume_link(&store, SECRET, 104).await.is_err());
    }

    #[dialog_common::test]
    async fn it_rejects_substitution_and_rolls_back_the_device() {
        let (store, root, device, delegation) = fixture().await;
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device, "terminal", 100)
            .await
            .unwrap();
        assert!(
            complete_link(&store, &root, &hash, &device, "other", &delegation, 102)
                .await
                .is_err()
        );
        assert!(store.device_by_did(&device).await.unwrap().is_none());
    }

    #[dialog_common::test]
    async fn it_expires_without_registering_the_device() {
        let (store, root, device, delegation) = fixture().await;
        let hash = hash_secret(SECRET).unwrap();
        create_link(&store, &hash, &device, "terminal", 100)
            .await
            .unwrap();
        assert!(
            complete_link(&store, &root, &hash, &device, "terminal", &delegation, 401)
                .await
                .is_err()
        );
        assert!(store.device_by_did(&device).await.unwrap().is_none());
    }
}

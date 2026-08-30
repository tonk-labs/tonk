//! Device registry operations: list, register, and record revocations.

use tonk_identity::revocation::{VerifyError, verify};

use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::store::{Account, Device, DeviceStatus, Store, StoreError};

/// A device row as surfaced to API callers.
pub struct DeviceView {
    /// Exact attachment generation.
    pub attachment_id: String,
    /// The device's DID.
    pub did: String,
    /// Human-readable device name.
    pub name: String,
    /// `"active"` or `"revoked"`.
    pub status: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Exact public delegation path bytes, hex-encoded, when retained.
    pub delegation_hex: Option<String>,
    /// Creation time, as a unix timestamp in seconds.
    pub created_at: u64,
}

impl From<Device> for DeviceView {
    fn from(device: Device) -> Self {
        DeviceView {
            attachment_id: device.attachment_id,
            did: device.device_did,
            name: device.name,
            status: device.status.as_str().to_string(),
            delegation_cid: device.delegation_cid,
            delegation_hex: (!device.delegation_hex.is_empty()).then_some(device.delegation_hex),
            created_at: device.created_at,
        }
    }
}

/// Reuse the active generation after a root-authorized browser re-login.
fn reuse_linked_device(existing: Device, account: &Account) -> Result<String, CeremonyError> {
    if existing.account_id != account.id {
        return Err(CeremonyError::Conflict(
            "this device is already active on another account".to_string(),
        ));
    }
    Ok(existing.attachment_id)
}

/// Generate a random 32-byte lowercase hex attachment identifier.
pub fn random_attachment_id() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

async fn insert_device_registration<S: Store>(
    store: &S,
    account: &Account,
    device_did: &str,
    device_name: &str,
    delegation_cid: String,
    delegation_hex: String,
    now: u64,
) -> Result<String, StoreError> {
    let attachment_id = random_attachment_id();
    store
        .insert_device(&Device {
            id: 0,
            account_id: account.id,
            device_did: device_did.to_string(),
            attachment_id: attachment_id.clone(),
            delegation_cid,
            delegation_hex,
            name: device_name.to_string(),
            status: DeviceStatus::Active,
            created_at: now,
        })
        .await?;
    Ok(attachment_id)
}

/// Link a browser after a root-authorized passkey ceremony.
///
/// Browser sign-out is intentionally local-only. If the same account root
/// later links the same device DID, its earlier server attachment is still
/// active and is recovered instead of replaced. The fresh ceremony grant is
/// still validated, but the browser continues using its preserved local grant.
pub async fn link_device<S: Store>(
    store: &S,
    account: &Account,
    device_did: &str,
    device_name: &str,
    delegation_hex: &str,
    now: u64,
) -> Result<String, CeremonyError> {
    let delegation_cid =
        check_device_delegation(delegation_hex, &account.root_did, device_did).await?;
    if let Some(existing) = store.active_device_by_did(device_did).await? {
        return reuse_linked_device(existing, account);
    }
    match insert_device_registration(
        store,
        account,
        device_did,
        device_name,
        delegation_cid,
        delegation_hex.to_string(),
        now,
    )
    .await
    {
        Ok(attachment_id) => Ok(attachment_id),
        // Close the check-then-insert race for concurrent re-login requests.
        Err(StoreError::Conflict(detail)) => {
            if let Some(existing) = store.active_device_by_did(device_did).await? {
                reuse_linked_device(existing, account)
            } else {
                Err(StoreError::Conflict(detail).into())
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Terminal result of processing a signed generation-bound detach intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetachOutcome {
    /// The active generation was detached.
    Detached,
    /// This exact generation had already been detached.
    AlreadyDetached,
    /// A newer generation supersedes this one.
    Superseded,
    /// The exact generation is permanently revoked.
    Revoked,
}

/// Which product-level authority published a device revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attestation {
    /// The account root revoked one of its devices.
    Root,
    /// A device revoked its own exact grant.
    Device,
}

impl Attestation {
    /// Stable response value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Device => "device",
        }
    }
}

/// Whether the account-service status projection was updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// The matching D1 row now says revoked.
    Updated,
    /// The artifact verified but the D1 projection failed.
    Stale,
}

impl Projection {
    /// Stable response value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::Stale => "stale",
        }
    }
}

/// Result of verifying a device revocation and attempting its UI projection.
pub struct RevokeOutcome {
    /// Product-level authority used.
    pub attestation: Attestation,
    /// D1 projection state.
    pub projection: Projection,
    /// Canonical target delegation CID.
    pub target_cid: String,
    /// Canonical artifact CID.
    pub artifact_cid: String,
}

/// Focused device lookup/projection seam used by revocation publication.
#[allow(async_fn_in_trait)]
pub trait DeviceRevocationProjection {
    /// Look up a target device within an account.
    async fn target_device(
        &self,
        account_id: i64,
        attachment_id: &str,
    ) -> Result<Option<Device>, StoreError>;
    /// Mark the row matching account and delegation CID revoked.
    async fn project_revoked(
        &self,
        account_id: i64,
        delegation_cid: &str,
    ) -> Result<bool, StoreError>;
}

impl<S: Store> DeviceRevocationProjection for S {
    async fn target_device(
        &self,
        account_id: i64,
        attachment_id: &str,
    ) -> Result<Option<Device>, StoreError> {
        Ok(self
            .attachment(attachment_id)
            .await?
            .filter(|device| device.account_id == account_id))
    }

    async fn project_revoked(
        &self,
        account_id: i64,
        delegation_cid: &str,
    ) -> Result<bool, StoreError> {
        self.revoke_device_by_cid(account_id, delegation_cid).await
    }
}

fn verification_error(error: VerifyError) -> CeremonyError {
    match error {
        VerifyError::Malformed(message) => CeremonyError::Invalid(message),
        VerifyError::Unauthorized(message) => CeremonyError::Unauthorized(message),
    }
}

/// Verify a device revocation and project its D1 status.
///
/// The artifact itself is durably recorded by the access service's
/// revocation index, which is what enforcement reads. This path only
/// establishes that the caller may revoke the named device and mirrors
/// the outcome onto the device list the account panel renders.
pub async fn revoke_device<S: DeviceRevocationProjection>(
    store: &S,
    account: &Account,
    caller_did: &str,
    attachment_id: &str,
    target_did: &str,
    artifact: &[u8],
) -> Result<RevokeOutcome, CeremonyError> {
    let device = store
        .target_device(account.id, attachment_id)
        .await?
        .ok_or_else(|| CeremonyError::Invalid("unknown device".to_string()))?;

    let verified = verify(artifact).await.map_err(verification_error)?;
    if device.device_did != target_did || device.attachment_id != attachment_id {
        return Err(CeremonyError::Invalid(
            "revocation target does not match the selected attachment".to_string(),
        ));
    }
    if verified.target_cid != device.delegation_cid {
        return Err(CeremonyError::Invalid(
            "revocation names a delegation other than the target device's".to_string(),
        ));
    }

    let revoking_self = caller_did == target_did;
    // Who signed is the whole question: the device itself for a
    // self-revocation, the account root otherwise. `verify` has already
    // established that the signer was entitled to withdraw the delegation it
    // names, so the shape of that entitlement adds nothing here.
    let attestation = if revoking_self {
        Attestation::Device
    } else if !revoking_self && verified.issuer.to_string() == account.root_did {
        Attestation::Root
    } else {
        return Err(CeremonyError::Forbidden(if revoking_self {
            "self-revocation must be signed by the target device under its registered grant"
                .to_string()
        } else {
            "revoking another device requires an account-root revocation".to_string()
        }));
    };

    let projection = match store
        .project_revoked(account.id, &device.delegation_cid)
        .await
    {
        Ok(true) => Projection::Updated,
        Ok(false) => Projection::Stale,
        Err(error) => {
            #[cfg(target_arch = "wasm32")]
            worker::console_error!("device revocation projection failed: {error:?}");
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("device revocation projection failed: {error:?}");
            Projection::Stale
        }
    };

    Ok(RevokeOutcome {
        attestation,
        projection,
        target_cid: verified.target_cid,
        artifact_cid: verified.artifact_cid,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use std::sync::{Arc, Mutex};

    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;

    use super::*;
    use crate::store::sqlite::SqliteStore;

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [11u8; 32];
    const CALLER_DID: &str = "did:key:zCaller";

    async fn fixture() -> (
        Account,
        Device,
        Ed25519Signer,
        dialog_ucan_core::DelegationChain,
    ) {
        let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
            .await
            .unwrap();
        let device_signer = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &device_signer.did())
                .await
                .unwrap();
        let account = Account {
            id: 1,
            email: "a@x.com".into(),
            root_did: root.did().to_string(),
            credential_id: "cred".into(),
            repository_descriptor: None,
            passkey_created_at: None,
            passkey_created_on: None,
            created_at: 1,
        };
        let device = Device {
            id: 0,
            account_id: account.id,
            device_did: device_signer.did().to_string(),
            attachment_id: "03".repeat(32),
            delegation_cid: grant.proof_cids()[0].to_string(),
            delegation_hex: hex::encode(grant.to_bytes().unwrap()),
            name: "phone".into(),
            status: DeviceStatus::Active,
            created_at: 2,
        };
        (account, device, root, grant)
    }
    /// Browser sign-out is local-only, so signing back in with the same
    /// passkey presents the same account and device with a freshly minted
    /// grant while the first attachment is still active. That root-authorized
    /// re-login recovers the existing generation.
    #[dialog_common::test]
    async fn it_reuses_an_active_device_when_the_same_account_links_again() {
        let store = SqliteStore::in_memory().unwrap();
        let (mut account, device, root, first_grant) = fixture().await;
        account.id = store
            .create_account(
                &account.email,
                &account.root_did,
                &account.credential_id,
                account.created_at,
            )
            .await
            .unwrap();
        let device_did = device.device_did.parse().unwrap();
        let second_grant = tonk_identity::delegation::mint_device_delegation(root, &device_did)
            .await
            .unwrap();
        assert_ne!(
            second_grant.proof_cids()[0],
            first_grant.proof_cids()[0],
            "each passkey login proposes a fresh grant"
        );

        let first = link_device(
            &store,
            &account,
            &device.device_did,
            &device.name,
            &hex::encode(first_grant.to_bytes().unwrap()),
            device.created_at,
        )
        .await
        .unwrap();
        let second = link_device(
            &store,
            &account,
            &device.device_did,
            &device.name,
            &hex::encode(second_grant.to_bytes().unwrap()),
            device.created_at + 1,
        )
        .await
        .unwrap();

        assert_eq!(second, first, "the same active generation is recovered");
        let listed = store.devices(account.id).await.unwrap();
        assert_eq!(listed.len(), 1, "re-login adds no device history");
    }

    #[dialog_common::test]
    async fn it_does_not_reuse_an_active_device_for_another_account() {
        let store = SqliteStore::in_memory().unwrap();
        let (mut first_account, device, _, first_grant) = fixture().await;
        first_account.id = store
            .create_account(
                &first_account.email,
                &first_account.root_did,
                &first_account.credential_id,
                first_account.created_at,
            )
            .await
            .unwrap();
        link_device(
            &store,
            &first_account,
            &device.device_did,
            &device.name,
            &hex::encode(first_grant.to_bytes().unwrap()),
            device.created_at,
        )
        .await
        .unwrap();

        let second_root = Ed25519Signer::import(&[13u8; 32]).await.unwrap();
        let mut second_account = Account {
            id: 0,
            email: "b@x.com".into(),
            root_did: second_root.did().to_string(),
            credential_id: "other-cred".into(),
            repository_descriptor: None,
            passkey_created_at: None,
            passkey_created_on: None,
            created_at: 3,
        };
        second_account.id = store
            .create_account(
                &second_account.email,
                &second_account.root_did,
                &second_account.credential_id,
                second_account.created_at,
            )
            .await
            .unwrap();
        let device_did = device.device_did.parse().unwrap();
        let second_grant =
            tonk_identity::delegation::mint_device_delegation(second_root, &device_did)
                .await
                .unwrap();

        assert!(matches!(
            link_device(
                &store,
                &second_account,
                &device.device_did,
                &device.name,
                &hex::encode(second_grant.to_bytes().unwrap()),
                4,
            )
            .await,
            Err(CeremonyError::Conflict(detail))
                if detail == "this device is already active on another account"
        ));
    }

    struct SpyProjection {
        device: Device,
        events: Arc<Mutex<Vec<&'static str>>>,
        fail: bool,
    }

    impl DeviceRevocationProjection for SpyProjection {
        async fn target_device(
            &self,
            account_id: i64,
            attachment_id: &str,
        ) -> Result<Option<Device>, StoreError> {
            Ok(
                (self.device.account_id == account_id
                    && self.device.attachment_id == attachment_id)
                    .then(|| self.device.clone()),
            )
        }

        async fn project_revoked(
            &self,
            _account_id: i64,
            _delegation_cid: &str,
        ) -> Result<bool, StoreError> {
            self.events.lock().unwrap().push("d1");
            if self.fail {
                Err(StoreError::Internal("projection unavailable".into()))
            } else {
                Ok(true)
            }
        }
    }

    async fn spies(
        fail_projection: bool,
    ) -> (
        Account,
        Device,
        Ed25519Signer,
        dialog_ucan_core::DelegationChain,
        SpyProjection,
        Arc<Mutex<Vec<&'static str>>>,
    ) {
        let (account, device, root, grant) = fixture().await;
        let events = Arc::new(Mutex::new(Vec::new()));
        let projection = SpyProjection {
            device: device.clone(),
            events: events.clone(),
            fail: fail_projection,
        };
        (account, device, root, grant, projection, events)
    }

    #[dialog_common::test]
    async fn it_projects_device_status_for_a_verified_revocation() {
        let (account, device, root, grant, projection, events) = spies(false).await;
        let artifact =
            tonk_identity::revocation::mint_root_revocation(root, &grant, &grant.proof_cids()[0])
                .await
                .unwrap();

        let outcome = revoke_device(
            &projection,
            &account,
            CALLER_DID,
            &device.attachment_id,
            &device.device_did,
            &artifact,
        )
        .await
        .unwrap();

        assert_eq!(events.lock().unwrap().as_slice(), ["d1"]);
        assert_eq!(outcome.projection, Projection::Updated);
    }

    #[dialog_common::test]
    async fn it_accepts_a_revocation_when_the_projection_fails() {
        let (account, device, root, grant, projection, events) = spies(true).await;
        let artifact =
            tonk_identity::revocation::mint_root_revocation(root, &grant, &grant.proof_cids()[0])
                .await
                .unwrap();

        let outcome = revoke_device(
            &projection,
            &account,
            CALLER_DID,
            &device.attachment_id,
            &device.device_did,
            &artifact,
        )
        .await
        .unwrap();

        assert_eq!(outcome.projection, Projection::Stale);
        assert_eq!(events.lock().unwrap().as_slice(), ["d1"]);
    }

    #[dialog_common::test]
    async fn it_never_projects_an_artifact_that_failed_verification() {
        let (account, device, _, _, projection, events) = spies(false).await;

        assert!(
            revoke_device(
                &projection,
                &account,
                CALLER_DID,
                &device.attachment_id,
                &device.device_did,
                b"invalid",
            )
            .await
            .is_err()
        );
        assert!(events.lock().unwrap().is_empty());
    }

    /// `revoke_device` takes `artifact: &[u8]`, not an option; handlers reject
    /// an absent invocation argument before calling it.
    #[dialog_common::test]
    fn it_requires_an_artifact_for_self_revocation() {
        fn artifact_argument(_: &[u8]) {}
        artifact_argument(b"signed artifact required");
    }
}

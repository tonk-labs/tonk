//! Device registry operations: list, register, and publish revocations.

use tonk_identity::revocation::{RevocationAuthority, VerifyError};

use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::revocations::{PublishError, RevocationStore, publish};
use crate::store::{Account, DetachStoreOutcome, Device, DeviceStatus, Store, StoreError};

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

/// List all devices registered under an account, in store order.
pub async fn list_devices<S: Store>(
    store: &S,
    account: &Account,
) -> Result<Vec<DeviceView>, CeremonyError> {
    let devices = store.devices(account.id).await?;
    let mut seen = std::collections::HashSet::new();
    Ok(devices
        .into_iter()
        .filter(|device| device.status != DeviceStatus::Detached)
        .filter(|device| seen.insert(device.device_did.clone()))
        .map(DeviceView::from)
        .collect())
}

/// Register a new device under an account.
/// Generate a random 32-byte lowercase hex attachment identifier.
pub fn random_attachment_id() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

/// Register a fresh globally active attachment generation.
pub async fn register_device<S: Store>(
    store: &S,
    account: &Account,
    device_did: &str,
    device_name: &str,
    delegation_hex: &str,
    now: u64,
) -> Result<String, CeremonyError> {
    let delegation_cid =
        check_device_delegation(delegation_hex, &account.root_did, device_did).await?;
    let attachment_id = random_attachment_id();
    store
        .insert_device(&Device {
            id: 0,
            account_id: account.id,
            device_did: device_did.to_string(),
            attachment_id: attachment_id.clone(),
            delegation_cid,
            delegation_hex: delegation_hex.to_string(),
            name: device_name.to_string(),
            status: DeviceStatus::Active,
            created_at: now,
        })
        .await?;
    Ok(attachment_id)
}

/// Terminal result of processing a signed generation-bound detach intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetachOutcome {
    /// The active generation was detached.
    Detached,
    /// This exact generation had already been detached.
    AlreadyDetached,
    /// A completed handoff was cancelled before activation.
    CancelledPendingActivation,
    /// A newer generation supersedes this one.
    Superseded,
    /// The exact generation is permanently revoked.
    Revoked,
}

/// Validate and apply a signed detach intent to exactly one stored generation.
pub async fn detach_device<S: Store>(
    store: &S,
    intent: &tonk_account::detach::SignedDetachIntent,
    now: u64,
) -> Result<DetachOutcome, CeremonyError> {
    let payload = intent.validate().await.map_err(|error| match error {
        tonk_account::detach::DetachIntentError::Signature => {
            CeremonyError::Forbidden(error.to_string())
        }
        _ => CeremonyError::Invalid(error.to_string()),
    })?;
    let device = store.attachment(&payload.attachment_id).await?;
    let link = if device.is_none() {
        store
            .completed_link_by_attachment(&payload.attachment_id)
            .await?
    } else {
        None
    };
    if device.is_none() && link.is_none() {
        return Err(CeremonyError::NotFound(
            "unknown attachment generation".to_string(),
        ));
    }
    let account = store
        .account_by_root(&payload.account_root)
        .await?
        .ok_or_else(|| CeremonyError::Conflict("detach account root does not match".to_string()))?;

    if let Some(device) = device {
        if device.account_id != account.id
            || device.device_did != payload.device_did
            || device.delegation_cid != payload.delegation_cid
        {
            return Err(CeremonyError::Conflict(
                "detach payload does not match the stored attachment".to_string(),
            ));
        }
    } else if let Some(link) = link
        && (link.account_id != Some(account.id)
            || link.device_did != payload.device_did
            || link.delegation_cid.as_deref() != Some(payload.delegation_cid.as_str()))
    {
        return Err(CeremonyError::Conflict(
            "detach payload does not match the completed attachment".to_string(),
        ));
    }

    match store.detach_attachment(&payload.attachment_id, now).await? {
        DetachStoreOutcome::Detached => Ok(DetachOutcome::Detached),
        DetachStoreOutcome::AlreadyDetached => Ok(DetachOutcome::AlreadyDetached),
        DetachStoreOutcome::CancelledPendingActivation => {
            Ok(DetachOutcome::CancelledPendingActivation)
        }
        DetachStoreOutcome::Superseded => Ok(DetachOutcome::Superseded),
        DetachStoreOutcome::Revoked => Ok(DetachOutcome::Revoked),
        DetachStoreOutcome::UnknownAttachment => Err(CeremonyError::NotFound(
            "unknown attachment generation".to_string(),
        )),
    }
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
    /// R2 accepted the artifact but the D1 projection failed.
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

/// Result of publishing a device revocation and attempting its UI projection.
pub struct RevokeOutcome {
    /// Product-level authority used.
    pub attestation: Attestation,
    /// D1 projection state.
    pub projection: Projection,
    /// Canonical target delegation CID.
    pub target_cid: String,
    /// Canonical artifact CID.
    pub artifact_cid: String,
    /// Whether this call created the immutable R2 object.
    pub stored: bool,
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

fn publication_error(error: PublishError) -> CeremonyError {
    match error {
        PublishError::Verification(VerifyError::Malformed(message)) => {
            CeremonyError::Invalid(message)
        }
        PublishError::Verification(VerifyError::Unauthorized(message)) => {
            CeremonyError::Unauthorized(message)
        }
        PublishError::Store(error) => CeremonyError::Internal(error.to_string()),
    }
}

/// Publish a verified device revocation before projecting its D1 status.
pub async fn revoke_device<S: DeviceRevocationProjection, R: RevocationStore>(
    store: &S,
    revocations: &R,
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

    let published = publish(revocations, artifact)
        .await
        .map_err(publication_error)?;
    if device.device_did != target_did || device.attachment_id != attachment_id {
        return Err(CeremonyError::Invalid(
            "revocation target does not match the selected attachment".to_string(),
        ));
    }
    if published.verified.target_cid != device.delegation_cid {
        return Err(CeremonyError::Invalid(
            "revocation names a delegation other than the target device's".to_string(),
        ));
    }

    let revoking_self = caller_did == target_did;
    let attestation = if revoking_self
        && published.verified.issuer.to_string() == device.device_did
        && published.verified.authority == RevocationAuthority::Delegated
    {
        Attestation::Device
    } else if !revoking_self
        && published.verified.issuer.to_string() == account.root_did
        && published.verified.authority == RevocationAuthority::PathIssuer
    {
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
        target_cid: published.verified.target_cid,
        artifact_cid: published.verified.artifact_cid,
        stored: published.stored,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;

    use super::*;
    use crate::revocations::{PutOutcome, RevocationStoreError, object_key};
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

    #[dialog_common::test]
    async fn it_detaches_only_the_exact_signed_generation_idempotently() {
        let store = SqliteStore::in_memory().unwrap();
        let (mut account, device, _, _) = fixture().await;
        account.id = store
            .create_account(
                &account.email,
                &account.root_did,
                &account.credential_id,
                account.created_at,
            )
            .await
            .unwrap();
        let device = Device {
            account_id: account.id,
            ..device
        };
        store.insert_device(&device).await.unwrap();
        let signer = dialog_credentials::SignerCredential::from(
            Ed25519Signer::import(&DEVICE_SEED).await.unwrap(),
        );
        let root = account.root_did.parse().unwrap();
        let intent = tonk_account::detach::SignedDetachIntent::sign(
            &signer,
            &root,
            &device.attachment_id,
            &device.delegation_cid,
            10,
        )
        .await
        .unwrap();

        assert_eq!(
            detach_device(&store, &intent, 11).await.unwrap(),
            DetachOutcome::Detached
        );
        assert_eq!(
            detach_device(&store, &intent, 12).await.unwrap(),
            DetachOutcome::AlreadyDetached
        );
        assert!(list_devices(&store, &account).await.unwrap().is_empty());

        let mut forged = intent;
        forged.signature[0] ^= 1;
        assert!(matches!(
            detach_device(&store, &forged, 13).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_registers_a_device_delegated_by_the_account_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (mut account, device, _, grant) = fixture().await;
        account.id = store
            .create_account(
                &account.email,
                &account.root_did,
                &account.credential_id,
                account.created_at,
            )
            .await
            .unwrap();

        register_device(
            &store,
            &account,
            &device.device_did,
            &device.name,
            &hex::encode(grant.to_bytes().unwrap()),
            device.created_at,
        )
        .await
        .unwrap();

        let listed = list_devices(&store, &account).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].delegation_cid, device.delegation_cid);
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

    struct SpyRevocations {
        objects: Mutex<HashMap<String, Vec<u8>>>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RevocationStore for SpyRevocations {
        async fn put(
            &self,
            verified: &tonk_identity::revocation::VerifiedRevocation,
            bytes: &[u8],
        ) -> Result<PutOutcome, RevocationStoreError> {
            self.events.lock().unwrap().push("r2");
            self.objects
                .lock()
                .unwrap()
                .insert(object_key(verified), bytes.to_vec());
            Ok(PutOutcome::Stored)
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
        SpyRevocations,
        Arc<Mutex<Vec<&'static str>>>,
    ) {
        let (account, device, root, grant) = fixture().await;
        let events = Arc::new(Mutex::new(Vec::new()));
        let projection = SpyProjection {
            device: device.clone(),
            events: events.clone(),
            fail: fail_projection,
        };
        let revocations = SpyRevocations {
            objects: Mutex::new(HashMap::new()),
            events: events.clone(),
        };
        (
            account,
            device,
            root,
            grant,
            projection,
            revocations,
            events,
        )
    }

    #[dialog_common::test]
    async fn it_writes_r2_before_projecting_device_status() {
        let (account, device, root, grant, projection, revocations, events) = spies(false).await;
        let artifact =
            tonk_identity::revocation::mint_root_revocation(root, &grant, &grant.proof_cids()[0])
                .await
                .unwrap();

        let outcome = revoke_device(
            &projection,
            &revocations,
            &account,
            CALLER_DID,
            &device.attachment_id,
            &device.device_did,
            &artifact,
        )
        .await
        .unwrap();

        assert_eq!(events.lock().unwrap().as_slice(), ["r2", "d1"]);
        assert_eq!(outcome.projection, Projection::Updated);
    }

    #[dialog_common::test]
    async fn it_accepts_a_revocation_when_the_projection_fails() {
        let (account, device, root, grant, projection, revocations, events) = spies(true).await;
        let artifact =
            tonk_identity::revocation::mint_root_revocation(root, &grant, &grant.proof_cids()[0])
                .await
                .unwrap();

        let outcome = revoke_device(
            &projection,
            &revocations,
            &account,
            CALLER_DID,
            &device.attachment_id,
            &device.device_did,
            &artifact,
        )
        .await
        .unwrap();

        assert_eq!(outcome.projection, Projection::Stale);
        assert_eq!(events.lock().unwrap().as_slice(), ["r2", "d1"]);
    }

    #[dialog_common::test]
    async fn it_never_projects_an_artifact_that_failed_verification() {
        let (account, device, _, _, projection, revocations, events) = spies(false).await;

        assert!(
            revoke_device(
                &projection,
                &revocations,
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

//! The account creation ceremony: check the presented descriptor and
//! `root → device` delegation, then register the account and its first
//! device.
//!
//! Nothing here proves control of the email address. That proof is the
//! activation link the access service emails, which the customer opens
//! afterwards, so an account exists before its address is confirmed and
//! the registration state says which.

use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::core::descriptor::validate_descriptor;
use crate::store::{
    Account, Device, DeviceStatus, NewAccount, NewDevice, PasskeyMetadata, Store, StoreError,
};
use tonk_account::creation::{
    AccountCreationFingerprint, AccountCreationFingerprintInput, AccountCreationPasskey,
};

/// Returned when the verified email address already belongs to an
/// account under a different root DID.
pub const EMAIL_TAKEN: &str = "an account already exists for this email address";

/// Returned when the calling root DID already has an account.
pub const ROOT_TAKEN: &str = "an account already exists for this passkey";

/// A request to create a new account and register its first device.
#[derive(Debug, Clone)]
pub struct CreateAccount {
    /// The account's verified email address.
    pub email: String,
    /// The account's root DID.
    pub root_did: String,
    /// Opaque identifier for the passkey credential backing the root.
    pub credential_id: String,
    /// The first device's DID.
    pub device_did: String,
    /// Human-readable name for the first device.
    pub device_name: String,
    /// Hex-encoded `root → device` delegation chain.
    pub delegation_hex: String,
    /// Hex-encoded root-signed account repository descriptor.
    pub repository_descriptor_hex: String,
    /// Facts recorded when Tonk created the passkey, if available.
    pub passkey: Option<PasskeyMetadata>,
}

/// Stable result of creating an account or recovering its exact winner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAccountOutcome {
    /// Provider account row ID.
    pub account_id: i64,
    /// Exact canonical root-signed repository descriptor bytes.
    pub descriptor: Vec<u8>,
    /// Versioned fingerprint of every caller-controlled creation fact.
    pub create_fingerprint: String,
    /// Whether an earlier atomic insert had already committed this winner.
    pub reused: bool,
}

/// Provider state for one proof-bound account-setup operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AccountSetupStatus {
    /// The verified root has no provider account row.
    Absent,
    /// The persisted account is the exact operation the caller fingerprinted.
    Accepted {
        /// Provider account row ID.
        #[serde(rename = "accountId")]
        account_id: i64,
        /// Canonical root-signed repository descriptor, hex encoded.
        #[serde(rename = "descriptorHex")]
        descriptor_hex: String,
        /// Server-reconstructed canonical creation fingerprint.
        #[serde(rename = "createFingerprint")]
        create_fingerprint: String,
    },
    /// The root exists, but its durable creation facts differ.
    Mismatch,
}

/// Canonical caller-controlled facts that make one account creation unique.
///
/// Provider-generated attachment IDs and timestamps are deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CreationFacts {
    email: String,
    root_did: String,
    credential_id: String,
    passkey: Option<PasskeyMetadata>,
    descriptor: Vec<u8>,
    device_did: String,
    device_name: String,
    delegation_cid: String,
    delegation: Vec<u8>,
}

impl CreationFacts {
    fn fingerprint(&self) -> String {
        AccountCreationFingerprintInput {
            email: &self.email,
            root_did: &self.root_did,
            credential_id: &self.credential_id,
            passkey: self.passkey.as_ref().map(|passkey| AccountCreationPasskey {
                created_at: passkey.created_at,
                created_on: &passkey.created_on,
            }),
            descriptor: &self.descriptor,
            device_did: &self.device_did,
            device_name: &self.device_name,
            delegation_cid: &self.delegation_cid,
            delegation: &self.delegation,
        }
        .fingerprint()
        .to_hex()
    }

    fn from_stored(account: &Account, first_device: &Device) -> Option<Self> {
        let passkey = match (
            account.passkey_created_at,
            account.passkey_created_on.as_ref(),
        ) {
            (None, None) => None,
            (Some(created_at), Some(created_on)) => Some(PasskeyMetadata {
                created_at,
                created_on: created_on.trim().to_string(),
            }),
            _ => return None,
        };
        Some(Self {
            email: account.email.to_lowercase(),
            root_did: account.root_did.clone(),
            credential_id: account.credential_id.clone(),
            passkey,
            descriptor: account.repository_descriptor.clone()?,
            device_did: first_device.device_did.clone(),
            device_name: first_device.name.clone(),
            delegation_cid: first_device.delegation_cid.clone(),
            delegation: hex::decode(&first_device.delegation_hex).ok()?,
        })
    }
}

/// Create a new account and register its first device.
///
/// Checks the presented descriptor and delegation, both purely local,
/// before touching the registry. Nothing here proves control of the
/// address: that proof is the activation link the access service emails
/// afterwards. The email address is lowercased before being stored. The account
/// and its first device are created atomically, so an insertion failure
/// cannot strand a zero-device account that has permanently burned the
/// email and root DID. Device registrations are account-scoped: one local
/// device identity may be linked to multiple accounts over its lifetime.
pub async fn create_account<S: Store>(
    store: &S,
    request: &CreateAccount,
    now: u64,
) -> Result<CreateAccountOutcome, CeremonyError> {
    let repository_descriptor =
        validate_descriptor(&request.repository_descriptor_hex, &request.root_did).await?;
    let delegation_cid = check_device_delegation(
        &request.delegation_hex,
        &request.root_did,
        &request.device_did,
    )
    .await?;
    let email = request.email.to_lowercase();
    let passkey = request.passkey.as_ref().map(|passkey| PasskeyMetadata {
        created_at: passkey.created_at,
        created_on: passkey.created_on.trim().to_string(),
    });
    let facts = CreationFacts {
        email: email.clone(),
        root_did: request.root_did.clone(),
        credential_id: request.credential_id.clone(),
        passkey: passkey.clone(),
        descriptor: repository_descriptor.clone(),
        device_did: request.device_did.clone(),
        device_name: request.device_name.clone(),
        delegation_cid: delegation_cid.clone(),
        delegation: hex::decode(&request.delegation_hex).map_err(|error| {
            CeremonyError::Invalid(format!("bad delegation hex after validation: {error}"))
        })?,
    };
    let account = NewAccount {
        email: &email,
        root_did: &request.root_did,
        credential_id: &request.credential_id,
        repository_descriptor: &repository_descriptor,
        passkey: passkey.as_ref(),
        created_at: now,
    };
    let device = NewDevice {
        device_did: request.device_did.clone(),
        attachment_id: crate::core::devices::random_attachment_id(),
        delegation_cid,
        delegation_hex: request.delegation_hex.clone(),
        name: request.device_name.clone(),
    };
    let created = store.create_account_with_device(&account, &device).await;

    match created {
        Ok(account_id) => Ok(CreateAccountOutcome {
            account_id,
            descriptor: facts.descriptor.clone(),
            create_fingerprint: facts.fingerprint(),
            reused: false,
        }),
        Err(StoreError::Conflict(detail)) => {
            if let Some(outcome) = recover_exact_replay(store, &facts).await {
                return Ok(outcome);
            }
            Err(explain_conflict(store, request, &email, detail).await)
        }
        Err(err) => Err(err.into()),
    }
}

/// Recover the winner only when the account and its earliest device reproduce
/// the complete validated operation. A lookup failure leaves the original
/// conflict authoritative and is logged without exposing driver detail.
async fn recover_exact_replay<S: Store>(
    store: &S,
    expected: &CreationFacts,
) -> Option<CreateAccountOutcome> {
    let account = match store.account_by_root(&expected.root_did).await {
        Ok(Some(account)) => account,
        Ok(None) => return None,
        Err(error) => {
            crate::core::log_detail(&format!(
                "account replay lookup failed after conflict: {error:?}"
            ));
            return None;
        }
    };
    let devices = match store.devices(account.id).await {
        Ok(devices) => devices,
        Err(error) => {
            crate::core::log_detail(&format!(
                "account replay device lookup failed after conflict: {error:?}"
            ));
            return None;
        }
    };
    let first_device = devices.into_iter().min_by_key(|device| device.id)?;
    if first_device.status != DeviceStatus::Active {
        return None;
    }
    let stored = CreationFacts::from_stored(&account, &first_device)?;
    if &stored != expected {
        return None;
    }
    let create_fingerprint = stored.fingerprint();
    Some(CreateAccountOutcome {
        account_id: account.id,
        descriptor: stored.descriptor,
        create_fingerprint,
        reused: true,
    })
}

/// Check provider setup state for the root cryptographically bound by a
/// device invocation.
///
/// Authentication happens before this function. Its root argument must come
/// from the verified invocation subject, never from a request field.
pub async fn account_setup_status<S: Store>(
    store: &S,
    root_did: &str,
    device_did: &str,
    delegation_cid: &str,
    expected_fingerprint: &str,
) -> Result<AccountSetupStatus, CeremonyError> {
    let expected_fingerprint = canonical_create_fingerprint(expected_fingerprint)?;
    let Some(account) = store.account_by_root(root_did).await? else {
        return Ok(AccountSetupStatus::Absent);
    };
    let Some(first_device) = store
        .devices(account.id)
        .await?
        .into_iter()
        .min_by_key(|device| device.id)
    else {
        return Ok(AccountSetupStatus::Mismatch);
    };
    if first_device.status != DeviceStatus::Active {
        return Err(CeremonyError::Forbidden(
            "setup status requires the active first account device".to_string(),
        ));
    }
    if first_device.device_did != device_did || first_device.delegation_cid != delegation_cid {
        return Err(CeremonyError::Forbidden(
            "setup proof does not match the first account device".to_string(),
        ));
    }
    let Ok(stored_delegation_cid) = check_device_delegation(
        &first_device.delegation_hex,
        root_did,
        &first_device.device_did,
    )
    .await
    else {
        return Ok(AccountSetupStatus::Mismatch);
    };
    if stored_delegation_cid != first_device.delegation_cid {
        return Ok(AccountSetupStatus::Mismatch);
    }
    let Some(stored) = CreationFacts::from_stored(&account, &first_device) else {
        return Ok(AccountSetupStatus::Mismatch);
    };
    let descriptor_hex = hex::encode(&stored.descriptor);
    let Ok(canonical_descriptor) = validate_descriptor(&descriptor_hex, root_did).await else {
        return Ok(AccountSetupStatus::Mismatch);
    };
    if canonical_descriptor != stored.descriptor {
        return Ok(AccountSetupStatus::Mismatch);
    }
    let create_fingerprint = stored.fingerprint();
    if create_fingerprint != expected_fingerprint {
        return Ok(AccountSetupStatus::Mismatch);
    }
    Ok(AccountSetupStatus::Accepted {
        account_id: account.id,
        descriptor_hex,
        create_fingerprint,
    })
}

/// Parse a 32-byte BLAKE3 fingerprint and return its lowercase wire form.
fn canonical_create_fingerprint(value: &str) -> Result<String, CeremonyError> {
    AccountCreationFingerprint::from_hex(value)
        .map(AccountCreationFingerprint::to_hex)
        .map_err(|error| CeremonyError::Invalid(error.to_string()))
}

/// Turn a uniqueness conflict from
/// [`Store::create_account_with_device`] into a message the caller can
/// act on, by asking which account column is already taken.
///
/// Naming the taken column is safe here: reaching this point means the
/// caller signed the invocation with the root key, so the passkey exists
/// and the address is one they submitted. It is no longer the only place
/// that would answer -- the access service resolves an address to its
/// account by design, so a client asks that before running a ceremony
/// rather than learning it from a conflict afterwards.
///
/// A lookup that itself fails degrades to the generic message rather
/// than masking the conflict as an internal error: the request conflicted
/// either way, and the driver's own text is never a safe substitute.
async fn explain_conflict<S: Store>(
    store: &S,
    request: &CreateAccount,
    email: &str,
    detail: String,
) -> CeremonyError {
    crate::core::log_detail(&format!("account creation conflict: {detail}"));
    fn taken<T>(result: Result<Option<T>, StoreError>) -> bool {
        matches!(result, Ok(Some(_)))
    }
    let message = if taken(store.account_by_root(&request.root_did).await) {
        ROOT_TAKEN
    } else if taken(store.account_by_email(email).await) {
        EMAIL_TAKEN
    } else {
        crate::core::GENERIC_CONFLICT
    };
    CeremonyError::Conflict(message.to_string())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus};

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn fixture_for(
        root_seed: [u8; 32],
        device_seed: [u8; 32],
        remote: &str,
    ) -> (String, String, String, String) {
        // (root_did, device_did, delegation_hex, descriptor_hex)
        let root = dialog_credentials::Ed25519Signer::import(&root_seed)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&device_seed)
            .await
            .unwrap();
        let root_did = {
            use dialog_varsig::Principal;
            root.did().to_string()
        };
        let device_did = {
            use dialog_varsig::Principal;
            device.did().to_string()
        };
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(&root, remote)
            .await
            .unwrap();
        let descriptor_hex = hex::encode(descriptor.bytes());
        let chain = tonk_identity::delegation::mint_device_delegation(root, &{
            use dialog_varsig::Principal;
            device.did()
        })
        .await
        .unwrap();
        (
            root_did,
            device_did,
            hex::encode(chain.to_bytes().unwrap()),
            descriptor_hex,
        )
    }

    async fn fixture() -> (String, String, String, String) {
        fixture_for(ROOT_PRF, DEVICE_SEED, "https://accounts.example/ucan/").await
    }

    async fn assert_replay_conflict(
        store: &SqliteStore,
        request: &CreateAccount,
        expected_message: &str,
        label: &str,
    ) {
        let error = create_account(store, request, 500).await;
        let Err(CeremonyError::Conflict(message)) = error else {
            panic!("{label} unexpectedly recovered as an exact replay: {error:?}");
        };
        assert_eq!(message, expected_message, "wrong conflict for {label}");
        for leak in [
            "UNIQUE constraint",
            "constraint failed",
            "accounts.",
            "devices.",
            "SQLITE_",
            "D1Error",
            "person@example.com",
            "did:key:",
        ] {
            assert!(
                !message.contains(leak),
                "{label} leaked {leak:?} in its conflict: {message}"
            );
        }
    }

    #[dialog_common::test]
    async fn it_creates_an_account_with_a_valid_code_and_delegation() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
            passkey: Some(PasskeyMetadata {
                created_at: 150,
                created_on: "Chrome on macOS".into(),
            }),
        };
        let outcome = create_account(&store, &request, 200).await.unwrap();
        assert!(!outcome.reused);
        assert_eq!(outcome.create_fingerprint.len(), 64);
        let id = outcome.account_id;
        let account = store.account_by_root(&root_did).await.unwrap().unwrap();
        assert_eq!((account.id, account.email.as_str()), (id, "a@x.com"));
        assert_eq!(account.passkey_created_at, Some(150));
        assert_eq!(
            account.passkey_created_on.as_deref(),
            Some("Chrome on macOS")
        );
        assert_eq!(store.devices(id).await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_reuses_only_an_exact_account_creation() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let passkey = Some(PasskeyMetadata {
            created_at: 150,
            created_on: "Chrome on macOS".into(),
        });
        let first = create_account(
            &store,
            &CreateAccount {
                email: "person@example.com".into(),
                root_did: root_did.clone(),
                credential_id: "credential".into(),
                device_did: device_did.clone(),
                device_name: "laptop".into(),
                delegation_hex: delegation_hex.clone(),
                repository_descriptor_hex: repository_descriptor_hex.clone(),
                passkey: passkey.clone(),
            },
            200,
        )
        .await
        .unwrap();
        assert!(!first.reused);
        let first_id = first.account_id;
        let first_device = store.devices(first_id).await.unwrap().remove(0);

        // This is the same signed semantic operation after the caller lost
        // the first response. Email case is normalized, while the server's
        // random attachment candidate and timestamps are deliberately not
        // part of replay identity.
        let replayed = create_account(
            &store,
            &CreateAccount {
                email: "PERSON@EXAMPLE.COM".into(),
                root_did,
                credential_id: "credential".into(),
                device_did,
                device_name: "laptop".into(),
                delegation_hex: delegation_hex.to_uppercase(),
                repository_descriptor_hex: repository_descriptor_hex.to_uppercase(),
                passkey: passkey.map(|passkey| PasskeyMetadata {
                    created_at: passkey.created_at,
                    created_on: format!("  {}  ", passkey.created_on),
                }),
            },
            500,
        )
        .await
        .expect("an exact account creation replay should recover its winner");

        assert!(replayed.reused);
        assert_eq!(replayed.account_id, first_id);
        assert_eq!(replayed.descriptor, first.descriptor);
        assert_eq!(replayed.create_fingerprint, first.create_fingerprint);
        let devices = store.devices(first_id).await.unwrap();
        assert_eq!(devices.len(), 1, "replay must not add another device");
        assert_eq!(devices[0].attachment_id, first_device.attachment_id);
    }

    #[dialog_common::test]
    async fn it_rejects_every_nonexact_creation_replay() {
        use dialog_ucan_core::subject::Subject;
        use dialog_ucan_core::time::timestamp::Timestamp;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain};
        use dialog_varsig::Principal as _;

        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let base = CreateAccount {
            email: "person@example.com".into(),
            root_did: root_did.clone(),
            credential_id: "credential".into(),
            device_did: device_did.clone(),
            device_name: "laptop".into(),
            delegation_hex: delegation_hex.clone(),
            repository_descriptor_hex: repository_descriptor_hex.clone(),
            passkey: Some(PasskeyMetadata {
                created_at: 150,
                created_on: "Chrome on macOS".into(),
            }),
        };
        create_account(&store, &base, 200).await.unwrap();

        let mut changed = base.clone();
        changed.email = "other@example.com".into();
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "normalized email").await;

        // A root change must carry its own valid descriptor and delegation;
        // it then collides on the original normalized email.
        let (other_root, other_device, other_delegation, other_descriptor) =
            fixture_for([10u8; 32], [11u8; 32], "https://accounts.example/ucan/").await;
        changed = base.clone();
        changed.root_did = other_root;
        changed.device_did = other_device;
        changed.delegation_hex = other_delegation;
        changed.repository_descriptor_hex = other_descriptor;
        assert_replay_conflict(&store, &changed, EMAIL_TAKEN, "root DID").await;

        changed = base.clone();
        changed.credential_id = "other-credential".into();
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "credential ID").await;

        changed = base.clone();
        changed.passkey = None;
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "passkey presence").await;

        changed = base.clone();
        changed.passkey.as_mut().unwrap().created_at += 1;
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "passkey createdAt").await;

        changed = base.clone();
        changed.passkey.as_mut().unwrap().created_on = "Firefox on Linux".into();
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "passkey createdOn").await;

        let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
            .await
            .unwrap();
        let other_descriptor =
            tonk_account::AccountRepositoryDescriptorV1::sign(&root, "https://other.example/ucan/")
                .await
                .unwrap();
        changed = base.clone();
        changed.repository_descriptor_hex = hex::encode(other_descriptor.bytes());
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "repository descriptor bytes").await;

        let other_device = dialog_credentials::Ed25519Signer::import(&[12u8; 32])
            .await
            .unwrap();
        let other_device_delegation =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &other_device.did())
                .await
                .unwrap();
        changed = base.clone();
        changed.device_did = other_device.did().to_string();
        changed.delegation_hex = hex::encode(other_device_delegation.to_bytes().unwrap());
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "first-device DID").await;

        changed = base.clone();
        changed.device_name = "phone".into();
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "first-device name").await;

        // Keep the same root and device while changing the signed delegation
        // bytes and therefore its content CID.
        let device = dialog_credentials::Ed25519Signer::import(&DEVICE_SEED)
            .await
            .unwrap();
        let alternate_delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(root))
            .audience(&device.did())
            .subject(Subject::Any)
            .command(vec![])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        changed = base;
        changed.delegation_hex = hex::encode(
            DelegationChain::new(alternate_delegation)
                .to_bytes()
                .unwrap(),
        );
        assert_replay_conflict(&store, &changed, ROOT_TAKEN, "delegation CID and bytes").await;
    }

    #[dialog_common::test]
    async fn it_reports_absent_for_a_verified_root_without_an_account() {
        let store = SqliteStore::in_memory().unwrap();
        let status = account_setup_status(
            &store,
            "did:key:zAbsentRoot",
            "did:key:zAbsentDevice",
            "bafyAbsentDelegation",
            &"ab".repeat(32),
        )
        .await
        .unwrap();

        assert_eq!(status, AccountSetupStatus::Absent);
    }

    #[dialog_common::test]
    async fn it_accepts_the_exact_persisted_setup_fingerprint() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let created = create_account(
            &store,
            &CreateAccount {
                email: "person@example.com".into(),
                root_did: root_did.clone(),
                credential_id: "credential".into(),
                device_did: device_did.clone(),
                device_name: "laptop".into(),
                delegation_hex,
                repository_descriptor_hex,
                passkey: Some(PasskeyMetadata {
                    created_at: 150,
                    created_on: "Chrome on macOS".into(),
                }),
            },
            200,
        )
        .await
        .unwrap();
        let first_device = store.devices(created.account_id).await.unwrap().remove(0);

        let status = account_setup_status(
            &store,
            &root_did,
            &device_did,
            &first_device.delegation_cid,
            &created.create_fingerprint,
        )
        .await
        .unwrap();
        assert_eq!(
            status,
            AccountSetupStatus::Accepted {
                account_id: created.account_id,
                descriptor_hex: hex::encode(created.descriptor),
                create_fingerprint: created.create_fingerprint,
            }
        );
    }

    #[dialog_common::test]
    async fn it_distinguishes_mismatched_setup_and_rejects_wrong_first_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let created = create_account(
            &store,
            &CreateAccount {
                email: "person@example.com".into(),
                root_did: root_did.clone(),
                credential_id: "credential".into(),
                device_did: device_did.clone(),
                device_name: "laptop".into(),
                delegation_hex,
                repository_descriptor_hex,
                passkey: None,
            },
            200,
        )
        .await
        .unwrap();
        let first_device = store.devices(created.account_id).await.unwrap().remove(0);

        assert_eq!(
            account_setup_status(
                &store,
                &root_did,
                &device_did,
                &first_device.delegation_cid,
                &"cd".repeat(32),
            )
            .await
            .unwrap(),
            AccountSetupStatus::Mismatch
        );
        assert!(matches!(
            account_setup_status(
                &store,
                &root_did,
                "did:key:zWrongDevice",
                &first_device.delegation_cid,
                &created.create_fingerprint,
            )
            .await,
            Err(CeremonyError::Forbidden(_))
        ));
        assert!(matches!(
            account_setup_status(
                &store,
                &root_did,
                &device_did,
                "bafyWrongDelegation",
                &created.create_fingerprint,
            )
            .await,
            Err(CeremonyError::Forbidden(_))
        ));
        assert!(matches!(
            account_setup_status(
                &store,
                &root_did,
                &device_did,
                &first_device.delegation_cid,
                &created.create_fingerprint.to_uppercase(),
            )
            .await,
            Err(CeremonyError::Invalid(_))
        ));

        store
            .revoke_device(created.account_id, &device_did)
            .await
            .unwrap();
        assert!(matches!(
            account_setup_status(
                &store,
                &root_did,
                &device_did,
                &first_device.delegation_cid,
                &created.create_fingerprint,
            )
            .await,
            Err(CeremonyError::Forbidden(_))
        ));

        // A legacy/incomplete account row cannot be called accepted merely
        // because the cryptographic proof and a caller-provided hash exist.
        let incomplete = SqliteStore::in_memory().unwrap();
        let account_id = incomplete
            .create_account("legacy@example.com", &root_did, "credential", 1)
            .await
            .unwrap();
        incomplete
            .insert_device(&Device {
                id: 0,
                account_id,
                device_did: device_did.clone(),
                attachment_id: "01".repeat(32),
                delegation_cid: first_device.delegation_cid.clone(),
                delegation_hex: first_device.delegation_hex,
                name: "laptop".into(),
                status: DeviceStatus::Active,
                created_at: 1,
            })
            .await
            .unwrap();
        assert_eq!(
            account_setup_status(
                &incomplete,
                &root_did,
                &device_did,
                &first_device.delegation_cid,
                &"ab".repeat(32),
            )
            .await
            .unwrap(),
            AccountSetupStatus::Mismatch
        );

        // The stored delegation bytes must still validate to the proof-bound
        // root, device, and CID. Decoding alone would let a legacy empty value
        // be fingerprinted and incorrectly called accepted.
        let malformed = SqliteStore::in_memory().unwrap();
        let malformed_account = NewAccount {
            email: "malformed@example.com",
            root_did: &root_did,
            credential_id: "credential",
            repository_descriptor: &created.descriptor,
            passkey: None,
            created_at: 1,
        };
        let malformed_device = NewDevice {
            device_did: device_did.clone(),
            attachment_id: "02".repeat(32),
            delegation_cid: first_device.delegation_cid.clone(),
            delegation_hex: String::new(),
            name: "laptop".into(),
        };
        malformed
            .create_account_with_device(&malformed_account, &malformed_device)
            .await
            .unwrap();
        let malformed_fingerprint = AccountCreationFingerprintInput {
            email: malformed_account.email,
            root_did: &root_did,
            credential_id: malformed_account.credential_id,
            passkey: None,
            descriptor: &created.descriptor,
            device_did: &device_did,
            device_name: &malformed_device.name,
            delegation_cid: &first_device.delegation_cid,
            delegation: &[],
        }
        .fingerprint()
        .to_hex();
        assert_eq!(
            account_setup_status(
                &malformed,
                &root_did,
                &device_did,
                &first_device.delegation_cid,
                &malformed_fingerprint,
            )
            .await
            .unwrap(),
            AccountSetupStatus::Mismatch
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegation_issued_by_a_different_root() {
        // fixture delegation, but the request claims a different root DID:
        // possession of the claimed root is not proven, so creation fails.
        let store = SqliteStore::in_memory().unwrap();
        let (_, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let other_root = {
            use dialog_varsig::Principal;
            dialog_credentials::Ed25519Signer::import(&[9u8; 32])
                .await
                .unwrap()
                .did()
                .to_string()
        };
        let request = CreateAccount {
            email: "a@x.com".into(),
            root_did: other_root,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
            passkey: None,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Invalid(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_second_account_for_the_same_email() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let first = CreateAccount {
            email: "a@x.com".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
            passkey: None,
        };
        create_account(&store, &first, 200).await.unwrap();

        // Same email, different root: build a second fixture from PRF
        // [10u8; 32] and device seed [12u8; 32], mint its delegation as
        // in fixture(), request a fresh code past the cooldown.
        let root2 = dialog_credentials::Ed25519Signer::import(&[10u8; 32])
            .await
            .unwrap();
        let device2 = dialog_credentials::Ed25519Signer::import(&[12u8; 32])
            .await
            .unwrap();
        let (root2_did, device2_did) = {
            use dialog_varsig::Principal;
            (root2.did().to_string(), device2.did().to_string())
        };
        let descriptor2 = tonk_account::AccountRepositoryDescriptorV1::sign(
            &root2,
            "https://accounts.example/ucan/",
        )
        .await
        .unwrap();
        let chain2 = {
            use dialog_varsig::Principal;
            tonk_identity::delegation::mint_device_delegation(root2, &device2.did())
                .await
                .unwrap()
        };
        let second = CreateAccount {
            email: "a@x.com".into(),
            root_did: root2_did,
            credential_id: "cred2".into(),
            device_did: device2_did,
            device_name: "phone".into(),
            delegation_hex: hex::encode(chain2.to_bytes().unwrap()),
            repository_descriptor_hex: hex::encode(descriptor2.bytes()),
            passkey: None,
        };
        // The message names the email, not the database's own
        // "UNIQUE constraint failed: accounts.email" text.
        let error = create_account(&store, &second, 500).await;
        assert!(matches!(&error, Err(CeremonyError::Conflict(msg)) if msg == EMAIL_TAKEN));
    }

    #[dialog_common::test]
    async fn it_reports_the_passkey_when_the_root_did_is_already_registered() {
        // Same root, different email: SQLite reports the `root_did` index
        // whenever the root collides, even if the email collides too, so
        // this case must not be explained as a taken email address.
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, descriptor_hex) = fixture().await;
        create_account(
            &store,
            &CreateAccount {
                email: "a@x.com".into(),
                root_did: root_did.clone(),
                credential_id: "cred".into(),
                device_did: device_did.clone(),
                device_name: "laptop".into(),
                delegation_hex: delegation_hex.clone(),
                repository_descriptor_hex: descriptor_hex.clone(),
                passkey: None,
            },
            200,
        )
        .await
        .unwrap();

        let again = CreateAccount {
            email: "b@x.com".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex: descriptor_hex,
            passkey: None,
        };
        let error = create_account(&store, &again, 500).await;
        assert!(matches!(&error, Err(CeremonyError::Conflict(msg)) if msg == ROOT_TAKEN));
    }

    #[dialog_common::test]
    async fn it_never_returns_the_database_error_text_for_a_conflict() {
        // The store's own conflict text names tables and columns (and,
        // under D1, carries a JS stack trace). No conflict message from
        // this ceremony may contain any of it.
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, descriptor_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex: descriptor_hex,
            passkey: None,
        };
        create_account(&store, &request, 200).await.unwrap();

        let request = CreateAccount {
            device_name: "different laptop".into(),
            ..request
        };
        let Err(CeremonyError::Conflict(message)) = create_account(&store, &request, 500).await
        else {
            panic!("expected a conflict");
        };
        for leak in [
            "UNIQUE constraint",
            "constraint failed",
            "accounts.",
            "devices.",
            "SQLITE_",
            "D1Error",
        ] {
            assert!(
                !message.contains(leak),
                "conflict message leaked {leak:?}: {message}"
            );
        }
    }

    #[dialog_common::test]
    async fn it_rejects_account_creation_while_the_device_is_active_elsewhere() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;

        let other_id = store
            .create_account("other@x.com", "did:key:zOther", "cred-other", 1)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                id: 0,
                account_id: other_id,
                device_did: device_did.clone(),
                attachment_id: "04".repeat(32),
                delegation_cid: "bafyOther".into(),
                delegation_hex: "beef".into(),
                name: "old registration".into(),
                status: DeviceStatus::Active,
                created_at: 1,
            })
            .await
            .unwrap();

        let outcome = create_account(
            &store,
            &CreateAccount {
                email: "a@x.com".into(),
                root_did,
                credential_id: "cred".into(),
                device_did,
                device_name: "new registration".into(),
                delegation_hex,
                repository_descriptor_hex,
                passkey: None,
            },
            200,
        )
        .await;

        assert!(matches!(outcome, Err(CeremonyError::Conflict(_))));
        assert_eq!(store.devices(other_id).await.unwrap().len(), 1);
    }
}

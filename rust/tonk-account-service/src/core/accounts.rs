//! The account creation ceremony: check the presented descriptor and
//! `root → device` delegation, consume the verified email code, then
//! register the account and its first device.

use crate::core::CeremonyError;
use crate::core::codes::verify_code;
use crate::core::delegation::check_device_delegation;
use crate::core::descriptor::validate_descriptor;
use crate::store::{NewDevice, Store, StoreError};

/// Returned when the verified email address already belongs to an
/// account under a different root DID.
pub const EMAIL_TAKEN: &str = "an account already exists for this email address";

/// Returned when the calling root DID already has an account.
pub const ROOT_TAKEN: &str = "an account already exists for this passkey";

/// Returned when the first device's DID is already registered, under
/// this account or another.
pub const DEVICE_TAKEN: &str = "this browser profile is already registered to an account";

/// A request to create a new account and register its first device.
pub struct CreateAccount {
    /// The account's verified email address.
    pub email: String,
    /// The verification code sent to `email`.
    pub code: String,
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
}

/// Create a new account and register its first device.
///
/// Checks the presented descriptor and delegation before consuming
/// `request.code`. Both checks are purely local, and [`verify_code`] is
/// one-shot: validating after it would let a malformed request burn the
/// user's code and leave them waiting out the resend cooldown for a new
/// one. The email address is lowercased before being stored. The account
/// and its first device are created atomically: a conflict on the device
/// DID (for example, an attacker who has pre-registered a delegation to
/// the victim's device DID under a different account) rolls back the
/// account row too, rather than stranding a zero-device account that
/// has permanently burned the email and root DID.
pub async fn create_account<S: Store>(
    store: &S,
    request: &CreateAccount,
    now: u64,
) -> Result<i64, CeremonyError> {
    let repository_descriptor =
        validate_descriptor(&request.repository_descriptor_hex, &request.root_did).await?;
    let delegation_cid = check_device_delegation(
        &request.delegation_hex,
        &request.root_did,
        &request.device_did,
    )
    .await?;
    verify_code(store, &request.email, &request.code, now).await?;

    let email = request.email.to_lowercase();
    let created = store
        .create_account_with_device(
            &email,
            &request.root_did,
            &request.credential_id,
            &repository_descriptor,
            &NewDevice {
                device_did: request.device_did.clone(),
                delegation_cid,
                delegation_hex: request.delegation_hex.clone(),
                name: request.device_name.clone(),
            },
            now,
        )
        .await;

    match created {
        Ok(account_id) => Ok(account_id),
        Err(StoreError::Conflict(detail)) => {
            Err(explain_conflict(store, request, &email, detail).await)
        }
        Err(err) => Err(err.into()),
    }
}

/// Turn a uniqueness conflict from
/// [`Store::create_account_with_device`] into a message the caller can
/// act on, by asking which of the three unique columns is already taken.
///
/// Naming the taken column is safe here and only here: reaching this
/// point means the caller both verified an emailed code (proving control
/// of the address) and signed the invocation with the root key (proving
/// possession of the passkey), so nothing is disclosed that they did not
/// already supply. The registry is never consulted *before* those
/// proofs, which is what keeps `POST /accounts` from answering "is this
/// email registered?" for an arbitrary address.
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
    } else if taken(store.device_by_did(&request.device_did).await) {
        DEVICE_TAKEN
    } else {
        crate::core::GENERIC_CONFLICT
    };
    CeremonyError::Conflict(message.to_string())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::core::codes::request_code;
    use crate::email::CapturedEmail;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus};

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn fixture() -> (String, String, String, String) {
        // (root_did, device_did, delegation_hex, descriptor_hex)
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&DEVICE_SEED)
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
        let descriptor = tonk_account::AccountRepositoryDescriptorV1::sign(
            &root,
            "https://accounts.example/ucan/",
        )
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

    #[dialog_common::test]
    async fn it_creates_an_account_with_a_valid_code_and_delegation() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        let id = create_account(&store, &request, 200).await.unwrap();
        let account = store.account_by_root(&root_did).await.unwrap().unwrap();
        assert_eq!((account.id, account.email.as_str()), (id, "a@x.com"));
        assert_eq!(store.devices(id).await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegation_issued_by_a_different_root() {
        // fixture delegation, but the request claims a different root DID:
        // possession of the claimed root is not proven, so creation fails.
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (_, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let other_root = {
            use dialog_varsig::Principal;
            tonk_identity::derive::derive_root_signer(&[9u8; 32])
                .await
                .unwrap()
                .did()
                .to_string()
        };
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: other_root,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Invalid(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_bad_code_before_touching_the_registry() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "000000".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::CodeInvalid)
        ));
        assert!(store.account_by_root(&root_did).await.unwrap().is_none());
    }

    #[dialog_common::test]
    async fn it_keeps_the_code_usable_after_a_malformed_descriptor() {
        // The code is one-shot and rate limited behind a resend cooldown, so a
        // locally detectable descriptor fault must not consume it.
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let mut request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        let good_descriptor = request.repository_descriptor_hex.clone();
        request.repository_descriptor_hex = "not hex".into();
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Invalid(_))
        ));

        request.repository_descriptor_hex = good_descriptor;
        create_account(&store, &request, 200).await.unwrap();
        assert!(store.account_by_root(&root_did).await.unwrap().is_some());
    }

    #[dialog_common::test]
    async fn it_keeps_the_code_usable_after_a_mismatched_delegation() {
        // Same one-shot reasoning as the descriptor: the delegation check is
        // local, so it runs before the code is spent.
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let mut request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did: device_did.clone(),
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        request.device_did = {
            use dialog_varsig::Principal;
            dialog_credentials::Ed25519Signer::import(&[13u8; 32])
                .await
                .unwrap()
                .did()
                .to_string()
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Invalid(_))
        ));

        request.device_did = device_did;
        create_account(&store, &request, 200).await.unwrap();
        assert!(store.account_by_root(&root_did).await.unwrap().is_some());
    }

    #[dialog_common::test]
    async fn it_rejects_a_second_account_for_the_same_email() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;
        let first = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        create_account(&store, &first, 200).await.unwrap();

        // Same email, different root: build a second fixture from PRF
        // [10u8; 32] and device seed [12u8; 32], mint its delegation as
        // in fixture(), request a fresh code past the cooldown.
        let root2 = tonk_identity::derive::derive_root_signer(&[10u8; 32])
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
        request_code(&store, &sender, "a@x.com", "654321", 400)
            .await
            .unwrap();
        let second = CreateAccount {
            email: "a@x.com".into(),
            code: "654321".into(),
            root_did: root2_did,
            credential_id: "cred2".into(),
            device_did: device2_did,
            device_name: "phone".into(),
            delegation_hex: hex::encode(chain2.to_bytes().unwrap()),
            repository_descriptor_hex: hex::encode(descriptor2.bytes()),
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
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, descriptor_hex) = fixture().await;
        create_account(
            &store,
            &CreateAccount {
                email: "a@x.com".into(),
                code: "123456".into(),
                root_did: root_did.clone(),
                credential_id: "cred".into(),
                device_did: device_did.clone(),
                device_name: "laptop".into(),
                delegation_hex: delegation_hex.clone(),
                repository_descriptor_hex: descriptor_hex.clone(),
            },
            200,
        )
        .await
        .unwrap();

        request_code(&store, &sender, "b@x.com", "654321", 400)
            .await
            .unwrap();
        let again = CreateAccount {
            email: "b@x.com".into(),
            code: "654321".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex: descriptor_hex,
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
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, descriptor_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex: descriptor_hex,
        };
        create_account(&store, &request, 200).await.unwrap();

        request_code(&store, &sender, "a@x.com", "654321", 400)
            .await
            .unwrap();
        let request = CreateAccount {
            code: "654321".into(),
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
    async fn it_does_not_strand_an_account_when_the_device_insert_conflicts() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex, repository_descriptor_hex) = fixture().await;

        // Pre-register the fixture's device DID under a different
        // account, as an attacker front-running the victim's device DID
        // would. Without atomicity, the account insert below would
        // still succeed and the device insert would fail, permanently
        // burning the email and root DID with zero devices registered.
        let other_id = store
            .create_account("other@x.com", "did:key:zOther", "cred-other", 1)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: other_id,
                device_did: device_did.clone(),
                delegation_cid: "bafyStolen".into(),
                delegation_hex: "beef".into(),
                name: "attacker".into(),
                status: DeviceStatus::Active,
                created_at: 1,
            })
            .await
            .unwrap();

        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
            repository_descriptor_hex,
        };
        let error = create_account(&store, &request, 200).await;
        assert!(matches!(&error, Err(CeremonyError::Conflict(msg)) if msg == DEVICE_TAKEN));
        assert!(store.account_by_root(&root_did).await.unwrap().is_none());
    }
}

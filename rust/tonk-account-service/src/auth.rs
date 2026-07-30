//! UCAN authentication for registry endpoints.
//!
//! Endpoints outside the email-code bootstrap ceremonies are invoked as
//! signed UCAN containers: the invocation subject is the account's root
//! DID, and the invocation issuer is a device delegated under that
//! root. [`authorize`] parses and cryptographically verifies such a
//! container, checks the command matches what the caller expects, then
//! binds it to a registered account and one of its active devices.

use std::collections::BTreeMap;

use dialog_credentials::Ed25519KeyResolver;
use dialog_ucan_core::InvocationChain;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
use dialog_varsig::algorithm::eddsa::Ed25519Signature;

use crate::core::CeremonyError;
use crate::store::{Account, Device, DeviceStatus, Store};

/// An authenticated caller: the account and device bound by a verified
/// UCAN invocation, plus the invocation's arguments.
pub struct Caller {
    /// The account the invocation's subject DID resolved to.
    pub account: Account,
    /// The device the invocation's issuer DID resolved to.
    pub device: Device,
    /// The invocation's arguments.
    pub arguments: BTreeMap<String, Promised>,
}

/// A request signed directly by the account root during a passkey ceremony.
pub struct RootCaller {
    /// The root DID that signed and subjects the invocation.
    pub root_did: String,
    /// The invocation's signed arguments.
    pub arguments: BTreeMap<String, Promised>,
}

async fn verified_chain(
    body: &[u8],
    expected_command: &[&str],
) -> Result<InvocationChain<Ed25519Signature>, CeremonyError> {
    let chain = InvocationChain::try_from(body)
        .map_err(|err| CeremonyError::Invalid(format!("bad invocation container: {err}")))?;

    chain.verify(&Ed25519KeyResolver).await.map_err(|err| {
        CeremonyError::Unauthorized(format!("invocation failed to verify: {err}"))
    })?;

    let command_segments: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if command_segments.as_slice() != expected_command {
        return Err(CeremonyError::Forbidden(format!(
            "expected command {expected_command:?}, got {command_segments:?}"
        )));
    }

    Ok(chain)
}

/// Get the latest expiration a ceremony invocation may carry: five
/// minutes from now, plus a one-minute allowance for clock skew between
/// the caller and this service. Mirrors [`Timestamp::five_minutes_from_now`].
///
/// # Panics
///
/// This function will panic if the current time is before the Unix epoch.
#[allow(clippy::expect_used)]
fn ceremony_expiration_upper_bound() -> Timestamp {
    Timestamp::new(SystemTime::now() + Duration::from_secs(5 * 60) + CEREMONY_SKEW_ALLOWANCE)
        .expect("the current time to be sometime in the 3rd millennium CE")
}

/// Clock-skew allowance applied to the upper bound of the ceremony
/// expiration window, so a caller whose clock runs a little fast isn't
/// rejected for an invocation that is, in practice, well within the
/// five-minute ceremony window.
const CEREMONY_SKEW_ALLOWANCE: Duration = Duration::from_secs(60);

/// Require an expiration on the invocation and bound it to the
/// five-minute ceremony window every account-service request uses,
/// plus a one-minute allowance for clock skew on the upper bound. The
/// lower bound (already expired) carries no such allowance.
fn require_ceremony_expiration(
    chain: &InvocationChain<Ed25519Signature>,
) -> Result<(), CeremonyError> {
    let expiration = chain.invocation.expiration().ok_or_else(|| {
        CeremonyError::Unauthorized("invocation must carry an expiration".to_string())
    })?;
    let now = Timestamp::now();
    if expiration < now {
        return Err(CeremonyError::Unauthorized(
            "invocation has expired".to_string(),
        ));
    }
    if expiration > ceremony_expiration_upper_bound() {
        return Err(CeremonyError::Unauthorized(
            "invocation expiration exceeds the five-minute ceremony window plus skew allowance"
                .to_string(),
        ));
    }
    Ok(())
}

/// Parse + cryptographically verify an invocation container, require the
/// exact command, then bind it to a registered account and an active
/// device. The invocation subject is the root DID; the invocation issuer
/// must be a non-revoked device of that account.
pub async fn authorize<S: Store>(
    store: &S,
    body: &[u8],
    expected_command: &[&str],
) -> Result<Caller, CeremonyError> {
    let chain = verified_chain(body, expected_command).await?;
    require_ceremony_expiration(&chain)?;

    let account = store
        .account_by_root(chain.subject().as_ref())
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;

    let device = store
        .device_by_did(chain.issuer().as_ref())
        .await?
        .filter(|device| device.account_id == account.id && device.status == DeviceStatus::Active)
        .ok_or_else(|| {
            CeremonyError::Forbidden("device is not an active member of this account".to_string())
        })?;

    Ok(Caller {
        account,
        device,
        arguments: chain.arguments().clone(),
    })
}

/// Verify a passkey ceremony invocation signed directly by its root DID.
///
/// Root bootstrap invocations carry no delegation proof: the issuer must equal
/// the subject, so successful signature verification proves control of the
/// claimed account root. Account lookup, when required, happens only after
/// this function returns.
pub async fn authorize_root(
    body: &[u8],
    expected_command: &[&str],
) -> Result<RootCaller, CeremonyError> {
    let chain = verified_chain(body, expected_command).await?;
    if chain.issuer() != chain.subject() {
        return Err(CeremonyError::Unauthorized(
            "root invocation issuer must equal its subject".to_string(),
        ));
    }
    require_ceremony_expiration(&chain)?;

    Ok(RootCaller {
        root_did: chain.subject().to_string(),
        arguments: chain.arguments().clone(),
    })
}

/// Extract a required string from an invocation argument map.
pub fn required_string(
    arguments: &BTreeMap<String, Promised>,
    name: &str,
) -> Result<String, CeremonyError> {
    match arguments.get(name) {
        Some(Promised::String(value)) => Ok(value.clone()),
        _ => Err(CeremonyError::Invalid(format!(
            "missing or invalid argument: {name}"
        ))),
    }
}

/// Extract a required string argument.
pub fn string_argument(caller: &Caller, name: &str) -> Result<String, CeremonyError> {
    required_string(&caller.arguments, name)
}

/// The optional `revocation` argument, hex-decoded.
///
/// Absent means the caller could not mint a signed revocation — every
/// caller without the account root, today. Present but malformed is a
/// client bug worth surfacing rather than ignoring.
pub fn optional_revocation(caller: &Caller) -> Result<Option<Vec<u8>>, CeremonyError> {
    match caller.arguments.get("revocation") {
        None => Ok(None),
        Some(Promised::String(hex_bytes)) => hex::decode(hex_bytes)
            .map(Some)
            .map_err(|err| CeremonyError::Invalid(format!("bad revocation hex: {err}"))),
        Some(_) => Err(CeremonyError::Invalid(
            "revocation must be a hex string".to_string(),
        )),
    }
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationBuilder;
    use dialog_varsig::Principal;

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn container_with_expiration(
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
        expiration: Option<Timestamp>,
    ) -> (String, String, Vec<u8>) {
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let root_did = root.did();
        let chain = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let mut builder = InvocationBuilder::new()
            .issuer(device.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(command)
            .arguments(args)
            .proofs(vec![cid]);
        if let Some(expiration) = expiration {
            builder = builder.expiration(expiration);
        }
        let invocation = builder.try_build().await.unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (root_did.to_string(), device.did().to_string(), bytes)
    }

    async fn container(
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
    ) -> (String, String, Vec<u8>) {
        container_with_expiration(command, args, Some(Timestamp::five_minutes_from_now())).await
    }

    async fn seed_device(
        store: &SqliteStore,
        root_did: &str,
        device_did: &str,
        status: DeviceStatus,
    ) -> i64 {
        let account_id = store
            .create_account("a@x.com", root_did, "cred", 0)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id,
                device_did: device_did.to_string(),
                delegation_cid: "cid".to_string(),
                delegation_hex: "beef".to_string(),
                name: "laptop".to_string(),
                status,
                created_at: 0,
            })
            .await
            .unwrap();
        account_id
    }

    #[dialog_common::test]
    async fn it_authorizes_a_registered_device_acting_as_its_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        let caller = authorize(&store, &bytes, &["account", "device", "list"])
            .await
            .unwrap();
        assert_eq!(caller.account.root_did, root_did);
        assert_eq!(caller.device.device_did, device_did);
    }

    #[dialog_common::test]
    async fn it_rejects_an_unknown_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (_, _, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_revoked_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        let account_id = seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;
        store.revoke_device(account_id, &device_did).await.unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_command_mismatch() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "revoke"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_of_a_different_account() {
        let store = SqliteStore::in_memory().unwrap();

        let root_a = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let root_b = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let device_b = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();

        let root_a_did = root_a.did();
        let root_b_did = root_b.did();
        let device_b_did = device_b.did();

        store
            .create_account("a@x.com", root_a_did.as_ref(), "cred-a", 0)
            .await
            .unwrap();
        let account_b = store
            .create_account("b@x.com", root_b_did.as_ref(), "cred-b", 0)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: account_b,
                device_did: device_b_did.to_string(),
                delegation_cid: "cid".to_string(),
                delegation_hex: "beef".to_string(),
                name: "phone".to_string(),
                status: DeviceStatus::Active,
                created_at: 0,
            })
            .await
            .unwrap();

        // The delegation is legitimately root B -> device B, but the
        // invocation claims subject = root A: the chain from subject to
        // issuer breaks, so verification itself must fail.
        let chain = tonk_identity::delegation::mint_device_delegation(root_b, &device_b_did)
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device_b.clone())
            .audience(&root_a_did)
            .subject(&root_a_did)
            .command(vec!["account".into(), "device".into(), "list".into()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_registered_device_presenting_another_accounts_root() {
        let store = SqliteStore::in_memory().unwrap();

        // The device is registered under account A…
        let root_a = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let root_a_did = root_a.did();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let device_did = device.did();
        seed_device(
            &store,
            root_a_did.as_ref(),
            device_did.as_ref(),
            DeviceStatus::Active,
        )
        .await;

        // …but invokes as a delegate of account B, with a chain that
        // verifies: root B really did delegate to this device key.
        let root_b = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let root_b_did = root_b.did();
        store
            .create_account("b@x.com", root_b_did.as_ref(), "cred-b", 0)
            .await
            .unwrap();

        let chain = tonk_identity::delegation::mint_device_delegation(root_b, &device_did)
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device)
            .audience(&root_b_did)
            .subject(&root_b_did)
            .command(vec!["account".into(), "device".into(), "list".into()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_invocation_without_expiration() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container_with_expiration(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
            None,
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        match authorize(&store, &bytes, &["account", "device", "list"]).await {
            Err(CeremonyError::Unauthorized(msg)) => {
                assert!(
                    msg.contains("must carry an expiration"),
                    "unexpected message: {msg}"
                );
            }
            Err(other) => panic!("expected Unauthorized, got a different error: {other:?}"),
            Ok(_) => panic!("expected Unauthorized, got Ok"),
        }
    }

    #[dialog_common::test]
    async fn it_rejects_an_expired_device_invocation() {
        use std::time::{Duration, UNIX_EPOCH};

        let store = SqliteStore::in_memory().unwrap();
        let expired = Timestamp::new(UNIX_EPOCH + Duration::from_secs(1)).unwrap();
        let (root_did, device_did, bytes) = container_with_expiration(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
            Some(expired),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        match authorize(&store, &bytes, &["account", "device", "list"]).await {
            Err(CeremonyError::Unauthorized(msg)) => {
                assert!(msg.contains("has expired"), "unexpected message: {msg}");
            }
            Err(other) => panic!("expected Unauthorized, got a different error: {other:?}"),
            Ok(_) => panic!("expected Unauthorized, got Ok"),
        }
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_invocation_expiring_beyond_the_ceremony_window() {
        // Ten minutes comfortably clears the five-minute window plus the
        // one-minute skew allowance, so this must still trip the bound.
        let too_far_out = Timestamp::new(SystemTime::now() + Duration::from_secs(10 * 60)).unwrap();
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container_with_expiration(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
            Some(too_far_out),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        match authorize(&store, &bytes, &["account", "device", "list"]).await {
            Err(CeremonyError::Unauthorized(msg)) => {
                assert!(
                    msg.contains("exceeds the five-minute ceremony window"),
                    "unexpected message: {msg}"
                );
            }
            Err(other) => panic!("expected Unauthorized, got a different error: {other:?}"),
            Ok(_) => panic!("expected Unauthorized, got Ok"),
        }
    }

    #[dialog_common::test]
    async fn it_authorizes_a_request_signed_directly_by_the_root() {
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let expected_root = root.did().to_string();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let ceremony = tonk_identity::ceremony::link_device(root, device.did(), "phone".into())
            .await
            .unwrap();
        let bytes = hex::decode(ceremony.invocation_hex).unwrap();

        let caller = authorize_root(&bytes, &["account", "device", "link"])
            .await
            .unwrap();
        assert_eq!(caller.root_did, expected_root);
        assert_eq!(
            required_string(&caller.arguments, "deviceName").unwrap(),
            "phone"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_root_command_mismatch() {
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let ceremony = tonk_identity::ceremony::link_device(root, device.did(), "phone".into())
            .await
            .unwrap();
        let bytes = hex::decode(ceremony.invocation_hex).unwrap();

        assert!(matches!(
            authorize_root(&bytes, &["account", "create"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_an_expired_root_invocation() {
        use std::time::{Duration, UNIX_EPOCH};

        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let root_did = root.did();
        let expiration = Timestamp::new(UNIX_EPOCH + Duration::from_secs(1)).unwrap();
        let invocation = InvocationBuilder::new()
            .issuer(root)
            .audience(&root_did)
            .subject(&root_did)
            .command(vec!["account".into(), "device".into(), "link".into()])
            .arguments(BTreeMap::new())
            .proofs(vec![])
            .expiration(expiration)
            .try_build()
            .await
            .unwrap();
        let bytes = InvocationChain::new(invocation, std::collections::HashMap::new())
            .to_bytes()
            .unwrap();

        assert!(matches!(
            authorize_root(&bytes, &["account", "device", "link"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_treats_an_absent_revocation_as_none() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "revoke".into()],
            BTreeMap::new(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;
        let caller = authorize(&store, &bytes, &["account", "device", "revoke"])
            .await
            .unwrap();

        assert_eq!(optional_revocation(&caller).unwrap(), None);
    }

    /// Present-but-not-a-string is a malformed request, not an absent
    /// argument — silently dropping it would read as "no artifact" and
    /// change which authority rule applies.
    #[dialog_common::test]
    async fn it_rejects_a_revocation_that_is_not_a_string() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "revoke".into()],
            [("revocation".to_owned(), Promised::Integer(7))]
                .into_iter()
                .collect(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;
        let caller = authorize(&store, &bytes, &["account", "device", "revoke"])
            .await
            .unwrap();

        assert!(matches!(
            optional_revocation(&caller),
            Err(CeremonyError::Invalid(_))
        ));
    }
}

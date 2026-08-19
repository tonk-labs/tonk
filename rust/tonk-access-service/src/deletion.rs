//! Capability-authorized, denial-first hosted-space deletion.

use async_trait::async_trait;
use dialog_credentials::Ed25519KeyResolver;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::{Container, Delegation, Invocation, InvocationChain};
use dialog_varsig::Did;
use dialog_varsig::algorithm::eddsa::Ed25519Signature;
use serde::{Deserialize, Serialize};

use crate::store::{Consumer, ConsumerDeletionState, DeletionGrantKind, Store};

/// Exact destructive command recognized at the UCAN endpoint.
pub const COMMAND: [&str; 2] = ["space", "delete"];
/// Root-authenticated inventory command used before a destructive ceremony.
pub const CUSTOMER_PLAN_COMMAND: [&str; 3] = ["customer", "deletion", "plan"];
/// Root-authenticated access-service customer finalization command.
pub const CUSTOMER_DELETE_COMMAND: [&str; 2] = ["customer", "delete"];

/// Successful, idempotent hosted-space deletion receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub space: Did,
    pub state: String,
    pub deleted_at: u64,
}

/// One hosted space the access service associates with the deleting account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedSpace {
    pub space: String,
    pub deletion_ready: bool,
    pub deletion_kind: Option<String>,
    pub deletion_state: String,
}

/// Authoritative access-service inventory. `owner` discovers candidates; it
/// never substitutes for each row's registered destructive proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerDeletionPlan {
    pub customer: String,
    pub spaces: Vec<HostedSpace>,
}

/// Successful removal of access-service customer state and its email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerDeletionReceipt {
    pub customer: String,
    pub state: String,
}

/// Stable deletion refusal returned by Worker and native helper routes.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "code")]
pub enum Error {
    #[error("deletion invocation is malformed: {0}")]
    Malformed(String),
    #[error("deletion invocation failed cryptographic verification: {0}")]
    Unauthorized(String),
    #[error("the hosted space has no registered deletion authority")]
    NotRegistered,
    #[error("the invocation does not present the registered deletion proof")]
    WrongGrant,
    #[error("registered deletion authority does not have the required direct shape")]
    Forbidden,
    #[error("hosted-space deletion is temporarily incomplete: {0}")]
    Incomplete(String),
    #[error("owned hosted spaces must be deleted first: {0:?}")]
    OwnedSpacesRemain(Vec<String>),
    #[error("hosted-space deletion failed internally: {0}")]
    Internal(String),
}

impl Error {
    pub fn status(&self) -> u16 {
        match self {
            Self::Malformed(_) => 400,
            Self::Unauthorized(_) => 401,
            Self::NotRegistered => 404,
            Self::WrongGrant | Self::Forbidden => 403,
            Self::Incomplete(_) => 503,
            Self::OwnedSpacesRemain(_) => 409,
            Self::Internal(_) => 500,
        }
    }
}

/// Storage namespace removal, abstracted so lifecycle behavior is unit-testable.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait SpacePurger {
    async fn purge(&self, prefix: &str) -> Result<(), String>;
}

/// Return true only for the exact destructive command.
pub fn is_deletion(container: &[u8]) -> bool {
    let Ok(tokens) = Container::from_bytes(container).map(Container::into_tokens) else {
        return false;
    };
    let Some(first) = tokens.first() else {
        return false;
    };
    let Ok(invocation) = serde_ipld_dagcbor::from_slice::<Invocation<Ed25519Signature>>(first)
    else {
        return false;
    };
    invocation.command().0 == COMMAND.map(str::to_string)
}

/// Whether a container names either root-authenticated customer deletion command.
pub fn is_customer_deletion(container: &[u8]) -> bool {
    command(container).is_some_and(|command| {
        command == CUSTOMER_PLAN_COMMAND.map(str::to_string)
            || command == CUSTOMER_DELETE_COMMAND.map(str::to_string)
    })
}

fn command(container: &[u8]) -> Option<Vec<String>> {
    let tokens = Container::from_bytes(container).ok()?.into_tokens();
    let invocation =
        serde_ipld_dagcbor::from_slice::<Invocation<Ed25519Signature>>(tokens.first()?).ok()?;
    Some(invocation.command().0.clone())
}

/// Parsed command for the thin HTTP adapter; authority is never inferred here.
#[cfg(target_arch = "wasm32")]
pub(crate) fn command_for_handler(container: &[u8]) -> Vec<String> {
    command(container).unwrap_or_default()
}

/// Parsed command for the native HTTP adapter.
#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub(crate) fn command_for_native_handler(container: &[u8]) -> Vec<String> {
    command(container).unwrap_or_default()
}

/// Subject named before authorization, used only to locate the row whose
/// registered proof is then checked cryptographically.
pub fn subject(container: &[u8]) -> Option<Did> {
    InvocationChain::try_from(container)
        .ok()
        .map(|chain| chain.subject().clone())
}

/// Delete one hosted space. State flips to deleting before object removal,
/// so stale replicas cannot resurrect a partial purge.
pub async fn delete<S: Store, P: SpacePurger>(
    store: &S,
    purger: &P,
    container: &[u8],
    now: u64,
) -> Result<Receipt, Error> {
    let space = subject(container).ok_or_else(|| Error::Malformed("missing subject".into()))?;
    let consumer = store
        .consumer(space.as_str())
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .ok_or(Error::NotRegistered)?;
    verify(container, &consumer, now).await?;

    if consumer.deletion_state == ConsumerDeletionState::Deleted {
        return Ok(receipt(&space, consumer.deleted_at.unwrap_or_default()));
    }
    if consumer.deletion_state == ConsumerDeletionState::Active {
        let cid = consumer
            .deletion_grant_cid
            .as_deref()
            .ok_or(Error::NotRegistered)?;
        if !store
            .mark_consumer_deleting(space.as_str(), cid)
            .await
            .map_err(|error| Error::Internal(error.to_string()))?
        {
            let current = store
                .consumer(space.as_str())
                .await
                .map_err(|error| Error::Internal(error.to_string()))?
                .ok_or(Error::NotRegistered)?;
            if current.deletion_state == ConsumerDeletionState::Deleted {
                return Ok(receipt(&space, current.deleted_at.unwrap_or_default()));
            }
            if current.deletion_state != ConsumerDeletionState::Deleting {
                return Err(Error::Internal(
                    "could not enter deletion denial state".into(),
                ));
            }
        }
    }

    purger
        .purge(&format!("{space}/"))
        .await
        .map_err(Error::Incomplete)?;
    if !store
        .finish_consumer_deletion(space.as_str(), now)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
    {
        let current = store
            .consumer(space.as_str())
            .await
            .map_err(|error| Error::Internal(error.to_string()))?
            .ok_or(Error::NotRegistered)?;
        if current.deletion_state != ConsumerDeletionState::Deleted {
            return Err(Error::Internal("could not finalize deletion state".into()));
        }
        return Ok(receipt(&space, current.deleted_at.unwrap_or_default()));
    }
    Ok(receipt(&space, now))
}

/// List every non-account consumer originally associated with this customer.
pub async fn customer_plan<S: Store>(
    store: &S,
    container: &[u8],
    now: u64,
) -> Result<CustomerDeletionPlan, Error> {
    let customer =
        verify_customer_command(store, container, &CUSTOMER_PLAN_COMMAND, now, true).await?;
    let spaces = store
        .consumers_by_owner(customer.as_str())
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .into_iter()
        .filter(|consumer| consumer.did != customer)
        .map(|consumer| HostedSpace {
            space: consumer.did,
            deletion_ready: consumer.deletion_grant_cid.is_some()
                && consumer.deletion_grant_kind.is_some(),
            deletion_kind: consumer
                .deletion_grant_kind
                .map(|kind| kind.as_str().to_string()),
            deletion_state: consumer.deletion_state.as_str().to_string(),
        })
        .collect();
    Ok(CustomerDeletionPlan { customer, spaces })
}

/// Purge the account space and remove access-service customer state only after
/// every other owned hosted space has completed its capability-authorized purge.
pub async fn delete_customer<S: Store, P: SpacePurger>(
    store: &S,
    purger: &P,
    container: &[u8],
    now: u64,
) -> Result<CustomerDeletionReceipt, Error> {
    let customer =
        verify_customer_command(store, container, &CUSTOMER_DELETE_COMMAND, now, false).await?;
    let remaining: Vec<_> = store
        .consumers_by_owner(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .into_iter()
        .filter(|consumer| {
            consumer.did != customer && consumer.deletion_state != ConsumerDeletionState::Deleted
        })
        .map(|consumer| consumer.did)
        .collect();
    if !remaining.is_empty() {
        return Err(Error::OwnedSpacesRemain(remaining));
    }

    let self_consumer = store
        .consumer(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .ok_or(Error::NotRegistered)?;
    if self_consumer.deletion_state == ConsumerDeletionState::Active
        && !store
            .mark_self_consumer_deleting(&customer)
            .await
            .map_err(|error| Error::Internal(error.to_string()))?
    {
        return Err(Error::Internal(
            "could not deny the account-space consumer".into(),
        ));
    }
    purger
        .purge(&format!("{customer}/"))
        .await
        .map_err(Error::Incomplete)?;
    if !store
        .delete_customer(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
    {
        return Err(Error::Internal(
            "could not remove access-service customer state".into(),
        ));
    }
    let _ = now;
    Ok(CustomerDeletionReceipt {
        customer,
        state: "deleted".into(),
    })
}

async fn verify_customer_command<S: Store, const N: usize>(
    store: &S,
    container: &[u8],
    expected: &[&str; N],
    now: u64,
    allow_device: bool,
) -> Result<String, Error> {
    let chain = InvocationChain::try_from(container)
        .map_err(|error| Error::Malformed(error.to_string()))?;
    chain
        .verify(&Ed25519KeyResolver)
        .await
        .map_err(|error| Error::Unauthorized(error.to_string()))?;
    if chain.command().0 != expected.map(str::to_string) {
        return Err(Error::Forbidden);
    }
    let expiration = chain
        .invocation
        .expiration()
        .ok_or_else(|| Error::Unauthorized("invocation must expire".into()))?;
    if expiration.to_unix() < now {
        return Err(Error::Unauthorized("invocation has expired".into()));
    }
    let customer = chain.subject().to_string();
    if store
        .customer(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .is_none()
    {
        return Err(Error::NotRegistered);
    }
    if chain.issuer() == chain.subject() {
        if !chain.proofs().is_empty() {
            return Err(Error::Forbidden);
        }
    } else {
        if !allow_device || chain.proofs().len() != 1 {
            return Err(Error::Forbidden);
        }
        let cid = chain.proofs()[0].to_string();
        let proof = deposited_proof(container, &cid)?;
        let customer_did: Did = customer.parse().map_err(|_| Error::Forbidden)?;
        if proof.issuer() != &customer_did
            || proof.audience() != chain.issuer()
            || proof.subject() != &Subject::Any
            || !proof.command().0.is_empty()
        {
            return Err(Error::Forbidden);
        }
    }
    Ok(customer)
}

fn receipt(space: &Did, deleted_at: u64) -> Receipt {
    Receipt {
        space: space.clone(),
        state: "deleted".to_string(),
        deleted_at,
    }
}

async fn verify(container: &[u8], consumer: &Consumer, now: u64) -> Result<(), Error> {
    let chain = InvocationChain::try_from(container)
        .map_err(|error| Error::Malformed(error.to_string()))?;
    chain
        .verify(&Ed25519KeyResolver)
        .await
        .map_err(|error| Error::Unauthorized(error.to_string()))?;
    let space: Did = consumer
        .did
        .parse()
        .map_err(|_| Error::Internal("stored consumer DID is invalid".into()))?;
    if chain.subject() != &space || chain.command().0 != COMMAND.map(str::to_string) {
        return Err(Error::Forbidden);
    }
    let expiration = chain
        .invocation
        .expiration()
        .ok_or_else(|| Error::Unauthorized("invocation must expire".into()))?;
    if expiration.to_unix() < now {
        return Err(Error::Unauthorized("invocation has expired".into()));
    }
    if chain.proofs().len() != 1 {
        return Err(Error::Forbidden);
    }
    let registered = consumer
        .deletion_grant_cid
        .as_deref()
        .ok_or(Error::NotRegistered)?;
    if chain.proofs()[0].to_string() != registered {
        return Err(Error::WrongGrant);
    }
    let proof = deposited_proof(container, registered)?;
    if proof.issuer() != &space
        || proof.audience() != chain.issuer()
        || proof.subject() != &Subject::Specific(space)
    {
        return Err(Error::Forbidden);
    }
    match consumer.deletion_grant_kind.ok_or(Error::NotRegistered)? {
        DeletionGrantKind::Exact if proof.command().0 == COMMAND.map(str::to_string) => Ok(()),
        DeletionGrantKind::LegacyDirect if proof.command().0.is_empty() => Ok(()),
        _ => Err(Error::Forbidden),
    }
}

fn deposited_proof(
    container: &[u8],
    registered: &str,
) -> Result<Delegation<Ed25519Signature>, Error> {
    Container::from_bytes(container)
        .map_err(|error| Error::Malformed(error.to_string()))?
        .into_tokens()
        .into_iter()
        .skip(1)
        .filter_map(|bytes| {
            serde_ipld_dagcbor::from_slice::<Delegation<Ed25519Signature>>(&bytes).ok()
        })
        .find(|proof| proof.to_cid().to_string() == registered)
        .ok_or(Error::WrongGrant)
}

/// Production R2 namespace purger.
#[cfg(target_arch = "wasm32")]
pub struct R2SpacePurger(worker::Bucket);

#[cfg(target_arch = "wasm32")]
impl R2SpacePurger {
    pub fn new(bucket: worker::Bucket) -> Self {
        Self(bucket)
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl SpacePurger for R2SpacePurger {
    async fn purge(&self, prefix: &str) -> Result<(), String> {
        loop {
            let listed = self
                .0
                .list()
                .prefix(prefix.to_string())
                .limit(1000)
                .execute()
                .await
                .map_err(|error| error.to_string())?;
            let keys: Vec<String> = listed
                .objects()
                .into_iter()
                .map(|object| object.key())
                .collect();
            if keys.is_empty() {
                return Ok(());
            }
            self.0
                .delete_multiple(keys)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
}

/// S3-compatible purger used by the native helper service.
#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub struct NativeSpacePurger {
    address: dialog_remote_s3::Address,
    credential: dialog_remote_s3::s3::S3Credential,
}

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
impl NativeSpacePurger {
    pub fn new(
        address: dialog_remote_s3::Address,
        credential: dialog_remote_s3::s3::S3Credential,
    ) -> Self {
        Self {
            address,
            credential,
        }
    }

    async fn send(
        &self,
        request: dialog_remote_s3::request::S3Request,
    ) -> Result<reqwest::Response, String> {
        let permit = request
            .attest(self.credential.clone())
            .redeem(&self.address)
            .await
            .map_err(|error| error.to_string())?;
        permit
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())
    }
}

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
#[async_trait]
impl SpacePurger for NativeSpacePurger {
    async fn purge(&self, prefix: &str) -> Result<(), String> {
        loop {
            let response = self
                .send(dialog_remote_s3::request::S3Request {
                    method: "GET".to_string(),
                    path: "/".to_string(),
                    params: Some(vec![
                        ("list-type".to_string(), "2".to_string()),
                        ("prefix".to_string(), prefix.to_string()),
                        ("max-keys".to_string(), "1000".to_string()),
                    ]),
                    ..Default::default()
                })
                .await?;
            let xml = response.text().await.map_err(|error| error.to_string())?;
            let keys = xml_values(&xml, "Key");
            if keys.is_empty() {
                return Ok(());
            }
            for key in keys {
                self.send(dialog_remote_s3::request::S3Request {
                    method: "DELETE".to_string(),
                    path: key,
                    ..Default::default()
                })
                .await?;
            }
        }
    }
}

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
fn xml_values(xml: &str, element: &str) -> Vec<String> {
    let open = format!("<{element}>");
    let close = format!("</{element}>");
    let mut values = Vec::new();
    let mut remaining = xml;
    while let Some(start) = remaining.find(&open) {
        let value = &remaining[start + open.len()..];
        let Some(end) = value.find(&close) else {
            break;
        };
        values.push(value[..end].to_string());
        remaining = &value[end + close.len()..];
    }
    values
}

#[cfg(all(test, not(target_arch = "wasm32"), feature = "helpers"))]
mod tests {
    use std::sync::Mutex;

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationBuilder;
    use dialog_ucan_core::time::timestamp::Timestamp;
    use dialog_varsig::Principal as _;
    use tonk_account::deletion::{build_deletion_invocation, mint_deletion_grant};

    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{SIGNUP_PLAN, Store};

    struct FlakyPurger {
        fail_once: Mutex<bool>,
        prefixes: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SpacePurger for FlakyPurger {
        async fn purge(&self, prefix: &str) -> Result<(), String> {
            self.prefixes.lock().unwrap().push(prefix.to_string());
            let mut fail = self.fail_once.lock().unwrap();
            if *fail {
                *fail = false;
                return Err("injected R2 failure".to_string());
            }
            Ok(())
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    async fn root_container(root: Ed25519Signer, command: &[&str]) -> Vec<u8> {
        let did = root.did();
        let invocation = InvocationBuilder::new()
            .issuer(root)
            .audience(&did)
            .subject(&did)
            .command(
                command
                    .iter()
                    .map(|segment| (*segment).to_string())
                    .collect(),
            )
            .proofs(vec![])
            .issue_now()
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        InvocationChain::new(invocation, Default::default())
            .to_bytes()
            .unwrap()
    }

    struct RecordingPurger(Mutex<Vec<String>>);

    #[async_trait]
    impl SpacePurger for RecordingPurger {
        async fn purge(&self, prefix: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(prefix.to_string());
            Ok(())
        }
    }

    #[dialog_common::test]
    async fn denial_precedes_purge_and_retry_finishes_idempotently() {
        let store = SqliteStore::in_memory().unwrap();
        let root = Ed25519Signer::import(&[71; 32]).await.unwrap();
        let space = Ed25519Signer::import(&[72; 32]).await.unwrap();
        let grant = mint_deletion_grant(&space, &root.did()).await.unwrap();
        let cid = grant.proof_cids()[0].to_string();
        let at = now();
        store
            .enroll_customer(
                root.did().as_str(),
                "owner@example.com",
                b"access",
                SIGNUP_PLAN,
                at,
            )
            .await
            .unwrap();
        store
            .add_consumer(
                space.did().as_str(),
                root.did().as_str(),
                at,
                Some(&cid),
                Some(DeletionGrantKind::Exact),
            )
            .await
            .unwrap();
        let invocation = build_deletion_invocation(root.clone(), &grant)
            .await
            .unwrap();
        let purger = FlakyPurger {
            fail_once: Mutex::new(true),
            prefixes: Mutex::new(Vec::new()),
        };

        assert!(matches!(
            delete(&store, &purger, &invocation, at + 1).await,
            Err(Error::Incomplete(_))
        ));
        let denied = store.consumer(space.did().as_str()).await.unwrap().unwrap();
        assert_eq!(denied.deletion_state, ConsumerDeletionState::Deleting);
        assert_eq!(denied.provider.as_deref(), Some(root.did().as_str()));

        let receipt = delete(&store, &purger, &invocation, at + 2).await.unwrap();
        assert_eq!(receipt.space, space.did());
        let deleted = store.consumer(space.did().as_str()).await.unwrap().unwrap();
        assert_eq!(deleted.deletion_state, ConsumerDeletionState::Deleted);
        assert!(deleted.provider.is_none());

        let replay = delete(&store, &purger, &invocation, at + 3).await.unwrap();
        assert_eq!(replay.deleted_at, receipt.deleted_at);
        assert_eq!(
            purger.prefixes.lock().unwrap().as_slice(),
            &[format!("{}/", space.did()), format!("{}/", space.did())]
        );
    }

    #[dialog_common::test]
    async fn customer_finalization_requires_every_owned_space_then_removes_email_state() {
        let store = SqliteStore::in_memory().unwrap();
        let root = Ed25519Signer::import(&[81; 32]).await.unwrap();
        let space = Ed25519Signer::import(&[82; 32]).await.unwrap();
        let at = now();
        store
            .enroll_customer(
                root.did().as_str(),
                "delete@example.com",
                b"access",
                SIGNUP_PLAN,
                at,
            )
            .await
            .unwrap();
        let grant = mint_deletion_grant(&space, &root.did()).await.unwrap();
        store
            .add_consumer(
                space.did().as_str(),
                root.did().as_str(),
                at,
                Some(&grant.proof_cids()[0].to_string()),
                Some(DeletionGrantKind::Exact),
            )
            .await
            .unwrap();
        let device = Ed25519Signer::import(&[83; 32]).await.unwrap();
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let plan_invocation = tonk_identity::request::build_device_invocation(
            device,
            &link,
            CUSTOMER_PLAN_COMMAND.map(str::to_string).to_vec(),
            Default::default(),
        )
        .await
        .unwrap();
        let plan = customer_plan(&store, &plan_invocation, at).await.unwrap();
        assert_eq!(plan.spaces.len(), 1);
        assert_eq!(plan.spaces[0].space, space.did().to_string());
        assert!(plan.spaces[0].deletion_ready);

        let customer_invocation = root_container(root.clone(), &CUSTOMER_DELETE_COMMAND).await;
        let purger = RecordingPurger(Mutex::new(Vec::new()));
        assert!(matches!(
            delete_customer(&store, &purger, &customer_invocation, at + 1).await,
            Err(Error::OwnedSpacesRemain(spaces)) if spaces == vec![space.did().to_string()]
        ));
        assert!(store.customer(root.did().as_str()).await.unwrap().is_some());

        let space_invocation = build_deletion_invocation(root.clone(), &grant)
            .await
            .unwrap();
        delete(&store, &purger, &space_invocation, at + 2)
            .await
            .unwrap();
        let receipt = delete_customer(&store, &purger, &customer_invocation, at + 3)
            .await
            .unwrap();
        assert_eq!(receipt.customer, root.did().to_string());
        assert!(store.customer(root.did().as_str()).await.unwrap().is_none());
        assert!(
            store
                .consumers_by_owner(root.did().as_str())
                .await
                .unwrap()
                .is_empty()
        );
        let denial = store.consumer(space.did().as_str()).await.unwrap().unwrap();
        assert_eq!(denial.deletion_state, ConsumerDeletionState::Deleted);
        assert!(denial.provider.is_none());
        assert!(denial.owner.is_none());
        assert!(denial.deletion_grant_cid.is_none());
        assert!(denial.deletion_grant_kind.is_none());
        assert!(
            !store
                .add_consumer(
                    space.did().as_str(),
                    root.did().as_str(),
                    at + 4,
                    Some(&grant.proof_cids()[0].to_string()),
                    Some(DeletionGrantKind::Exact),
                )
                .await
                .unwrap()
        );
        assert_eq!(
            store
                .consumer(space.did().as_str())
                .await
                .unwrap()
                .unwrap()
                .deletion_state,
            ConsumerDeletionState::Deleted
        );
        assert_eq!(
            purger.0.lock().unwrap().as_slice(),
            &[format!("{}/", space.did()), format!("{}/", root.did())]
        );
    }
}

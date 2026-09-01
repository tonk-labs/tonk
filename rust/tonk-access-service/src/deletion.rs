//! Denial-first hosted-space deletion as deprovisioning.
//!
//! Deleting a hosted space is the owning customer ending its hosting
//! relationship — the reverse of `/provider/add` — not an operation on
//! the space. The invocation's subject is the CUSTOMER, authorized by
//! the customer's own chain (root-signed, or device-signed through the
//! root's delegation), so no chain rooted in the space — invites
//! included — can ever reach it. No per-space deletion artifact is
//! registered or presented.

use async_trait::async_trait;
use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::revocation::UnverifiedRevocations;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::verification::{Environment, VerificationContext};
use dialog_ucan_core::{Container, Delegation, Invocation, InvocationChain};
use dialog_varsig::AnySignature;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

use crate::store::{Store, SubscriptionKind};

/// The deprovisioning command: the reverse of `/provider/add`.
pub const COMMAND: [&str; 2] = ["provider", "remove"];
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
    /// When deletion began, if a purge is already in flight. A finished
    /// deletion takes the row with it, so such a space is simply absent
    /// from the plan rather than listed as gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleting_since: Option<u64>,
}

/// Authoritative access-service inventory of the customer's owned
/// hosted spaces.
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
///
/// Serialized as `{"code": …, "message": …}` via a manual impl:
/// an internally-tagged serde enum cannot represent newtype variants
/// over strings — it panics at serialization time — which turned every
/// refusal on the native helper into a dropped connection instead of
/// an error body.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("deletion invocation is malformed: {0}")]
    Malformed(String),
    #[error("deletion invocation failed cryptographic verification: {0}")]
    Unauthorized(String),
    #[error("the space is not hosted here")]
    NotRegistered,
    #[error("the invocation does not carry the required proof")]
    WrongGrant,
    #[error("only the owning customer may deprovision a hosted space")]
    Forbidden,
    #[error("hosted-space deletion is temporarily incomplete: {0}")]
    Incomplete(String),
    #[error("owned hosted spaces must be deleted first: {0:?}")]
    OwnedSpacesRemain(Vec<String>),
    #[error("hosted-space deletion failed internally: {0}")]
    Internal(String),
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut record = serializer.serialize_struct("Error", 2)?;
        record.serialize_field("code", self.code())?;
        record.serialize_field("message", &self.to_string())?;
        record.end()
    }
}

impl Error {
    /// Stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "Malformed",
            Self::Unauthorized(_) => "Unauthorized",
            Self::NotRegistered => "NotRegistered",
            Self::WrongGrant => "WrongGrant",
            Self::Forbidden => "Forbidden",
            Self::Incomplete(_) => "Incomplete",
            Self::OwnedSpacesRemain(_) => "OwnedSpacesRemain",
            Self::Internal(_) => "Internal",
        }
    }

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

/// Return true only for the deprovisioning command.
pub fn is_deletion(container: &[u8]) -> bool {
    let Ok(tokens) = Container::from_bytes(container).map(Container::into_tokens) else {
        return false;
    };
    let Some(first) = tokens.first() else {
        return false;
    };
    let Ok(invocation) = serde_ipld_dagcbor::from_slice::<Invocation<AnySignature>>(first) else {
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
        serde_ipld_dagcbor::from_slice::<Invocation<AnySignature>>(tokens.first()?).ok()?;
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

/// Deprovision one hosted space. State flips to deleting before object
/// removal, so stale replicas cannot resurrect a partial purge.
pub async fn delete<S: Store, P: SpacePurger>(
    store: &S,
    purger: &P,
    container: &[u8],
    now: u64,
) -> Result<Receipt, Error> {
    let customer = verify_customer_command(store, container, &COMMAND, now, true).await?;
    let space = consumer_argument(container)?;
    if space.as_str() == customer {
        // The customer's own account space is finalized through
        // `/customer/delete`, after every other owned space is gone.
        return Err(Error::Forbidden);
    }
    // No row is the finished state: deletion takes it with the data, so
    // a retry after one completed lands here. Idempotent-successful,
    // reporting this attempt's clock — the original moment went with the
    // row, and nothing reads the field.
    let Some(consumer) = store
        .consumer(space.as_str())
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
    else {
        return Ok(receipt(&space, now));
    };
    if consumer.provider != customer.as_str() {
        return Err(Error::Forbidden);
    }
    if consumer.kind == SubscriptionKind::Custody {
        // Custody namespaces hold the account's own sealed key
        // material. They are purged by customer finalization, last —
        // deprovisioning one earlier would destroy the account's
        // ability to retry anything that still needs a passkey.
        return Err(Error::Forbidden);
    }

    // Stop serving before any object goes: purging is neither atomic nor
    // certain to finish in one attempt, and a client must not read a
    // half-purged space. The write is a compare-and-set, so a second
    // request arriving now finds the deletion already begun and joins it
    // rather than starting its own.
    let began = match consumer.deleted_at {
        Some(began) => began,
        None => {
            store
                .mark_consumer_deleting(space.as_str(), now)
                .await
                .map_err(|error| Error::Internal(error.to_string()))?;
            // Whoever won the race, the row now carries the moment
            // deletion began, and the receipt reports that rather than
            // this attempt's clock.
            store
                .consumer(space.as_str())
                .await
                .map_err(|error| Error::Internal(error.to_string()))?
                .and_then(|current| current.deleted_at)
                .unwrap_or(now)
        }
    };

    purger
        .purge(&format!("{space}/"))
        .await
        .map_err(Error::Incomplete)?;
    // The row goes with the data. A retry after this finds no row and is
    // answered by the caller above as already deleted.
    store
        .finish_consumer_deletion(space.as_str())
        .await
        .map_err(|error| Error::Internal(error.to_string()))?;
    Ok(receipt(&space, began))
}

/// The `consumer` argument naming which hosted space to deprovision.
fn consumer_argument(container: &[u8]) -> Result<Did, Error> {
    use dialog_ucan_core::promise::Promised;

    let chain = InvocationChain::try_from(container)
        .map_err(|error| Error::Malformed(error.to_string()))?;
    match chain.arguments().get("consumer") {
        Some(Promised::String(did)) => did
            .parse()
            .map_err(|_| Error::Malformed("consumer argument is not a DID".into())),
        _ => Err(Error::Malformed("missing consumer argument".into())),
    }
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
        .subscriptions_by_provider(customer.as_str())
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        .into_iter()
        .filter(|consumer| {
            consumer.consumer != customer && consumer.kind == SubscriptionKind::Space
        })
        .map(|consumer| HostedSpace {
            space: consumer.consumer,
            deleting_since: consumer.deleted_at,
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
        match verify_customer_command(store, container, &CUSTOMER_DELETE_COMMAND, now, true).await {
            Ok(customer) => customer,
            // No customer row is the finished state: finalization takes
            // it with the data, so a repeat lands here. The worker's
            // deletion sequence repeats deliberately when the account
            // and access services share a deployment — both halves post
            // the same finalization — and the second half must join the
            // first rather than 404 a deletion that succeeded. The
            // refusal escapes `verify_customer_command` only after the
            // chain verified cryptographically, so this answers no one
            // but the (former) customer.
            Err(Error::NotRegistered) => {
                let customer = subject(container)
                    .ok_or_else(|| Error::Malformed("invocation names no subject".into()))?;
                return Ok(CustomerDeletionReceipt {
                    customer: customer.to_string(),
                    state: "deleted".into(),
                });
            }
            Err(error) => return Err(error),
        };
    let consumers = store
        .subscriptions_by_provider(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?;
    let remaining: Vec<_> = consumers
        .iter()
        .filter(|consumer| {
            consumer.consumer != customer && consumer.kind == SubscriptionKind::Space
        })
        .map(|consumer| consumer.consumer.clone())
        .collect();
    if !remaining.is_empty() {
        return Err(Error::OwnedSpacesRemain(remaining));
    }

    // Custody namespaces are purged here — after every data space is
    // gone and the finalization is already authorized, so nothing that
    // could still need the account's key material can be left waiting
    // on it. Denial-first like any other purge, and idempotent.
    for consumer in &consumers {
        if consumer.kind != SubscriptionKind::Custody {
            continue;
        }
        if consumer.deleted_at.is_none() {
            let _ = store
                .mark_consumer_deleting(&consumer.consumer, now)
                .await
                .map_err(|error| Error::Internal(error.to_string()))?;
        }
        purger
            .purge(&format!("{}/", consumer.consumer))
            .await
            .map_err(Error::Incomplete)?;
        let _ = store
            .finish_consumer_deletion(&consumer.consumer)
            .await
            .map_err(|error| Error::Internal(error.to_string()))?;
    }

    // An absent self consumer is a finalization already past this
    // point (the row goes before the customer row does): join it and
    // finish the rest, rather than 404 a retry of a deletion that is
    // mostly done.
    if let Some(self_consumer) = store
        .consumer(&customer)
        .await
        .map_err(|error| Error::Internal(error.to_string()))?
        && self_consumer.deleted_at.is_none()
        && !store
            .mark_self_consumer_deleting(&customer, now)
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
        .verify(&VerificationContext::new(&Environment::new(
            chain.proof_store(),
            DidKeyResolver,
            UnverifiedRevocations,
        )))
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

fn deposited_proof(container: &[u8], registered: &str) -> Result<Delegation<AnySignature>, Error> {
    Container::from_bytes(container)
        .map_err(|error| Error::Malformed(error.to_string()))?
        .into_tokens()
        .into_iter()
        .skip(1)
        .filter_map(|bytes| serde_ipld_dagcbor::from_slice::<Delegation<AnySignature>>(&bytes).ok())
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
            // Path-style resolution appends this to "{bucket}/", so the
            // bucket listing is the empty path — "/" would produce
            // "{bucket}//", which S3 reads as an object named "/".
            let response = self
                .send(dialog_remote_s3::request::S3Request {
                    method: "GET".to_string(),
                    path: String::new(),
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

#[cfg(test)]
mod serialization_tests {
    use super::Error;

    /// Every refusal must serialize — the internally-tagged derive this
    /// replaced panicked on newtype-over-string variants, which turned
    /// every native-helper refusal into a dropped connection (a bare
    /// 500 through the dev proxy) instead of an error body.
    #[test]
    fn every_refusal_serializes_with_code_and_message() {
        let refusals = [
            Error::Malformed("bad container".into()),
            Error::Unauthorized("bad signature".into()),
            Error::NotRegistered,
            Error::WrongGrant,
            Error::Forbidden,
            Error::Incomplete("purge failed".into()),
            Error::OwnedSpacesRemain(vec!["did:key:z6MkSpace".into()]),
            Error::Internal("boom".into()),
        ];
        for refusal in refusals {
            let value = serde_json::to_value(&refusal).expect("refusal serializes");
            assert_eq!(value["code"], refusal.code(), "{refusal:?}");
            assert!(
                value["message"].as_str().is_some_and(|m| !m.is_empty()),
                "{refusal:?}"
            );
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), feature = "helpers"))]
mod tests {
    use std::sync::Mutex;

    use std::collections::BTreeMap;

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationBuilder;
    use dialog_ucan_core::promise::Promised;
    use dialog_ucan_core::time::timestamp::Timestamp;
    use dialog_varsig::Principal as _;

    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Enrollment, SIGNUP_PLAN, Store};

    /// A device-signed deprovision invocation, proving through the
    /// root's delegation — the shape the worker sends.
    async fn remove_container(
        root: &Ed25519Signer,
        device: &Ed25519Signer,
        consumer: &dialog_varsig::Did,
    ) -> Vec<u8> {
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        tonk_identity::request::build_device_invocation(
            device.clone(),
            &link,
            COMMAND.map(str::to_string).to_vec(),
            BTreeMap::from([(
                "consumer".to_string(),
                Promised::String(consumer.to_string()),
            )]),
        )
        .await
        .unwrap()
    }

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
        let device = Ed25519Signer::import(&[73; 32]).await.unwrap();
        let space = Ed25519Signer::import(&[72; 32]).await.unwrap();
        let at = now();
        store
            .enroll_customer(Enrollment {
                did: root.did().as_str(),
                email: "owner@example.com",
                plan: SIGNUP_PLAN,
                ledger: root.did().as_str(),
                custody: &format!("{}-custody", root.did().as_str()),
                now: at,
                expires_at: u64::MAX,
            })
            .await
            .unwrap();
        store
            .add_subscription(
                space.did().as_str(),
                root.did().as_str(),
                at,
                SubscriptionKind::Space,
            )
            .await
            .unwrap();
        let invocation = remove_container(&root, &device, &space.did()).await;
        let purger = FlakyPurger {
            fail_once: Mutex::new(true),
            prefixes: Mutex::new(Vec::new()),
        };

        assert!(matches!(
            delete(&store, &purger, &invocation, at + 1).await,
            Err(Error::Incomplete(_))
        ));
        // The purge failed, so the row stays and carries the moment
        // deletion began: service is already refused.
        let denied = store.consumer(space.did().as_str()).await.unwrap().unwrap();
        assert_eq!(denied.deleted_at, Some(at + 1));
        assert_eq!(denied.provider, root.did().to_string());

        let receipt = delete(&store, &purger, &invocation, at + 2).await.unwrap();
        assert_eq!(receipt.space, space.did());
        // A finished deletion takes the row with the data.
        assert!(
            store
                .consumer(space.did().as_str())
                .await
                .unwrap()
                .is_none()
        );

        // Replaying is still successful: nothing is left to delete.
        let replay = delete(&store, &purger, &invocation, at + 3).await.unwrap();
        assert_eq!(replay.space, receipt.space);
        assert_eq!(
            purger.prefixes.lock().unwrap().as_slice(),
            &[format!("{}/", space.did()), format!("{}/", space.did())]
        );
    }

    /// The bug this pins: the deletion machinery treated the account's
    /// custody namespaces (the passkey-sealed key material) as ordinary
    /// owned spaces, deprovisioned one mid-flight, and thereby
    /// destroyed the account's own ability to retry — a permanent
    /// half-deleted lockout. Custody consumers must be invisible to the
    /// review plan, must refuse /provider/remove, and must be purged
    /// only by customer finalization, last.
    #[dialog_common::test]
    async fn custody_namespaces_are_finalization_scope_not_review_scope() {
        let store = SqliteStore::in_memory().unwrap();
        let root = Ed25519Signer::import(&[91; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[92; 32]).await.unwrap();
        let space = Ed25519Signer::import(&[93; 32]).await.unwrap();
        let custody = Ed25519Signer::import(&[94; 32]).await.unwrap();
        let at = now();
        store
            .enroll_customer(Enrollment {
                did: root.did().as_str(),
                email: "custody@example.com",
                plan: SIGNUP_PLAN,
                ledger: root.did().as_str(),
                custody: custody.did().as_str(),
                now: at,
                expires_at: u64::MAX,
            })
            .await
            .unwrap();
        store
            .add_subscription(
                space.did().as_str(),
                root.did().as_str(),
                at,
                SubscriptionKind::Space,
            )
            .await
            .unwrap();
        store
            .add_subscription(
                custody.did().as_str(),
                root.did().as_str(),
                at,
                SubscriptionKind::Custody,
            )
            .await
            .unwrap();
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();

        // The review plan shows only the data space.
        let plan_invocation = tonk_identity::request::build_device_invocation(
            device.clone(),
            &link,
            CUSTOMER_PLAN_COMMAND.map(str::to_string).to_vec(),
            Default::default(),
        )
        .await
        .unwrap();
        let plan = customer_plan(&store, &plan_invocation, at).await.unwrap();
        assert_eq!(plan.spaces.len(), 1);
        assert_eq!(plan.spaces[0].space, space.did().to_string());

        // Deprovisioning the custody namespace is refused outright.
        let purger = RecordingPurger(Mutex::new(Vec::new()));
        let remove_custody = remove_container(&root, &device, &custody.did()).await;
        assert!(matches!(
            delete(&store, &purger, &remove_custody, at + 1).await,
            Err(Error::Forbidden)
        ));
        assert!(purger.0.lock().unwrap().is_empty());

        // Data space deleted; a still-active custody namespace does NOT
        // block finalization — finalization is what purges it, last.
        let remove_space = remove_container(&root, &device, &space.did()).await;
        delete(&store, &purger, &remove_space, at + 2)
            .await
            .unwrap();
        let customer_invocation = root_container(root.clone(), &CUSTOMER_DELETE_COMMAND).await;
        let receipt = delete_customer(&store, &purger, &customer_invocation, at + 3)
            .await
            .unwrap();
        assert_eq!(receipt.customer, root.did().to_string());
        // Finalization takes the subscription rows with the customer:
        // a subscription names who pays for it, and nobody does now.
        assert!(
            store
                .consumer(custody.did().as_str())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            purger.0.lock().unwrap().as_slice(),
            &[
                format!("{}/", space.did()),
                format!("{}/", custody.did()),
                format!("{}/", root.did()),
            ],
            "custody purges inside finalization, after every data space and before the account space"
        );
    }

    /// Deleting a customer releases the address: the row holds it under
    /// a unique index, so an account that is gone must not keep an email
    /// claimed. The whole point of releasing it is enrolling again.
    #[dialog_common::test]
    async fn deleting_a_customer_frees_the_address_for_another_account() {
        let store = SqliteStore::in_memory().unwrap();
        let purger = RecordingPurger(Mutex::new(Vec::new()));
        let root = Ed25519Signer::generate().await.unwrap();
        let at = now();
        store
            .enroll_customer(Enrollment {
                did: root.did().as_str(),
                email: "alice@example.com",
                plan: SIGNUP_PLAN,
                ledger: root.did().as_str(),
                custody: &format!("{}-custody", root.did().as_str()),
                now: at,
                expires_at: u64::MAX,
            })
            .await
            .unwrap();
        assert!(
            store
                .customer_by_email("alice@example.com")
                .await
                .unwrap()
                .is_some()
        );

        let invocation = root_container(root.clone(), &CUSTOMER_DELETE_COMMAND).await;
        delete_customer(&store, &purger, &invocation, at + 1)
            .await
            .expect("a customer with no other spaces finalizes");

        assert!(
            store
                .customer_by_email("alice@example.com")
                .await
                .unwrap()
                .is_none(),
            "the address is free once the account holding it is gone"
        );

        // And a different account may take it.
        let other = Ed25519Signer::generate().await.unwrap();
        store
            .enroll_customer(Enrollment {
                did: other.did().as_str(),
                email: "alice@example.com",
                plan: SIGNUP_PLAN,
                ledger: other.did().as_str(),
                custody: &format!("{}-custody", other.did().as_str()),
                now: at + 2,
                expires_at: u64::MAX,
            })
            .await
            .expect("the released address enrolls again");
    }

    #[dialog_common::test]
    async fn customer_finalization_requires_every_owned_space_then_removes_email_state() {
        let store = SqliteStore::in_memory().unwrap();
        let root = Ed25519Signer::import(&[81; 32]).await.unwrap();
        let space = Ed25519Signer::import(&[82; 32]).await.unwrap();
        let at = now();
        store
            .enroll_customer(Enrollment {
                did: root.did().as_str(),
                email: "delete@example.com",
                plan: SIGNUP_PLAN,
                ledger: root.did().as_str(),
                custody: &format!("{}-custody", root.did().as_str()),
                now: at,
                expires_at: u64::MAX,
            })
            .await
            .unwrap();
        store
            .add_subscription(
                space.did().as_str(),
                root.did().as_str(),
                at,
                SubscriptionKind::Space,
            )
            .await
            .unwrap();
        let device = Ed25519Signer::import(&[83; 32]).await.unwrap();
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let plan_invocation = tonk_identity::request::build_device_invocation(
            device.clone(),
            &link,
            CUSTOMER_PLAN_COMMAND.map(str::to_string).to_vec(),
            Default::default(),
        )
        .await
        .unwrap();
        let plan = customer_plan(&store, &plan_invocation, at).await.unwrap();
        assert_eq!(plan.spaces.len(), 1);
        assert_eq!(plan.spaces[0].space, space.did().to_string());

        // Finalization arrives device-signed, exactly as the worker
        // builds it: the account's chain to this device is the
        // deletion authority, no root-key signature involved.
        let customer_invocation = tonk_identity::request::build_device_invocation(
            device.clone(),
            &link,
            CUSTOMER_DELETE_COMMAND.map(str::to_string).to_vec(),
            Default::default(),
        )
        .await
        .unwrap();
        let purger = RecordingPurger(Mutex::new(Vec::new()));
        assert!(matches!(
            delete_customer(&store, &purger, &customer_invocation, at + 1).await,
            Err(Error::OwnedSpacesRemain(spaces)) if spaces == vec![space.did().to_string()]
        ));
        assert!(store.customer(root.did().as_str()).await.unwrap().is_some());

        let space_invocation = remove_container(&root, &device, &space.did()).await;
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
                .subscriptions_by_provider(root.did().as_str())
                .await
                .unwrap()
                .is_empty()
        );
        // Finalization leaves nothing behind: the subscription rows go
        // with the customer that paid for them. Nothing marks the space
        // DID as spent, so provisioning it again succeeds — only the
        // holder of that space's key can present the DID, and a customer
        // who deletes their account and comes back with the same space
        // is a customer rather than an attacker.
        assert!(
            store
                .consumer(space.did().as_str())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            purger.0.lock().unwrap().as_slice(),
            &[
                format!("{}/", space.did()),
                // Enrollment claims the passkey's custody space, so
                // finalization purges it: after the data spaces, before
                // the account's own.
                format!("{}-custody/", root.did()),
                format!("{}/", root.did()),
            ]
        );
    }
}

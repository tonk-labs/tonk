//! Customer registration tests: `/customer/enroll` and
//! `/customer/activate` driven both directly against the registration
//! environment and over HTTP against the local access server.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test registration
//! ```

#![cfg(feature = "integration-tests")]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{
    Container, Delegation, DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain,
};
use dialog_varsig::algorithm::eddsa::Ed25519Signature;
use dialog_varsig::{Did, Principal};
use tonk_access_service::email::CapturedEmail;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_access_service::registration::Registration;
use tonk_access_service::store::Store;
use tonk_access_service::store::sqlite::SqliteStore;
use tonk_account::customer::RegistrationError;

/// Current time as unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is past the epoch")
        .as_secs()
}

/// Timestamp from unix seconds.
fn at(seconds: u64) -> Timestamp {
    Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).expect("timestamp in range")
}

/// A native registration fixture: sqlite store, captured email, and a
/// service signer, playing the part the worker environment plays in
/// production.
struct Fixture {
    store: SqliteStore,
    emails: CapturedEmail,
    service: Ed25519Signer,
    origin: String,
}

impl Fixture {
    async fn new() -> Self {
        Fixture {
            store: SqliteStore::in_memory().expect("in-memory control store"),
            emails: CapturedEmail::default(),
            service: Ed25519Signer::generate().await.expect("service signer"),
            origin: "https://hub.test".to_string(),
        }
    }

    fn registration<'a>(
        &'a self,
        container: &'a [u8],
    ) -> Registration<'a, SqliteStore, CapturedEmail> {
        Registration {
            store: &self.store,
            email: &self.emails,
            service: &self.service,
            origin: &self.origin,
            activation_ttl: 60 * 60,
            now: unix_now(),
            container,
        }
    }

    fn last_email(&self) -> (String, String) {
        self.emails
            .0
            .lock()
            .expect("captured email mutex poisoned")
            .last()
            .cloned()
            .expect("an activation email was captured")
    }
}

/// Build an enroll container: a self-signed `/customer/enroll` invocation
/// carrying the deposited access delegation as an extra container token.
async fn enroll_container(customer: &Ed25519Signer, service: &Did, email: &str) -> Vec<u8> {
    let deposit = DelegationBuilder::new()
        .issuer(customer.clone())
        .audience(service)
        .subject(DelegatedSubject::Specific(customer.did()))
        .command(vec![])
        .try_build()
        .await
        .expect("deposit delegation");
    enroll_container_with_deposit(customer, email, &deposit, true).await
}

/// Build an enroll container with an explicit deposit, optionally leaving
/// its bytes out of the container.
async fn enroll_container_with_deposit(
    customer: &Ed25519Signer,
    email: &str,
    deposit: &Delegation<Ed25519Signature>,
    carry_deposit: bool,
) -> Vec<u8> {
    let invocation = InvocationBuilder::new()
        .issuer(customer.clone())
        .audience(&customer.did())
        .subject(&customer.did())
        .command(vec!["customer".to_string(), "enroll".to_string()])
        .arguments(BTreeMap::from([
            ("email".to_string(), Promised::String(email.to_string())),
            ("access".to_string(), Promised::Link(deposit.to_cid())),
        ]))
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .expect("enroll invocation");
    let mut tokens = vec![serde_ipld_dagcbor::to_vec(&invocation).expect("invocation encodes")];
    if carry_deposit {
        tokens.push(deposit.encoded().to_vec());
    }
    Container::new(tokens)
        .to_bytes()
        .expect("container encodes")
}

/// Decode the delegation carried by an activation link.
fn link_delegation(link: &str) -> Delegation<Ed25519Signature> {
    let encoded = link
        .split("ucan=")
        .nth(1)
        .expect("link carries the delegation");
    let bytes = URL_SAFE_NO_PAD.decode(encoded).expect("valid base64url");
    let chain = DelegationChain::try_from(bytes.as_slice()).expect("delegation chain decodes");
    chain
        .proofs()
        .next()
        .expect("chain carries one delegation")
        .clone()
}

/// Build an activate container: the customer invokes the emailed
/// delegation against the service's subject.
async fn activate_container(
    invoker: &Ed25519Signer,
    service: &Did,
    customer: &Did,
    link: &Delegation<Ed25519Signature>,
    terms: &str,
) -> Vec<u8> {
    let invocation = InvocationBuilder::new()
        .issuer(invoker.clone())
        .audience(service)
        .subject(service)
        .command(vec!["customer".to_string(), "activate".to_string()])
        .arguments(BTreeMap::from([
            (
                "customer".to_string(),
                Promised::String(customer.to_string()),
            ),
            ("terms".to_string(), Promised::String(terms.to_string())),
        ]))
        .proofs(vec![link.to_cid()])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .expect("activate invocation");
    InvocationChain::new(
        invocation,
        HashMap::from([(link.to_cid(), Arc::new(link.clone()))]),
    )
    .to_bytes()
    .expect("container encodes")
}

#[dialog_common::test]
async fn it_enrolls_a_customer_and_emails_an_activation_link() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;

    let receipt = fixture.registration(&container).handle().await.unwrap();
    assert_eq!(receipt.customer, customer.did());
    assert_eq!(serde_json::to_value(receipt.status)?, "Registered");

    let (address, link) = fixture.last_email();
    assert_eq!(address, "alice@example.com");
    assert!(link.starts_with("https://hub.test/activate?ucan="));

    let delegation = link_delegation(&link);
    assert_eq!(delegation.issuer(), &fixture.service.did());
    assert_eq!(delegation.audience(), &customer.did());
    Ok(())
}

#[dialog_common::test]
async fn it_activates_through_the_emailed_delegation_and_replays_as_a_noop() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    let link = link_delegation(&fixture.last_email().1);
    let container =
        activate_container(&customer, &service, &customer.did(), &link, "2026-08").await;
    let receipt = fixture.registration(&container).handle().await.unwrap();
    assert_eq!(serde_json::to_value(receipt.status)?, "Active");

    let stored = fixture
        .store
        .customer(customer.did().as_str())
        .await
        .unwrap()
        .expect("customer row exists");
    assert_eq!(stored.terms_version.as_deref(), Some("2026-08"));
    assert!(stored.verified > 0);

    // Clicking twice leaves the customer active and writes no duplicate
    // state.
    let container =
        activate_container(&customer, &service, &customer.did(), &link, "2026-08").await;
    let receipt = fixture.registration(&container).handle().await.unwrap();
    assert_eq!(serde_json::to_value(receipt.status)?, "Active");
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_enrolling_an_active_customer_and_resends_while_registered() -> anyhow::Result<()>
{
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();

    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();
    // Re-enrolling while registered is idempotent and resends the link.
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();
    assert_eq!(
        fixture
            .emails
            .0
            .lock()
            .expect("captured email mutex poisoned")
            .len(),
        2
    );

    let link = link_delegation(&fixture.last_email().1);
    let container =
        activate_container(&customer, &service, &customer.did(), &link, "2026-08").await;
    fixture.registration(&container).handle().await.unwrap();

    let container = enroll_container(&customer, &service, "alice@example.com").await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::CustomerActive));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_an_intercepted_link_invoked_by_another_key() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let interceptor = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    let link = link_delegation(&fixture.last_email().1);
    // The interceptor holds the link but not the customer's key: the
    // delegation is audience-bound, so their invocation cannot verify.
    let container =
        activate_container(&interceptor, &service, &customer.did(), &link, "2026-08").await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Unauthorized { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_an_expired_activation_link_without_a_storage_lookup() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();

    // A link minted in the past, already expired. No enrollment exists,
    // so a storage lookup would answer UnknownCustomer; the expired
    // window must refuse before any lookup happens.
    let expired = DelegationBuilder::new()
        .issuer(fixture.service.clone())
        .audience(&customer.did())
        .subject(DelegatedSubject::Specific(service.clone()))
        .command(vec!["customer".to_string(), "activate".to_string()])
        .expiration(at(unix_now() - 60))
        .try_build()
        .await?;
    let container =
        activate_container(&customer, &service, &customer.did(), &expired, "2026-08").await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Unauthorized { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_link_minted_for_a_different_customer() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let alice = Ed25519Signer::generate().await?;
    let mallory = Ed25519Signer::generate().await?;
    let service = fixture.service.did();

    let container = enroll_container(&alice, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();
    let container = enroll_container(&mallory, &service, "mallory@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    // Mallory holds their own link but names Alice in the arguments.
    let link = link_delegation(&fixture.last_email().1);
    let container = activate_container(&mallory, &service, &alice.did(), &link, "2026-08").await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_requires_the_deposited_delegation_to_travel_in_the_container() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let deposit = DelegationBuilder::new()
        .issuer(customer.clone())
        .audience(&fixture.service.did())
        .subject(DelegatedSubject::Specific(customer.did()))
        .command(vec![])
        .try_build()
        .await?;
    let container =
        enroll_container_with_deposit(&customer, "alice@example.com", &deposit, false).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Invalid { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_deposit_issued_to_someone_else() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    // The deposit must be issued to this service, not a third party.
    let deposit = DelegationBuilder::new()
        .issuer(customer.clone())
        .audience(&stranger.did())
        .subject(DelegatedSubject::Specific(customer.did()))
        .command(vec![])
        .try_build()
        .await?;
    let container =
        enroll_container_with_deposit(&customer, "alice@example.com", &deposit, true).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_registers_the_account_consumer_atomically_with_the_customer() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    let consumer = fixture
        .store
        .consumer(customer.did().as_str())
        .await
        .unwrap()
        .expect("the account space is a consumer");
    assert_eq!(consumer.provider.as_deref(), Some(customer.did().as_str()));
    Ok(())
}

#[dialog_common::test]
async fn it_drives_registration_over_http(env: AccessServiceAddress) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();

    let service: serde_json::Value = client
        .get(format!("{base}/_test/service"))
        .send()
        .await?
        .json()
        .await?;
    let service_did: Did = service["did"].as_str().expect("service did").parse()?;

    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &service_did, "alice@example.com").await;
    let response = client
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    let receipt: serde_json::Value = response.json().await?;
    assert_eq!(receipt["status"], "Registered");

    let emails: Vec<(String, String)> = client
        .get(format!("{base}/_test/emails"))
        .send()
        .await?
        .json()
        .await?;
    let (address, link) = emails.last().cloned().expect("an activation email");
    assert_eq!(address, "alice@example.com");

    let delegation = link_delegation(&link);
    let container = activate_container(
        &customer,
        &service_did,
        &customer.did(),
        &delegation,
        "2026-08",
    )
    .await;
    let response = client
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    let receipt: serde_json::Value = response.json().await?;
    assert_eq!(receipt["status"], "Active");

    // The DID document names the same key the service signs with.
    let document: serde_json::Value = client
        .get(format!("{base}/.well-known/did.json"))
        .send()
        .await?
        .json()
        .await?;
    let multibase = document["verificationMethod"][0]["publicKeyMultibase"]
        .as_str()
        .expect("a multikey verification method");
    assert_eq!(format!("did:key:{multibase}"), service_did.to_string());
    Ok(())
}

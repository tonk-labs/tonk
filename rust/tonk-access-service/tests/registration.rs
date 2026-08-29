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

use tonk_access_service::revocation::checker::IndexedRevocations;
use tonk_access_service::revocation::index::MemoryRevocationIndex;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::cid::dagcbor_cid;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{
    Container, Delegation, DelegationBuilder, InvocationBuilder, InvocationChain,
};
use dialog_varsig::AnySignature;
use dialog_varsig::{Did, Principal};
use tonk_access_service::email::CapturedEmail;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_access_service::registration::{Answer, Registration, SIGNUP_TERMS};
use tonk_access_service::store::Store;
use tonk_access_service::store::sqlite::SqliteStore;
use tonk_account::customer::{Receipt, RegistrationError, deposit_scopes};

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
    /// Revocations this fixture enforces. Registration consults it per
    /// link, so a revoked delegation cannot enroll or activate.
    revocations: IndexedRevocations<Arc<MemoryRevocationIndex>>,
    emails: CapturedEmail,
    service: Ed25519Signer,
    /// The seed `service` was built from: customer spaces derive from
    /// it, and a signer cannot give its seed back.
    service_seed: String,
    origin: String,
}

impl Fixture {
    async fn new() -> Self {
        Fixture {
            store: SqliteStore::in_memory().expect("in-memory control store"),
            emails: CapturedEmail::default(),
            service: tonk_access_service::service::signer_from_hex(&"ab".repeat(32))
                .expect("service signer"),
            service_seed: "ab".repeat(32),
            origin: "https://hub.test".to_string(),
            revocations: IndexedRevocations(Default::default()),
        }
    }

    fn registration<'a>(
        &'a self,
        container: &'a [u8],
    ) -> Registration<'a, SqliteStore, CapturedEmail, IndexedRevocations<Arc<MemoryRevocationIndex>>>
    {
        Registration {
            store: &self.store,
            email: &self.emails,
            service: &self.service,
            service_seed: &self.service_seed,
            origin: &self.origin,
            activation_ttl: 60 * 60,
            now: unix_now(),
            container,
            revocations: &self.revocations,
        }
    }

    /// Enroll `customer` and finalize it by presenting the emailed
    /// activation invocation, leaving it `Active`. Provisioning requires
    /// activation, so most consumer tests need a customer past this
    /// point rather than a freshly enrolled one.
    async fn enroll_and_activate(&self, customer: &Ed25519Signer, email: &str) {
        let container = enroll_container(customer, &self.service.did(), email).await;
        self.registration(&container)
            .handle()
            .await
            .expect("enrollment succeeds");
        let container = link_container(&self.last_email().1);
        self.registration(&container)
            .handle()
            .await
            .expect("activation succeeds");
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

/// Unwrap a customer receipt, refusing the consumer-shaped answer.
fn as_customer(answer: Answer) -> Receipt {
    match answer {
        Answer::Customer(receipt) => receipt,
        Answer::Consumer(receipt) => panic!("expected a customer receipt, got {receipt:?}"),
    }
}

/// Mint the scoped deposits enrollment requires, issued directly by the
/// customer to `audience` (the service, except in refusal tests).
async fn scoped_deposits(
    customer: &Ed25519Signer,
    service: &Did,
    audience: &Did,
) -> Vec<Delegation<AnySignature>> {
    let mut deposits = Vec::new();
    for scope in deposit_scopes(&customer.did(), service) {
        deposits.push(
            DelegationBuilder::new()
                .issuer(dialog_credentials::Signer::from(customer.clone()))
                .audience(audience)
                .subject(scope.subject.clone())
                .command(scope.command.segments().clone())
                .policy(scope.policy())
                .try_build()
                .await
                .expect("deposit delegation"),
        );
    }
    deposits
}

/// Build an enroll container: a self-signed `/customer/enroll` invocation
/// carrying the deposited access delegations as extra container tokens.
/// The custody material an enrollment carries, with one knob per thing
/// that can be wrong.
///
/// Every rejection test starts from a VALID set and breaks exactly one
/// field, so a refusal proves the check it names rather than tripping an
/// earlier one. `Default` is the valid set.
#[derive(Clone)]
struct Custody {
    /// Signs the recovery invocation and the consent.
    key_seed: [u8; 32],
    /// The custody DID the enrollment names. `None` means "whatever
    /// `key_seed` produces", which is the honest case.
    claimed_did: Option<Did>,
    sealed: Vec<u8>,
    /// Content the invocation checksums, when it should differ from what
    /// is carried.
    checksum_over: Option<Vec<u8>>,
    command: Vec<String>,
    space: String,
    cell: String,
    /// Seconds from now the recovery invocation expires.
    expires_in: u64,
    /// Set to make the write an overwrite rather than a first write.
    when: Option<Vec<u8>>,
    /// Audience of the consent; `None` means the enrolling account.
    consent_audience: Option<Did>,
    /// Command the consent grants.
    consent_command: Vec<String>,
    /// Drop these blocks from the container, though the arguments still
    /// name them.
    omit: Vec<&'static str>,
}

impl Default for Custody {
    fn default() -> Self {
        Custody {
            key_seed: [9u8; 32],
            claimed_did: None,
            sealed: b"sealed-account-secret".to_vec(),
            checksum_over: None,
            command: ["use", "put", "memory", "cell"]
                .iter()
                .map(ToString::to_string)
                .collect(),
            space: "custody".to_string(),
            cell: "secret".to_string(),
            // Comfortably past the fixture's one-hour activation TTL.
            expires_in: 30 * 24 * 60 * 60,
            when: None,
            consent_audience: None,
            consent_command: vec![],
            omit: Vec::new(),
        }
    }
}

fn sha256_multihash(content: &[u8]) -> Vec<u8> {
    use sha2_0_10::{Digest, Sha256};
    let digest = Sha256::digest(content);
    let mut bytes = vec![0x12, 0x20];
    bytes.extend_from_slice(&digest);
    bytes
}

async fn enroll_container(customer: &Ed25519Signer, service: &Did, email: &str) -> Vec<u8> {
    let deposits = scoped_deposits(customer, service, service).await;
    enroll_container_with_deposits(customer, email, &deposits, true).await
}

/// An enrollment carrying `custody`, valid unless a knob says otherwise.
async fn enroll_container_with_custody(
    customer: &Ed25519Signer,
    service: &Did,
    email: &str,
    custody: &Custody,
) -> Vec<u8> {
    let deposits = scoped_deposits(customer, service, service).await;
    enroll_container_parts(customer, email, &deposits, true, custody).await
}

/// The one place an enrollment container is assembled.
async fn enroll_container_parts(
    customer: &Ed25519Signer,
    email: &str,
    deposits: &[Delegation<AnySignature>],
    carry_deposits: bool,
    custody: &Custody,
) -> Vec<u8> {
    let key = Ed25519Signer::import(&custody.key_seed)
        .await
        .expect("custody signer");
    let custody_did = custody.claimed_did.clone().unwrap_or_else(|| key.did());

    let checksum = sha256_multihash(custody.checksum_over.as_ref().unwrap_or(&custody.sealed));
    let mut arguments = BTreeMap::from([
        ("space".to_string(), Promised::String(custody.space.clone())),
        ("cell".to_string(), Promised::String(custody.cell.clone())),
        ("checksum".to_string(), Promised::Bytes(checksum)),
    ]);
    if let Some(when) = &custody.when {
        arguments.insert("when".to_string(), Promised::Bytes(when.clone()));
    }
    let recovery = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(key.clone()))
        .audience(&key.did())
        .subject(&key.did())
        .command(custody.command.clone())
        .arguments(arguments)
        .proofs(vec![])
        .expiration(
            Timestamp::new(
                dialog_ucan_core::time::timestamp::SystemTime::now()
                    + dialog_ucan_core::time::timestamp::Duration::from_secs(custody.expires_in),
            )
            .expect("recovery expiration"),
        )
        .try_build()
        .await
        .expect("recovery invocation");
    // A carried block is a bare token, the same unit the enclosing
    // container holds — not a container of its own.
    let recovery_bytes = serde_ipld_dagcbor::to_vec(&recovery).expect("recovery encodes");

    let consent = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(key.clone()))
        .audience(custody.consent_audience.as_ref().unwrap_or(&customer.did()))
        .subject(dialog_ucan_core::subject::Subject::Specific(key.did()))
        .command(custody.consent_command.clone())
        .try_build()
        .await
        .expect("consent");
    let consent_bytes = consent.encoded().to_vec();

    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(customer.clone()))
        .audience(&customer.did())
        .subject(&customer.did())
        .command(vec!["customer".to_string(), "enroll".to_string()])
        .arguments(BTreeMap::from([
            ("email".to_string(), Promised::String(email.to_string())),
            (
                "access".to_string(),
                Promised::List(
                    deposits
                        .iter()
                        .map(|deposit| Promised::Link(deposit.to_cid()))
                        .collect(),
                ),
            ),
            (
                "custody".to_string(),
                Promised::String(custody_did.to_string()),
            ),
            (
                "recovery".to_string(),
                Promised::Link(dagcbor_cid(&recovery_bytes)),
            ),
            (
                "consent".to_string(),
                Promised::Link(dagcbor_cid(&consent_bytes)),
            ),
            (
                "sealed".to_string(),
                Promised::Link(dagcbor_cid(&custody.sealed)),
            ),
        ]))
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .expect("enroll invocation");

    let mut tokens = vec![serde_ipld_dagcbor::to_vec(&invocation).expect("invocation encodes")];
    if carry_deposits {
        for deposit in deposits {
            tokens.push(deposit.encoded().to_vec());
        }
    }
    for (name, bytes) in [
        ("recovery", recovery_bytes),
        ("consent", consent_bytes),
        ("sealed", custody.sealed.clone()),
    ] {
        if !custody.omit.contains(&name) {
            tokens.push(bytes);
        }
    }
    Container::new(tokens)
        .to_bytes()
        .expect("container encodes")
}

/// Build an enroll container with explicit deposits, optionally leaving
/// their bytes out of the container.
/// An enrollment with the given deposits, carrying valid custody
/// material.
///
/// Every enrollment must carry it, so a fixture testing something else
/// still needs a well-formed set — these tests are about deposits and
/// customer lifecycle, not custody, and each would otherwise fail for a
/// reason it is not asking about.
async fn enroll_container_with_deposits(
    customer: &Ed25519Signer,
    email: &str,
    deposits: &[Delegation<AnySignature>],
    carry_deposits: bool,
) -> Vec<u8> {
    enroll_container_parts(
        customer,
        email,
        deposits,
        carry_deposits,
        &Custody::default(),
    )
    .await
}

/// Decode the invocation container carried by an activation link. It is
/// complete and service-signed, so activating is presenting these bytes;
/// no key is needed on the presenting device.
fn link_container(link: &str) -> Vec<u8> {
    let encoded = link
        .split("ucan=")
        .nth(1)
        .expect("link carries the invocation");
    URL_SAFE_NO_PAD.decode(encoded).expect("valid base64url")
}

/// Mint an activation-shaped invocation with an arbitrary issuer,
/// subject, and expiration, for the refusal tests.
async fn activation_invocation(
    issuer: &Ed25519Signer,
    subject: &Did,
    customer: &Did,
    expiration: Timestamp,
) -> Vec<u8> {
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(issuer.clone()))
        .audience(subject)
        .subject(subject)
        .command(vec!["customer".to_string(), "activate".to_string()])
        .arguments(BTreeMap::from([
            (
                "customer".to_string(),
                Promised::String(customer.to_string()),
            ),
            (
                "terms".to_string(),
                Promised::String(SIGNUP_TERMS.to_string()),
            ),
        ]))
        .proofs(vec![])
        .expiration(expiration)
        .try_build()
        .await
        .expect("activation invocation");
    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .expect("container encodes")
}

#[dialog_common::test]
async fn it_enrolls_a_customer_and_emails_an_activation_link() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;

    let receipt = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(receipt.customer, customer.did());
    assert_eq!(serde_json::to_value(receipt.status)?, "Registered");

    let (address, link) = fixture.last_email();
    assert_eq!(address, "alice@example.com");
    assert!(link.starts_with("https://hub.test/activate?ucan="));
    Ok(())
}

/// Activation names the provider; enrollment does not.
///
/// The service decides which provider serves its customers and says so
/// in the receipt, so a client records one authoritative address instead
/// of deriving `https://{origin}/ucan/` from whichever origin its
/// request happened to reach.
///
/// Only at activation, though. The address is what says "this service
/// serves you", and for an unactivated customer it does not — it gets
/// neither service nor provisioning. Withholding it until activation is
/// what lets a client tell "enrolled, awaiting the email" from "ready to
/// sync" by looking at the recorded address alone.
#[dialog_common::test]
async fn it_answers_the_provider_only_once_the_customer_activates() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;

    let enrolled = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(
        enrolled.provider, None,
        "enrollment must name no provider: this customer is not served yet",
    );
    // Absent from the wire, not present-and-null: a client reading the
    // JSON must see no key at all rather than an explicit empty value.
    let wire = serde_json::to_value(&enrolled)?;
    assert!(
        wire.get("provider").is_none(),
        "an unserved receipt must omit the provider key entirely, got {wire}",
    );

    let container = link_container(&fixture.last_email().1);
    let activated = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(
        activated.provider.as_deref(),
        Some("https://hub.test/ucan/"),
        "activation names the provider this customer's spaces attach to",
    );
    Ok(())
}

#[dialog_common::test]
async fn it_activates_by_presenting_the_emailed_invocation_from_any_device() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    // The link carries a complete service-signed invocation: presenting
    // it is activating, no customer key involved, so a click on any
    // device finalizes.
    let container = link_container(&fixture.last_email().1);
    let receipt = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(receipt.customer, customer.did());
    assert_eq!(serde_json::to_value(receipt.status)?, "Active");

    let stored = fixture
        .store
        .customer(customer.did().as_str())
        .await
        .unwrap()
        .expect("customer row exists");
    assert_eq!(stored.terms_version.as_deref(), Some(SIGNUP_TERMS));
    assert!(stored.verified > 0);

    // Clicking twice leaves the customer active and writes no duplicate
    // state.
    let receipt = as_customer(fixture.registration(&container).handle().await.unwrap());
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

    let container = link_container(&fixture.last_email().1);
    fixture.registration(&container).handle().await.unwrap();

    let container = enroll_container(&customer, &service, "alice@example.com").await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::CustomerActive));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_forged_activation_invocation() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let attacker = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    // An attacker naming the service as subject cannot prove the chain.
    let forged = activation_invocation(
        &attacker,
        &service,
        &customer.did(),
        Timestamp::five_minutes_from_now(),
    )
    .await;
    let refusal = fixture.registration(&forged).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Unauthorized { .. }));

    // A self-signed invocation on the attacker's own subject verifies,
    // but it is not an invocation this service minted.
    let forged = activation_invocation(
        &attacker,
        &attacker.did(),
        &customer.did(),
        Timestamp::five_minutes_from_now(),
    )
    .await;
    let refusal = fixture.registration(&forged).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_an_expired_activation_link_without_a_storage_lookup() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();

    // A genuinely service-signed link, already expired. No enrollment
    // exists, so a storage lookup would answer UnknownCustomer; the
    // expired window must refuse before any lookup happens.
    let expired = activation_invocation(
        &fixture.service,
        &service,
        &customer.did(),
        at(unix_now() - 60),
    )
    .await;
    let refusal = fixture.registration(&expired).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Unauthorized { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_requires_the_deposited_delegations_to_travel_in_the_container() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    let deposits = scoped_deposits(&customer, &service, &service).await;
    let container =
        enroll_container_with_deposits(&customer, "alice@example.com", &deposits, false).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Invalid { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_deposit_issued_to_someone_else() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    // The deposits must be issued to this service, not a third party.
    let service = fixture.service.did();
    let deposits = scoped_deposits(&customer, &service, &stranger.did()).await;
    let container =
        enroll_container_with_deposits(&customer, "alice@example.com", &deposits, true).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_deposit_broader_than_the_scopes() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    // The old shape of the deposit: an unscoped grant of `/` over the
    // whole account space. Enrollment must refuse it rather than hold it.
    let unscoped = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(customer.clone()))
        .audience(&service)
        .subject(DelegatedSubject::Specific(customer.did()))
        .command(vec![])
        .try_build()
        .await?;
    let container =
        enroll_container_with_deposits(&customer, "alice@example.com", &[unscoped], true).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

#[dialog_common::test]
async fn it_walks_a_device_issued_deposit_back_to_the_customer() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    // The fallback shape a device without ceremony-minted deposits
    // presents: deposits issued by the device, chained to the customer
    // through the `root → device` grant riding in the same container.
    let root = Ed25519Signer::generate().await?;
    let device = Ed25519Signer::generate().await?;
    let link =
        tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did()).await?;
    let custody_key = Ed25519Signer::generate().await?;
    let custody = tonk_identity::request::mint_custody_material(
        &custody_key,
        &root.did(),
        b"sealed-account-secret".to_vec(),
    )
    .await?;
    let container = tonk_identity::request::build_enroll_invocation(
        device,
        &link,
        &fixture.service.did(),
        "alice@example.com",
        &custody.borrow(),
    )
    .await?;
    let receipt = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(receipt.customer, root.did());
    Ok(())
}

#[dialog_common::test]
async fn it_requires_the_deposits_to_cover_both_scopes() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let service = fixture.service.did();
    // Only the memory grant, no index catalog: the service could publish
    // a branch pointer but never ship its nodes, so enrollment refuses
    // the incomplete set.
    let mut deposits = scoped_deposits(&customer, &service, &service).await;
    deposits.truncate(1);
    let container =
        enroll_container_with_deposits(&customer, "alice@example.com", &deposits, true).await;
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

/// Build a `/provider/add` container: a customer-signed invocation
/// depositing the space's consent delegation.
async fn add_container(
    customer: &Ed25519Signer,
    space: &Ed25519Signer,
    consent_to: &Did,
) -> Vec<u8> {
    let consent = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(space.clone()))
        .audience(consent_to)
        .subject(DelegatedSubject::Specific(space.did()))
        .command(vec![])
        .try_build()
        .await
        .expect("consent delegation");
    let arguments = BTreeMap::from([
        (
            "consumer".to_string(),
            Promised::String(space.did().to_string()),
        ),
        ("consent".to_string(), Promised::Link(consent.to_cid())),
    ]);
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(customer.clone()))
        .audience(&customer.did())
        .subject(&customer.did())
        .command(vec!["provider".to_string(), "add".to_string()])
        .arguments(arguments)
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .expect("add invocation");
    let tokens = vec![
        serde_ipld_dagcbor::to_vec(&invocation).expect("invocation encodes"),
        consent.encoded().to_vec(),
    ];
    Container::new(tokens)
        .to_bytes()
        .expect("container encodes")
}

#[dialog_common::test]
async fn it_provisions_a_consumer_with_the_spaces_consent() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let space = Ed25519Signer::generate().await?;
    fixture
        .enroll_and_activate(&customer, "alice@example.com")
        .await;

    let container = add_container(&customer, &space, &customer.did()).await;
    let Answer::Consumer(receipt) = fixture.registration(&container).handle().await.unwrap() else {
        panic!("expected a consumer receipt");
    };
    assert_eq!(receipt.consumer, space.did());
    assert_eq!(receipt.provider, customer.did());

    let consumer = fixture
        .store
        .consumer(space.did().as_str())
        .await
        .unwrap()
        .expect("the consumer row exists");
    assert_eq!(consumer.provider.as_deref(), Some(customer.did().as_str()));

    // Re-provisioning under the same customer succeeds and changes
    // nothing: clients retry provisioning freely (a queued entry
    // replayed twice, two devices racing), so it must be idempotent
    // rather than a conflict.
    let registered_at = consumer.registered;
    let container = add_container(&customer, &space, &customer.did()).await;
    let Answer::Consumer(receipt) = fixture.registration(&container).handle().await.unwrap() else {
        panic!("expected a consumer receipt");
    };
    assert_eq!(receipt.provider, customer.did());
    let again = fixture
        .store
        .consumer(space.did().as_str())
        .await
        .unwrap()
        .expect("the consumer row still exists");
    assert_eq!(again.provider.as_deref(), Some(customer.did().as_str()));
    assert_eq!(
        again.registered, registered_at,
        "re-provisioning must not re-register the consumer"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_provisioning_until_the_customer_confirms_their_email() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let space = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    // Enrolled but not activated: nothing may be provisioned yet, and
    // the refusal names the cause so the client can say so.
    let container = add_container(&customer, &space, &customer.did()).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::CustomerInactive));
    assert!(
        fixture
            .store
            .consumer(space.did().as_str())
            .await
            .unwrap()
            .is_none(),
        "a refused add writes no consumer row"
    );

    // The same add replays successfully once the email is confirmed,
    // which is what the client's pending-work queue relies on.
    let container = link_container(&fixture.last_email().1);
    fixture.registration(&container).handle().await.unwrap();
    let container = add_container(&customer, &space, &customer.did()).await;
    assert!(fixture.registration(&container).handle().await.is_ok());
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_provisioning_someone_elses_consumer() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let alice = Ed25519Signer::generate().await?;
    let mallory = Ed25519Signer::generate().await?;
    let space = Ed25519Signer::generate().await?;
    for (customer, email) in [
        (&alice, "alice@example.com"),
        (&mallory, "mallory@example.com"),
    ] {
        fixture.enroll_and_activate(customer, email).await;
    }

    let container = add_container(&alice, &space, &alice.did()).await;
    fixture.registration(&container).handle().await.unwrap();

    // Mallory holds the space's consent to herself, and is active in her
    // own right; what stops her is that the space already has a
    // provider. A consumer has exactly one.
    let container = add_container(&mallory, &space, &mallory.did()).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::ConsumerProvided));
    let consumer = fixture
        .store
        .consumer(space.did().as_str())
        .await
        .unwrap()
        .expect("the consumer row exists");
    assert_eq!(
        consumer.provider.as_deref(),
        Some(alice.did().as_str()),
        "a refused takeover must leave the original provider paying"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_consent_issued_to_another_customer() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let alice = Ed25519Signer::generate().await?;
    let mallory = Ed25519Signer::generate().await?;
    let space = Ed25519Signer::generate().await?;
    fixture
        .enroll_and_activate(&mallory, "mallory@example.com")
        .await;

    // The space consented to Alice; Mallory presenting that consent must
    // not become its provider.
    let container = add_container(&mallory, &space, &alice.did()).await;
    let refusal = fixture.registration(&container).handle().await.unwrap_err();
    assert!(matches!(refusal, RegistrationError::Forbidden { .. }));
    Ok(())
}

/// A state-dir service survives its own restart: same signing identity,
/// same customers, and the blob snapshot refills the fresh store. This
/// is the dev-durability contract — a restarted local service must not
/// orphan the clients holding credentials against it.
#[dialog_common::test]
async fn it_keeps_state_across_restarts_with_a_state_dir() -> anyhow::Result<()> {
    use tonk_access_service::helpers::{AccessServiceSettings, access_service};

    let state = tempfile::tempdir()?;
    let settings = AccessServiceSettings {
        state_dir: Some(state.path().to_path_buf()),
        ..Default::default()
    };

    let service = access_service(settings.clone()).await?;
    let first_did = service.address.service_did.clone();
    let base = service
        .address
        .access_service_url
        .trim_end_matches('/')
        .to_string();
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &first_did.parse()?, "alice@example.com").await;
    let response = reqwest::Client::new()
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    service.stop().await?;

    let service = access_service(settings).await?;
    assert_eq!(
        service.address.service_did, first_did,
        "the signing identity survives a restart"
    );
    let base = service
        .address
        .access_service_url
        .trim_end_matches('/')
        .to_string();
    let probe = reqwest::get(format!("{base}/customer/{}", customer.did())).await?;
    assert_eq!(
        probe.status(),
        200,
        "an enrolled customer survives a restart"
    );
    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
async fn it_drives_registration_over_http(env: AccessServiceAddress) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();

    let service_did: Did = env.service_did.parse()?;
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

    let response = client
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(link_container(&link))
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

/// The presign gate: a subject is served only while an active customer
/// pays for it, and confirming the email is what lifts the denial for
/// every consumer that customer funds at once.
#[dialog_common::test]
async fn it_denies_presign_until_the_customer_confirms_their_email(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    use tonk_identity::custody;

    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();
    let ucan = format!("{base}/ucan/");
    let service_did: Did = env.service_did.parse()?;
    let custody_key = custody_signer_for(&[31u8; 32]).await?;

    // A subject nobody pays for is never served.
    let resolve = custody::build_resolve_invocation(custody_key.clone()).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(resolve.clone())
        .send()
        .await?;
    assert_eq!(response.status(), 403, "an unprovisioned subject is served");
    let reason: serde_json::Value = response.json().await?;
    assert_eq!(reason["kind"], "Declined");
    assert_eq!(
        reason["recourse"], "None",
        "nobody is coming to provision this subject: {reason}"
    );
    assert!(
        reason["reason"]
            .as_str()
            .expect("the refusal says why")
            .contains("not provisioned"),
        "the refusal must say why: {reason}"
    );

    // Enrolled but unactivated: the customer's own account subject is
    // refused, and the refusal points at the unopened email.
    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "gate@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "enrollment refused");

    let account_resolve = custody::build_resolve_invocation(account.clone()).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(account_resolve.clone())
        .send()
        .await?;
    assert_eq!(response.status(), 403, "a Registered customer is served");
    let reason: serde_json::Value = response.json().await?;
    assert_eq!(reason["kind"], "Declined");
    // The bit a waiting client acts on: this refusal clears when
    // somebody opens the link, so the client holds its work and retries
    // rather than treating the account as unusable.
    assert_eq!(
        reason["recourse"], "Retry",
        "an unconfirmed email is worth waiting on: {reason}"
    );
    assert!(
        reason["reason"]
            .as_str()
            .expect("the refusal says why")
            .contains("awaits email activation"),
        "the refusal must name the unopened email: {reason}"
    );

    // Confirming the email lifts it, without any further provisioning
    // call: enrollment already wrote the account's own consumer row
    // self-provided, so activation alone makes the account space
    // servable — along with everything else this customer funds.
    activate_over_http(&client, &base).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(account_resolve)
        .send()
        .await?;
    assert_ne!(
        response.status(),
        403,
        "activation did not lift the provisioning denial"
    );
    Ok(())
}

/// A custody signer at a fixed entry-function output, standing in for
/// what a PRF assertion would produce.
async fn custody_signer_for(key: &[u8; 32]) -> anyhow::Result<Ed25519Signer> {
    tonk_identity::envelope::custody_signer(key).await
}

/// Activate the customer the last captured email was sent to, over
/// HTTP. Provisioning and presign both require an `Active` customer, so
/// a test driving the real endpoints enrolls and then comes through
/// here before it can do anything else.
async fn activate_over_http(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    let emails: Vec<(String, String)> = client
        .get(format!("{base}/_test/emails"))
        .send()
        .await?
        .json()
        .await?;
    let (_, link) = emails.last().cloned().expect("an activation email");
    let response = client
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(link_container(&link))
        .send()
        .await?;
    assert_eq!(response.status(), 200, "activation refused");
    Ok(())
}

/// The custody protocol end to end (`plan/Account custody.md`): an
/// activated account provisions the custody DID as a consumer, the
/// custody key publishes the wrapped account secret as a raw memory
/// cell, and a fresh resolve reads the same bytes back holding nothing
/// but the custody key. No repository exists anywhere in this flow.
#[dialog_common::test]
async fn it_publishes_and_resolves_the_custody_cell(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    use dialog_remote_s3::Permit;
    use tonk_identity::envelope::{
        AccountSecret, Envelope, KekMethod, custody_kek, custody_signer,
    };
    use tonk_identity::{custody, delegation};

    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();
    let ucan = format!("{base}/ucan/");
    let service_did: Did = env.service_did.parse()?;

    // The account enrolls as a customer.
    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "custody@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "enrollment refused");

    // Confirming the email is what unlocks everything below: an
    // unactivated customer provisions nothing and is served nothing.
    activate_over_http(&client, &base).await?;

    // The entry function's two outputs; a PRF would produce these
    // inside one assertion, at the two custody salts.
    let custody_key = custody_signer(&[21u8; 32]).await?;
    let kek = custody_kek(&[22u8; 32]);

    // The account provisions the custody DID, depositing the consent
    // the custody key minted — the ordinary provisioning contract.
    let device = Ed25519Signer::generate().await?;
    let link = delegation::mint_device_delegation(account.clone(), &device.did()).await?;
    let consent = custody::mint_custody_consent(custody_key.clone(), &account.did()).await?;
    let add = tonk_identity::request::build_provider_add_invocation(
        device,
        &link,
        &custody_key.did(),
        &consent,
        Some("custody"),
    )
    .await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(add)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "provisioning refused");

    // Seal the secret and publish the cell: permit, then presigned PUT.
    let secret = AccountSecret::generate()?;
    let sealed = kek.seal(&secret, KekMethod::Passkey)?.encode();
    let publish = custody::build_publish_invocation(
        custody_key.clone(),
        &sealed,
        None,
        dialog_ucan_core::time::Timestamp::five_minutes_from_now(),
    )
    .await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(publish)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "publish permit refused");
    let permit: Permit = serde_ipld_dagcbor::from_slice(&response.bytes().await?)?;
    let stored = permit.upload(sealed.clone()).await?;
    assert!(
        stored.status().is_success(),
        "storage PUT failed: {}",
        stored.status(),
    );

    // A fresh device resolves with nothing but the custody key.
    let resolve = custody::build_resolve_invocation(custody_key.clone()).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(resolve)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "resolve permit refused");
    let permit: Permit = serde_ipld_dagcbor::from_slice(&response.bytes().await?)?;
    let fetched = permit.send().await?;
    assert!(
        fetched.status().is_success(),
        "storage GET failed: {}",
        fetched.status(),
    );
    let bytes = fetched.bytes().await?.to_vec();
    assert_eq!(bytes, sealed, "the resolved cell is the sealed envelope");

    // The envelope opens back to the same account.
    let opened = custody_kek(&[22u8; 32]).open(&Envelope::decode(&bytes)?)?;
    assert_eq!(
        opened.signing_seed().as_ref(),
        secret.signing_seed().as_ref(),
        "the unwrapped secret derives the same account",
    );
    Ok(())
}

/// The deferred variant of the custody publish: the invocation is
/// pre-signed with the month-long expiration a queued-at-creation
/// publish carries, and must still redeem — the service bounds
/// registration ceremonies, not the memory permit an owner signs for
/// its own cell.
#[dialog_common::test]
async fn it_redeems_a_deferred_publish_invocation(env: AccessServiceAddress) -> anyhow::Result<()> {
    use dialog_remote_s3::Permit;
    use tonk_identity::envelope::{
        AccountSecret, Envelope, KekMethod, custody_kek, custody_signer,
    };
    use tonk_identity::{custody, delegation};

    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();
    let ucan = format!("{base}/ucan/");
    let service_did: Did = env.service_did.parse()?;

    // The account enrolls as a customer.
    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "custody@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "enrollment refused");

    // Confirming the email is what unlocks everything below: an
    // unactivated customer provisions nothing and is served nothing.
    activate_over_http(&client, &base).await?;

    // The entry function's two outputs; a PRF would produce these
    // inside one assertion, at the two custody salts.
    let custody_key = custody_signer(&[21u8; 32]).await?;
    let kek = custody_kek(&[22u8; 32]);

    // The account provisions the custody DID, depositing the consent
    // the custody key minted — the ordinary provisioning contract.
    let device = Ed25519Signer::generate().await?;
    let link = delegation::mint_device_delegation(account.clone(), &device.did()).await?;
    let consent = custody::mint_custody_consent(custody_key.clone(), &account.did()).await?;
    let add = tonk_identity::request::build_provider_add_invocation(
        device,
        &link,
        &custody_key.did(),
        &consent,
        Some("custody"),
    )
    .await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(add)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "provisioning refused");

    // Seal the secret and publish the cell: permit, then presigned PUT.
    let secret = AccountSecret::generate()?;
    let sealed = kek.seal(&secret, KekMethod::Passkey)?.encode();
    // The ceremony's pre-signed shape: bounded to a month, redeemed
    // long after signing — what the worker drains once activation lands.
    let publish = custody::build_deferred_publish_invocation(custody_key.clone(), &sealed).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(publish)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "publish permit refused");
    let permit: Permit = serde_ipld_dagcbor::from_slice(&response.bytes().await?)?;
    let stored = permit.upload(sealed.clone()).await?;
    assert!(
        stored.status().is_success(),
        "storage PUT failed: {}",
        stored.status(),
    );

    // A fresh device resolves with nothing but the custody key.
    let resolve = custody::build_resolve_invocation(custody_key.clone()).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(resolve)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "resolve permit refused");
    let permit: Permit = serde_ipld_dagcbor::from_slice(&response.bytes().await?)?;
    let fetched = permit.send().await?;
    assert!(
        fetched.status().is_success(),
        "storage GET failed: {}",
        fetched.status(),
    );
    let bytes = fetched.bytes().await?.to_vec();
    assert_eq!(bytes, sealed, "the resolved cell is the sealed envelope");

    // The envelope opens back to the same account.
    let opened = custody_kek(&[22u8; 32]).open(&Envelope::decode(&bytes)?)?;
    assert_eq!(
        opened.signing_seed().as_ref(),
        secret.signing_seed().as_ref(),
        "the unwrapped secret derives the same account",
    );
    Ok(())
}

/// Enrollment is the only place the custody material is examined —
/// activation replays what was verified rather than checking it again —
/// so every one of these refusals is the difference between saying no
/// while the person is watching and handing them an account no second
/// device can open.
///
/// Each starts from a valid `Custody` and breaks exactly one thing, so a
/// refusal proves the check it names instead of tripping an earlier one.
mod custody {
    use super::*;

    async fn enroll(custody: Custody) -> Result<Answer, RegistrationError> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await.expect("customer");
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &custody,
        )
        .await;
        fixture.registration(&container).handle().await
    }

    /// The control. Without this every refusal below could be passing
    /// for a reason that has nothing to do with what it claims.
    #[dialog_common::test]
    async fn it_accepts_a_well_formed_enrollment() -> anyhow::Result<()> {
        let answer = enroll(Custody::default()).await;
        assert!(
            matches!(answer, Ok(Answer::Customer(_))),
            "a valid enrollment must be accepted, got {answer:?}"
        );
        Ok(())
    }

    /// The account's own bookkeeping space is named in the receipt, so a
    /// client knows where its record lives without asking again.
    #[dialog_common::test]
    async fn it_names_the_bookkeeping_space_in_the_receipt() -> anyhow::Result<()> {
        let Ok(Answer::Customer(receipt)) = enroll(Custody::default()).await else {
            panic!("a valid enrollment is accepted");
        };
        let space = receipt
            .customer_space
            .expect("the receipt names the customer space");
        assert!(
            !space.read_hex.is_empty(),
            "the space carries the account's authority to read it"
        );
        assert_ne!(
            space.did, receipt.customer,
            "the bookkeeping space is not the account itself"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_material_the_container_does_not_carry() -> anyhow::Result<()> {
        for missing in ["recovery", "consent", "sealed"] {
            let answer = enroll(Custody {
                omit: vec![missing],
                ..Default::default()
            })
            .await;
            assert!(
                matches!(answer, Err(RegistrationError::Invalid { .. })),
                "`{missing}` named but not carried must be refused, got {answer:?}"
            );
        }
        Ok(())
    }

    /// A recovery invocation acting on some other space would provision
    /// and write a custody namespace this enrollment never vouched for.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_for_a_different_space() -> anyhow::Result<()> {
        let stranger = Ed25519Signer::generate().await?;
        let answer = enroll(Custody {
            claimed_did: Some(stranger.did()),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_recovery_that_is_not_a_cell_write() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            command: ["use", "get", "memory", "cell"]
                .iter()
                .map(ToString::to_string)
                .collect(),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// The cell address is fixed. A write aimed anywhere else is not
    /// custody, whatever it claims.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_aimed_at_another_cell() -> anyhow::Result<()> {
        for custody in [
            Custody {
                space: "elsewhere".to_string(),
                ..Default::default()
            },
            Custody {
                cell: "elsewhere".to_string(),
                ..Default::default()
            },
        ] {
            let answer = enroll(custody).await;
            assert!(
                matches!(answer, Err(RegistrationError::Forbidden { .. })),
                "got {answer:?}"
            );
        }
        Ok(())
    }

    /// The invocation names its content by checksum and the bytes travel
    /// beside it. Unbound, activation would write an envelope nobody
    /// checked into the cell that recovers the account.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_that_checksums_other_content() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            checksum_over: Some(b"a different secret".to_vec()),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// `when` makes the write an overwrite, which a replayed activation
    /// link could use to destroy an envelope the passkey has since
    /// rotated.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_that_would_overwrite_the_cell() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            when: Some(b"etag-1".to_vec()),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// The one check that cannot be deferred: everything else verified
    /// here stays true until activation, but an invocation that lapses
    /// first cannot be re-minted, and the account is stranded exactly as
    /// it would have been without any of this.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_that_expires_before_the_link() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            // The fixture's activation TTL is an hour.
            expires_in: 60,
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Unauthorized { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// A consent given to one account must not enroll another, which is
    /// what stops someone claiming a custody space they merely saw.
    #[dialog_common::test]
    async fn it_refuses_a_consent_issued_to_someone_else() -> anyhow::Result<()> {
        let stranger = Ed25519Signer::generate().await?;
        let answer = enroll(Custody {
            consent_audience: Some(stranger.did()),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_refuses_a_consent_that_does_not_cover_provisioning() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            consent_command: vec!["unrelated".to_string()],
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// Nothing may be recorded by an enrollment that is refused: a
    /// customer row whose activation link cannot work is the stranded
    /// account this flow exists to prevent.
    #[dialog_common::test]
    async fn it_records_nothing_when_it_refuses() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &Custody {
                when: Some(b"etag-1".to_vec()),
                ..Default::default()
            },
        )
        .await;
        assert!(fixture.registration(&container).handle().await.is_err());
        assert!(
            fixture
                .store
                .customer(customer.did().as_str())
                .await?
                .is_none(),
            "a refused enrollment leaves no customer row"
        );
        assert!(
            fixture
                .emails
                .0
                .lock()
                .expect("captured email mutex poisoned")
                .is_empty(),
            "a refused enrollment sends no activation email"
        );
        Ok(())
    }
}

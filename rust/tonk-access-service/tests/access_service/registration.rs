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
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::cid::dagcbor_cid;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::time::timestamp::{Duration, Timestamp, UNIX_EPOCH};
use dialog_ucan_core::{Container, DelegationBuilder, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};
use tonk_access_service::email::CapturedEmail;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_access_service::registration::{Answer, Registration, SIGNUP_TERMS};
use tonk_access_service::store::Store;
use tonk_access_service::store::sqlite::SqliteStore;
use tonk_access_service::vault::CapturedVault;
use tonk_account::customer::{Receipt, RegistrationError};

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
    /// Custody cells this fixture wrote, so a test can see that
    /// enrolling put the sealed envelope somewhere.
    vault: CapturedVault,
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
            vault: CapturedVault::default(),
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
    ) -> Registration<
        'a,
        SqliteStore,
        CapturedEmail,
        IndexedRevocations<Arc<MemoryRevocationIndex>>,
        CapturedVault,
    > {
        Registration {
            store: &self.store,
            email: &self.emails,
            vault: &self.vault,
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
        Answer::Subscription(receipt) => panic!("expected a customer receipt, got {receipt:?}"),
        Answer::Done => panic!("expected a customer receipt, got an operator command's answer"),
    }
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
    /// Give the consent an expiration this many seconds from now.
    /// Negative puts it in the past. `None` leaves it open-ended, which
    /// is the honest case.
    consent_expires_in: Option<i64>,
    /// Issue the recovery through a delegate the custody key granted
    /// to, rather than self-signing it. `None` is the self-signed case.
    /// The delegation travels in the container as the invocation's
    /// proof, like any other chain.
    recovery_delegate: Option<[u8; 32]>,
}

impl Default for Custody {
    fn default() -> Self {
        Custody {
            key_seed: [9u8; 32],
            recovery_delegate: None,
            consent_expires_in: None,
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
    let _ = service;
    // A custody key PER ACCOUNT, derived from the enrolling key: a
    // custody space belongs to one account forever (the service refuses
    // a claim from another), and the fixed default seed quietly enrolled
    // every test account under one custodian. Deriving rather than
    // randomizing keeps re-enrollments of the SAME account sharing their
    // custody, which is what a real passkey does.
    let mut key_seed = [9u8; 32];
    let fingerprint = sha256_multihash(customer.did().as_str().as_bytes());
    key_seed.copy_from_slice(&fingerprint[2..34]);
    enroll_container_parts(
        customer,
        email,
        &Custody {
            key_seed,
            ..Custody::default()
        },
    )
    .await
    .0
}

/// An enrollment carrying `custody`, valid unless a knob says otherwise.
async fn enroll_container_with_custody(
    customer: &Ed25519Signer,
    service: &Did,
    email: &str,
    custody: &Custody,
) -> Vec<u8> {
    let _ = service;
    enroll_container_parts(customer, email, custody).await.0
}

/// The one place an enrollment container is assembled.
async fn enroll_container_parts(
    customer: &Ed25519Signer,
    email: &str,
    custody: &Custody,
) -> (Vec<u8>, Option<ipld_core::cid::Cid>, ipld_core::cid::Cid) {
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
    // Self-signed by default; through a delegate when the fixture asks,
    // which is the shape a passkey-to-profile grant produces.
    let (recovery_issuer, recovery_proofs, delegation) = match &custody.recovery_delegate {
        None => (key.clone(), vec![], None),
        Some(seed) => {
            let delegate = Ed25519Signer::import(seed).await.expect("delegate signer");
            let grant = DelegationBuilder::new()
                .issuer(dialog_credentials::Signer::from(key.clone()))
                .audience(&delegate.did())
                .subject(DelegatedSubject::Specific(key.did()))
                .command(custody.command.clone())
                .try_build()
                .await
                .expect("recovery delegation");
            (delegate, vec![grant.to_cid()], Some(grant))
        }
    };
    let recovery = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(recovery_issuer.clone()))
        .audience(&key.did())
        .subject(&key.did())
        .command(custody.command.clone())
        .arguments(arguments)
        .proofs(recovery_proofs)
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

    let mut consent = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(key.clone()))
        .audience(custody.consent_audience.as_ref().unwrap_or(&customer.did()))
        .subject(dialog_ucan_core::subject::Subject::Specific(key.did()))
        .command(custody.consent_command.clone());
    if let Some(seconds) = custody.consent_expires_in {
        let now = dialog_ucan_core::time::timestamp::SystemTime::now();
        let at = if seconds < 0 {
            now - dialog_ucan_core::time::timestamp::Duration::from_secs(seconds.unsigned_abs())
        } else {
            now + dialog_ucan_core::time::timestamp::Duration::from_secs(seconds as u64)
        };
        consent = consent.expiration(Timestamp::new(at).expect("consent expiration"));
    }
    let consent = consent.try_build().await.expect("consent");
    let consent_bytes = consent.encoded().to_vec();

    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(customer.clone()))
        .audience(&customer.did())
        .subject(&customer.did())
        .command(vec!["customer".to_string(), "enroll".to_string()])
        .arguments(BTreeMap::from([
            ("email".to_string(), Promised::String(email.to_string())),
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
    for (name, bytes) in [
        ("recovery", recovery_bytes),
        ("consent", consent_bytes),
        ("sealed", custody.sealed.clone()),
    ] {
        if !custody.omit.contains(&name) {
            tokens.push(bytes);
        }
    }
    // The recovery's own proof, when it was issued through a delegate:
    // the chain resolves from the container like any other.
    if let Some(grant) = &delegation {
        tokens.push(grant.encoded().to_vec());
    }
    let bytes = Container::new(tokens)
        .to_bytes()
        .expect("container encodes");
    (
        bytes,
        delegation.map(|grant| grant.to_cid()),
        consent.to_cid(),
    )
}

/// A `/customer/resend`, self-issued by the service. The account is an
/// argument because nobody waiting for the mail can sign as it.
async fn resend_container(service: &Ed25519Signer, account: &Did) -> Vec<u8> {
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(service.clone()))
        .audience(&service.did())
        .subject(&service.did())
        .command(vec!["customer".to_string(), "resend".to_string()])
        .arguments(BTreeMap::from([(
            "account".to_string(),
            Promised::String(account.to_string()),
        )]))
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .expect("resend invocation");
    InvocationChain::new(invocation, std::collections::HashMap::new())
        .to_bytes()
        .expect("container encodes")
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

/// Enrollment names the provider; activation does not repeat it.
///
/// The service decides which provider serves its customers and says so
/// in the receipt, so a client records one authoritative address instead
/// of deriving `https://{origin}/ucan/` from whichever origin its
/// request happened to reach.
///
/// At ENROLLMENT, because that is when a client needs it. The address is
/// where the account attaches its remote, and attaching it is what makes
/// the provisioning gate's 403 -> 200 observable: a client that has the
/// address learns it was activated from its own sync going through,
/// rather than asking a status endpoint. Withheld until activation, the
/// client had nowhere to sync and so no way to notice activation
/// happening, which is what the polling it replaced was for.
///
/// Registration state, not the address, is what says "awaiting the
/// email" -- and a client that syncs anyway is refused, retryably.
#[dialog_common::test]
async fn it_answers_the_provider_from_enrollment() -> anyhow::Result<()> {
    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;

    let enrolled = as_customer(fixture.registration(&container).handle().await.unwrap());
    assert_eq!(
        enrolled.provider.as_deref(),
        Some("https://hub.test/ucan/"),
        "enrollment names where this account will sync",
    );
    assert_eq!(
        serde_json::to_value(enrolled.status)?,
        "Registered",
        "having somewhere to sync is not being served yet",
    );

    // Activation carries no provider: it was settled at enrollment, and a
    // second copy is a second thing to disagree.
    let container = link_container(&fixture.last_email().1);
    let activated = as_customer(fixture.registration(&container).handle().await.unwrap());
    let wire = serde_json::to_value(&activated)?;
    assert!(
        wire.get("provider").is_none(),
        "activation must omit the provider key entirely, got {wire}",
    );
    assert_eq!(serde_json::to_value(activated.status)?, "Active");
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
    assert!(stored.verified_at > 0);

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
    // An impatient re-enrollment inside the resend interval keeps its
    // rows and sends nothing: enrollment is rate limited against its own
    // mail, so a loop of re-enrollments cannot pump messages at an
    // address its owner never confirmed.
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();
    assert_eq!(
        fixture
            .emails
            .0
            .lock()
            .expect("captured email mutex poisoned")
            .len(),
        1,
        "the interval suppressed the second mail"
    );
    // Past the interval, re-enrolling while registered is idempotent and
    // resends the link.
    let container = enroll_container(&customer, &service, "alice@example.com").await;
    let later = Registration {
        now: unix_now() + tonk_account::customer::RESEND_INTERVAL_SECONDS + 1,
        ..fixture.registration(&container)
    };
    later.handle().await.unwrap();
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
    assert_eq!(consumer.provider, customer.did().to_string());
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
    let Answer::Subscription(receipt) = fixture.registration(&container).handle().await.unwrap()
    else {
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
    assert_eq!(consumer.provider, customer.did().to_string());

    // Re-provisioning under the same customer succeeds and changes
    // nothing: clients retry provisioning freely (a queued entry
    // replayed twice, two devices racing), so it must be idempotent
    // rather than a conflict.
    let registered_at = consumer.registered_at;
    let container = add_container(&customer, &space, &customer.did()).await;
    let Answer::Subscription(receipt) = fixture.registration(&container).handle().await.unwrap()
    else {
        panic!("expected a consumer receipt");
    };
    assert_eq!(receipt.provider, customer.did());
    let again = fixture
        .store
        .consumer(space.did().as_str())
        .await
        .unwrap()
        .expect("the consumer row still exists");
    assert_eq!(again.provider, customer.did().to_string());
    assert_eq!(
        again.registered_at, registered_at,
        "re-provisioning must not re-register the consumer"
    );
    Ok(())
}

/// An unconfirmed customer may CLAIM a space, and still not be served.
///
/// Two different questions that were answered as one. Provisioning is a
/// fact about the space -- whose namespace it is -- and activation is a
/// fact about the customer. Refusing the claim conflated them: a second
/// device signing in before the emailed link was opened could not claim
/// the custody space its passkey needs, so the gate answered "is not
/// provisioned", a dead end no email clears, instead of "awaits email
/// activation", which one does.
///
/// The claim lands; the gate still refuses to serve, and refuses
/// retryably.
#[dialog_common::test]
async fn it_provisions_before_the_email_but_serves_only_after() -> anyhow::Result<()> {
    use dialog_capability::access::{AuthorizeError, Recourse};
    use tonk_access_service::provisioning::screen;

    let fixture = Fixture::new().await;
    let customer = Ed25519Signer::generate().await?;
    let space = Ed25519Signer::generate().await?;
    let container = enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
    fixture.registration(&container).handle().await.unwrap();

    // Enrolled, email unconfirmed: the space is claimed.
    let container = add_container(&customer, &space, &customer.did()).await;
    fixture.registration(&container).handle().await.unwrap();
    assert_eq!(
        fixture
            .store
            .consumer(space.did().as_str())
            .await
            .unwrap()
            .expect("the claim wrote a consumer row")
            .provider,
        customer.did().to_string(),
    );

    // Claimed is not served. The refusal says waiting clears it, and
    // names the email as what is outstanding.
    let refusal = screen(&fixture.store, space.did().as_str(), unix_now())
        .await
        .unwrap()
        .expect_err("an unconfirmed customer's space is not served");
    let AuthorizeError::Declined { recourse, reason } = refusal else {
        panic!("expected a declined refusal, got {refusal:?}");
    };
    assert_eq!(recourse, Recourse::Retry, "confirming the email clears it");
    assert!(
        reason.contains("awaits email activation"),
        "the refusal must name the email as what is outstanding, got {reason}",
    );

    // Confirming the email serves the space that was already claimed.
    let container = link_container(&fixture.last_email().1);
    fixture.registration(&container).handle().await.unwrap();
    screen(&fixture.store, space.did().as_str(), unix_now())
        .await
        .unwrap()
        .expect("activation serves the space claimed before it");
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
        consumer.provider,
        alice.did().to_string(),
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

/// A body over the limit is refused on size, before anything decodes
/// it: running the parser over an oversized request is the work the
/// limit exists to avoid, so the answer must not depend on the bytes
/// being a valid UCAN.
#[dialog_common::test]
async fn it_refuses_a_request_body_over_the_limit(env: AccessServiceAddress) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();

    // Not a container, not signed, not anything: only large.
    let response = client
        .post(format!("{base}/ucan/"))
        .header("Content-Type", "application/cbor")
        .body(vec![0u8; 64 * 1024 + 1])
        .send()
        .await?;
    assert_eq!(response.status(), 413, "an oversized body is refused");
    let body: serde_json::Value = response.json().await?;
    assert_eq!(body["error"]["code"], "PAYLOAD_TOO_LARGE");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("65536")),
        "the refusal names the limit, got {body}"
    );
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
    let resolve = custody::build_resolve_invocation(Signer::from(custody_key.clone())).await?;
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

    let account_resolve = custody::build_resolve_invocation(Signer::from(account.clone())).await?;
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

/// The ledger grant in the receipt is authority that works.
///
/// Enrollment hands back a `/use/get` over a space the service owns,
/// and the point of it is reading the record kept there. So: enroll,
/// activate, then present that exact delegation and get a presigned
/// URL. A receipt naming a grant the gate then refuses would be worse
/// than naming none.
#[dialog_common::test]
async fn it_presigns_a_ledger_read_with_the_grant_the_receipt_named(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let base = env.access_service_url.trim_end_matches('/').to_string();
    let ucan = format!("{base}/ucan/");
    let service_did: Did = env.service_did.parse()?;

    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "ledger@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "enrollment refused");
    let receipt: Receipt = response.json().await?;
    let ledger = receipt.ledger.expect("the receipt names the ledger");

    activate_over_http(&client, &base).await?;

    // The account invokes `/use/get` on the ledger space, proving with
    // the grant the receipt handed it.
    let grant =
        dialog_ucan_core::DelegationChain::try_from(hex::decode(&ledger.read_hex)?.as_slice())?;
    let read = tonk_identity::request::build_device_invocation(
        account.clone(),
        &grant,
        vec![
            "use".to_string(),
            "get".to_string(),
            "memory".to_string(),
            "cell".to_string(),
        ],
        BTreeMap::from([
            (
                "space".to_string(),
                Promised::String(ledger.did.to_string()),
            ),
            ("cell".to_string(), Promised::String("state".to_string())),
        ]),
    )
    .await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(read)
        .send()
        .await?;
    assert_eq!(
        response.status(),
        200,
        "the grant the receipt named must presign a read of the space it named: {}",
        response.text().await.unwrap_or_default()
    );
    Ok(())
}

/// Adding a passkey to an account that already exists, end to end
/// (`plan/Account custody.md`).
///
/// Not the signup path: enrollment writes the first passkey's cell
/// itself, and the key here is a different one, provisioned afterwards
/// through `/provider/add`. This is the flow for a second authenticator
/// — provision its DID, publish the account secret sealed under it, and
/// read that back from a device holding nothing but the passkey.
///
/// Six steps, each against the live service over HTTP. No repository
/// exists anywhere in this flow: the cell is raw bytes at one address.
#[dialog_common::test]
async fn it_adds_a_second_passkey_to_an_existing_account(
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

    // 1. Enroll. The account becomes a customer, `Registered`, and the
    // service emails an activation link. Nothing is served yet.
    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "custody@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(status, 200, "enrollment refused: {body}");

    // The account's own space is not served yet, which is what makes
    // the next step matter rather than being ceremony: `Registered`
    // denies everything behind this customer.
    let resolve_before = custody::build_resolve_invocation(Signer::from(account.clone())).await?;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(resolve_before)
        .send()
        .await?;
    assert_eq!(
        response.status(),
        403,
        "an unactivated customer is served nothing"
    );
    let refusal = response.text().await.unwrap_or_default();
    assert!(
        refusal.contains("awaits email activation"),
        "and it says so, rather than refusing for some other reason: {refusal}"
    );
    assert!(
        refusal.contains("Retry"),
        "clicking the link is what fixes it, so the client should try again: {refusal}"
    );

    // 2. Activate, by presenting the link from that email. This is
    // where the customer becomes `Active`, and it is what unlocks every
    // step below. `activate_over_http` reads the captured mail, takes
    // the invocation out of the link, and posts it back.
    activate_over_http(&client, &base).await?;

    // 3. Derive the new passkey's two keys. A real ceremony produces
    // both inside one PRF assertion, at the two custody salts; the
    // fixed bytes here stand in for that.
    let custody_key = custody_signer(&[21u8; 32]).await?;
    let kek = custody_kek(&[22u8; 32]);

    // 4. Provision the new custody DID as a consumer the account pays
    // for, depositing the consent that key minted. The ordinary
    // provisioning contract, nothing custody-specific.
    let device = Ed25519Signer::generate().await?;
    let link = delegation::mint_device_delegation(account.clone(), &device.did()).await?;
    let consent =
        custody::mint_custody_consent(Signer::from(custody_key.clone()), &account.did()).await?;
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

    // 5. Seal the account secret under the new passkey's KEK and write
    // it: the invocation buys a presigned permit, the permit carries the
    // PUT. Two round trips because the service signs but never holds the
    // bytes.
    let secret = AccountSecret::generate()?;
    let sealed = kek.seal(&secret, KekMethod::Passkey)?.encode();
    let publish = custody::build_publish_invocation(
        Signer::from(custody_key.clone()),
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

    // 6. And a device holding only the passkey reads it back. This is
    // the recovery the whole design exists for.
    let resolve = custody::build_resolve_invocation(Signer::from(custody_key.clone())).await?;
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
        opened.signer().await?.did(),
        secret.signer().await?.did(),
        "the unwrapped secret derives the same account",
    );
    Ok(())
}

/// A month-old pre-signed publish still redeems.
///
/// The same flow as [`it_adds_a_second_passkey_to_an_existing_account`],
/// differing in one line: the publish invocation carries the 30-day
/// expiry a ceremony signs rather than a five-minute one. That is the
/// whole question — the service bounds registration ceremonies to a
/// short window, and this pins that the bound does not reach a memory
/// permit a space signs for its own cell.
///
/// It matters because enrollment carries exactly this invocation and
/// verifies its expiry outlives the activation window. If a long-lived
/// one stopped redeeming, every signup would verify at enrollment and
/// fail at the write.
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

    // Setup, identical to the test named above: enroll, activate, and
    // provision a second passkey's DID. The question is the publish
    // below, not any of this.
    let account = Ed25519Signer::generate().await?;
    let container = enroll_container(&account, &service_did, "custody@example.com").await;
    let response = client
        .post(&ucan)
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;
    assert_eq!(response.status(), 200, "enrollment refused");
    activate_over_http(&client, &base).await?;

    let custody_key = custody_signer(&[21u8; 32]).await?;
    let kek = custody_kek(&[22u8; 32]);
    let device = Ed25519Signer::generate().await?;
    let link = delegation::mint_device_delegation(account.clone(), &device.did()).await?;
    let consent =
        custody::mint_custody_consent(Signer::from(custody_key.clone()), &account.did()).await?;
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
    // The ceremony's pre-signed shape: bounded to a month, and redeemed
    // long after signing. Enrollment now redeems the first passkey's
    // copy itself; this proves the shape survives the delay either way.
    let publish =
        custody::build_deferred_publish_invocation(Signer::from(custody_key.clone()), &sealed)
            .await?;
    // The same invocation enrollment verifies, so the command it accepts
    // and the one this endpoint redeems cannot drift apart. Reading it
    // back from the built container rather than restating it is the
    // point: a constant restated in two places is how they diverged.
    assert_eq!(
        InvocationChain::try_from(publish.as_slice())?.command().0,
        tonk_access_service::registration::CUSTODY_PUBLISH_COMMAND,
        "the command enrollment demands must be the one the service redeems"
    );
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
    let resolve = custody::build_resolve_invocation(Signer::from(custody_key.clone())).await?;
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
        opened.signer().await?.did(),
        secret.signer().await?.did(),
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
    use dialog_capability::access::{AuthorizeError, Recourse};
    use tonk_access_service::provisioning::screen;
    use tonk_account::customer::RESEND_INTERVAL_SECONDS;

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

    /// Enrollment provisions the passkey's custody space, so reading it
    /// before the email is confirmed is a WAIT, not a dead end.
    ///
    /// This is the second-device sign-in: someone enrolls on one device
    /// and signs in on another before opening the link. That device reads
    /// its custody cell to recover the account, and what the gate says
    /// about that read is the whole experience.
    ///
    /// Enrollment used to provision the customer and the ledger but not
    /// the custody space, so the read was refused `not provisioned` with
    /// `Recourse::None` — nothing to retry, nothing to do, and the browser
    /// reported "We couldn't finish logging you in". The space is claimed
    /// at enrollment now: the gate still refuses, because the customer is
    /// unconfirmed, but it refuses with `Retry` and names the email as the
    /// step that clears it.
    #[dialog_common::test]
    async fn it_tells_a_second_device_to_retry_after_the_email() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await.expect("customer");
        let custody = Custody::default();
        let custody_did = Ed25519Signer::import(&custody.key_seed)
            .await
            .expect("custody signer")
            .did();

        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "second-device@example.com",
            &custody,
        )
        .await;
        fixture
            .registration(&container)
            .handle()
            .await
            .expect("enrollment succeeds");

        // The read a signing-in device makes: `/use/get/memory/cell` over
        // its own custody space, which the gate screens by subject.
        let (recourse, reason) = match screen(&fixture.store, custody_did.as_str(), unix_now())
            .await
            .expect("the store answers")
        {
            Ok(()) => panic!("an unconfirmed customer must not be served"),
            Err(AuthorizeError::Declined { recourse, reason }) => (recourse, reason),
            Err(other) => panic!("the gate refused with {other:?}, not a declined request"),
        };

        assert_eq!(
            recourse,
            Recourse::Retry,
            "opening the emailed link is what clears this, so it is worth retrying: {reason}"
        );
        assert!(
            reason.contains("awaits email activation"),
            "and the reason has to name that step, got {reason:?}"
        );
        assert!(
            !reason.contains("not provisioned"),
            "the custody space is claimed at enrollment, so this is never the answer: {reason:?}"
        );

        // The ledger the receipt hands a read grant for answers the same
        // way. Both spaces are claimed by the same enrollment, and neither
        // is served until the customer confirms — so nothing about this is
        // special-cased per space: the gate screens a consumer by its
        // provider's registration, whatever the consumer is.
        let ledger = fixture
            .store
            .customer(customer.did().as_str())
            .await
            .expect("the store answers")
            .expect("enrollment wrote the customer")
            .ledger
            .expect("enrollment named a ledger");
        let (recourse, reason) = match screen(&fixture.store, &ledger, unix_now())
            .await
            .expect("the store answers")
        {
            Ok(()) => panic!("an unconfirmed customer's ledger must not be served"),
            Err(AuthorizeError::Declined { recourse, reason }) => (recourse, reason),
            Err(other) => panic!("the gate refused with {other:?}, not a declined request"),
        };
        assert_eq!(
            recourse,
            Recourse::Retry,
            "the ledger waits on the same email: {reason}"
        );
        assert!(
            reason.contains("awaits email activation"),
            "and says so the same way, got {reason:?}"
        );
        Ok(())
    }

    /// The bug this flow exists to prevent: a signup that finishes with
    /// the customer registered and no custody cell, so a second device
    /// has nothing to unlock the account from.
    ///
    /// Enrolling writes the cell. Not activation, not a queue the client
    /// drains later — the same act that records the customer.
    #[dialog_common::test]
    async fn it_writes_the_custody_cell_while_enrolling() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let custody = Custody::default();
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &custody,
        )
        .await;

        assert!(
            fixture.vault.sealed().is_empty(),
            "nothing is written before enrolling"
        );
        fixture
            .registration(&container)
            .handle()
            .await
            .expect("the enrollment is accepted");

        assert_eq!(
            fixture.vault.sealed(),
            vec![custody.sealed.clone()],
            "the sealed envelope the enrollment carried is what was stored"
        );

        // And it is there before activation, which is the point: the
        // cell does not wait on an email nobody may click.
        assert_eq!(
            fixture
                .store
                .customer(customer.did().as_str())
                .await?
                .expect("the customer row exists")
                .status,
            tonk_account::customer::CustomerStatus::Registered
        );
        Ok(())
    }

    /// Enrollment writes rows that lapse on their own, and activation
    /// is what makes them permanent.
    ///
    /// The whole state goes in at enrollment — cell, customer,
    /// subscriptions — and none of it is served while the customer is
    /// `Registered`. So a link nobody clicks leaves nothing to clean up:
    /// the rows expire at the same moment the link does.
    #[dialog_common::test]
    async fn it_expires_the_subscriptions_a_link_never_activates() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &Custody::default(),
        )
        .await;
        fixture
            .registration(&container)
            .handle()
            .await
            .expect("the enrollment is accepted");

        let subscription = fixture
            .store
            .consumer(customer.did().as_str())
            .await?
            .expect("enrolling writes the account's own subscription");
        let expires_at = subscription
            .expires_at
            .expect("it expires when the activation link does");

        // Not served now, and it would not be served later either: the
        // link is what clears the deadline.
        assert!(
            screen(&fixture.store, customer.did().as_str(), expires_at + 1)
                .await?
                .is_err(),
            "past the deadline an unactivated subscription is gone"
        );

        let activation = activation_invocation(
            &fixture.service,
            &fixture.service.did(),
            &customer.did(),
            Timestamp::five_minutes_from_now(),
        )
        .await;
        fixture
            .registration(&activation)
            .handle()
            .await
            .expect("the activation is accepted");

        assert_eq!(
            fixture
                .store
                .consumer(customer.did().as_str())
                .await?
                .expect("the subscription survives activation")
                .expires_at,
            None,
            "activating lifts the deadline"
        );
        assert!(
            screen(&fixture.store, customer.did().as_str(), expires_at + 1)
                .await?
                .is_ok(),
            "and the same moment that would have expired it now serves"
        );
        Ok(())
    }

    /// Resending is what a person does when the mail never arrived, so
    /// it sends again — and refuses to send twice in a row, because the
    /// button is right there and they will press it.
    #[dialog_common::test]
    async fn it_sends_the_activation_link_again_but_not_twice() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &Custody::default(),
        )
        .await;
        fixture.registration(&container).handle().await.unwrap();
        assert_eq!(
            fixture.emails.0.lock().expect("email mutex").len(),
            1,
            "enrolling sends the first"
        );

        let resend = resend_container(&fixture.service, &customer.did()).await;
        fixture
            .registration(&resend)
            .handle()
            .await
            .expect("resending is accepted");
        assert_eq!(
            fixture.emails.0.lock().expect("email mutex").len(),
            1,
            "the enrollment just sent one, so this is too soon"
        );

        // Past the interval it sends, and to the address on the row
        // rather than anything the caller supplied.
        let later = Registration {
            now: unix_now() + RESEND_INTERVAL_SECONDS + 1,
            ..fixture.registration(&resend)
        };
        later.handle().await.expect("resending is accepted");
        let sent = fixture.emails.0.lock().expect("email mutex").clone();
        assert_eq!(sent.len(), 2, "past the interval it sends again");
        assert_eq!(sent[1].0, "alice@example.com");
        Ok(())
    }

    /// A resend needs no service key: the waiting person's own device
    /// signs as itself, about the account it cannot yet sign for. The
    /// command is deliberately unauthenticated beyond that signature —
    /// the mail only goes to the address on the row, and the interval
    /// bounds it — and requiring the service as issuer made the command
    /// uninvocable by the only party that ever wants it.
    #[dialog_common::test]
    async fn it_resends_for_a_self_subjected_client_invocation() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container =
            enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
        fixture.registration(&container).handle().await.unwrap();

        // Signed by a key the service has never seen: the device's own.
        let device = Ed25519Signer::generate().await?;
        let resend = resend_container(&device, &customer.did()).await;
        let later = Registration {
            now: unix_now() + RESEND_INTERVAL_SECONDS + 1,
            ..fixture.registration(&resend)
        };
        later.handle().await.expect("a client resend is accepted");
        let sent = fixture.emails.0.lock().expect("email mutex").clone();
        assert_eq!(sent.len(), 2, "the link went out again");
        assert_eq!(sent[1].0, "alice@example.com");
        Ok(())
    }

    /// An activated customer is never mailed: the link would do nothing,
    /// and mailing on demand for any account anyone names is a nuisance
    /// worth closing off.
    #[dialog_common::test]
    async fn it_does_not_resend_to_an_active_customer() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        fixture
            .enroll_and_activate(&customer, "alice@example.com")
            .await;
        let before = fixture.emails.0.lock().expect("email mutex").len();

        let resend = resend_container(&fixture.service, &customer.did()).await;
        let later = Registration {
            now: unix_now() + RESEND_INTERVAL_SECONDS + 1,
            ..fixture.registration(&resend)
        };
        later.handle().await.expect("resending is accepted");
        assert_eq!(
            fixture.emails.0.lock().expect("email mutex").len(),
            before,
            "an active customer gets no activation mail"
        );
        Ok(())
    }

    /// An account nobody enrolled is answered the same way as one that
    /// asked too soon: silently. Refusing would tell a caller which
    /// addresses have accounts.
    #[dialog_common::test]
    async fn it_says_nothing_about_an_account_it_does_not_have() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let stranger = Ed25519Signer::generate().await?;
        let resend = resend_container(&fixture.service, &stranger.did()).await;

        fixture
            .registration(&resend)
            .handle()
            .await
            .expect("resending an unknown account is not an error");
        assert!(fixture.emails.0.lock().expect("email mutex").is_empty());
        Ok(())
    }

    /// A refused enrollment writes no cell. Everything is verified
    /// before anything is stored, so a malformed one leaves nothing
    /// behind — in the vault as much as in the database.
    #[dialog_common::test]
    async fn it_writes_no_cell_when_it_refuses() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container = enroll_container_with_custody(
            &customer,
            &fixture.service.did(),
            "alice@example.com",
            &Custody {
                // Signed by a key that is not the custody DID it claims.
                claimed_did: Some(Ed25519Signer::generate().await?.did()),
                ..Custody::default()
            },
        )
        .await;

        assert!(fixture.registration(&container).handle().await.is_err());
        assert!(
            fixture.vault.sealed().is_empty(),
            "a refused enrollment stores nothing"
        );
        Ok(())
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
            .ledger
            .expect("the receipt names the customer space");
        assert_ne!(
            space.did, receipt.customer,
            "the bookkeeping space is not the account itself"
        );

        // The authority the receipt hands over, in full: a `/use/get`
        // the space issued to the account. Read means the service keeps
        // its own ledger out of the customer's reach.
        let bytes = hex::decode(&space.read_hex)?;
        let chain = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())?;
        let read = chain.proofs().next().expect("the chain carries a grant");
        assert_eq!(
            read.command().segments(),
            &["use".to_string(), "get".to_string()],
            "the account may read its ledger and nothing more"
        );
        assert_eq!(read.audience(), &receipt.customer, "issued to the account");
        assert_eq!(
            read.subject(),
            &DelegatedSubject::Specific(space.did.clone()),
            "over the ledger space"
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

    /// Recovery need not be self-signed: a passkey may delegate to a
    /// profile that issues the setup, and the chain verifies like any
    /// other invocation the endpoint accepts.
    #[dialog_common::test]
    async fn it_accepts_a_recovery_issued_through_a_delegate() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            recovery_delegate: Some([21u8; 32]),
            ..Default::default()
        })
        .await;
        assert!(answer.is_ok(), "got {answer:?}");
        Ok(())
    }

    /// A recovery whose delegation was revoked is refused. This is the
    /// case that proves the nested chain is verified for real rather
    /// than merely parsed: the same checker the outer chain uses.
    #[dialog_common::test]
    async fn it_refuses_a_recovery_whose_delegation_was_revoked() -> anyhow::Result<()> {
        use tonk_access_service::revocation::index::RevocationIndex as _;

        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let custody = Custody {
            recovery_delegate: Some([21u8; 32]),
            ..Default::default()
        };

        let (container, grant, _) =
            enroll_container_parts(&customer, "alice@example.com", &custody).await;
        // The exact grant the container carries — a delegation carries a
        // nonce, so a rebuilt one would name a different CID.
        let grant = grant.expect("a delegated recovery carries its proof");
        let key = Ed25519Signer::import(&custody.key_seed).await?;
        fixture
            .revocations
            .0
            .record(&grant.to_string(), key.did().as_ref())
            .await?;
        let answer = fixture.registration(&container).handle().await;
        assert!(
            matches!(answer, Err(RegistrationError::Unauthorized { .. })),
            "a revoked proof must refuse the enrollment, got {answer:?}"
        );
        Ok(())
    }

    /// A consent whose authority was withdrawn is not consent: the
    /// delegation is checked against the same revocations the chain is.
    #[dialog_common::test]
    async fn it_refuses_a_consent_that_was_revoked() -> anyhow::Result<()> {
        use tonk_access_service::revocation::index::RevocationIndex as _;

        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let custody = Custody::default();
        let (container, _, consent) =
            enroll_container_parts(&customer, "alice@example.com", &custody).await;

        let key = Ed25519Signer::import(&custody.key_seed).await?;
        fixture
            .revocations
            .0
            .record(&consent.to_string(), key.did().as_ref())
            .await?;

        let answer = fixture.registration(&container).handle().await;
        assert!(
            matches!(answer, Err(RegistrationError::Unauthorized { .. })),
            "a revoked consent must refuse the enrollment, got {answer:?}"
        );
        Ok(())
    }

    /// One address holds one customer, so a second account cannot claim
    /// an address the first already registered.
    #[dialog_common::test]
    async fn it_refuses_an_address_registered_to_another_account() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let first = Ed25519Signer::generate().await?;
        let container = enroll_container(&first, &fixture.service.did(), "alice@example.com").await;
        fixture.registration(&container).handle().await?;

        let second = Ed25519Signer::generate().await?;
        let container =
            enroll_container(&second, &fixture.service.did(), "alice@example.com").await;
        let answer = fixture.registration(&container).handle().await;
        assert!(
            matches!(answer, Err(RegistrationError::AddressTaken)),
            "got {answer:?}"
        );
        Ok(())
    }

    /// The other direction: an account that enrolled once may correct
    /// the address it named, because the row is keyed on the account.
    #[dialog_common::test]
    async fn it_lets_a_registered_account_change_its_address() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let container =
            enroll_container(&customer, &fixture.service.did(), "alice@example.com").await;
        fixture.registration(&container).handle().await?;

        let container =
            enroll_container(&customer, &fixture.service.did(), "alice2@example.com").await;
        let answer = fixture.registration(&container).handle().await;
        assert!(answer.is_ok(), "got {answer:?}");
        Ok(())
    }

    /// A consent that has lapsed is refused: activation provisions from
    /// what enrollment verified, so a consent alive only now would let
    /// the provisioning it authorizes happen after it expired.
    #[dialog_common::test]
    async fn it_refuses_an_expired_consent() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            consent_expires_in: Some(-60),
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Unauthorized { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// An enrollment naming none of the custody arguments is refused
    /// before anything is recorded — the fields are what make an
    /// account openable on a second device.
    #[dialog_common::test]
    async fn it_refuses_an_enrollment_with_no_custody_arguments() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let customer = Ed25519Signer::generate().await?;
        let invocation = InvocationBuilder::new()
            .issuer(dialog_credentials::Signer::from(customer.clone()))
            .audience(&customer.did())
            .subject(&customer.did())
            .command(vec!["customer".to_string(), "enroll".to_string()])
            .arguments(BTreeMap::from([(
                "email".to_string(),
                Promised::String("alice@example.com".to_string()),
            )]))
            .proofs(vec![])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await?;
        let container =
            Container::new(vec![serde_ipld_dagcbor::to_vec(&invocation)?]).to_bytes()?;

        let answer = fixture.registration(&container).handle().await;
        assert!(
            matches!(answer, Err(RegistrationError::Invalid { .. })),
            "an enrollment naming no custody must be refused, got {answer:?}"
        );
        assert!(
            fixture.vault.sealed().is_empty(),
            "and writes no cell for it"
        );
        Ok(())
    }

    /// One custody space belongs to one account, forever. The custody
    /// DID is PRF-derived from the passkey and the sealed secret in its
    /// cell IS the account — the same passkey always reopens the same
    /// account — so a claim from a different account is never
    /// legitimate, and its cell could not be replaced anyway: the vault
    /// write is create-only. The refusal lands before anything durable,
    /// and points at signing in instead.
    #[dialog_common::test]
    async fn it_refuses_a_custodian_enrolled_to_another_account() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let custody = Custody::default();

        let first = Ed25519Signer::generate().await?;
        let (container, _, _) = enroll_container_parts(&first, "alice@example.com", &custody).await;
        fixture.registration(&container).handle().await?;

        let second = Ed25519Signer::generate().await?;
        let (container, _, _) = enroll_container_parts(&second, "bob@example.com", &custody).await;
        let answer = fixture.registration(&container).handle().await;
        assert!(
            matches!(answer, Err(RegistrationError::Forbidden { .. })),
            "a second account claiming the same custodian is refused, got {answer:?}"
        );
        assert_eq!(
            fixture.vault.sealed().len(),
            1,
            "and the first account's cell stands alone"
        );
        assert!(
            fixture
                .store
                .customer(second.did().as_str())
                .await
                .unwrap()
                .is_none(),
            "a refused enrollment leaves no customer row"
        );
        Ok(())
    }

    /// The cell is written before the customer is served, so an
    /// unbounded `sealed` would be a store anyone could write to for
    /// free. A real envelope is 68 bytes.
    #[dialog_common::test]
    async fn it_refuses_a_sealed_secret_over_the_limit() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            sealed: vec![0u8; 257],
            ..Default::default()
        })
        .await;
        assert!(
            matches!(answer, Err(RegistrationError::Invalid { .. })),
            "got {answer:?}"
        );
        Ok(())
    }

    /// The bound is headroom for a format change, so an envelope at the
    /// limit is still accepted.
    #[dialog_common::test]
    async fn it_accepts_a_sealed_secret_at_the_limit() -> anyhow::Result<()> {
        let answer = enroll(Custody {
            sealed: vec![0u8; 256],
            ..Default::default()
        })
        .await;
        assert!(answer.is_ok(), "got {answer:?}");
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

/// The operator commands, driven the way an operator tool drives them:
/// a signed invocation on the service's own subject, posted through the
/// same handler as everything else.
///
/// These are about the gate, so each one suspends or archives and then
/// asks `screen` what a client would be told. Poking the columns
/// directly would test the column rather than the command.
mod operator {
    use super::*;
    use dialog_capability::access::{AuthorizeError, Recourse};
    use tonk_access_service::provisioning::screen;
    use tonk_access_service::registration::{ARCHIVE_COMMAND, RESUME_COMMAND, SUSPEND_COMMAND};

    /// Sign an operator command on the service's own subject.
    async fn operator_container(
        service: &Ed25519Signer,
        command: [&str; 4],
        arguments: BTreeMap<String, Promised>,
    ) -> Vec<u8> {
        let invocation = InvocationBuilder::new()
            .issuer(dialog_credentials::Signer::from(service.clone()))
            .audience(&service.did())
            .subject(&service.did())
            .command(command.iter().map(ToString::to_string).collect())
            .arguments(arguments)
            .proofs(vec![])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .expect("operator invocation");
        InvocationChain::new(invocation, std::collections::HashMap::new())
            .to_bytes()
            .expect("container encodes")
    }

    fn consumer_argument(consumer: &Did) -> BTreeMap<String, Promised> {
        BTreeMap::from([(
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        )])
    }

    /// An active customer with one provisioned space, which is what
    /// every case here starts from.
    async fn served_space(fixture: &Fixture) -> (Ed25519Signer, Ed25519Signer) {
        let customer = Ed25519Signer::generate().await.expect("customer key");
        let space = Ed25519Signer::generate().await.expect("space key");
        fixture
            .enroll_and_activate(&customer, "alice@example.com")
            .await;
        let container = add_container(&customer, &space, &customer.did()).await;
        fixture
            .registration(&container)
            .handle()
            .await
            .expect("the space is provisioned");
        (customer, space)
    }

    /// What the gate says about `space` at `now`.
    async fn verdict(
        fixture: &Fixture,
        space: &Ed25519Signer,
        now: u64,
    ) -> Option<(Recourse, String)> {
        match screen(&fixture.store, space.did().as_str(), now)
            .await
            .expect("the store answers")
        {
            Ok(()) => None,
            Err(AuthorizeError::Declined { recourse, reason }) => Some((recourse, reason)),
            Err(other) => panic!("the gate refused with {other:?}, not a declined request"),
        }
    }

    /// The round trip: a served space, suspended, refused with the
    /// reason the operator gave, then resumed and served again.
    #[dialog_common::test]
    async fn it_suspends_a_subscription_and_resumes_it() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let (_, space) = served_space(&fixture).await;
        let now = unix_now();
        assert_eq!(verdict(&fixture, &space, now).await, None);

        let suspend = operator_container(
            &fixture.service,
            SUSPEND_COMMAND,
            BTreeMap::from([
                (
                    "consumer".to_string(),
                    Promised::String(space.did().to_string()),
                ),
                ("code".to_string(), Promised::String("unpaid".to_string())),
                (
                    "reason".to_string(),
                    Promised::String("the subscription is past due".to_string()),
                ),
            ]),
        )
        .await;
        fixture
            .registration(&suspend)
            .handle()
            .await
            .expect("the suspension is recorded");

        let (recourse, reason) = verdict(&fixture, &space, now)
            .await
            .expect("a suspended space is not served");
        assert_eq!(
            recourse,
            Recourse::None,
            "an indefinite suspension is not worth retrying"
        );
        assert!(
            reason.contains("past due"),
            "the refusal carries the operator's own words, got: {reason}"
        );

        let resume = operator_container(
            &fixture.service,
            RESUME_COMMAND,
            consumer_argument(&space.did()),
        )
        .await;
        fixture
            .registration(&resume)
            .handle()
            .await
            .expect("the suspension is lifted");
        assert_eq!(
            verdict(&fixture, &space, now).await,
            None,
            "resuming serves the space again"
        );
        Ok(())
    }

    /// A deadline lifts the suspension by itself. The row still carries
    /// the reason afterwards; what changed is that it no longer applies.
    #[dialog_common::test]
    async fn it_lifts_a_suspension_once_its_deadline_passes() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let (_, space) = served_space(&fixture).await;
        let now = unix_now();

        let suspend = operator_container(
            &fixture.service,
            SUSPEND_COMMAND,
            BTreeMap::from([
                (
                    "consumer".to_string(),
                    Promised::String(space.did().to_string()),
                ),
                ("code".to_string(), Promised::String("review".to_string())),
                (
                    "reason".to_string(),
                    Promised::String("under review until tomorrow".to_string()),
                ),
                ("until".to_string(), Promised::Integer((now + 60) as i128)),
            ]),
        )
        .await;
        fixture
            .registration(&suspend)
            .handle()
            .await
            .expect("the suspension is recorded");

        let (recourse, _) = verdict(&fixture, &space, now)
            .await
            .expect("the deadline has not passed");
        assert_eq!(
            recourse,
            Recourse::Retry,
            "a suspension that lifts itself is worth waiting out"
        );

        assert_eq!(
            verdict(&fixture, &space, now + 61).await,
            None,
            "past its deadline the suspension no longer applies"
        );
        Ok(())
    }

    /// A suspension already past when it is written never applies. The
    /// operator gets no error: recording a lapsed deadline is a
    /// statement about the past, not a failed command.
    #[dialog_common::test]
    async fn it_serves_a_subscription_suspended_until_a_past_moment() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let (_, space) = served_space(&fixture).await;
        let now = unix_now();

        let suspend = operator_container(
            &fixture.service,
            SUSPEND_COMMAND,
            BTreeMap::from([
                (
                    "consumer".to_string(),
                    Promised::String(space.did().to_string()),
                ),
                ("code".to_string(), Promised::String("stale".to_string())),
                (
                    "reason".to_string(),
                    Promised::String("a hold that has already expired".to_string()),
                ),
                ("until".to_string(), Promised::Integer((now - 1) as i128)),
            ]),
        )
        .await;
        fixture
            .registration(&suspend)
            .handle()
            .await
            .expect("the suspension is recorded");

        assert_eq!(
            verdict(&fixture, &space, now).await,
            None,
            "a deadline in the past withholds nothing"
        );
        Ok(())
    }

    /// Archival drops the data, so the space stops being served and
    /// stays that way: unlike a suspension, nothing lifts it.
    #[dialog_common::test]
    async fn it_stops_serving_an_archived_subscription() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let (_, space) = served_space(&fixture).await;
        let now = unix_now();

        let archive = operator_container(
            &fixture.service,
            ARCHIVE_COMMAND,
            consumer_argument(&space.did()),
        )
        .await;
        fixture
            .registration(&archive)
            .handle()
            .await
            .expect("the archival is recorded");

        let (recourse, reason) = verdict(&fixture, &space, now)
            .await
            .expect("an archived space is not served");
        assert_eq!(recourse, Recourse::None);
        assert!(
            reason.contains("archived"),
            "the refusal says what happened, got: {reason}"
        );

        // Resuming is about suspension, and this is not one.
        let resume = operator_container(
            &fixture.service,
            RESUME_COMMAND,
            consumer_argument(&space.did()),
        )
        .await;
        fixture
            .registration(&resume)
            .handle()
            .await
            .expect("resume is accepted");
        assert!(
            verdict(&fixture, &space, now).await.is_some(),
            "resuming does not bring archived data back"
        );
        Ok(())
    }

    /// The subject is the service, so a customer cannot suspend anyone
    /// — including themselves, and including a space they own.
    #[dialog_common::test]
    async fn it_refuses_an_operator_command_from_a_customer() -> anyhow::Result<()> {
        let fixture = Fixture::new().await;
        let (customer, space) = served_space(&fixture).await;

        let invocation = InvocationBuilder::new()
            .issuer(dialog_credentials::Signer::from(customer.clone()))
            .audience(&customer.did())
            .subject(&customer.did())
            .command(SUSPEND_COMMAND.iter().map(ToString::to_string).collect())
            .arguments(BTreeMap::from([
                (
                    "consumer".to_string(),
                    Promised::String(space.did().to_string()),
                ),
                ("code".to_string(), Promised::String("mine".to_string())),
                (
                    "reason".to_string(),
                    Promised::String("I say so".to_string()),
                ),
            ]))
            .proofs(vec![])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await?;
        let container = InvocationChain::new(invocation, std::collections::HashMap::new())
            .to_bytes()
            .expect("container encodes");

        assert!(
            fixture.registration(&container).handle().await.is_err(),
            "an operator command on any subject but the service is refused"
        );
        assert_eq!(
            verdict(&fixture, &space, unix_now()).await,
            None,
            "and the space is still served"
        );
        Ok(())
    }
}

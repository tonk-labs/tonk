use super::*;
use crate::email::CapturedEmail;
use crate::revocation::{
    checker::IndexedRevocations,
    index::{MemoryRevocationIndex, RevocationIndex},
};
use crate::store::{Enrollment, SIGNUP_PLAN, SubscriptionKind, sqlite::SqliteStore};
use dialog_ucan_core::{DelegationBuilder, subject::Subject};

struct Fixture {
    store: SqliteStore,
    directory: tempfile::TempDir,
    sender: Ed25519Signer,
    carol: Ed25519Signer,
    mallory: Ed25519Signer,
    space: Ed25519Signer,
    service: Ed25519Signer,
    seed: String,
    email: CapturedEmail,
    revocations: IndexedRevocations<MemoryRevocationIndex>,
    now: u64,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&directory.path().join("control.sqlite")).unwrap();
        let seed = "11".repeat(32);
        let fixture = Self {
            store,
            directory,
            sender: Ed25519Signer::generate().await.unwrap(),
            carol: Ed25519Signer::generate().await.unwrap(),
            mallory: Ed25519Signer::generate().await.unwrap(),
            space: Ed25519Signer::generate().await.unwrap(),
            service: crate::service::signer_from_hex(&seed).unwrap(),
            seed,
            email: CapturedEmail::default(),
            revocations: IndexedRevocations(MemoryRevocationIndex::default()),
            now: Timestamp::now().to_unix(),
        };
        fixture
            .enroll(&fixture.sender, "bob@example.com", true)
            .await;
        fixture
            .enroll(&fixture.mallory, "mallory@example.com", true)
            .await;
        fixture
            .store
            .add_subscription(
                fixture.space.did().as_str(),
                fixture.sender.did().as_str(),
                fixture.now,
                SubscriptionKind::Space,
            )
            .await
            .unwrap();
        fixture
    }
    /// An invocation expiry five minutes past the fixture's clock. The
    /// service bounds expiry by its own `now`, the fixture's, so one read off
    /// the wall clock is refused once a second has ticked since the fixture
    /// was made.
    fn expiration(&self) -> Timestamp {
        Timestamp::new(std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.now + 300))
            .unwrap()
    }
    fn service(
        &self,
    ) -> InvitationService<'_, SqliteStore, CapturedEmail, IndexedRevocations<MemoryRevocationIndex>>
    {
        InvitationService {
            store: &self.store,
            email: &self.email,
            service: &self.service,
            service_seed: &self.seed,
            origin: "https://tonk.example",
            now: self.now,
            revocations: &self.revocations,
        }
    }
    async fn enroll(&self, actor: &Ed25519Signer, email: &str, active: bool) {
        let ledger = Ed25519Signer::generate().await.unwrap().did().to_string();
        let custody = Ed25519Signer::generate().await.unwrap().did().to_string();
        self.store
            .enroll_customer(Enrollment {
                did: actor.did().as_str(),
                email,
                plan: SIGNUP_PLAN,
                ledger: &ledger,
                custody: &custody,
                now: self.now,
                expires_at: self.now + 86400,
            })
            .await
            .unwrap();
        if active {
            self.store
                .activate_customer(actor.did().as_str(), "test", self.now)
                .await
                .unwrap();
        }
    }
    async fn request(
        &self,
        actor: &Ed25519Signer,
        root: &Did,
        proof: Option<&DelegationChain>,
        operation: &str,
        args: Vec<(&str, String)>,
    ) -> Result<Value, InvitationError> {
        let call = InvocationBuilder::new()
            .issuer(Signer::from(actor.clone()))
            .audience(&self.service.did())
            .subject(root)
            .command(vec!["email-invitation".into(), operation.into()])
            .expiration(self.expiration())
            .arguments(
                args.into_iter()
                    .map(|(key, value)| (key.into(), Promised::String(value)))
                    .collect(),
            )
            .proofs(proof.map(|p| p.proof_cids().to_vec()).unwrap_or_default())
            .try_build()
            .await
            .unwrap();
        let body = InvocationChain::new(
            call,
            proof.map(|p| p.export().collect()).unwrap_or_default(),
        )
        .to_bytes()
        .unwrap();
        self.service().handle(&body).await
    }
    async fn prepare(&self, id: &str, email: &str) -> Result<Value, InvitationError> {
        self.request(
            &self.sender,
            &self.sender.did(),
            None,
            "prepare",
            vec![
                ("invitation", id.into()),
                ("email", email.into()),
                ("space", self.space.did().to_string()),
            ],
        )
        .await
    }
    async fn original(&self, id: &str, recipient: &Did) -> String {
        let root = DelegationBuilder::new()
            .issuer(Signer::from(self.space.clone()))
            .audience(&self.sender.did())
            .subject(Subject::Specific(self.space.did()))
            .command(vec!["use".into()])
            .try_build()
            .await
            .unwrap();
        let mut meta = tonk_invite::inviter_meta(&self.sender.did());
        meta.extend(tonk_invite::home_address_meta(
            &"https://tonk.example/ucan/".parse().unwrap(),
        ));
        meta.insert(DELIVERY_ID.into(), Ipld::String(id.into()));
        let leaf = DelegationBuilder::new()
            .issuer(Signer::from(self.sender.clone()))
            .audience(recipient)
            .subject(Subject::Specific(self.space.did()))
            .command(vec!["use".into()])
            .meta(meta)
            .try_build()
            .await
            .unwrap();
        bs58::encode(
            DelegationChain::new(root)
                .push(leaf)
                .unwrap()
                .to_bytes()
                .unwrap(),
        )
        .into_string()
    }
    async fn send(&self, id: &str, original: &str) -> Result<Value, InvitationError> {
        self.request(
            &self.sender,
            &self.sender.did(),
            None,
            "send",
            vec![("invitation", id.into()), ("original", original.into())],
        )
        .await
    }
    async fn claim(&self, id: &str, actor: &Ed25519Signer) -> Result<Value, InvitationError> {
        self.request(
            actor,
            &actor.did(),
            None,
            "claim",
            vec![("invitation", id.into())],
        )
        .await
    }
}

#[dialog_common::test]
async fn it_dates_invocations_from_the_fixture_clock() {
    // A second boundary passing between fixture setup and a request leaves
    // the service's `now` one second behind the wall clock.
    let mut f = Fixture::new().await;
    f.now -= 1;
    f.prepare(&"e".repeat(64), "carol@example.com")
        .await
        .unwrap();
}

#[dialog_common::test]
async fn confirmed_recipient_claims_once_across_restart_and_concurrent_retries() {
    let mut f = Fixture::new().await;
    let id = "a".repeat(64);
    let prepared = f.prepare(&id, " CAROL@Example.com ").await.unwrap();
    let recipient = prepared["recipientRoot"].as_str().unwrap().parse().unwrap();
    let original = f.original(&id, &recipient).await;
    f.send(&id, &original).await.unwrap();
    f.send(&id, &original).await.unwrap();
    assert_eq!(f.email.0.lock().unwrap().len(), 1);
    assert!(matches!(
        f.claim(&id, &f.mallory).await,
        Err(InvitationError::Forbidden(Refusal::EmailMismatch))
    ));
    f.enroll(&f.carol, "carol@example.com", false).await;
    assert!(matches!(
        f.claim(&id, &f.carol).await,
        Err(InvitationError::Unconfirmed)
    ));
    f.store
        .activate_customer(f.carol.did().as_str(), "test", f.now)
        .await
        .unwrap();
    f.store = SqliteStore::open(&f.directory.path().join("control.sqlite")).unwrap();
    let (one, two) = futures_util::join!(f.claim(&id, &f.carol), f.claim(&id, &f.carol));
    let one = one.unwrap();
    assert_eq!(one, two.unwrap());
    let envelope: DeliveryEnvelope = serde_json::from_value(one).unwrap();
    let invite = envelope.into_invite(&f.carol.did(), &id).await.unwrap();
    assert_eq!(
        tonk_invite::inviter(invite.invitation_chain()).unwrap(),
        Some(f.sender.did())
    );
    assert_eq!(invite.subject(), &f.space.did());
    assert!(matches!(
        f.claim(&id, &f.mallory).await,
        Err(InvitationError::Forbidden(Refusal::OtherRecipient))
    ));
    assert!(matches!(
        f.prepare(&id, "other@example.com").await,
        Err(InvitationError::Conflict)
    ));
}

#[dialog_common::test]
async fn cancellation_expiry_and_revoked_sender_device_refuse() {
    let mut f = Fixture::new().await;
    let id = "b".repeat(64);
    f.prepare(&id, "carol@example.com").await.unwrap();
    f.request(
        &f.sender,
        &f.sender.did(),
        None,
        "cancel",
        vec![("invitation", id.clone())],
    )
    .await
    .unwrap();
    assert!(matches!(
        f.prepare(&id, "carol@example.com").await,
        Err(InvitationError::Gone)
    ));
    let expired = "c".repeat(64);
    f.prepare(&expired, "carol@example.com").await.unwrap();
    f.now += LIFETIME;
    assert!(matches!(
        f.service()
            .live(&f.store.invitation(&expired).await.unwrap().unwrap()),
        Err(InvitationError::Gone)
    ));
    f.now -= LIFETIME;
    let device = Ed25519Signer::generate().await.unwrap();
    let identity = DelegationBuilder::new()
        .issuer(Signer::from(f.sender.clone()))
        .audience(&device.did())
        .subject(Subject::Any)
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    f.revocations
        .0
        .record(&identity.to_cid().to_string(), f.sender.did().as_str(), b"")
        .await
        .unwrap();
    let proof = DelegationChain::new(identity);
    assert!(matches!(
        f.request(
            &device,
            &f.sender.did(),
            Some(&proof),
            "prepare",
            vec![
                ("invitation", "d".repeat(64)),
                ("email", "carol@example.com".into()),
                ("space", f.space.did().to_string())
            ]
        )
        .await,
        Err(InvitationError::Forbidden(Refusal::Authority))
    ));
}

#[dialog_common::test]
async fn storage_limits_delivery_leases_and_claim_cancel_races() {
    let f = Fixture::new().await;
    for index in 0..5 {
        f.prepare(&format!("{index:064x}"), "carol@example.com")
            .await
            .unwrap();
    }
    assert!(matches!(
        f.prepare(&format!("{:064x}", 5), "carol@example.com").await,
        Err(InvitationError::Limited)
    ));
    let id = format!("{:064x}", 0);
    let row = f.store.invitation(&id).await.unwrap().unwrap();
    assert_eq!(row.attempts, 0);
    assert!(
        f.store
            .publish_invitation(&id, "test-original", f.now)
            .await
            .unwrap()
    );
    assert!(
        !f.store
            .publish_invitation(&id, "replacement", f.now)
            .await
            .unwrap()
    );
    let (a, b) = futures_util::join!(
        f.store.lease_invitation(&id, f.now),
        f.store.lease_invitation(&id, f.now)
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert!(f.store.lease_invitation(&id, f.now + 60).await.unwrap());
    assert!(f.store.lease_invitation(&id, f.now + 120).await.unwrap());
    assert!(!f.store.lease_invitation(&id, f.now + 180).await.unwrap());
    f.enroll(&f.carol, "carol@example.com", true).await;
    let carol = f.carol.did();
    let sender = f.sender.did();
    let (claimed, cancelled) = futures_util::join!(
        f.store
            .complete_invitation(&id, carol.as_str(), "completed", f.now),
        f.store.cancel_invitation(&id, sender.as_str())
    );
    assert_ne!(claimed.unwrap(), cancelled.unwrap());
}

#[dialog_common::test]
async fn lost_transport_acknowledgement_retries_the_same_delivery_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct LostAck {
        mail: CapturedEmail,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl InvitationEmailSender for LostAck {
        async fn send_invitation(
            &self,
            email: &str,
            link: &str,
            id: &str,
        ) -> Result<(), InvitationError> {
            self.mail.send_invitation(email, link, id).await?;
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(InvitationError::Unavailable);
            }
            Ok(())
        }
    }
    let f = Fixture::new().await;
    let id = "d3".repeat(32);
    let prepared = f.prepare(&id, "carol@example.com").await.unwrap();
    let original = f
        .original(
            &id,
            &prepared["recipientRoot"].as_str().unwrap().parse().unwrap(),
        )
        .await;
    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(f.sender.clone()))
        .audience(&f.service.did())
        .subject(&f.sender.did())
        .command(vec!["email-invitation".into(), "send".into()])
        .expiration(f.expiration())
        .proofs(vec![])
        .arguments(std::collections::BTreeMap::from([
            ("invitation".into(), Promised::String(id.clone())),
            ("original".into(), Promised::String(original.clone())),
        ]))
        .try_build()
        .await
        .unwrap();
    let body = InvocationChain::new(invocation, Default::default())
        .to_bytes()
        .unwrap();
    let mail = LostAck {
        mail: CapturedEmail::default(),
        calls: AtomicUsize::new(0),
    };
    let mut service = InvitationService {
        store: &f.store,
        email: &mail,
        service: &f.service,
        service_seed: &f.seed,
        origin: "https://tonk.example",
        now: f.now,
        revocations: &f.revocations,
    };
    assert!(matches!(
        service.handle(&body).await,
        Err(InvitationError::Unavailable)
    ));
    let reserved = f.prepare(&id, "carol@example.com").await.unwrap();
    assert_eq!(
        reserved["original"], original,
        "restart can retrieve the same original"
    );
    assert!(matches!(
        service.handle(&body).await,
        Err(InvitationError::Limited)
    ));
    service.now += 61;
    assert_eq!(service.handle(&body).await.unwrap()["state"], "sent");
    assert_eq!(service.handle(&body).await.unwrap()["state"], "sent");
    assert_eq!(mail.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        mail.mail.0.lock().unwrap().len(),
        1,
        "transport idempotency suppresses the duplicate"
    );
}

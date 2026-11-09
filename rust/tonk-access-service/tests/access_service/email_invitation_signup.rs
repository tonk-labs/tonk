//! Test-only pending-invitation protocol spike. No production delivery endpoint.
//! COLLAB-02: unknown email -> signup -> account-bound authority, inviter offline.
#![cfg(all(feature = "integration-tests", not(target_arch = "wasm32")))]

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Signer, Signer};
use dialog_remote_s3::Permit;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::revocation::UnverifiedRevocations;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::verification::{Environment, VerificationContext};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tonk_access_service::email::normalize_email;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_invite::{Invite, InviteAudience};

const EMAIL_BINDING: &str = "spike.delivery.email";

fn now() -> u64 {
    Timestamp::now().to_unix()
}

/// Prototype only: production needs sealed key custody and an atomic store.
#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    email: String,
    deadline: u64,
    cancelled: bool,
    original: Vec<u8>,
    seed: [u8; 32],
    completed: Option<Delivery>,
}

/// No seed travels to the recipient. The original carries Bob's signature;
/// completion carries the delivery key's grant specifically to Carol.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Delivery {
    original: Vec<u8>,
    completed: Vec<u8>,
    recipient: Did,
}

impl Delivery {
    async fn validate(&self, expected: &Did, locator: &str) -> Result<Did> {
        ensure!(&self.recipient == expected, "wrong recipient");
        let original = DelegationChain::try_from(self.original.as_slice())?;
        ensure!(
            original.proof_cids().last().unwrap().to_string() == locator,
            "wrong invitation locator"
        );
        let completed = DelegationChain::try_from(self.completed.as_slice())?;
        ensure!(
            completed.audience() == expected,
            "wrong completion audience"
        );
        let prefix: Vec<_> = original.proofs().collect();
        let proofs: Vec<_> = completed.proofs().collect();
        ensure!(
            proofs.len() == prefix.len() + 1,
            "not a single completion hop"
        );
        for (a, b) in prefix.iter().zip(&proofs) {
            ensure!(
                a.to_cid() == b.to_cid(),
                "original invitation was substituted"
            );
        }
        for proof in &proofs {
            proof.verify_signature(&DidKeyResolver).await?;
        }
        let leaf = proofs.last().unwrap();
        ensure!(
            leaf.issuer() == original.audience(),
            "wrong delivery signer"
        );
        ensure!(completed.subject() == original.subject(), "changed space");
        // Signature/provenance validation here complements, and does not
        // replace, UCAN scope, expiration and revocation admission at storage.
        tonk_invite::inviter(&original)?.context("original has no bound sender")
    }
}

impl Pending {
    fn locator(&self) -> Result<String> {
        let original = DelegationChain::try_from(self.original.as_slice())?;
        Ok(original.proof_cids().last().unwrap().to_string())
    }

    async fn complete(
        &mut self,
        env: &AccessServiceAddress,
        request: &[u8],
        at: u64,
    ) -> Result<Delivery> {
        ensure!(!self.cancelled, "invitation cancelled");
        ensure!(at < self.deadline, "invitation expired");
        let claim = InvocationChain::try_from(request)?;
        claim
            .verify(&VerificationContext::new(&Environment::new(
                claim.proof_store(),
                DidKeyResolver,
                // This spike does not implement the delivery service's
                // revocation index. Production must enforce account revocation.
                UnverifiedRevocations,
            )))
            .await?;
        ensure!(
            claim.invocation.audience().as_str() == env.service_did,
            "wrong service"
        );
        ensure!(
            claim.command().0 == ["invitation", "claim"],
            "wrong command"
        );
        let expiry = claim
            .invocation
            .expiration()
            .context("claim has no expiry")?;
        ensure!(
            expiry.to_unix() > at && expiry.to_unix() <= at + 300,
            "claim expiry"
        );
        ensure!(
            claim.invocation.arguments()
                == &BTreeMap::from([("invitation".into(), Promised::String(self.locator()?))]),
            "wrong invitation"
        );
        let recipient = claim.subject();
        if let Some(delivery) = &self.completed {
            ensure!(
                &delivery.recipient == recipient,
                "already bound to another account"
            );
            return Ok(delivery.clone());
        }
        let original = DelegationChain::try_from(self.original.as_slice())?;
        let bound_email = original
            .proofs()
            .last()
            .unwrap()
            .meta()
            .get(EMAIL_BINDING)
            .cloned();
        ensure!(
            bound_email == Some(ipld_core::ipld::Ipld::String(self.email.clone())),
            "email binding changed"
        );
        let (status, document) = lookup(env, &self.email).await?;
        ensure!(status == 200, "email is not confirmed ({status})");
        ensure!(document["status"] == "Active", "account is not active");
        let found = document["alsoKnownAs"][0]
            .as_str()
            .context("no account DID")?;
        ensure!(
            found == recipient.as_str(),
            "confirmed email belongs to another account"
        );
        ensure!(
            format!(
                "did:key:{}",
                document["verificationMethod"][0]["publicKeyMultibase"]
                    .as_str()
                    .context("no key")?
            ) == found,
            "lookup key mismatch"
        );
        let origin = url::Url::parse(&env.access_service_url)?;
        ensure!(
            document["id"]
                == tonk_access_service::lookup::customer_did(origin.authority(), &self.email)
                    .context("invalid bound email")?,
            "lookup document mismatch"
        );
        let invite = Invite::new(original, InviteAudience::Open { seed: self.seed }, None).await?;
        // The bearer exists only inside the delivery service. Its claim result
        // is account-bound; the email and recipient never receive its seed.
        let completed = invite.claim(recipient).await?.chain.to_bytes()?;
        let delivery = Delivery {
            original: self.original.clone(),
            completed,
            recipient: recipient.clone(),
        };
        delivery.validate(recipient, &self.locator()?).await?;
        self.completed = Some(delivery.clone());
        Ok(delivery)
    }
}

async fn lookup(env: &AccessServiceAddress, email: &str) -> Result<(u16, serde_json::Value)> {
    let normalized = normalize_email(email);
    let (local, domain) = normalized.rsplit_once('@').context("invalid email")?;
    let mut url = url::Url::parse(&env.access_service_url)?;
    url.path_segments_mut()
        .unwrap()
        .extend(["customer", domain, local, "did.json"]);
    let response = reqwest::get(url).await?;
    Ok((response.status().as_u16(), response.json().await?))
}

/// All Alice/Bob/device signers go out of scope when this returns. Carol has
/// not been created yet. Retain only the one-space delivery capability.
async fn leave_invitation(email: &str) -> Result<(Pending, Did, Did, Did)> {
    let space = Ed25519Signer::generate().await?;
    let alice = Ed25519Signer::generate().await?;
    let bob = Ed25519Signer::generate().await?;
    let device = Ed25519Signer::generate().await?;
    // Fresh key per invitation; production must seal it before retention.
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("delivery key: {error}"))?;
    let delivery = Ed25519Signer::import(&seed).await?;
    let expiry = Timestamp::new(SystemTime::now() + Duration::from_secs(3600))?;
    let mut chain: Option<DelegationChain> = None;
    let mut meta = tonk_invite::inviter_meta(&bob.did());
    meta.insert(
        EMAIL_BINDING.into(),
        ipld_core::ipld::Ipld::String(normalize_email(email)),
    );
    for (issuer, audience, identity, metadata) in [
        (&space, alice.did(), false, BTreeMap::new()),
        (&alice, bob.did(), false, BTreeMap::new()),
        (&bob, device.did(), true, BTreeMap::new()),
        (&device, delivery.did(), false, meta),
    ] {
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(issuer.clone()))
            .audience(&audience)
            .subject(if identity {
                Subject::Any
            } else {
                Subject::Specific(space.did())
            })
            .command(if identity { vec![] } else { vec!["use".into()] })
            .expiration(expiry)
            .meta(metadata)
            .try_build()
            .await?;
        chain = Some(match chain {
            None => DelegationChain::new(grant),
            Some(chain) => chain.push(grant)?,
        });
    }
    Ok((
        Pending {
            email: normalize_email(email),
            deadline: expiry.to_unix(),
            cancelled: false,
            original: chain.unwrap().to_bytes()?,
            seed,
            completed: None,
        },
        space.did(),
        alice.did(),
        bob.did(),
    ))
}

async fn claim_request(
    env: &AccessServiceAddress,
    pending: &Pending,
    signer: &Ed25519Signer,
    root: &Did,
    identity: Option<&DelegationChain>,
) -> Result<Vec<u8>> {
    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(signer.clone()))
        .audience(&env.service_did.parse::<Did>()?)
        .subject(root)
        .command(vec!["invitation".into(), "claim".into()])
        .expiration(Timestamp::five_minutes_from_now())
        .arguments(BTreeMap::from([(
            "invitation".into(),
            Promised::String(pending.locator()?),
        )]))
        .proofs(
            identity
                .map(|chain| chain.proof_cids().to_vec())
                .unwrap_or_default(),
        )
        .try_build()
        .await?;
    let proofs = identity
        .map(|chain| chain.export().collect::<HashMap<_, _>>())
        .unwrap_or_default();
    Ok(InvocationChain::new(invocation, proofs).to_bytes()?)
}

async fn read_request(
    env: &AccessServiceAddress,
    chain: &DelegationChain,
    holder: &Ed25519Signer,
) -> Result<reqwest::Response> {
    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(holder.clone()))
        .audience(&env.service_did.parse::<Did>()?)
        .subject(chain.subject().unwrap())
        .command(vec![
            "use".into(),
            "get".into(),
            "memory".into(),
            "cell".into(),
        ])
        .arguments(BTreeMap::from([
            ("space".into(), Promised::String("branch/main".into())),
            ("cell".into(), Promised::String("revision".into())),
        ]))
        .proofs(chain.proof_cids().to_vec())
        .try_build()
        .await?;
    let bytes =
        InvocationChain::new(invocation, chain.export().collect::<HashMap<_, _>>()).to_bytes()?;
    Ok(reqwest::Client::new()
        .post(env.ucan_endpoint())
        .header("Content-Type", "application/cbor")
        .body(bytes)
        .send()
        .await?)
}

fn refused(result: Result<Delivery>, reason: &str) {
    let error = result.err().expect("claim must be refused");
    assert!(
        error.to_string().contains(reason),
        "expected {reason}: {error}"
    );
}

#[dialog_common::test]
async fn signup_completes_an_offline_invitation_without_losing_bob(
    env: AccessServiceAddress,
) -> Result<()> {
    let email = "carol+signup@example.com";
    let (pending, space, alice, bob) = leave_invitation(email).await?;
    env.provision_subject(space.as_str()).await?;
    assert_eq!(lookup(&env, email).await?.0, 404);
    // The proposed invitation email contains only this locator. It carries
    // neither a space grant nor a seed; knowing it is not claim authority.
    let locator = pending.locator()?;
    let message = format!("https://tonk.test/invitations/{locator}");
    assert!(!message.contains('#'));
    assert!(!message.contains("access="));
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("pending.json");
    std::fs::write(&path, serde_json::to_vec(&pending)?)?;
    drop(pending);
    // Simulate service restart; no inviter keys or sessions survive.
    let mut pending: Pending = serde_json::from_slice(&std::fs::read(&path)?)?;
    let carol = Ed25519Signer::generate().await?;
    let mallory = Ed25519Signer::generate().await?;
    env.activate_customer(&mallory, "mallory@example.com")
        .await?;
    let wrong = claim_request(&env, &pending, &mallory, &mallory.did(), None).await?;
    refused(
        pending.complete(&env, &wrong, now()).await,
        "email is not confirmed (404)",
    );
    assert!(pending.completed.is_none());
    env.enroll_customer(&carol, email).await?;
    assert_eq!(lookup(&env, email).await?.0, 202);
    let claim = claim_request(&env, &pending, &carol, &carol.did(), None).await?;
    refused(
        pending.complete(&env, &claim, now()).await,
        "email is not confirmed (202)",
    );
    env.confirm_email(email).await?;
    assert_eq!(lookup(&env, email).await?.0, 200);
    refused(
        pending.complete(&env, &wrong, now()).await,
        "confirmed email belongs to another account",
    );
    let forged = claim_request(&env, &pending, &mallory, &carol.did(), None).await?;
    assert!(pending.complete(&env, &forged, now()).await.is_err());
    let mut changed = pending.clone();
    changed.email = "mallory@example.com".into();
    refused(
        changed.complete(&env, &wrong, now()).await,
        "email binding changed",
    );
    let mut cancelled = pending.clone();
    cancelled.cancelled = true;
    refused(
        cancelled.complete(&env, &claim, now()).await,
        "invitation cancelled",
    );
    refused(
        pending.complete(&env, &claim, pending.deadline).await,
        "invitation expired",
    );
    assert!(pending.completed.is_none());

    let device = Ed25519Signer::generate().await?;
    let identity = DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(carol.clone()))
            .audience(&device.did())
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await?,
    );
    let device_claim =
        claim_request(&env, &pending, &device, &carol.did(), Some(&identity)).await?;
    let delivery = pending.complete(&env, &device_claim, now()).await?;
    assert_eq!(delivery.validate(&carol.did(), &locator).await?, bob);
    assert_ne!(bob, alice);
    std::fs::write(&path, serde_json::to_vec(&pending)?)?;
    let mut restored: Pending = serde_json::from_slice(&std::fs::read(&path)?)?;
    refused(
        restored.complete(&env, &wrong, now()).await,
        "already bound to another account",
    );
    assert!(restored.complete(&env, &device_claim, now()).await? == delivery);
    assert!(delivery.validate(&mallory.did(), &locator).await.is_err());
    let mut substituted = delivery.clone();
    substituted.original = leave_invitation(email).await?.0.original;
    assert!(substituted.validate(&carol.did(), &locator).await.is_err());

    let (mut other, _, _, _) = leave_invitation(email).await?;
    let other_request = claim_request(&env, &other, &carol, &carol.did(), None).await?;
    refused(
        restored.complete(&env, &other_request, now()).await,
        "wrong invitation",
    );
    let other_delivery = other.complete(&env, &other_request, now()).await?;
    assert!(
        other_delivery
            .validate(&carol.did(), &locator)
            .await
            .is_err()
    );
    let mut wrong_completion = delivery.clone();
    wrong_completion.completed = other_delivery.completed;
    assert!(
        wrong_completion
            .validate(&carol.did(), &locator)
            .await
            .is_err()
    );

    let completed = DelegationChain::try_from(delivery.completed.as_slice())?;
    let original = DelegationChain::try_from(delivery.original.as_slice())?;
    // Do not silently import a completion as a new invitation: the existing
    // leaf-only contract correctly finds no Bob assertion on the service hop.
    assert_eq!(tonk_invite::inviter(&completed)?, None);
    assert_eq!(tonk_invite::inviter(&original)?, Some(bob));
    assert_ne!(completed.proof_cids().last(), original.proof_cids().last());
    let scoped = Invite::new(completed.clone(), InviteAudience::Scoped, None).await?;
    let scoped_url = scoped.to_url(tonk_invite::DEFAULT_BASE_URL)?;
    assert!(url::Url::parse(&scoped_url)?.fragment().is_none());
    assert!(scoped.clone().claim(&mallory.did()).await.is_err());
    scoped.claim(&carol.did()).await?;

    // Real access service verifies signatures, authority and provisioning.
    // A memory read of an absent cell still returns an authorized object permit.
    let response = read_request(&env, &completed, &carol).await?;
    let status = response.status();
    let body = response.bytes().await?;
    ensure!(
        status.is_success(),
        "Carol storage admission failed: {status}: {}",
        String::from_utf8_lossy(&body)
    );
    let permit: Permit = serde_ipld_dagcbor::from_slice(&body)?;
    assert_eq!(permit.method, "GET");
    let mut request = reqwest::Client::new().get(permit.url);
    for (key, value) in &permit.headers {
        request = request.header(key, value);
    }
    let response = request.send().await?;
    assert_eq!(
        response.status().as_u16(),
        404,
        "authorized read reaches the absent cell"
    );
    assert_eq!(
        read_request(&env, &completed, &mallory)
            .await?
            .status()
            .as_u16(),
        401
    );
    Ok(())
}

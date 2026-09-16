//! Consumer check for the unreleased authorizer API. Registered only in the
//! isolated snapshot prepared by scripts/test-connection-authorizer.sh.
//!
//! Management and audience binding are trusted fixture setup here. This proves
//! policy enforcement and signed S3 transport, not authenticated issuance,
//! redemption, SQLite/D1 consistency, or integration into Tonk's /ucan/ handler.

use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use dialog_capability::access::AuthorizeError;
use dialog_credentials::Ed25519Signer;
use dialog_remote_s3::{Address, Permit, S3Credential, S3Error, helpers::LocalS3};
use dialog_remote_ucan_s3::{UcanAuthorizer, VerifiedInvocation};
use dialog_ucan_core::{
    Delegation, DelegationBuilder, InvocationBuilder, InvocationChain,
    promise::Promised,
    subject::Subject,
    time::{Duration, SystemTime, Timestamp},
};
use dialog_varsig::{Did, Principal, eddsa::Ed25519Signature};
use ipld_core::{cid::Cid, ipld::Ipld};

const MARKER: &str = "tonk/connection/spike";
const CONTENT: &[u8] = b"connection policy transport roundtrip";

struct Policy {
    service: Did,
    subject: Did,
    session: Did,
    grants: Vec<Cid>,
    revoked: AtomicBool,
}

impl Policy {
    fn check(&self, view: VerifiedInvocation) -> Result<Option<Timestamp>, AuthorizeError> {
        let deny = || AuthorizeError::Revoked {
            subject: view.subject().clone(),
        };
        if self.revoked.load(Ordering::SeqCst)
            || view.audience() != &self.service
            || view.subject() != &self.subject
        {
            return Err(deny());
        }
        let Some((position, grant)) = view
            .proofs()
            .iter()
            .enumerate()
            .find(|(_, cid)| self.grants.contains(cid))
            .and_then(|(position, cid)| view.delegation(cid).map(|grant| (position, grant)))
        else {
            return Err(deny());
        };
        if grant.meta().get(MARKER) != Some(&Ipld::String("fixture-connection".into())) {
            return Err(deny());
        }
        // A bootstrap may add operator hops, but it cannot sign as the bound
        // session. Verification already proved adjacency/signatures of this path.
        if !view.proofs()[position + 1..].iter().any(|cid| {
            view.delegation(cid)
                .is_some_and(|hop| hop.audience() == &self.session)
        }) {
            return Err(deny());
        }
        let command = view
            .command()
            .0
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !matches!(
            command.as_slice(),
            ["use", "get" | "put", "archive", "block"]
        ) || view.arguments().get("catalog") != Some(&Promised::String("index".into()))
        {
            return Err(deny());
        }
        Ok(None)
    }
}

async fn grant(
    owner: &Ed25519Signer,
    bootstrap: &Ed25519Signer,
    operation: &str,
) -> Delegation<Ed25519Signature> {
    DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&bootstrap.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec![
            "use".into(),
            operation.into(),
            "archive".into(),
            "block".into(),
        ])
        .expiration(Timestamp::new(SystemTime::now() + Duration::from_secs(180)).unwrap())
        .meta(BTreeMap::from([(
            MARKER.into(),
            Ipld::String("fixture-connection".into()),
        )]))
        .try_build()
        .await
        .unwrap()
}

async fn invocation(
    grant: &Delegation<Ed25519Signature>,
    bootstrap: &Ed25519Signer,
    session: Option<&Ed25519Signer>,
    operator: Option<&Ed25519Signer>,
    service: &Did,
) -> Vec<u8> {
    let mut proofs = vec![grant.to_cid()];
    let mut delegations = HashMap::from([(grant.to_cid(), Arc::new(grant.clone()))]);
    let mut issuer = bootstrap;
    for recipient in session.into_iter().chain(operator) {
        let child = DelegationBuilder::new()
            .issuer(issuer.clone())
            .audience(&recipient.did())
            .subject(grant.subject().clone())
            .command(grant.command().0.clone())
            // A leaf can shadow convenience metadata. The registered ancestor
            // must remain the policy's source of truth.
            .meta(BTreeMap::from([(
                MARKER.into(),
                Ipld::String("shadow".into()),
            )]))
            .try_build()
            .await
            .unwrap();
        proofs.push(child.to_cid());
        delegations.insert(child.to_cid(), Arc::new(child));
        issuer = recipient;
    }
    let checksum = dialog_common::Hasher::Sha256.checksum(CONTENT);
    let checksum: Promised =
        serde_ipld_dagcbor::from_slice(&serde_ipld_dagcbor::to_vec(&checksum).unwrap()).unwrap();
    let subject = match grant.subject() {
        Subject::Specific(subject) => subject,
        _ => panic!("fixture uses an exact subject"),
    };
    let invocation = InvocationBuilder::new()
        .issuer(issuer.clone())
        .audience(service)
        .subject(subject)
        .command(grant.command().0.clone())
        .arguments(BTreeMap::from([
            ("catalog".into(), Promised::String("index".into())),
            (
                "digest".into(),
                Promised::Bytes(blake3::hash(CONTENT).as_bytes().to_vec()),
            ),
            ("checksum".into(), checksum),
        ]))
        .proofs(proofs)
        .try_build()
        .await
        .unwrap();
    InvocationChain::new(invocation, delegations)
        .to_bytes()
        .unwrap()
}

fn ttl(permit: &Permit) -> u64 {
    permit
        .url
        .query_pairs()
        .find(|(key, _)| key == "X-Amz-Expires")
        .unwrap()
        .1
        .parse()
        .unwrap()
}

#[tokio::test]
async fn connection_policy_roundtrip_and_revocation_with_rotated_operator() -> anyhow::Result<()> {
    let s3 = LocalS3::start_with_auth("test", "test", &["connections"]).await?;
    let address = Address::builder(&s3.endpoint)
        .region("us-east-1")
        .bucket("connections")
        .path_style(true)
        .build()?;
    let authorizer = UcanAuthorizer::new(address, Some(S3Credential::new("test", "test")));
    let owner = Ed25519Signer::import(&[81; 32]).await?;
    let bootstrap = Ed25519Signer::import(&[82; 32]).await?;
    let session = Ed25519Signer::import(&[83; 32]).await?;
    let operator = Ed25519Signer::import(&[84; 32]).await?;
    let next_operator = Ed25519Signer::import(&[87; 32]).await?;
    let stranger = Ed25519Signer::import(&[85; 32]).await?;
    let service = Ed25519Signer::import(&[86; 32]).await?;
    let read = grant(&owner, &bootstrap, "get").await;
    let write = grant(&owner, &bootstrap, "put").await;
    let policy = Policy {
        service: service.did(),
        subject: owner.did(),
        session: session.did(),
        grants: vec![read.to_cid(), write.to_cid()],
        revoked: AtomicBool::new(false),
    };

    // Neither raw bootstrap use nor choosing a different recipient satisfies
    // the bound lineage, even though both chains have ordinary space authority.
    for recipient in [None, Some(&stranger)] {
        let bytes = invocation(&read, &bootstrap, recipient, None, &service.did()).await;
        assert!(matches!(
            authorizer
                .authorize_with_policy(&bytes, |view| async { policy.check(view) })
                .await,
            Err(S3Error::Authorization(AuthorizeError::Revoked { .. }))
        ));
    }
    let write_bytes = invocation(&write, &bootstrap, Some(&session), None, &service.did()).await;
    let put = authorizer
        .authorize_with_policy(&write_bytes, |view| async { policy.check(view) })
        .await?;
    assert!((1..=60).contains(&ttl(&put)));
    let client = reqwest::Client::new();
    let mut request = client.put(put.url.clone());
    for (name, value) in &put.headers {
        request = request.header(name, value);
    }
    request.body(CONTENT).send().await?.error_for_status()?;

    let read_bytes = invocation(
        &read,
        &bootstrap,
        Some(&session),
        Some(&operator),
        &service.did(),
    )
    .await;
    let get = authorizer
        .authorize_with_policy(&read_bytes, |view| async { policy.check(view) })
        .await?;
    assert!((1..=60).contains(&ttl(&get)));
    let mut request = client.get(get.url.clone());
    for (name, value) in &get.headers {
        request = request.header(name, value);
    }
    let retained = request.send().await?.error_for_status()?.bytes().await?;
    assert_eq!(retained.as_ref(), CONTENT);

    let rotated_bytes = invocation(
        &read,
        &bootstrap,
        Some(&session),
        Some(&next_operator),
        &service.did(),
    )
    .await;
    let rotated = authorizer
        .authorize_with_policy(&rotated_bytes, |view| async { policy.check(view) })
        .await?;
    assert!((1..=60).contains(&ttl(&rotated)));
    assert_eq!(
        client
            .get(rotated.url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .as_ref(),
        CONTENT
    );

    // Trusted fixture management only: a production revoke endpoint must prove
    // management authority and commit durable control state before reporting it.
    policy.revoked.store(true, Ordering::SeqCst);
    for bytes in [&read_bytes, &rotated_bytes, &write_bytes] {
        assert!(matches!(
            authorizer
                .authorize_with_policy(bytes, |view| async { policy.check(view) })
                .await,
            Err(S3Error::Authorization(AuthorizeError::Revoked { .. }))
        ));
    }
    assert_eq!(
        retained.as_ref(),
        CONTENT,
        "revocation preserves local copies"
    );
    // Already signed URLs remain usable during their bounded lifetime.
    assert_eq!(
        client
            .get(get.url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .as_ref(),
        CONTENT
    );
    Ok(())
}

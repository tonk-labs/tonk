//! Ordinary connection grants at the native HTTP and authenticated S3 boundary.
//! No connection marker, confirmation, or management row participates in access.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_remote_s3::Permit;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};
use std::collections::{BTreeMap, HashMap};
use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};

const CONTENT: &[u8] = b"ordinary connection storage roundtrip";

struct Fixture {
    _server: AccessServer,
    env: AccessServiceAddress,
    client: reqwest::Client,
}
impl Fixture {
    async fn new() -> anyhow::Result<Self> {
        let s3 =
            dialog_remote_s3::helpers::LocalS3::start_with_auth("test", "test", &["connections"])
                .await?;
        let server =
            AccessServer::start(s3, "connections", "test", "test", None, None, None).await?;
        let env = AccessServiceAddress {
            access_service_url: server.endpoint.clone(),
            s3_endpoint: server.s3_server.endpoint.clone(),
            bucket: "connections".into(),
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
            service_did: server.service_did.clone(),
        };
        Ok(Self {
            _server: server,
            env,
            client: reqwest::Client::new(),
        })
    }
    async fn post(&self, bytes: Vec<u8>) -> anyhow::Result<reqwest::Response> {
        Ok(self
            .client
            .post(self.env.ucan_endpoint())
            .header("Content-Type", "application/cbor")
            .body(bytes)
            .send()
            .await?)
    }
    async fn permit(&self, bytes: Vec<u8>) -> anyhow::Result<Permit> {
        let response = self.post(bytes).await?;
        let status = response.status();
        let body = response.bytes().await?;
        anyhow::ensure!(
            status.is_success(),
            "{status}: {}",
            String::from_utf8_lossy(&body)
        );
        let permit: Permit = serde_ipld_dagcbor::from_slice(&body)?;
        let ttl: u64 = permit
            .url
            .query_pairs()
            .find(|(key, _)| key == "X-Amz-Expires")
            .expect("signed transport expiry")
            .1
            .parse()?;
        assert!((1..=60).contains(&ttl));
        Ok(permit)
    }
    async fn transfer(&self, permit: &Permit) -> anyhow::Result<Vec<u8>> {
        let mut request = self
            .client
            .request(permit.method.parse()?, permit.url.clone());
        for (name, value) in &permit.headers {
            request = request.header(name, value);
        }
        if permit.method == "PUT" {
            request = request.body(CONTENT);
        }
        Ok(request
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec())
    }
}

fn command(operation: &str, resource: &str) -> Vec<String> {
    format!("use/{operation}/{resource}")
        .split('/')
        .map(str::to_owned)
        .collect()
}
fn arguments(resource: &str) -> BTreeMap<String, Promised> {
    let checksum = dialog_common::Hasher::Sha256.checksum(CONTENT);
    let checksum =
        serde_ipld_dagcbor::from_slice(&serde_ipld_dagcbor::to_vec(&checksum).unwrap()).unwrap();
    let mut args = BTreeMap::from([("checksum".into(), checksum)]);
    match resource {
        "memory/cell" => {
            args.insert("space".into(), Promised::String("branch/main".into()));
            args.insert("cell".into(), Promised::String("revision".into()));
            args.insert("when".into(), Promised::Null);
        }
        "archive/block" | "archive/blob" => {
            args.insert(
                "digest".into(),
                Promised::Bytes(blake3::hash(CONTENT).as_bytes().to_vec()),
            );
            if resource == "archive/block" {
                args.insert("catalog".into(), Promised::String("index".into()));
            } else {
                args.insert("size".into(), Promised::Integer(CONTENT.len() as i128));
                args.insert("chunks".into(), Promised::List(vec![]));
                args.insert("range".into(), Promised::Null);
            }
        }
        _ => panic!("unknown resource"),
    }
    args
}
async fn grant(
    owner: &Ed25519Signer,
    recipient: &Ed25519Signer,
    operation: &str,
    resource: &str,
    expiration: Timestamp,
) -> DelegationChain {
    let scope = tonk_invite::connection::candidate_build_scopes(&owner.did())
        .into_iter()
        .find(|scope| scope.command.0 == command(operation, resource))
        .unwrap();
    // Model a browser's durable upstream delegation before the issued grant.
    let browser = Ed25519Signer::import(&[73; 32]).await.unwrap();
    let upstream = DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(&browser.did())
            .subject(Subject::Specific(owner.did()))
            .command(command(operation, resource))
            .policy(scope.policy())
            .expiration(expiration)
            .try_build()
            .await
            .unwrap(),
    );
    upstream
        .push(
            DelegationBuilder::new()
                .issuer(Signer::from(browser))
                .audience(&recipient.did())
                .subject(Subject::Specific(owner.did()))
                .command(command(operation, resource))
                .policy(scope.policy())
                .expiration(expiration)
                .try_build()
                .await
                .unwrap(),
        )
        .unwrap()
}
async fn request(
    chain: &DelegationChain,
    holder: &Ed25519Signer,
    service: &str,
    subject: &Did,
    cmd: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(holder.clone()))
        .audience(&service.parse::<Did>().unwrap())
        .subject(subject)
        .command(cmd)
        .arguments(args)
        .proofs(chain.proof_cids().to_vec())
        .try_build()
        .await
        .unwrap();
    InvocationChain::new(invocation, chain.export().collect::<HashMap<_, _>>())
        .to_bytes()
        .unwrap()
}
fn expiry() -> Timestamp {
    Timestamp::new(SystemTime::now() + Duration::from_secs(90 * 86400)).unwrap()
}

#[tokio::test]
async fn connection_six_leaf_grants_transfer_storage_and_reject_escalation() -> anyhow::Result<()> {
    let fixture = Fixture::new().await?;
    let owner = Ed25519Signer::generate().await?;
    fixture.env.provision_subject(owner.did().as_str()).await?;
    let holder = Ed25519Signer::generate().await?;
    for resource in ["memory/cell", "archive/block", "archive/blob"] {
        for operation in ["put", "get"] {
            let chain = grant(&owner, &holder, operation, resource, expiry()).await;
            let bytes = request(
                &chain,
                &holder,
                &fixture.env.service_did,
                &owner.did(),
                command(operation, resource),
                arguments(resource),
            )
            .await;
            let permit = fixture.permit(bytes).await?;
            let result = fixture.transfer(&permit).await?;
            if operation == "get" {
                assert_eq!(result, CONTENT);
            }
            let mut invalid = arguments(resource);
            if resource == "memory/cell" {
                invalid.insert("space".into(), Promised::String("branch/meta".into()));
            } else if resource == "archive/block" {
                invalid.insert("catalog".into(), Promised::String("other".into()));
            } else {
                continue;
            }
            let denied = fixture
                .post(
                    request(
                        &chain,
                        &holder,
                        &fixture.env.service_did,
                        &owner.did(),
                        command(operation, resource),
                        invalid,
                    )
                    .await,
                )
                .await?;
            assert!(
                !denied.status().is_success(),
                "signed argument constraints must be enforced"
            );
        }
    }
    let chain = grant(&owner, &holder, "get", "archive/block", expiry()).await;
    for cmd in [
        command("put", "archive/block"),
        vec!["account".into(), "delete".into()],
        command("get", "memory/cell"),
    ] {
        assert!(
            !fixture
                .post(
                    request(
                        &chain,
                        &holder,
                        &fixture.env.service_did,
                        &owner.did(),
                        cmd,
                        arguments("archive/block")
                    )
                    .await
                )
                .await?
                .status()
                .is_success()
        );
    }
    Ok(())
}

async fn operator_chain(
    chain: &DelegationChain,
    holder: &Ed25519Signer,
    operator: &Ed25519Signer,
) -> DelegationChain {
    let leaf = chain.proofs().last().unwrap();
    let child = DelegationBuilder::new()
        .issuer(Signer::from(holder.clone()))
        .audience(&operator.did())
        .subject(leaf.subject().clone())
        .command(leaf.command().0.clone())
        .policy(leaf.policy().to_vec())
        .expiration(Timestamp::new(SystemTime::now() + Duration::from_secs(3600)).unwrap())
        .try_build()
        .await
        .unwrap();
    chain.push(child).unwrap()
}

#[tokio::test]
async fn connection_standard_revocation_denies_bearer_copies_and_rotated_descendants_only()
-> anyhow::Result<()> {
    let fixture = Fixture::new().await?;
    let owner = Ed25519Signer::generate().await?;
    fixture.env.provision_subject(owner.did().as_str()).await?;
    // Two independent imports of the invitation seed represent the same authority.
    let first = Ed25519Signer::import(&[72; 32]).await?;
    let second = Ed25519Signer::import(&[72; 32]).await?;
    let cli = Ed25519Signer::generate().await?;
    let another_invite = Ed25519Signer::generate().await?;
    let op1 = Ed25519Signer::generate().await?;
    let op2 = Ed25519Signer::generate().await?;
    let write = grant(&owner, &first, "put", "archive/block", expiry()).await;
    let read = grant(&owner, &first, "get", "archive/block", expiry()).await;
    let sibling = grant(&owner, &another_invite, "get", "archive/block", expiry()).await;
    let linked = grant(&owner, &cli, "get", "archive/block", expiry()).await;
    let derived1 = operator_chain(&read, &first, &op1).await;
    let derived2 = operator_chain(&read, &second, &op2).await;
    let make = |chain: DelegationChain, holder: Ed25519Signer| {
        let service = fixture.env.service_did.clone();
        let subject = owner.did();
        async move {
            let cmd = chain.proofs().last().unwrap().command().0.clone();
            request(
                &chain,
                &holder,
                &service,
                &subject,
                cmd,
                arguments("archive/block"),
            )
            .await
        }
    };
    fixture
        .transfer(
            &fixture
                .permit(make(write.clone(), first.clone()).await)
                .await?,
        )
        .await?;
    let holders = [
        (&read, &first),
        (&read, &second),
        (&derived1, &op1),
        (&derived2, &op2),
    ];
    let mut issued = Vec::new();
    for (chain, holder) in holders {
        let permit = fixture
            .permit(make(chain.clone(), holder.clone()).await)
            .await?;
        assert_eq!(fixture.transfer(&permit).await?, CONTENT);
        issued.push(permit);
    }
    for (chain, holder) in [(&sibling, &another_invite), (&linked, &cli)] {
        assert_eq!(
            fixture
                .transfer(
                    &fixture
                        .permit(make(chain.clone(), holder.clone()).await)
                        .await?
                )
                .await?,
            CONTENT
        );
    }
    // An unrelated key cannot withdraw someone else's grant.
    let unauthorized = tonk_identity::revocation::mint_root_revocation(
        cli.clone(),
        &read,
        read.proof_cids().last().unwrap(),
    )
    .await?;
    assert!(!fixture.post(unauthorized).await?.status().is_success());
    // The ordinary authenticated endpoint records each CID in the managed set.
    for chain in [&read, &write] {
        let revocation = tonk_identity::revocation::mint_root_revocation(
            owner.clone(),
            chain,
            chain.proof_cids().last().unwrap(),
        )
        .await?;
        fixture.post(revocation).await?.error_for_status()?;
    }
    // The HTTP acknowledgement is our barrier; no fixture state or sleep.
    for (chain, holder) in holders
        .into_iter()
        .chain([(&write, &first), (&write, &second)])
    {
        let response = fixture
            .post(make(chain.clone(), holder.clone()).await)
            .await?;
        assert_eq!(response.status().as_u16(), 403);
        let reason: dialog_capability::access::AuthorizeError =
            serde_json::from_slice(&response.bytes().await?)?;
        assert!(matches!(
            reason,
            dialog_capability::access::AuthorizeError::Revoked { .. }
        ));
    }
    for (chain, holder) in [(&sibling, &another_invite), (&linked, &cli)] {
        assert_eq!(
            fixture
                .transfer(
                    &fixture
                        .permit(make(chain.clone(), holder.clone()).await)
                        .await?
                )
                .await?,
            CONTENT
        );
    }
    // Acknowledged revocation stops new permits, not an already issued URL.
    assert_eq!(fixture.transfer(&issued[0]).await?, CONTENT);
    Ok(())
}

#[tokio::test]
async fn connection_rejects_wrong_subject_key_tampered_proof_and_expired_ancestor()
-> anyhow::Result<()> {
    let fixture = Fixture::new().await?;
    let owner = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    let holder = Ed25519Signer::generate().await?;
    fixture.env.provision_subject(owner.did().as_str()).await?;
    fixture
        .env
        .activate_customer(&stranger, "connection-stranger@example.test")
        .await?;
    let read = grant(&owner, &holder, "get", "archive/block", expiry()).await;
    for (subject, key) in [(stranger.did(), &holder), (owner.did(), &stranger)] {
        assert!(
            !fixture
                .post(
                    request(
                        &read,
                        key,
                        &fixture.env.service_did,
                        &subject,
                        command("get", "archive/block"),
                        arguments("archive/block")
                    )
                    .await
                )
                .await?
                .status()
                .is_success()
        );
    }
    let mut tampered = request(
        &read,
        &holder,
        &fixture.env.service_did,
        &owner.did(),
        command("get", "archive/block"),
        arguments("archive/block"),
    )
    .await;
    let end = tampered.len() - 1;
    tampered[end] ^= 1;
    assert!(!fixture.post(tampered).await?.status().is_success());
    let expired = grant(
        &owner,
        &holder,
        "get",
        "archive/block",
        Timestamp::new(SystemTime::now() - Duration::from_secs(3600)).unwrap(),
    )
    .await;
    let operator = Ed25519Signer::generate().await?;
    let derived = operator_chain(&expired, &holder, &operator).await;
    let response = fixture
        .post(
            request(
                &derived,
                &operator,
                &fixture.env.service_did,
                &owner.did(),
                command("get", "archive/block"),
                arguments("archive/block"),
            )
            .await,
        )
        .await?;
    assert_eq!(response.status().as_u16(), 401);
    Ok(())
}

/// Opt-in because it executes the freshly built production Wasm worker in workerd.
/// `scripts/test-connection-worker.sh` supplies its shim and local Miniflare module.
#[tokio::test]
#[ignore = "requires worker-build and local Miniflare; run scripts/test-connection-worker.sh"]
async fn connection_worker_standard_revocation_survives_persisted_kv_restart() -> anyhow::Result<()>
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let shim = std::env::var("TONK_CONNECTION_WORKER_SHIM")?;
    // Exercise both direct issuer withdrawal and a different browser acting
    // through the same account. Each gets an independent persisted worker.
    for delegated in [false, true] {
        let space = Ed25519Signer::generate().await?;
        let holder = Ed25519Signer::import(&[74; 32]).await?;
        let second = Ed25519Signer::import(&[74; 32]).await?;
        let operator = Ed25519Signer::generate().await?;
        let sibling_key = Ed25519Signer::generate().await?;
        let service = Ed25519Signer::import(&[93; 32]).await?;
        let mut read = grant(&space, &holder, "get", "archive/block", expiry()).await;
        let account = Ed25519Signer::generate().await?;
        let other_browser = Ed25519Signer::generate().await?;
        let browser = Ed25519Signer::import(&[73; 32]).await?;
        let other_device = tonk_identity::delegation::mint_device_delegation(
            account.clone(),
            &other_browser.did(),
        )
        .await?;
        if delegated {
            let mut prefix = DelegationChain::new(
                DelegationBuilder::new()
                    .issuer(Signer::from(space.clone()))
                    .audience(&account.did())
                    .subject(Subject::Specific(space.did()))
                    .command(vec!["use".into()])
                    .expiration(expiry())
                    .try_build()
                    .await?,
            );
            let device =
                tonk_identity::delegation::mint_device_delegation(account.clone(), &browser.did())
                    .await?;
            for hop in device.proofs() {
                prefix = prefix.push(hop.clone())?;
            }
            let leaf = read.proofs().last().unwrap().clone();
            read = prefix.push(leaf)?;
        }
        let derived = operator_chain(&read, &holder, &operator).await;
        let sibling = grant(&space, &sibling_key, "get", "archive/block", expiry()).await;
        let mut invocations = Vec::new();
        for (chain, key) in [(&read, &holder), (&read, &second), (&derived, &operator)] {
            invocations.push(
                request(
                    chain,
                    key,
                    service.did().as_str(),
                    &space.did(),
                    command("get", "archive/block"),
                    arguments("archive/block"),
                )
                .await,
            );
        }
        let target = read.proof_cids().last().unwrap();
        let revocation = if delegated {
            tonk_identity::revocation::mint_delegated_revocation_with_witness(
                other_browser.clone(),
                &read,
                target,
                &other_device,
            )
            .await?
        } else {
            tonk_identity::revocation::mint_root_revocation(browser.clone(), &read, target).await?
        };
        let verified = tonk_identity::revocation::verify(&revocation).await?;
        assert_eq!(
            verified.subject,
            if delegated {
                account.did()
            } else {
                browser.did()
            }
        );
        assert_eq!(
            verified.issuer,
            if delegated {
                other_browser.did()
            } else {
                browser.did()
            }
        );
        let state = tempfile::tempdir()?;
        let fixture = state.path().join("fixture.json");
        std::fs::write(
            &fixture,
            serde_json::to_vec(&serde_json::json!({
                "subject": space.did().to_string(), "revoker": verified.subject.to_string(), "target": target.to_string(), "targetBytes": target.to_bytes(), "invocations": invocations,
                "revocation": revocation, "sibling": request(&sibling, &sibling_key, service.did().as_str(), &space.did(), command("get", "archive/block"), arguments("archive/block")).await,
            }))?,
        )?;
        let result = std::process::Command::new("node")
            .arg(root.join("scripts/connection-worker.cjs"))
            .arg(&shim)
            .arg(fixture)
            .arg(&root)
            .arg(state.path())
            .status()?;
        anyhow::ensure!(
            result.success(),
            "production worker revocation harness failed (delegated={delegated}): {result}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn connection_member_issuer_and_same_account_device_revoke_six_leaves() -> anyhow::Result<()>
{
    let fixture = Fixture::new().await?;
    let owner = Ed25519Signer::generate().await?;
    fixture.env.provision_subject(owner.did().as_str()).await?;
    let account = Ed25519Signer::generate().await?;
    let browser = Ed25519Signer::generate().await?;
    let other_browser = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    let member = DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(&account.did())
            .subject(Subject::Specific(owner.did()))
            .command(vec!["use".into()])
            .expiration(expiry())
            .try_build()
            .await?,
    );
    let device =
        tonk_identity::delegation::mint_device_delegation(account.clone(), &browser.did()).await?;
    let other_device =
        tonk_identity::delegation::mint_device_delegation(account.clone(), &other_browser.did())
            .await?;
    let unrelated_device =
        tonk_identity::delegation::mint_device_delegation(stranger, &other_browser.did()).await?;
    let mut prefix = member;
    for hop in device.proofs() {
        prefix = prefix.push(hop.clone())?;
    }
    let scopes = tonk_invite::connection::candidate_build_scopes(&owner.did());
    let mut groups = Vec::new();
    for _ in 0..3 {
        let recipient = Ed25519Signer::generate().await?;
        let mut chains = Vec::new();
        for scope in &scopes {
            let leaf = DelegationBuilder::new()
                .issuer(Signer::from(browser.clone()))
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(expiry())
                .try_build()
                .await?;
            chains.push(prefix.push(leaf)?);
        }
        groups.push((recipient, chains));
    }
    for (_, chains) in &groups {
        for chain in chains {
            let holder = &groups
                .iter()
                .find(|(_, candidate)| candidate[0].audience() == chain.audience())
                .unwrap()
                .0;
            let cmd = chain.proofs().last().unwrap().command().0.clone();
            let resource = format!("{}/{}", cmd[2], cmd[3]);
            fixture
                .permit(
                    request(
                        chain,
                        holder,
                        &fixture.env.service_did,
                        &owner.did(),
                        cmd,
                        arguments(&resource),
                    )
                    .await,
                )
                .await?;
        }
    }
    let bad = tonk_identity::revocation::mint_delegated_revocation_with_witness(
        other_browser.clone(),
        &groups[0].1[0],
        groups[0].1[0].proof_cids().last().unwrap(),
        &unrelated_device,
    )
    .await?;
    assert!(!fixture.post(bad).await?.status().is_success());
    for (index, (_, chains)) in groups.iter().take(2).enumerate() {
        for chain in chains {
            let target = chain.proof_cids().last().unwrap();
            let artifact = if index == 0 {
                tonk_identity::revocation::mint_root_revocation(browser.clone(), chain, target)
                    .await?
            } else {
                tonk_identity::revocation::mint_delegated_revocation_with_witness(
                    other_browser.clone(),
                    chain,
                    target,
                    &other_device,
                )
                .await?
            };
            let verified = tonk_identity::revocation::verify(&artifact)
                .await
                .map_err(|error| {
                    anyhow::anyhow!(
                        "mode={index} owner={} account={} browser={} other_browser={}: {error}",
                        owner.did(),
                        account.did(),
                        browser.did(),
                        other_browser.did()
                    )
                })?;
            let response = fixture.post(artifact.clone()).await?;
            let status = response.status();
            let bytes = response.bytes().await?;
            anyhow::ensure!(
                status.is_success(),
                "{status}: {}",
                String::from_utf8_lossy(&bytes)
            );
            let receipt: tonk_account::customer::RevokeReceipt = serde_json::from_slice(&bytes)?;
            assert_eq!(receipt.revoked, *target);
            assert_eq!(receipt.subject, verified.subject);
            let replay: tonk_account::customer::RevokeReceipt =
                fixture.post(artifact).await?.json().await?;
            assert!(!replay.recorded);
        }
    }
    for (index, (holder, chains)) in groups.iter().enumerate() {
        for chain in chains {
            let cmd = chain.proofs().last().unwrap().command().0.clone();
            let resource = format!("{}/{}", cmd[2], cmd[3]);
            let response = fixture
                .post(
                    request(
                        chain,
                        holder,
                        &fixture.env.service_did,
                        &owner.did(),
                        cmd,
                        arguments(&resource),
                    )
                    .await,
                )
                .await?;
            if index < 2 {
                assert_eq!(response.status().as_u16(), 403);
            } else {
                assert!(response.status().is_success());
            }
        }
    }
    // Account device revocations concern the account consumer itself.
    fixture
        .env
        .activate_customer(&account, "connection-revocation-authority@example.test")
        .await?;
    let revoke_a = tonk_identity::revocation::mint_root_revocation(
        account.clone(),
        &device,
        &device.proof_cids()[0],
    )
    .await?;
    fixture.post(revoke_a).await?.error_for_status()?;
    // Historical invitation witness can include a revoked device: B's live prf
    // independently authenticates this ordinary withdrawal.
    let accepted = tonk_identity::revocation::mint_delegated_revocation_with_witness(
        other_browser.clone(),
        &groups[2].1[0],
        groups[2].1[0].proof_cids().last().unwrap(),
        &other_device,
    )
    .await?;
    fixture.post(accepted.clone()).await?.error_for_status()?;
    let revoke_b = tonk_identity::revocation::mint_root_revocation(
        account.clone(),
        &other_device,
        &other_device.proof_cids()[0],
    )
    .await?;
    fixture.post(revoke_b).await?.error_for_status()?;
    let stale = tonk_identity::revocation::mint_delegated_revocation_with_witness(
        other_browser.clone(),
        &groups[2].1[1],
        groups[2].1[1].proof_cids().last().unwrap(),
        &other_device,
    )
    .await?;
    let response = fixture.post(stale).await?;
    assert_eq!(
        response.status().as_u16(),
        401,
        "a revoked invocation prf cannot withdraw a new target"
    );
    // Already-recorded exact requests remain idempotent after authority withdrawal.
    let replay: tonk_account::customer::RevokeReceipt = fixture
        .post(accepted)
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(!replay.recorded);
    // An issuer's own proofless withdrawal does not depend on a historical prf.
    let own = tonk_identity::revocation::mint_root_revocation(
        browser,
        &groups[2].1[2],
        groups[2].1[2].proof_cids().last().unwrap(),
    )
    .await?;
    fixture.post(own).await?.error_for_status()?;
    Ok(())
}

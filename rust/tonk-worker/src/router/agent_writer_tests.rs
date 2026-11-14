//! The hosted agent's writer ([`tonk_mcp::SpaceWriter`], each space's
//! kernel on Wasmtime) against what a person's own worker does to the
//! grants it holds: revoking them, and withdrawing what a share link gave
//! it. Against the native access service.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use dialog_common::helpers::{Provisionable as _, Service};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal as _};
use http_body_util::BodyExt as _;
use serde_json::json;
use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};
use tonk_mcp::native::NativeKernels;
use tonk_mcp::{SpaceWriter, WriterSession};
use zeroize::Zeroizing;

const SCHEMA: &str = "attribute!: &writer-note-title
  description: Title
  the: writer.note/title
  as: text
attribute!: &writer-note-tag
  description: Tag
  the: writer.note/tag
  as: text
concept!: &writer-note
  description: A note
  with:
    title: writer-note-title
    tag: writer-note-tag
";

/// An access service serving a fresh agent space, with the space's
/// schema in place.
struct Fixture {
    service: Service<AccessServiceAddress, AccessServer>,
    space: Ed25519Signer,
    remote: url::Url,
    run: u64,
    /// What boots every writer's kernel: one Wasmtime engine.
    kernels: NativeKernels,
}

impl Fixture {
    async fn start() -> Self {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let space = Ed25519Signer::generate().await.unwrap();
        service
            .address
            .provision_subject(space.did().as_ref())
            .await
            .unwrap();
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        let fixture = Self {
            service,
            space,
            remote,
            run: rand::random(),
            kernels: NativeKernels::default(),
        };
        let session = session(&fixture.space, "schema", &fixture.remote).await;
        let schema = writer(&fixture, &session, &fixture.space.did()).await;
        let (status, body) = evaluate(&schema, SCHEMA.to_owned()).await;
        assert!(status.is_success(), "schema: {status}: {body}");
        fixture
    }

    fn note(&self, label: &str) -> String {
        format!(
            "writer-note!:\n  this: urn:writer:{}:{label}\n  title: \"{label}\"\n  tag: \"{}\"\n",
            self.run, self.run
        )
    }

    async fn stop(self) {
        self.service.stop().await.unwrap();
    }
}

/// A session of the agent space `space`: its seed, and the powerline the
/// space delegates to it.
async fn session(space: &Ed25519Signer, mcp_session: &str, remote: &url::Url) -> WriterSession {
    let seed = Zeroizing::new(rand::random::<[u8; 32]>());
    let key = Ed25519Signer::import(&*seed).await.unwrap();
    let powerline = tonk_identity::delegation::mint_addressed_device_delegation(
        space.clone(),
        &key.did(),
        remote,
    )
    .await
    .unwrap();
    WriterSession {
        seed,
        powerline,
        mcp_session: mcp_session.to_owned(),
        home: remote.clone(),
    }
}

/// The writer of `space` for `session`, handed the grants its agent space
/// holds for it, as the MCP front hands them.
async fn writer(
    fixture: &Fixture,
    session: &WriterSession,
    space: &Did,
) -> SpaceWriter<NativeKernels> {
    let agent = session.agent().clone();
    let grants = if *space == agent {
        Vec::new()
    } else {
        let own = SpaceWriter::open(
            fixture.kernels.clone(),
            session,
            &agent,
            &fixture.remote,
            &[],
        )
        .await
        .unwrap();
        own.agent_access()
            .await
            .unwrap()
            .get(space)
            .map(|access| access.grants.clone())
            .unwrap_or_default()
    };
    SpaceWriter::open(
        fixture.kernels.clone(),
        session,
        space,
        &fixture.remote,
        &grants,
    )
    .await
    .unwrap()
}

fn request(method: Method, uri: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(body.into())
        .unwrap()
}

async fn json(response: Response<Body>) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&body).into_owned()));
    (status, value)
}

async fn evaluate(
    writer: &SpaceWriter<NativeKernels>,
    document: String,
) -> (StatusCode, serde_json::Value) {
    json(
        writer
            .write(request(Method::POST, "/evaluate?transact=true", document))
            .await,
    )
    .await
}

async fn query(
    writer: &SpaceWriter<NativeKernels>,
    document: &str,
) -> (StatusCode, serde_json::Value) {
    json(
        writer
            .read(request(
                Method::POST,
                "/evaluate?transact=false",
                document.to_owned(),
            ))
            .await,
    )
    .await
}

/// The notes of `run` that `writer` reads.
async fn notes(writer: &SpaceWriter<NativeKernels>, run: u64) -> BTreeSet<String> {
    let (status, body) = query(
        writer,
        &format!("writer.note:\n  this: ?n\n  tag: \"{run}\"\n"),
    )
    .await;
    assert!(status.is_success(), "query: {status}: {body}");
    let prefix = format!("urn:writer:{run}:");
    let text = body["matches_after"].to_string();
    let mut found = BTreeSet::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find(&prefix) {
        let tail = &rest[at..];
        let end = tail.find('"').unwrap_or(tail.len());
        found.insert(tail[..end].to_owned());
        rest = &tail[end..];
    }
    found
}

fn labels(run: u64, labels: &[String]) -> BTreeSet<String> {
    labels
        .iter()
        .map(|label| format!("urn:writer:{run}:{label}"))
        .collect()
}

/// A share link a member made through their device, as Tonk makes
/// one: `space -> member -> device` (a powerline, naming no subject)
/// `-> link key`, the key's seed beside it.
async fn member_share_link(space: &Ed25519Signer, home: &url::Url) -> tonk_invite::Invite {
    use dialog_credentials::Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    let member = Ed25519Signer::generate().await.unwrap();
    let device = Ed25519Signer::generate().await.unwrap();
    let seed: [u8; 32] = rand::random();
    let key = Ed25519Signer::import(&seed).await.unwrap();
    let first = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&member.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .meta(tonk_invite::home_address_meta(home))
        .try_build()
        .await
        .unwrap();
    let second = DelegationBuilder::new()
        .issuer(Signer::from(member))
        .audience(&device.did())
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    let third = DelegationBuilder::new()
        .issuer(Signer::from(device))
        .audience(&key.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    let chain = DelegationChain::new(first)
        .push(second)
        .unwrap()
        .push(third)
        .unwrap();
    tonk_invite::Invite::new(
        chain,
        tonk_invite::InviteAudience::Open { seed },
        Some(home.clone()),
    )
    .await
    .unwrap()
}

/// A member's share link to `space`, as `tonk_join_space` reads one:
/// `space -> link key` for every command, syncing at `home`, the key's
/// seed beside it.
async fn share_link(space: &Ed25519Signer, home: &url::Url) -> tonk_invite::Invite {
    use dialog_credentials::Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    let seed: [u8; 32] = rand::random();
    let key = Ed25519Signer::import(&seed).await.unwrap();
    let root = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&key.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .meta(tonk_invite::home_address_meta(home))
        .try_build()
        .await
        .unwrap();
    tonk_invite::Invite::new(
        DelegationChain::new(root),
        tonk_invite::InviteAudience::Open { seed },
        Some(home.clone()),
    )
    .await
    .unwrap()
}

/// The browser's authority over its agent space does not extend to an
/// external space granted to that agent. Contrast its rejected revocation
/// with the hosted session's successful withdrawal of the same grant.
#[dialog_common::test]
async fn browser_cannot_revoke_link_issued_agent_grants_but_session_can() {
    use crate::router::{agent_connections::issue_to_recipient, repository};
    use dialog_capability::{
        Subject,
        access::{Access, Prove},
    };
    use dialog_credentials::Signer;
    use dialog_ucan::{Parameters, Scope, Ucan};
    use dialog_ucan_core::{command::Command, subject::Subject as UcanSubject, time::Timestamp};
    use tonk_invite::connection::{AgentAccess, agent_grant_scopes};

    let fixture = Fixture::start().await;
    let browser = crate::router::command::tests::native::test_state().await;
    let (agent, agent_session, chains) = {
        let root = Ed25519Signer::generate().await.unwrap();
        {
            let tonk = browser.read().await;
            let device_grant = tonk_identity::delegation::mint_device_delegation(
                root.clone(),
                &tonk.profile.did(),
            )
            .await
            .unwrap();
            crate::router::identity::persist_root(
                &tonk,
                tonk_worker_api::SaveRootRequest {
                    credential_id: "spike-no-passkey".into(),
                    delegation_hex: hex::encode(device_grant.to_bytes().unwrap()),
                    passkey: None,
                    encryption_key: None,
                },
            )
            .await
            .unwrap();
        }
        let seed = Zeroizing::new(rand::random::<[u8; 32]>());
        let agent_signer = Ed25519Signer::import(&*seed).await.unwrap();
        let agent = agent_signer.did();
        repository::create_space_from_seed(&browser, "Revocation spike agent", None, seed)
            .await
            .unwrap();
        fixture
            .service
            .address
            .provision_subject(agent.as_ref())
            .await
            .unwrap();
        let agent_session = session(&agent_signer, "recipient-revocation", &fixture.remote).await;

        let invite = share_link(&fixture.space, &fixture.remote).await;
        let link_signer = invite.signer().await.unwrap().unwrap();
        let scopes = agent_grant_scopes(&fixture.space.did(), AgentAccess::Read);
        let now = Timestamp::now();
        let expires = Timestamp::try_from(i128::from(now.to_unix() + 3600)).unwrap();
        let chains = issue_to_recipient(
            &agent,
            Signer::from(link_signer),
            vec![invite.chain.clone(); scopes.len()],
            &scopes,
            &fixture.remote,
            now,
            expires,
        )
        .await
        .unwrap()
        .chains()
        .to_vec();
        (agent, agent_session, chains)
    };
    // No account root, agent key, link key, or passkey is available below.
    let own = writer(&fixture, &agent_session, &agent).await;
    own.approve(
        &chains,
        json!([{ "space": fixture.space.did(), "name": "Target", "home": fixture.remote.as_str() }]),
    )
    .await
    .unwrap();

    let reader = writer(&fixture, &agent_session, &fixture.space.did()).await;
    let (status, body) = query(&reader, "concept:\n").await;
    assert!(
        status.is_success(),
        "agent read before revocation: {status}: {body}"
    );

    let (device, authority) = {
        let tonk = browser.read().await;
        assert!(
            !crate::router::join::find_replica_for_subject(&tonk, &fixture.space.did())
                .await
                .unwrap(),
            "the browser must not join the target space"
        );
        let scope = Scope {
            subject: UcanSubject::Specific(agent.clone()),
            command: Command(vec![]),
            parameters: Parameters::default(),
        };
        let proof = Subject::from(tonk.profile.did())
            .attenuate(Access)
            .invoke(Prove::<Ucan>::new(tonk.profile.did(), scope))
            .perform(&tonk.operator)
            .await
            .unwrap();
        let mut certificates = proof.proofs.into_iter().map(|proof| proof.0);
        let mut authority = DelegationChain::new(certificates.next().unwrap());
        for certificate in certificates {
            authority = authority.push(certificate).unwrap();
        }
        assert_eq!(authority.issuer(), &agent);
        assert_eq!(authority.audience(), &tonk.profile.did());
        assert_eq!(authority.proofs().count(), 2, "agent -> account -> device");
        assert_eq!(
            authority.proofs().next().unwrap().subject(),
            &UcanSubject::Specific(agent.clone()),
            "the browser controls the agent's own space, not arbitrary external capabilities"
        );
        (tonk.profile.credential().signer().clone(), authority)
    };
    for chain in &chains {
        let target = *chain.proof_cids().last().unwrap();
        let artifact = tonk_identity::revocation::mint_delegated_revocation_with_witness(
            device.clone(),
            chain,
            &target,
            &authority,
        )
        .await
        .unwrap();
        assert!(
            matches!(
                tonk_identity::revocation::verify(&artifact).await,
                Err(tonk_identity::revocation::VerifyError::Unauthorized(_))
            ),
            "the scoped agent-space grant does not witness possession of target-space authority"
        );
        let refused = crate::router::http::post_cbor(&fixture.remote, &artifact)
            .await
            .expect_err("the service must reject the browser's revocation");
        assert!(
            matches!(refused, crate::router::http::HttpError::Upstream(ref error)
            if matches!(error.status, 401 | 403) && error.message.contains("witness path")),
            "{refused}"
        );
    }
    let (status, body) = query(&reader, "concept:\n").await;
    assert!(
        status.is_success(),
        "a refused browser revocation leaves the agent able to read: {status}: {body}"
    );

    // Positive control: the hosted session holds an actual powerline from
    // the agent. This secret belongs to the hosted client, not the browser.
    let session_signer = Ed25519Signer::import(&*agent_session.seed).await.unwrap();
    for chain in &chains {
        let target = *chain.proof_cids().last().unwrap();
        let artifact = tonk_identity::revocation::mint_delegated_revocation_with_witness(
            session_signer.clone(),
            chain,
            &target,
            &agent_session.powerline,
        )
        .await
        .unwrap();
        let verified = tonk_identity::revocation::verify(&artifact).await.unwrap();
        assert_eq!(verified.issuer, session_signer.did());
        assert_eq!(verified.subject, agent);
        let response = crate::router::http::post_cbor(&fixture.remote, &artifact)
            .await
            .unwrap();
        let receipt: tonk_account::customer::RevokeReceipt =
            serde_json::from_slice(&response.body).unwrap();
        assert_eq!(receipt.revoked, target);
    }
    // Keep both the writer's cached proofs and the published grants intact.
    let (status, body) = query(&reader, "concept:\n").await;
    assert!(
        !status.is_success(),
        "same writer read after revocation: {body}"
    );
    assert!(
        body.to_string().contains("CREDENTIAL_REVOKED"),
        "{status}: {body}"
    );
    assert!(
        own.agent_access()
            .await
            .unwrap()
            .get(&fixture.space.did())
            .unwrap()
            .read,
        "published grants are still present: revocation, not deletion, blocks the read"
    );
    fixture.stop().await;
}

/// The person removes access a share link gave their agent: the agent
/// revokes, as the grants' audience, every leaf the link issued to
/// it. The agent joined straight from the link; the person never
/// joined the space. The access service then refuses the grants
/// wherever they are presented, while the hosted copy still lists
/// them: revocation, not deletion, is what refuses them.
#[dialog_common::test]
async fn the_account_withdraws_a_link_grant_as_its_agent() {
    use crate::router::{
        agent_connections::issue_to_recipient,
        agent_session::{Granted, mark, record_grants},
        agents::withdraw,
        repository,
    };
    use dialog_credentials::Signer;
    use dialog_ucan_core::time::Timestamp;
    use tonk_invite::connection::{AgentAccess, agent_grant_scopes};
    use tonk_schema::prelude::DidExt as _;

    let fixture = Fixture::start().await;
    let space = Ed25519Signer::generate().await.unwrap();
    fixture
        .service
        .address
        .provision_subject(space.did().as_ref())
        .await
        .unwrap();
    {
        let owner = session(&space, "owner", &fixture.remote).await;
        let owner = writer(&fixture, &owner, &space.did()).await;
        let (status, body) = evaluate(&owner, SCHEMA.to_owned()).await;
        assert!(status.is_success(), "schema: {status}: {body}");
        let (status, body) = evaluate(&owner, fixture.note("owner")).await;
        assert!(status.is_success(), "owner: {status}: {body}");
    }

    // A browser signed in to an account, which makes the agent the
    // way the approval page does: its key seals to the account.
    let state = crate::router::command::tests::native::test_state().await;
    let root = Ed25519Signer::generate().await.unwrap();
    {
        let tonk = state.read().await;
        let device =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .unwrap();
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "withdraw-test".into(),
                delegation_hex: hex::encode(device.to_bytes().unwrap()),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .unwrap();
        let union = tonk_account::delegations::mint_account_union(
            &Signer::from(root.clone()),
            &tonk.profile.did(),
        )
        .await
        .unwrap();
        tonk_account::peer::hand_over(&tonk.profile, &union)
            .await
            .unwrap();
    }
    let seed = Zeroizing::new(rand::random::<[u8; 32]>());
    let agent_signer = Ed25519Signer::import(&*seed).await.unwrap();
    let agent = agent_signer.did();
    let key = repository::create_space_from_seed(&state, "Claude", None, seed)
        .await
        .unwrap();
    mark(&*state.read().await, &key, &agent, Some("claude"))
        .await
        .unwrap();
    fixture
        .service
        .address
        .provision_subject(agent.as_ref())
        .await
        .unwrap();
    let agent_session = session(&agent_signer, "recipient", &fixture.remote).await;

    // The link, made through a member's device, so its chain has a
    // powerline hop. Its key grants read and write straight to the
    // agent, as the chat card's join does, and grants read again, as a second
    // join from the same link does: every leaf is live.
    let invite = member_share_link(&space, &fixture.remote).await;
    let link_key = Signer::from(invite.signer().await.unwrap().unwrap());
    let mut chains = Vec::new();
    for access in [AgentAccess::ReadWrite, AgentAccess::Read] {
        let scopes = agent_grant_scopes(&space.did(), access);
        let now = Timestamp::now();
        let expires = Timestamp::try_from(i128::from(now.to_unix() + 3_600)).unwrap();
        chains.extend(
            issue_to_recipient(
                &agent,
                link_key.clone(),
                vec![invite.chain.clone(); scopes.len()],
                &scopes,
                &fixture.remote,
                now,
                expires,
            )
            .await
            .unwrap()
            .chains()
            .to_vec(),
        );
    }
    assert!(
        chains
            .iter()
            .any(|chain| chain.ability().starts_with("/use/put"))
            && chains
                .iter()
                .any(|chain| chain.ability().starts_with("/use/get")),
        "read and write leaves"
    );
    let own = writer(&fixture, &agent_session, &agent).await;
    own.approve(
        &chains,
        json!([{ "space": space.did(), "name": "Shared", "home": fixture.remote.as_str() }]),
    )
    .await
    .unwrap();
    // The browser's copy of the agent space, as it pulls it: here the
    // directory names an address that does not answer, to check a
    // failed publication first.
    let dead = url::Url::parse("http://127.0.0.1:9/ucan/").unwrap();
    record_grants(
        &*state.read().await,
        &key,
        &[Granted {
            subject: space.did(),
            name: "Shared".into(),
            home: dead.clone(),
            chains: chains.clone(),
        }],
    )
    .await
    .unwrap();
    {
        let tonk = state.read().await;
        assert!(
            !crate::router::join::find_replica_for_subject(&tonk, &space.did())
                .await
                .unwrap(),
            "the person never joins the space"
        );
        assert_eq!(
            withdraw::link_grants(&tonk, &agent, &space.did())
                .await
                .unwrap()
                .len(),
            chains.len(),
            "every leaf is listed for removal"
        );
    }

    let reader = writer(&fixture, &agent_session, &space.did()).await;
    assert_eq!(
        notes(&reader, fixture.run).await,
        labels(fixture.run, &["owner".into()]),
        "the agent reads the space"
    );
    let (status, body) = evaluate(&reader, fixture.note("agent")).await;
    assert!(status.is_success(), "the agent writes: {status}: {body}");

    // A publication that fails leaves every grant listed to retry.
    let revoker = Signer::from(agent_signer.clone());
    let failed = withdraw::withdraw(&*state.read().await, &agent, &revoker, &space.did())
        .await
        .unwrap();
    assert!(failed.revoked.is_empty(), "{failed:?}");
    assert_eq!(failed.unresolved.len(), chains.len());
    assert!(failed.finished().is_err());
    {
        let tonk = state.read().await;
        assert_eq!(
            withdraw::link_grants(&tonk, &agent, &space.did())
                .await
                .unwrap()
                .len(),
            chains.len(),
            "nothing unacknowledged is dropped"
        );
    }
    let (status, body) = query(&reader, "concept:\n").await;
    assert!(
        status.is_success(),
        "after a refused attempt the agent still reads: {status}: {body}"
    );

    // Retry once the directory names the space's access service.
    {
        let tonk = state.read().await;
        tonk.reactor
            .repository(agent.repo_key())
            .branch(crate::router::repository::CONTENT_BRANCH)
            .transaction()
            .retract(tonk_schema::SpaceHomeAddress::new(
                &space.did(),
                dead.as_str(),
            ))
            .assert(tonk_schema::SpaceHomeAddress::new(
                &space.did(),
                fixture.remote.as_str(),
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }
    let done = withdraw::withdraw(&*state.read().await, &agent, &revoker, &space.did())
        .await
        .unwrap();
    assert_eq!(done.revoked.len(), chains.len(), "{done:?}");
    assert_eq!(done.finished().unwrap(), chains.len());
    {
        let tonk = state.read().await;
        assert!(
            withdraw::link_grants(&tonk, &agent, &space.did())
                .await
                .unwrap()
                .is_empty(),
            "acknowledged grants are dropped from the agent space"
        );
        assert!(
            tonk.sync_queue.is_dirty(agent.repo_key()),
            "the agent space is queued to sync"
        );
    }

    // The writer already open, with its proofs in hand, is refused.
    let (status, body) = query(&reader, "concept:\n").await;
    assert!(!status.is_success(), "same writer reads after: {body}");
    assert!(
        body.to_string().contains("CREDENTIAL_REVOKED"),
        "{status}: {body}"
    );
    let (status, body) = evaluate(&reader, fixture.note("late")).await;
    assert!(!status.is_success(), "same writer writes after: {body}");
    // The hosted copy still holds every grant: deletion is not why.
    let held = own.agent_access().await.unwrap();
    let held = held.get(&space.did()).expect("published grants remain");
    assert!(held.read && held.write, "{held:?}");
    // A fresh session of the agent, presenting the same grants.
    let fresh = session(&agent_signer, "fresh", &fixture.remote).await;
    let fresh = writer(&fixture, &fresh, &space.did()).await;
    let (status, body) = query(&fresh, "concept:\n").await;
    assert!(!status.is_success(), "a fresh reader is refused: {body}");
    assert!(
        body.to_string().contains("CREDENTIAL_REVOKED"),
        "{status}: {body}"
    );
    fixture.stop().await;
}

/// A grant the person's account issued to the agent, under authority
/// the account holds over the space (here, a link it claimed): the
/// agent reads with it and the access service refuses its write.
/// When the account revokes the grant at the access service, the
/// agent's next read is refused too. No agent key is needed for this.
#[dialog_common::test]
async fn the_account_revokes_an_agent_it_let_in() {
    use crate::router::agent_connections::issue_to_recipient;
    use dialog_credentials::Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::time::Timestamp;
    use tonk_invite::connection::{AgentAccess, agent_grant_scopes};

    let fixture = Fixture::start().await;
    let space = Ed25519Signer::generate().await.unwrap();
    fixture
        .service
        .address
        .provision_subject(space.did().as_ref())
        .await
        .unwrap();
    let owner = session(&space, "owner", &fixture.remote).await;
    let owner = writer(&fixture, &owner, &space.did()).await;
    let (status, body) = evaluate(&owner, SCHEMA.to_owned()).await;
    assert!(status.is_success(), "schema: {status}: {body}");
    let (status, body) = evaluate(&owner, fixture.note("owner")).await;
    assert!(status.is_success(), "owner: {status}: {body}");

    // The account claims the link, as a person's own join does.
    let account = Ed25519Signer::generate().await.unwrap();
    let link = share_link(&space, &fixture.remote).await;
    let link_key = link.signer().await.unwrap().unwrap();
    let claim = DelegationBuilder::new()
        .issuer(Signer::from(link_key))
        .audience(&account.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    let held = link.chain.clone().push(claim).unwrap();

    // The account lets the agent read.
    let agent = fixture.space.did();
    let scopes = agent_grant_scopes(&space.did(), AgentAccess::Read);
    let now = Timestamp::now();
    let expires = Timestamp::try_from(i128::from(now.to_unix() + 3_600)).unwrap();
    let chains = issue_to_recipient(
        &agent,
        Signer::from(account.clone()),
        vec![held.clone(); scopes.len()],
        &scopes,
        &fixture.remote,
        now,
        expires,
    )
    .await
    .unwrap()
    .chains()
    .to_vec();
    let session = session(&fixture.space, "mcp-join", &fixture.remote).await;
    let own = writer(&fixture, &session, &fixture.space.did()).await;
    own.approve(
        &chains,
        json!([{ "space": space.did(), "name": "Shared", "home": fixture.remote.as_str() }]),
    )
    .await
    .unwrap();
    let entry = own.agent_access().await.unwrap();
    let entry = entry.get(&space.did()).expect("the grant").clone();
    assert!(entry.read && !entry.write, "{entry:?}");

    let reader = writer(&fixture, &session, &space.did()).await;
    assert_eq!(
        notes(&reader, fixture.run).await,
        labels(fixture.run, &["owner".into()]),
        "the agent reads the space"
    );
    let (status, body) = evaluate(&reader, fixture.note("agent")).await;
    assert!(!status.is_success(), "a read grant cannot write: {body}");

    // The account withdraws each leaf it signed, witnessed by its own
    // path to the space, at the space's access service.
    for chain in &chains {
        let target = *chain.proof_cids().last().unwrap();
        let artifact =
            tonk_identity::revocation::mint_root_revocation(account.clone(), chain, &target)
                .await
                .unwrap();
        let response = crate::router::http::post_cbor(&fixture.remote, &artifact)
            .await
            .unwrap();
        let receipt: tonk_account::customer::RevokeReceipt =
            serde_json::from_slice(&response.body).unwrap();
        assert_eq!(receipt.revoked, target);
    }

    let reader = writer(&fixture, &session, &space.did()).await;
    let (status, body) = query(
        &reader,
        &format!("writer.note:\n  this: ?n\n  tag: \"{}\"\n", fixture.run),
    )
    .await;
    assert!(
        !status.is_success(),
        "a revoked agent reads nothing: {body}"
    );
    assert!(
        body.to_string().contains("CREDENTIAL_REVOKED"),
        "refused for the revocation: {body}"
    );
    fixture.stop().await;
}

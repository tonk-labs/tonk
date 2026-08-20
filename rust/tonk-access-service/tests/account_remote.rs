#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::convert::Infallible;

use bytes::Bytes;
use dialog_artifacts::Entity;
use dialog_capability::Subject;
use dialog_common::helpers::Provisionable as _;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_operator::Operator;
use dialog_operator::helpers::{test_operator_with_profile, unique_name};
use dialog_query::{Attribute, Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Branch, RemoteBranch, Repository, RepositoryExt as _};
use dialog_storage::provider::storage::VolatileSpace;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_account::{CreateGenesis, RemotePresence, probe_remote_main, publish_genesis_if_absent};

#[derive(Attribute, Clone)]
#[domain("account-test")]
struct Note(String);

/// One device's view of the account repository: a verifier-only local replica
/// keyed on the root DID, tracking `origin` at `endpoint`. This is the shape
/// the worker and CLI adapters build in their `mount`/`hydrate` paths.
struct AccountDevice {
    operator: Operator<VolatileSpace>,
    branch: Branch,
    remote: RemoteBranch,
}

async fn account_device(root: &Ed25519Signer, endpoint: &str) -> anyhow::Result<AccountDevice> {
    let root_did = root.did();
    let (operator, profile) = test_operator_with_profile().await;
    let link = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root.clone()))
        .audience(&profile.did())
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await?;
    profile
        .access()
        .save(UcanDelegation(DelegationChain::new(link)))
        .perform(&operator)
        .await?;

    let verifier: Ed25519Verifier = root_did.to_string().parse()?;
    let local = Subject::from(profile.did()).attenuate(Space::new(root_did.to_string()));
    let repository = Repository::from(
        local
            .create(Credential::from(verifier))
            .perform(&operator)
            .await?,
    );
    let branch = repository.branch("main").open().perform(&operator).await?;
    let origin = repository
        .remote("origin")
        .create(UcanAddress::new(endpoint))
        .subject(root_did)
        .perform(&operator)
        .await?;
    let remote = origin.branch("main").open().perform(&operator).await?;
    branch
        .set_upstream(remote.clone())
        .perform(&operator)
        .await?;
    Ok(AccountDevice {
        operator,
        branch,
        remote,
    })
}

async fn static_access_endpoint(status: StatusCode, body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let service =
                    hyper::service::service_fn(move |_request: Request<Incoming>| async move {
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(status)
                                .body(Full::new(Bytes::from_static(body)))
                                .unwrap(),
                        )
                    });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    endpoint
}

#[dialog_common::test]
async fn it_reports_a_never_published_authorized_branch_as_absent() -> anyhow::Result<()> {
    let service = AccessServiceAddress::start(Default::default()).await?;
    let env = service.address.clone();
    let (operator, profile) = test_operator_with_profile().await;
    let repository = profile
        .repository(unique_name("account-absence"))
        .create()
        .perform(&operator)
        .await?;
    let ownership = repository
        .access()
        .claim(&repository)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    // The gate serves a subject only while an active customer pays for
    // it; this test is about absence probing, not registration.
    env.provision_subject(repository.did().as_str()).await?;

    let origin = repository
        .remote("origin")
        .create(UcanAddress::new(&env.access_service_url))
        .perform(&operator)
        .await?;
    let remote = origin.branch("main").open().perform(&operator).await?;

    assert_eq!(
        probe_remote_main(&remote, &operator).await?,
        RemotePresence::Absent
    );
    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
async fn it_never_classifies_remote_failures_as_absence() -> anyhow::Result<()> {
    let service = AccessServiceAddress::start(Default::default()).await?;
    let env = service.address.clone();
    let (operator, profile) = test_operator_with_profile().await;
    let repository = profile
        .repository(unique_name("account-errors"))
        .create()
        .perform(&operator)
        .await?;
    let ownership = repository
        .access()
        .claim(&repository)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    env.provision_subject(repository.did().as_str()).await?;

    let forbidden = static_access_endpoint(StatusCode::FORBIDDEN, b"forbidden").await;
    let unauthorized = static_access_endpoint(StatusCode::UNAUTHORIZED, b"unauthorized").await;
    let unavailable = static_access_endpoint(StatusCode::SERVICE_UNAVAILABLE, b"offline").await;
    let malformed = static_access_endpoint(StatusCode::OK, b"not dag-cbor").await;
    let endpoints = [
        forbidden.as_str(),
        unauthorized.as_str(),
        unavailable.as_str(),
        malformed.as_str(),
        "http://127.0.0.1:9",
    ];

    for (index, endpoint) in endpoints.into_iter().enumerate() {
        let origin = repository
            .remote(format!("failure-{index}"))
            .create(UcanAddress::new(endpoint))
            .perform(&operator)
            .await?;
        let remote = origin.branch("main").open().perform(&operator).await?;
        assert!(
            probe_remote_main(&remote, &operator).await.is_err(),
            "{endpoint} must be an error, never confirmed absence"
        );
    }

    let healthy = repository
        .remote("healthy-control")
        .create(UcanAddress::new(&env.access_service_url))
        .perform(&operator)
        .await?;
    let healthy = healthy.branch("main").open().perform(&operator).await?;
    assert_eq!(
        probe_remote_main(&healthy, &operator).await?,
        RemotePresence::Absent
    );

    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
async fn it_atomically_publishes_one_account_genesis_and_keeps_syncing() -> anyhow::Result<()> {
    let service = AccessServiceAddress::start(Default::default()).await?;
    let endpoint = service.address.access_service_url.clone();
    let root = Ed25519Signer::generate().await?;
    let root_did = root.did();
    // The account space is servable only once its customer confirms the
    // emailed activation link.
    service
        .address
        .activate_customer(&root, "genesis@example.com")
        .await?;

    let device_a = account_device(&root, &endpoint).await?;
    let device_b = account_device(&root, &endpoint).await?;
    let (operator_a, branch_a, remote_a) = (device_a.operator, device_a.branch, device_a.remote);
    let (operator_b, branch_b, remote_b) = (device_b.operator, device_b.branch, device_b.remote);

    assert_eq!(
        probe_remote_main(&remote_a, &operator_a).await?,
        RemotePresence::Absent
    );
    assert_eq!(
        probe_remote_main(&remote_b, &operator_b).await?,
        RemotePresence::Absent
    );

    let genesis_a = branch_a.transaction().commit().perform(&operator_a).await?;
    let genesis_b = branch_b.transaction().commit().perform(&operator_b).await?;
    // The head carries an opaque branch entity committing to
    // (profile, subject, name) rather than the subject DID itself.
    assert_eq!(
        genesis_a.branch,
        dialog_repository::branch_of(&root_did, &operator_a.profile_did(), "main")
    );
    assert_eq!(
        genesis_b.branch,
        dialog_repository::branch_of(&root_did, &operator_b.profile_did(), "main")
    );
    assert_ne!(genesis_a, genesis_b, "the race must use distinct revisions");

    let (result_a, result_b) = tokio::join!(
        publish_genesis_if_absent(&branch_a, &remote_a, &operator_a),
        publish_genesis_if_absent(&branch_b, &remote_b, &operator_b),
    );
    let result_a = result_a?;
    let result_b = result_b?;

    let (winner, loser_branch, loser_operator, winner_branch, winner_operator, winner_remote) =
        match (&result_a, &result_b) {
            (CreateGenesis::Winner(winner), CreateGenesis::Loser(observed)) => {
                assert_eq!(observed, winner);
                (
                    winner.clone(),
                    &branch_b,
                    &operator_b,
                    &branch_a,
                    &operator_a,
                    &remote_a,
                )
            }
            (CreateGenesis::Loser(observed), CreateGenesis::Winner(winner)) => {
                assert_eq!(observed, winner);
                (
                    winner.clone(),
                    &branch_a,
                    &operator_a,
                    &branch_b,
                    &operator_b,
                    &remote_b,
                )
            }
            other => panic!("expected exactly one genesis winner, got {other:?}"),
        };

    // Adopt the winner by pulling — the pull integrates the established
    // head and records it as the loser's sync base, so its next push
    // fast-forwards.
    assert!(loser_branch.pull().perform(loser_operator).await?.is_some());
    assert_eq!(
        publish_genesis_if_absent(winner_branch, winner_remote, winner_operator).await?,
        CreateGenesis::Loser(winner.clone()),
        "a retry must never replace the established revision"
    );

    let note_one: Entity = "account-test:one".parse()?;
    loser_branch
        .transaction()
        .assert(Note::of(note_one).is("loser write".to_string()))
        .commit()
        .perform(loser_operator)
        .await?;
    assert!(loser_branch.push().perform(loser_operator).await?.is_some());
    assert!(
        winner_branch
            .pull()
            .perform(winner_operator)
            .await?
            .is_some()
    );

    let note_two: Entity = "account-test:two".parse()?;
    winner_branch
        .transaction()
        .assert(Note::of(note_two).is("winner write".to_string()))
        .commit()
        .perform(winner_operator)
        .await?;
    assert!(
        winner_branch
            .push()
            .perform(winner_operator)
            .await?
            .is_some()
    );
    assert!(loser_branch.pull().perform(loser_operator).await?.is_some());
    assert_eq!(winner_branch.revision(), loser_branch.revision());

    service.stop().await?;
    Ok(())
}

/// A losing candidate adopts a winner that has already written past genesis.
///
/// [`probe_remote_main`] resolves the remote revision cell and caches its
/// edition; it downloads no blocks. So the revision handed back to a loser can
/// name content this device has never seen — the winning device establishes
/// genesis and then seeds an initial display name before the loser's publish
/// lands. Adopting it is still correct with no fetch in between, because fact
/// reads resolve missing blocks through the configured remote. This pins that:
/// the worker and CLI `hydrate` paths reset straight onto the winner and then
/// write the trusted-base marker, so a regression here would mark a base the
/// device cannot read.
#[dialog_common::test]
async fn it_adopts_a_losing_candidate_onto_the_winners_content() -> anyhow::Result<()> {
    let service = AccessServiceAddress::start(Default::default()).await?;
    let endpoint = service.address.access_service_url.clone();
    let root = Ed25519Signer::generate().await?;
    service
        .address
        .activate_customer(&root, "adopt@example.com")
        .await?;

    let winner = account_device(&root, &endpoint).await?;
    let loser = account_device(&root, &endpoint).await?;

    // The winning device establishes genesis and immediately writes past it,
    // exactly as an account-creating device seeds its initial display name.
    let genesis = winner
        .branch
        .transaction()
        .commit()
        .perform(&winner.operator)
        .await?;
    assert!(matches!(
        publish_genesis_if_absent(&winner.branch, &winner.remote, &winner.operator).await?,
        CreateGenesis::Winner(_)
    ));
    let note: Entity = "account-test:seeded".parse()?;
    winner
        .branch
        .transaction()
        .assert(Note::of(note).is("initial name".to_string()))
        .commit()
        .perform(&winner.operator)
        .await?;
    assert!(
        winner
            .branch
            .push()
            .perform(&winner.operator)
            .await?
            .is_some()
    );

    // The losing device built its own genesis before any of that landed, so
    // the revision it is handed is the winner's *content* revision, not a
    // matching empty tree.
    let local_genesis = loser
        .branch
        .transaction()
        .commit()
        .perform(&loser.operator)
        .await?;
    let CreateGenesis::Loser(adopted) =
        publish_genesis_if_absent(&loser.branch, &loser.remote, &loser.operator).await?
    else {
        anyhow::bail!("the second candidate must lose an established remote");
    };
    assert_ne!(
        adopted, genesis,
        "the winner wrote past genesis, so the adopted revision carries content"
    );
    assert_ne!(adopted, local_genesis);

    // Adopt as `hydrate` does: pull, integrating the winner's head and
    // recording it as the sync base, and read through.
    loser.branch.pull().perform(&loser.operator).await?;

    let seeded: Vec<_> = loser
        .branch
        .query()
        .select(Query::<Note> {
            of: Term::var("of"),
            is: Term::var("is"),
        })
        .perform(&loser.operator)
        .try_vec()
        .await?;
    assert_eq!(
        seeded.len(),
        1,
        "a device that adopted the winner must be able to read its content"
    );

    service.stop().await?;
    Ok(())
}

#[dialog_common::test]
#[ignore = "requires TONK_ACCOUNT_REMOTE_URL and a staging-authorized account"]
async fn it_proves_account_genesis_against_the_configured_live_remote() -> anyhow::Result<()> {
    let endpoint = std::env::var("TONK_ACCOUNT_REMOTE_URL")?;
    let (operator, profile) = test_operator_with_profile().await;
    let repository = profile
        .repository(unique_name("account-live"))
        .create()
        .perform(&operator)
        .await?;
    let ownership = repository
        .access()
        .claim(&repository)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    let branch = repository.branch("main").open().perform(&operator).await?;
    let origin = repository
        .remote("origin")
        .create(UcanAddress::new(endpoint))
        .perform(&operator)
        .await?;
    let remote = origin.branch("main").open().perform(&operator).await?;

    assert_eq!(
        probe_remote_main(&remote, &operator).await?,
        RemotePresence::Absent
    );
    let genesis = branch.transaction().commit().perform(&operator).await?;
    assert_eq!(
        publish_genesis_if_absent(&branch, &remote, &operator).await?,
        CreateGenesis::Winner(genesis.clone())
    );
    assert_eq!(
        publish_genesis_if_absent(&branch, &remote, &operator).await?,
        CreateGenesis::Loser(remote.fetch().perform(&operator).await?.unwrap())
    );
    Ok(())
}

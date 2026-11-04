//! A service behind a shaped link: a client that syncs through the link
//! reaches the service only through it, so a harness that shapes the link
//! shapes all of it. A browser's block reads and writes go to the
//! `/object/` address the service's permits name, which
//! `public_endpoint` points at the link as well.
//!
//! Native only: the link is a loopback TCP relay a harness runs beside
//! the service, which a browser cannot host. A browser reaches the same
//! service through `tonk-perf link` in `bench/trace`.

#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use dialog_artifacts::{ArtifactSelector, Entity};
use dialog_peer::helpers::{test_session_with_peer, unique_name};
use dialog_query::Attribute;
use dialog_remote_ucan::UcanAddress;
use dialog_repository::RepositoryExt as _;
use futures_util::StreamExt as _;
use tonk_access_service::helpers::{AccessServiceSettings, access_service};
use tonk_perf::link::{Link, Options, Shape};

#[derive(Attribute, Clone)]
#[domain("user")]
struct Name(String);

/// A port free a moment ago, for an address that must be known before
/// what listens on it can start.
fn free_port() -> anyhow::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_and_pull_behind_a_link_cross_only_the_link() -> anyhow::Result<()> {
    let port = free_port()?;
    let service = access_service(AccessServiceSettings {
        public_endpoint: Some(format!("http://127.0.0.1:{port}")),
        ..Default::default()
    })
    .await?;
    let link = Link::start_with(
        Options {
            name: "ucan".into(),
            listen: port,
            upstream: service.address.access_service_url.clone(),
            shape: Shape::profile("broadband").expect("a profile"),
        },
        None,
    )
    .await?;

    let (operator, profile) = test_session_with_peer().await;
    let repo = profile
        .space(unique_name("behind-link"))
        .create()
        .perform(&operator)
        .await?;
    let ownership = repo
        .access()
        .claim(&repo)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    service
        .address
        .provision_subject(repo.did().as_str())
        .await?;

    let origin = tonk_account::peer::connect(
        dialog_repository::SiteAddress::from(UcanAddress::new(link.url())),
        repo.did(),
        &operator,
    )
    .await?;
    let branch = repo.branch("main").open().perform(&operator).await?;
    let upstream = origin.branch("main").open().perform(&operator).await?;
    branch.set_upstream(upstream).perform(&operator).await?;
    let alice: Entity = "user:alice".parse()?;
    branch
        .transaction()
        .assert(Name::of(alice).is("Alice".to_string()))
        .commit()
        .publish()
        .perform(&operator)
        .await?;
    assert!(branch.push().perform(&operator).await?.is_some());
    assert!(branch.pull().perform(&operator).await?.is_none());
    let found = branch
        .claims()
        .select(ArtifactSelector::new().the("user/name".parse()?))
        .perform(&operator)
        .await?
        .collect::<Vec<_>>()
        .await;
    assert_eq!(found.len(), 1);

    // Closing the connections ends the exchanges still open on them.
    link.reset();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let stats = link.stats();
    let redeems = stats.exchanges.get("ucan").copied().unwrap_or(0);
    // The push and pull each publish or read the branch's cell and its
    // blocks: several redeems, and the service's identity read beside them.
    assert!(redeems >= 4, "redeems crossed the link: {stats:?}");
    assert!(
        stats.exchanges.contains_key("service"),
        "the service's identity was read through the link too: {stats:?}"
    );
    Ok(())
}
